pub mod fault;
pub use fault::{FaultInjector, FaultRule, FaultType};

use driftfs_core::{AccountId, ByteRange, FileId};
use driftfs_provider::{Change, ChangePage, CloudProvider, ObjectKind, ObjectMetadata, ReadOutput};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;

pub struct MockProvider {
    objects: RwLock<Vec<ObjectMetadata>>,
    contents: RwLock<HashMap<FileId, Vec<u8>>>,
    change_pages: std::sync::Mutex<std::collections::VecDeque<ChangePage>>,
    read_count: AtomicUsize,
    fault_injector: FaultInjector,
}

impl MockProvider {
    pub fn new() -> Self {
        Self {
            objects: RwLock::new(Vec::new()),
            contents: RwLock::new(HashMap::new()),
            change_pages: std::sync::Mutex::new(std::collections::VecDeque::new()),
            read_count: AtomicUsize::new(0),
            fault_injector: FaultInjector::new(),
        }
    }

    pub fn read_count(&self) -> usize {
        self.read_count.load(Ordering::Relaxed)
    }

    pub fn reset_read_count(&self) {
        self.read_count.store(0, Ordering::Relaxed);
    }

    pub fn enqueue_change_page(&self, page: ChangePage) {
        self.change_pages.lock().unwrap().push_back(page);
    }

    pub fn add_directory(&self, id: &str, name: &str, parent_id: Option<&str>) {
        self.objects.write().unwrap().push(ObjectMetadata {
            id: FileId(id.into()),
            name: name.into(),
            parent_id: parent_id.map(|p| FileId(p.into())),
            kind: ObjectKind::Directory,
            size_bytes: None,
            mime_type: Some("application/vnd.google-apps.folder".into()),
            created_at: None,
            modified_at: None,
            version: None,
        });
    }

    pub fn add_file(&self, id: &str, name: &str, parent_id: Option<&str>, size: u64) {
        let file_id = FileId(id.into());
        self.contents
            .write()
            .unwrap()
            .insert(file_id.clone(), vec![0u8; size as usize]);
        self.objects.write().unwrap().push(ObjectMetadata {
            id: file_id,
            name: name.into(),
            parent_id: parent_id.map(|p| FileId(p.into())),
            kind: ObjectKind::File,
            size_bytes: Some(size),
            mime_type: Some("application/octet-stream".into()),
            created_at: None,
            modified_at: None,
            version: Some("1".into()),
        });
    }

    pub fn add_file_with_bytes(&self, id: &str, name: &str, parent_id: Option<&str>, data: &[u8]) {
        let file_id = FileId(id.into());
        self.contents
            .write()
            .unwrap()
            .insert(file_id.clone(), data.to_vec());
        self.objects.write().unwrap().push(ObjectMetadata {
            id: file_id,
            name: name.into(),
            parent_id: parent_id.map(|p| FileId(p.into())),
            kind: ObjectKind::File,
            size_bytes: Some(data.len() as u64),
            mime_type: Some("application/octet-stream".into()),
            created_at: None,
            modified_at: None,
            version: Some("1".into()),
        });
    }

    pub fn add_workspace_doc(&self, id: &str, name: &str, parent_id: Option<&str>, mime: &str) {
        self.objects.write().unwrap().push(ObjectMetadata {
            id: FileId(id.into()),
            name: name.into(),
            parent_id: parent_id.map(|p| FileId(p.into())),
            kind: ObjectKind::File,
            size_bytes: None,
            mime_type: Some(mime.into()),
            created_at: None,
            modified_at: None,
            version: None,
        });
    }

    pub fn fault_injector(&self) -> &FaultInjector {
        &self.fault_injector
    }

    pub fn inject_fault(&self, operation: &str, rule: FaultRule) {
        self.fault_injector.add_rule(operation, rule);
    }

    pub fn inject_rate_limit(
        &self,
        operation: &str,
        after_calls: usize,
        count: usize,
        retry_after_secs: Option<u64>,
    ) {
        self.fault_injector.add_rule(
            operation,
            FaultRule::new(
                FaultType::RateLimit { retry_after_secs },
                after_calls,
                count,
            ),
        );
    }

