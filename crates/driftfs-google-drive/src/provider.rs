use crate::client::{DriveClientConfig, GoogleDriveHttpClient};
use crate::mapping::{drive_file_to_metadata, DriveFile, DriveFileList, GOOGLE_DRIVE_FOLDER_MIME};
use driftfs_auth::TokenProvider;
use driftfs_core::{AccountId, ByteRange, DriftFsError, FileId, Result};
use driftfs_provider::{
    Change, ChangePage, CloudProvider, ObjectMetadata, ReadOutput, SessionCreatedHook,
};
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use serde::Deserialize;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

const FILE_FIELDS: &str = "id,name,mimeType,size,createdTime,modifiedTime,parents,version,trashed";

pub struct GoogleDriveProvider {
    client: GoogleDriveHttpClient,
}

impl GoogleDriveProvider {
    pub fn new(token_provider: Arc<dyn TokenProvider>) -> Self {
        Self::with_config(token_provider, DriveClientConfig::default())
    }

    pub fn with_config(token_provider: Arc<dyn TokenProvider>, config: DriveClientConfig) -> Self {
        Self {
            client: GoogleDriveHttpClient::new(token_provider, config),
        }
    }

    pub async fn about(&self) -> Result<crate::mapping::DriveAbout> {
        self.client
            .get_json(|http, base_url, token| {
                let url = format!("{base_url}/about?fields=user,storageQuota");
                http.get(&url).bearer_auth(token)
            })
            .await
    }

    /// Chunked resumable upload for files >5MB.
    /// Initiates a session or resumes an existing one, then streams 5MB chunks with Content-Range headers.
    async fn resumable_upload(
        &self,
        id: &FileId,
        path: &Path,
        total_size: u64,
        existing_session_uri: Option<&str>,
        on_session_created: Option<SessionCreatedHook>,
    ) -> Result<ObjectMetadata> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};

        const CHUNK_SIZE: u64 = 5 * 1024 * 1024;
        let file_id = id.0.clone();

        let mut session_uri: Option<String> = None;
        let mut start_offset: u64 = 0;

        if let Some(uri) = existing_session_uri {
            let status_uri = uri.to_string();
            let check_resp = self
                .client
                .execute_request(move |http, _base_url, token| {
                    http.put(&status_uri)
                        .bearer_auth(token)
                        .header(CONTENT_RANGE.as_str(), format!("bytes */{total_size}"))
                        .header(CONTENT_LENGTH.as_str(), "0")
                })
                .await;

            if let Ok(resp) = check_resp {
                if resp.status().is_success() {
                    let file: DriveFile =
                        resp.json().await.map_err(|e| DriftFsError::Serialization {
                            message: format!("failed to parse completed resumable upload: {e}"),
                            source: Some(Box::new(e)),
                        })?;
                    return Ok(drive_file_to_metadata(file));
                } else if resp.status().as_u16() == 308 {
                    if let Some(range_hdr) = resp.headers().get(RANGE).and_then(|h| h.to_str().ok())
                    {
                        if let Some(dash_idx) = range_hdr.rfind('-') {
                            if let Ok(last_byte) = range_hdr[dash_idx + 1..].trim().parse::<u64>() {
                                start_offset = last_byte + 1;
                                session_uri = Some(uri.to_string());
                                tracing::info!(
                                    file_id = %id,
                                    resumed_from_byte = start_offset,
                                    "resuming interrupted upload session"
                                );
                            }
                        }
                    } else {
                        session_uri = Some(uri.to_string());
                    }
                }
            }
        }

        let session_uri = match session_uri {
            Some(uri) => uri,
            None => {
                let init_resp = self
                    .client
                    .execute_request(move |http, _base_url, token| {
                        let url = format!(
                            "https://www.googleapis.com/upload/drive/v3/files/{file_id}?uploadType=resumable&supportsAllDrives=true"
                        );
                        http.patch(&url)
                            .bearer_auth(token)
                            .header(CONTENT_TYPE.as_str(), "application/json; charset=UTF-8")
                            .header(CONTENT_LENGTH.as_str(), "0")
                            .body("")
                    })
                    .await?;

                let uri = init_resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string())
                    .ok_or_else(|| DriftFsError::Provider {
                        message: "resumable upload: missing session URI in response".into(),
                        source: None,
                    })?;

                if let Some(ref cb) = on_session_created {
                    cb(&uri);
                }
                start_offset = 0;
                uri
            }
        };

        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|e| DriftFsError::Filesystem {
                message: format!("failed to open staging file for upload: {e}"),
                source: Some(Box::new(e)),
            })?;

        if start_offset > 0 {
            file.seek(SeekFrom::Start(start_offset))
                .await
                .map_err(|e| DriftFsError::Filesystem {
                    message: format!("failed to seek staging file to offset {start_offset}: {e}"),
                    source: Some(Box::new(e)),
                })?;
        }

        let mut offset = start_offset;
        let mut last_response: Option<reqwest::Response> = None;

        while offset < total_size {
            let remaining = total_size - offset;
            let chunk_len = remaining.min(CHUNK_SIZE) as usize;
            let mut buf = vec![0u8; chunk_len];
            file.read_exact(&mut buf)
                .await
                .map_err(|e| DriftFsError::Filesystem {
                    message: format!("failed to read chunk at offset {offset}: {e}"),
                    source: Some(Box::new(e)),
                })?;

            let end_inclusive = offset + chunk_len as u64 - 1;
            let range_val = format!("bytes {offset}-{end_inclusive}/{total_size}");
            let uri = session_uri.clone();
            let chunk_data = buf;

            let resp = self
                .client
                .execute_request(move |http, _base_url, token| {
                    http.put(&uri)
                        .bearer_auth(token)
                        .header(CONTENT_RANGE.as_str(), &range_val)
                        .header(CONTENT_LENGTH.as_str(), chunk_data.len().to_string())
                        .body(chunk_data.clone())
                })
                .await?;

            offset += chunk_len as u64;
            last_response = Some(resp);
        }

        let resp = last_response.ok_or_else(|| DriftFsError::Provider {
            message: "resumable upload: no response from final chunk".into(),
            source: None,
        })?;

        let file: DriveFile = resp.json().await.map_err(|e| DriftFsError::Serialization {
            message: format!("failed to parse resumable upload response: {e}"),
            source: Some(Box::new(e)),
        })?;

        Ok(drive_file_to_metadata(file))
    }
}

