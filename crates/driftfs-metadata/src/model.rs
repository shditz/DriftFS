use driftfs_core::FileId;
use driftfs_provider::ObjectKind;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStatus {
    Synced,
    Pending,
    Conflict,
}

impl SyncStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Synced => "synced",
            Self::Pending => "pending",
            Self::Conflict => "conflict",
        }
    }
}

impl std::str::FromStr for SyncStatus {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let status = match s {
            "pending" => Self::Pending,
            "conflict" => Self::Conflict,
            _ => Self::Synced,
        };
        Ok(status)
    }
}

impl fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    pub id: FileId,
    pub parent_id: Option<FileId>,
    pub name: String,
    pub remote_name: String,
    pub kind: ObjectKind,
    pub size_bytes: Option<u64>,
    pub mime_type: Option<String>,
    pub created_at: Option<String>,
    pub modified_at: Option<String>,
    pub version: Option<String>,
    pub sync_status: SyncStatus,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagingState {
    Staging,
    Uploading,
    Failed,
}

impl StagingState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Staging => "staging",
            Self::Uploading => "uploading",
            Self::Failed => "failed",
        }
    }
}

impl std::str::FromStr for StagingState {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let state = match s {
            "uploading" => Self::Uploading,
            "failed" => Self::Failed,
            _ => Self::Staging,
        };
        Ok(state)
    }
}

impl fmt::Display for StagingState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagingJournalEntry {
    pub handle_id: u64,
    pub file_id: FileId,
    pub staging_path: String,
    pub parent_id: Option<FileId>,
    pub state: StagingState,
    pub session_uri: Option<String>,
    pub uploaded_bytes: u64,
    pub created_at: i64,
    pub updated_at: i64,
}
