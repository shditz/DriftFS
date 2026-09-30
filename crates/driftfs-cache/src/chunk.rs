use std::fmt;

use driftfs_core::FileId;

pub const DEFAULT_CHUNK_SIZE: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChunkKey {
    pub file_id: FileId,
    pub version: String,
    pub chunk_index: u64,
}

impl ChunkKey {
    pub fn new(file_id: FileId, version: impl Into<String>, chunk_index: u64) -> Self {
        Self {
            file_id,
            version: version.into(),
            chunk_index,
        }
    }

    pub fn to_filename(&self) -> String {
        let safe_file_id: String = self
            .file_id
            .0
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let safe_version: String = self
            .version
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("{safe_file_id}@{safe_version}@{}.chunk", self.chunk_index)
    }
}

impl fmt::Display for ChunkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file_id, self.version, self.chunk_index)
    }
}