    pub fn inject_auth_expired(&self, operation: &str, after_calls: usize, count: usize) {
        self.fault_injector.add_rule(
            operation,
            FaultRule::new(FaultType::AuthExpired, after_calls, count),
        );
    }

    pub fn inject_server_error(
        &self,
        operation: &str,
        after_calls: usize,
        count: usize,
        message: &str,
    ) {
        self.fault_injector.add_rule(
            operation,
            FaultRule::new(
                FaultType::ServerError {
                    message: message.to_string(),
                },
                after_calls,
                count,
            ),
        );
    }

    pub fn inject_network_drop(&self, operation: &str, after_calls: usize, count: usize) {
        self.fault_injector.add_rule(
            operation,
            FaultRule::new(FaultType::NetworkDrop, after_calls, count),
        );
    }

    pub fn inject_read_cutoff(&self, after_calls: usize, max_bytes: usize) {
        self.fault_injector.add_rule(
            "read_range",
            FaultRule::new(FaultType::MidStreamReadCutoff { max_bytes }, after_calls, 1),
        );
    }

    pub fn inject_upload_failure(&self, after_calls: usize, count: usize) {
        self.fault_injector.add_rule(
            "upload_file",
            FaultRule::new(FaultType::UploadFailure, after_calls, count),
        );
        self.fault_injector.add_rule(
            "upload",
            FaultRule::new(FaultType::UploadFailure, after_calls, count),
        );
    }

