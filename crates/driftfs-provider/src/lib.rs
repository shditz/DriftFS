use driftfs_core::{AccountId, ByteRange, FileId};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

pub type SessionCreatedHook = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectMetadata {
    pub id: FileId,
    pub name: String,
    pub parent_id: Option<FileId>,
    pub kind: ObjectKind,
    pub size_bytes: Option<u64>,
    pub mime_type: Option<String>,
    pub created_at: Option<String>,
    pub modified_at: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectKind {
    File,
    Directory,
}

#[derive(Debug, Clone)]
pub struct ChangePage {
    pub changes: Vec<Change>,
    pub next_checkpoint: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Change {
    Upsert(ObjectMetadata),
    Delete { id: FileId },
}

#[derive(Debug)]
pub struct ReadOutput {
    pub data: Vec<u8>,
    pub range: ByteRange,
}

pub trait CloudProvider: Send + Sync {
    fn get_metadata(
        &self,
        id: &FileId,
    ) -> impl std::future::Future<Output = driftfs_core::Result<ObjectMetadata>> + Send;

    fn list_children(
        &self,
        parent_id: &FileId,
    ) -> impl std::future::Future<Output = driftfs_core::Result<Vec<ObjectMetadata>>> + Send;

    fn read_range(
        &self,
        id: &FileId,
        range: ByteRange,
    ) -> impl std::future::Future<Output = driftfs_core::Result<ReadOutput>> + Send;

    fn create_file(
        &self,
        parent_id: &FileId,
        name: &str,
    ) -> impl std::future::Future<Output = driftfs_core::Result<ObjectMetadata>> + Send;

    fn upload(
        &self,
        id: &FileId,
        data: &[u8],
    ) -> impl std::future::Future<Output = driftfs_core::Result<ObjectMetadata>> + Send;

    fn create_directory(
        &self,
        parent_id: &FileId,
        name: &str,
    ) -> impl std::future::Future<Output = driftfs_core::Result<ObjectMetadata>> + Send;

    fn rename(
        &self,
        id: &FileId,
        new_name: &str,
    ) -> impl std::future::Future<Output = driftfs_core::Result<ObjectMetadata>> + Send;

    fn move_object(
        &self,
        id: &FileId,
        new_parent_id: &FileId,
    ) -> impl std::future::Future<Output = driftfs_core::Result<ObjectMetadata>> + Send;

    fn delete(
        &self,
        id: &FileId,
    ) -> impl std::future::Future<Output = driftfs_core::Result<()>> + Send;

    fn upload_file(
        &self,
        id: &FileId,
        path: &Path,
    ) -> impl std::future::Future<Output = driftfs_core::Result<ObjectMetadata>> + Send;

    fn upload_file_resumable(
        &self,
        id: &FileId,
        path: &Path,
        _existing_session_uri: Option<&str>,
        _on_session_created: Option<SessionCreatedHook>,
    ) -> impl std::future::Future<Output = driftfs_core::Result<ObjectMetadata>> + Send {
        self.upload_file(id, path)
    }

    fn trash(
        &self,
        id: &FileId,
    ) -> impl std::future::Future<Output = driftfs_core::Result<()>> + Send;

    fn fetch_changes(
        &self,
        checkpoint: Option<&str>,
    ) -> impl std::future::Future<Output = driftfs_core::Result<ChangePage>> + Send;

    fn account_id(
        &self,
    ) -> impl std::future::Future<Output = driftfs_core::Result<AccountId>> + Send;
}