impl CloudProvider for GoogleDriveProvider {
    async fn get_metadata(&self, id: &FileId) -> Result<ObjectMetadata> {
        let file_id = id.0.clone();
        let file: DriveFile = self
            .client
            .get_json(move |http, base_url, token| {
                let url = format!(
                    "{base_url}/files/{file_id}?fields={FILE_FIELDS}&supportsAllDrives=true"
                );
                http.get(&url).bearer_auth(token)
            })
            .await?;

        Ok(drive_file_to_metadata(file))
    }

    async fn list_children(&self, parent_id: &FileId) -> Result<Vec<ObjectMetadata>> {
        let parent = parent_id.0.clone();
        let mut results = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let parent_query = parent.clone();
            let current_token = page_token.clone();

            let file_list: DriveFileList = self
                .client
                .get_json(move |http, base_url, token| {
                    let mut url = format!(
                        "{base_url}/files?q='{}'+in+parents+and+trashed=false&fields=files({FILE_FIELDS}),nextPageToken&pageSize=1000&supportsAllDrives=true",
                        parent_query
                    );
                    if let Some(t) = &current_token {
                        url.push_str(&format!("&pageToken={t}"));
                    }
                    http.get(&url).bearer_auth(token)
                })
                .await?;

            if let Some(files) = file_list.files {
                for file in files {
                    results.push(drive_file_to_metadata(file));
                }
            }

            match file_list.next_page_token {
                Some(next) if !next.is_empty() => page_token = Some(next),
                _ => break,
            }
        }