    pub fn clear_faults(&self) {
        self.fault_injector.clear();
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CloudProvider for MockProvider {
    async fn get_metadata(&self, id: &FileId) -> driftfs_core::Result<ObjectMetadata> {
        self.fault_injector.check("get_metadata")?;
        self.objects
            .read()
            .unwrap()
            .iter()
            .find(|o| o.id == *id)
            .cloned()
            .ok_or_else(|| driftfs_core::DriftFsError::NotFound {
                message: format!("mock: object {id} not found"),
            })
    }

    async fn list_children(&self, parent_id: &FileId) -> driftfs_core::Result<Vec<ObjectMetadata>> {
        self.fault_injector.check("list_children")?;
        Ok(self
            .objects
            .read()
            .unwrap()
            .iter()
            .filter(|o| o.parent_id.as_ref() == Some(parent_id))
            .cloned()
            .collect())
    }

    async fn read_range(&self, id: &FileId, range: ByteRange) -> driftfs_core::Result<ReadOutput> {
        let cutoff = self.fault_injector.check("read_range")?;
        self.read_count.fetch_add(1, Ordering::Relaxed);
        let contents = self.contents.read().unwrap();
        let mut data = if let Some(bytes) = contents.get(id) {
            let start = (range.offset as usize).min(bytes.len());
            let end = ((range.offset + range.length) as usize).min(bytes.len());
            bytes[start..end].to_vec()
        } else {
            vec![0u8; range.length as usize]
        };
        if let Some(FaultType::MidStreamReadCutoff { max_bytes }) = cutoff {
            data.truncate(max_bytes);
        }
        Ok(ReadOutput { data, range })
    }

    async fn create_file(
        &self,
        parent_id: &FileId,
        name: &str,
    ) -> driftfs_core::Result<ObjectMetadata> {
        self.fault_injector.check("create_file")?;
        let meta = ObjectMetadata {
            id: FileId(format!("mock-{name}")),
            name: name.into(),
            parent_id: Some(parent_id.clone()),
            kind: ObjectKind::File,
            size_bytes: Some(0),
            mime_type: None,
            created_at: None,
            modified_at: None,
            version: Some("1".into()),
        };
        self.contents
            .write()
            .unwrap()
            .insert(meta.id.clone(), Vec::new());
        self.objects.write().unwrap().push(meta.clone());
        Ok(meta)
    }

    async fn upload(&self, id: &FileId, data: &[u8]) -> driftfs_core::Result<ObjectMetadata> {
        self.fault_injector.check("upload")?;
        let mut objects = self.objects.write().unwrap();
        let meta = objects.iter_mut().find(|o| o.id == *id).ok_or_else(|| {
            driftfs_core::DriftFsError::NotFound {
                message: format!("mock: object {id} not found"),
            }
        })?;
        meta.size_bytes = Some(data.len() as u64);
        self.contents
            .write()
            .unwrap()
            .insert(id.clone(), data.to_vec());
        Ok(meta.clone())
    }

    async fn create_directory(
        &self,
        parent_id: &FileId,
        name: &str,
    ) -> driftfs_core::Result<ObjectMetadata> {
        self.fault_injector.check("create_directory")?;
        let meta = ObjectMetadata {
            id: FileId(format!("mock-dir-{name}")),
            name: name.into(),
            parent_id: Some(parent_id.clone()),
            kind: ObjectKind::Directory,
            size_bytes: None,
            mime_type: Some("application/vnd.google-apps.folder".into()),
            created_at: None,
            modified_at: None,
            version: None,
        };
        self.objects.write().unwrap().push(meta.clone());
        Ok(meta)
    }

    async fn rename(&self, id: &FileId, new_name: &str) -> driftfs_core::Result<ObjectMetadata> {
        self.fault_injector.check("rename")?;
        let mut objects = self.objects.write().unwrap();
        let meta = objects.iter_mut().find(|o| o.id == *id).ok_or_else(|| {
            driftfs_core::DriftFsError::NotFound {
                message: format!("mock: object {id} not found"),
            }
        })?;
        meta.name = new_name.into();
        Ok(meta.clone())
    }

    async fn move_object(
        &self,
        id: &FileId,
        new_parent_id: &FileId,
    ) -> driftfs_core::Result<ObjectMetadata> {
        self.fault_injector.check("move_object")?;
        let mut objects = self.objects.write().unwrap();
        let meta = objects.iter_mut().find(|o| o.id == *id).ok_or_else(|| {
            driftfs_core::DriftFsError::NotFound {
                message: format!("mock: object {id} not found"),
            }
        })?;
        meta.parent_id = Some(new_parent_id.clone());
        Ok(meta.clone())
    }

    async fn delete(&self, id: &FileId) -> driftfs_core::Result<()> {
        self.fault_injector.check("delete")?;
        let mut objects = self.objects.write().unwrap();
        if let Some(pos) = objects.iter().position(|o| o.id == *id) {
            objects.remove(pos);
            self.contents.write().unwrap().remove(id);
            Ok(())
        } else {
            Err(driftfs_core::DriftFsError::NotFound {
                message: format!("mock: cannot delete {id}, not found"),
            })
        }
    }

    async fn upload_file(&self, id: &FileId, path: &Path) -> driftfs_core::Result<ObjectMetadata> {
        self.fault_injector.check("upload_file")?;
        let data = std::fs::read(path).map_err(|e| driftfs_core::DriftFsError::Filesystem {
            message: format!("mock upload_file failed to read: {e}"),
            source: Some(Box::new(e)),
        })?;
        let file_size = data.len() as u64;
        let mut objects = self.objects.write().unwrap();
        let meta = objects.iter_mut().find(|o| o.id == *id).ok_or_else(|| {
            driftfs_core::DriftFsError::NotFound {
                message: format!("mock: object {id} not found"),
            }
        })?;
        meta.size_bytes = Some(file_size);
        self.contents.write().unwrap().insert(id.clone(), data);
        Ok(meta.clone())
    }

    async fn trash(&self, id: &FileId) -> driftfs_core::Result<()> {
        self.fault_injector.check("trash")?;
        let mut objects = self.objects.write().unwrap();
        if let Some(pos) = objects.iter().position(|o| o.id == *id) {
            objects.remove(pos);
            self.contents.write().unwrap().remove(id);
            Ok(())
        } else {
            Err(driftfs_core::DriftFsError::NotFound {
                message: format!("mock: cannot trash {id}, not found"),
            })
        }
    }

    async fn fetch_changes(&self, _checkpoint: Option<&str>) -> driftfs_core::Result<ChangePage> {
        self.fault_injector.check("fetch_changes")?;
        if let Some(page) = self.change_pages.lock().unwrap().pop_front() {
            return Ok(page);
        }

        Ok(ChangePage {
            changes: self
                .objects
                .read()
                .unwrap()
                .iter()
                .cloned()
                .map(Change::Upsert)
                .collect(),
            next_checkpoint: None,
        })
    }

    async fn account_id(&self) -> driftfs_core::Result<AccountId> {
        self.fault_injector.check("account_id")?;
        Ok(AccountId("mock-account@test.local".into()))
    }
}

pub fn create_metadata(
    id: &str,
    name: &str,
    parent: Option<&str>,
    kind: ObjectKind,
    size: Option<u64>,
    mime: Option<&str>,
) -> ObjectMetadata {
    ObjectMetadata {
        id: FileId(id.into()),
        name: name.into(),
        parent_id: parent.map(|p| FileId(p.into())),
        kind,
        size_bytes: size,
        mime_type: mime.map(|m| m.into()),
        created_at: Some("2026-09-28T12:00:00Z".into()),
        modified_at: Some("2026-09-28T12:00:00Z".into()),
        version: Some("1".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_provider_list_children() {
        let provider = MockProvider::new();
        provider.add_directory("root", "My Drive", None);
        provider.add_file("f1", "readme.txt", Some("root"), 1024);
        provider.add_file("f2", "photo.jpg", Some("root"), 4096);
        provider.add_directory("d1", "Documents", Some("root"));

        let children = provider
            .list_children(&FileId("root".into()))
            .await
            .unwrap();

        assert_eq!(children.len(), 3);
    }

    #[tokio::test]
    async fn mock_provider_not_found() {
        let provider = MockProvider::new();
        let result = provider.get_metadata(&FileId("nonexistent".into())).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn mock_provider_inject_rate_limit() {
        let provider = MockProvider::new();
        provider.add_directory("root", "My Drive", None);

        // Fail call 1 with rate limit, call 0 and 2 should succeed
        provider.inject_rate_limit("get_metadata", 1, 1, Some(30));

        let res0 = provider.get_metadata(&FileId("root".into())).await;
        assert!(res0.is_ok());

        let res1 = provider.get_metadata(&FileId("root".into())).await;
        assert!(matches!(
            res1,
            Err(driftfs_core::DriftFsError::RateLimited {
                retry_after_secs: Some(30)
            })
        ));

        let res2 = provider.get_metadata(&FileId("root".into())).await;
        assert!(res2.is_ok());
    }

    #[tokio::test]
    async fn mock_provider_inject_auth_expired() {
        let provider = MockProvider::new();
        provider.inject_auth_expired("account_id", 0, 1);

        let res = provider.account_id().await;
        assert!(matches!(
            res,
            Err(driftfs_core::DriftFsError::Authentication { .. })
        ));
    }

    #[tokio::test]
    async fn mock_provider_inject_midstream_read_cutoff() {
        let provider = MockProvider::new();
        provider.add_file_with_bytes("f1", "test.bin", None, &[1, 2, 3, 4, 5, 6, 7, 8]);

        provider.inject_read_cutoff(0, 3);
        let out = provider
            .read_range(&FileId("f1".into()), ByteRange::new(0, 8))
            .await
            .unwrap();

        assert_eq!(out.data, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn mock_provider_inject_upload_failure() {
        let provider = MockProvider::new();
        provider.add_file("f1", "test.txt", None, 0);
        provider.inject_upload_failure(0, 1);

        let res = provider.upload(&FileId("f1".into()), b"hello").await;
        assert!(matches!(
            res,
            Err(driftfs_core::DriftFsError::Storage { .. })
        ));
    }
}
