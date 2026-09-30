use driftfs_core::FileId;
use driftfs_provider::{ObjectKind, ObjectMetadata};
use serde::{Deserialize, Serialize};

pub const GOOGLE_DRIVE_FOLDER_MIME: &str = "application/vnd.google-apps.folder";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    pub size: Option<String>,
    #[serde(rename = "createdTime")]
    pub created_time: Option<String>,
    #[serde(rename = "modifiedTime")]
    pub modified_time: Option<String>,
    pub parents: Option<Vec<String>>,
    pub version: Option<String>,
    pub trashed: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct DriveFileList {
    pub files: Option<Vec<DriveFile>>,
    #[serde(rename = "nextPageToken")]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DriveAbout {
    pub user: Option<DriveAboutUser>,
    #[serde(rename = "storageQuota")]
    pub storage_quota: Option<DriveStorageQuota>,
}

#[derive(Debug, Deserialize)]
pub struct DriveAboutUser {
    #[serde(rename = "permissionId")]
    pub permission_id: Option<String>,
    #[serde(rename = "emailAddress")]
    pub email_address: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DriveStorageQuota {
    pub limit: Option<String>,
    pub usage: Option<String>,
}

pub fn drive_file_to_metadata(file: DriveFile) -> ObjectMetadata {
    let kind = if file.mime_type.as_deref() == Some(GOOGLE_DRIVE_FOLDER_MIME) {
        ObjectKind::Directory
    } else {
        ObjectKind::File
    };

    let size_bytes = file.size.as_deref().and_then(|s| s.parse::<u64>().ok());
    let parent_id = file.parents.and_then(|p| p.into_iter().next()).map(FileId);

    ObjectMetadata {
        id: FileId(file.id),
        name: file.name,
        parent_id,
        kind,
        size_bytes,
        mime_type: file.mime_type,
        created_at: file.created_time,
        modified_at: file.modified_time,
        version: file.version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_drive_file_to_metadata() {
        let file = DriveFile {
            id: "1abc_test_id".into(),
            name: "document.pdf".into(),
            mime_type: Some("application/pdf".into()),
            size: Some("1048576".into()),
            created_time: Some("2026-09-01T10:00:00Z".into()),
            modified_time: Some("2026-09-02T12:00:00Z".into()),
            parents: Some(vec!["root_folder".into()]),
            version: Some("42".into()),
            trashed: Some(false),
        };

        let meta = drive_file_to_metadata(file);
        assert_eq!(meta.id.0, "1abc_test_id");
        assert_eq!(meta.name, "document.pdf");
        assert_eq!(meta.kind, ObjectKind::File);
        assert_eq!(meta.size_bytes, Some(1048576));
        assert_eq!(meta.parent_id, Some(FileId("root_folder".into())));
        assert_eq!(meta.version.as_deref(), Some("42"));
    }

    #[test]
    fn maps_drive_folder_to_metadata() {
        let folder = DriveFile {
            id: "folder_123".into(),
            name: "My Documents".into(),
            mime_type: Some(GOOGLE_DRIVE_FOLDER_MIME.into()),
            size: None,
            created_time: None,
            modified_time: None,
            parents: None,
            version: None,
            trashed: None,
        };

        let meta = drive_file_to_metadata(folder);
        assert_eq!(meta.kind, ObjectKind::Directory);
        assert_eq!(meta.size_bytes, None);
        assert_eq!(meta.parent_id, None);
    }
}