        Ok(results)
    }

    async fn read_range(&self, id: &FileId, range: ByteRange) -> Result<ReadOutput> {
        let file_id = id.0.clone();
        let end_inclusive = range.end_inclusive();
        let range_header_val = format!("bytes={}-{}", range.offset, end_inclusive);

        let resp = self
            .client
            .execute_request(move |http, base_url, token| {
                let url = format!("{base_url}/files/{file_id}?alt=media&supportsAllDrives=true");
                http.get(&url)
                    .bearer_auth(token)
                    .header(RANGE, &range_header_val)
            })
            .await?;

        let bytes = resp.bytes().await.map_err(|e| DriftFsError::Network {
            message: format!("failed to read byte range stream: {e}"),
            source: Some(Box::new(e)),
        })?;

        Ok(ReadOutput {
            data: bytes.to_vec(),
            range,
        })
    }

    async fn create_file(&self, parent_id: &FileId, name: &str) -> Result<ObjectMetadata> {
        let parent = parent_id.0.clone();
        let file_name = name.to_string();

        let body = json!({
            "name": file_name,
            "parents": [parent],
        });

        let file: DriveFile = self
            .client
            .get_json(move |http, base_url, token| {
                let url = format!("{base_url}/files?fields={FILE_FIELDS}&supportsAllDrives=true");
                http.post(&url).bearer_auth(token).json(&body)
            })
            .await?;

        Ok(drive_file_to_metadata(file))
    }

    async fn upload(&self, id: &FileId, data: &[u8]) -> Result<ObjectMetadata> {
        let file_id = id.0.clone();
        let data_vec = data.to_vec();

        let resp = self
            .client
            .execute_request(move |http, _base_url, token| {
                let upload_url = format!(
                    "https://www.googleapis.com/upload/drive/v3/files/{file_id}?uploadType=media&supportsAllDrives=true"
                );
                http.patch(&upload_url)
                    .bearer_auth(token)
                    .header("Content-Type", "application/octet-stream")
                    .body(data_vec.clone())
            })
            .await?;

        let file: DriveFile = resp.json().await.map_err(|e| DriftFsError::Serialization {
            message: format!("failed to parse upload response: {e}"),
            source: Some(Box::new(e)),
        })?;

        Ok(drive_file_to_metadata(file))
    }

    async fn create_directory(&self, parent_id: &FileId, name: &str) -> Result<ObjectMetadata> {
        let parent = parent_id.0.clone();
        let dir_name = name.to_string();

        let body = json!({
            "name": dir_name,
            "parents": [parent],
            "mimeType": GOOGLE_DRIVE_FOLDER_MIME,
        });

        let file: DriveFile = self
            .client
            .get_json(move |http, base_url, token| {
                let url = format!("{base_url}/files?fields={FILE_FIELDS}&supportsAllDrives=true");
                http.post(&url).bearer_auth(token).json(&body)
            })
            .await?;

        Ok(drive_file_to_metadata(file))
    }

    async fn rename(&self, id: &FileId, new_name: &str) -> Result<ObjectMetadata> {
        let file_id = id.0.clone();
        let name_str = new_name.to_string();
        let body = json!({ "name": name_str });

        let file: DriveFile = self
            .client
            .get_json(move |http, base_url, token| {
                let url = format!(
                    "{base_url}/files/{file_id}?fields={FILE_FIELDS}&supportsAllDrives=true"
                );
                http.patch(&url).bearer_auth(token).json(&body)
            })
            .await?;

        Ok(drive_file_to_metadata(file))
    }

    async fn move_object(&self, id: &FileId, new_parent_id: &FileId) -> Result<ObjectMetadata> {
        let current = self.get_metadata(id).await?;
        let old_parent = current
            .parent_id
            .map(|p| p.0)
            .unwrap_or_else(|| "root".into());

        let file_id = id.0.clone();
        let new_parent = new_parent_id.0.clone();

        let file: DriveFile = self
            .client
            .get_json(move |http, base_url, token| {
                let url = format!(
                    "{base_url}/files/{file_id}?addParents={new_parent}&removeParents={old_parent}&fields={FILE_FIELDS}&supportsAllDrives=true"
                );
                http.patch(&url).bearer_auth(token)
            })
            .await?;

        Ok(drive_file_to_metadata(file))
    }

    async fn delete(&self, id: &FileId) -> Result<()> {
        let file_id = id.0.clone();
        self.client
            .execute_request(move |http, base_url, token| {
                let url = format!("{base_url}/files/{file_id}?supportsAllDrives=true");
                http.delete(&url).bearer_auth(token)
            })
            .await?;

        Ok(())
    }

    async fn upload_file(&self, id: &FileId, path: &Path) -> Result<ObjectMetadata> {
        self.upload_file_resumable(id, path, None, None).await
    }

    async fn upload_file_resumable(
        &self,
        id: &FileId,
        path: &Path,
        existing_session_uri: Option<&str>,
        on_session_created: Option<SessionCreatedHook>,
    ) -> Result<ObjectMetadata> {
        let file_size = tokio::fs::metadata(path)
            .await
            .map_err(|e| DriftFsError::Filesystem {
                message: format!("failed to stat staging file: {e}"),
                source: Some(Box::new(e)),
            })?
            .len();

        const SIMPLE_UPLOAD_LIMIT: u64 = 5 * 1024 * 1024;

        if file_size <= SIMPLE_UPLOAD_LIMIT {
            let data = tokio::fs::read(path)
                .await
                .map_err(|e| DriftFsError::Filesystem {
                    message: format!("failed to read staging file: {e}"),
                    source: Some(Box::new(e)),
                })?;
            return self.upload(id, &data).await;
        }

        self.resumable_upload(
            id,
            path,
            file_size,
            existing_session_uri,
            on_session_created,
        )
        .await
    }

    async fn trash(&self, id: &FileId) -> Result<()> {
        let file_id = id.0.clone();
        let body = json!({ "trashed": true });

        self.client
            .execute_request(move |http, base_url, token| {
                let url = format!("{base_url}/files/{file_id}?supportsAllDrives=true");
                http.patch(&url).bearer_auth(token).json(&body)
            })
            .await?;

        Ok(())
    }

    async fn fetch_changes(&self, checkpoint: Option<&str>) -> Result<ChangePage> {
        let page_token = match checkpoint {
            Some(cp) => cp.to_string(),
            None => {
                #[derive(Deserialize)]
                struct StartPageTokenResp {
                    #[serde(rename = "startPageToken")]
                    start_page_token: String,
                }

                let resp: StartPageTokenResp = self
                    .client
                    .get_json(|http, base_url, token| {
                        let url =
                            format!("{base_url}/changes/startPageToken?supportsAllDrives=true");
                        http.get(&url).bearer_auth(token)
                    })
                    .await?;

                return Ok(ChangePage {
                    changes: vec![],
                    next_checkpoint: Some(resp.start_page_token),
                });
            }
        };

        #[derive(Deserialize)]
        struct ChangesListResp {
            changes: Option<Vec<DriveChangeItem>>,
            #[serde(rename = "nextPageToken")]
            next_page_token: Option<String>,
            #[serde(rename = "newStartPageToken")]
            new_start_page_token: Option<String>,
        }

        #[derive(Deserialize)]
        struct DriveChangeItem {
            #[serde(rename = "fileId")]
            file_id: String,
            removed: Option<bool>,
            file: Option<DriveFile>,
        }

        let token_param = page_token.clone();
        let resp: ChangesListResp = self
            .client
            .get_json(move |http, base_url, token| {
                let url = format!(
                    "{base_url}/changes?pageToken={token_param}&includeRemoved=true&fields=nextPageToken,newStartPageToken,changes(fileId,removed,file({FILE_FIELDS}))&supportsAllDrives=true"
                );
                http.get(&url).bearer_auth(token)
            })
            .await?;

        let mut changes = Vec::new();
        if let Some(items) = resp.changes {
            for item in items {
                let is_trashed = item.file.as_ref().and_then(|f| f.trashed).unwrap_or(false);

                if item.removed.unwrap_or(false) || is_trashed {
                    changes.push(Change::Delete {
                        id: FileId(item.file_id),
                    });
                } else if let Some(file) = item.file {
                    changes.push(Change::Upsert(drive_file_to_metadata(file)));
                }
            }
        }

        let next_checkpoint = resp.next_page_token.or(resp.new_start_page_token);

        Ok(ChangePage {
            changes,
            next_checkpoint,
        })
    }

    async fn account_id(&self) -> Result<AccountId> {
        Ok(self.client.token_provider().account_id().clone())
    }
}
