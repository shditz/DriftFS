use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use driftfs_cache::{BoundedChunkCache, ChunkKey};
use driftfs_core::{
    format_iso_timestamp, parse_iso_timestamp, sanitize_path, validate_file_name, ByteRange, FileId,
};
use driftfs_metadata::{MetadataStore, StagingState, StoredObject, SyncStatus};
use driftfs_provider::{CloudProvider, ObjectKind};
use tokio::sync::Semaphore;
use tracing::{debug, info, instrument, warn};

use crate::error::{Result, VfsError};
use crate::handle_table::HandleTable;
use crate::shortcut::GoogleWorkspaceShortcut;
use crate::staging::StagingManager;
use crate::types::{OpenFlags, VfsAttr, VfsDirEntry, VfsHandle, VfsNodeType, VfsStatFs};

pub const ROOT_INODE: u64 = 1;
const MAX_CONCURRENT_RANGE_READS: usize = 16;

/// Deterministic 64-bit FNV-1a hash algorithm for virtual filesystem inodes.
pub fn file_id_to_ino(id: &str) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in id.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    if hash <= 1 {
        hash + 2
    } else {
        hash
    }
}

pub struct DriftFsVfs<P: CloudProvider> {
    metadata: Arc<MetadataStore>,
    provider: Arc<P>,
    handles: HandleTable,
    staging: StagingManager,
    read_semaphore: Arc<Semaphore>,
    root_folder_id: Option<FileId>,
    chunk_cache: Option<Arc<BoundedChunkCache>>,
    prefetch_enabled: bool,
    quota: Arc<std::sync::RwLock<VfsStatFs>>,
}

impl<P: CloudProvider + 'static> DriftFsVfs<P> {
    pub fn new(
        metadata: Arc<MetadataStore>,
        provider: Arc<P>,
        staging_dir: PathBuf,
    ) -> Result<Self> {
        let staging = StagingManager::new(staging_dir)?;
        Ok(Self {
            metadata,
            provider,
            handles: HandleTable::new(),
            staging,
            read_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_RANGE_READS)),
            root_folder_id: Some(FileId("root".into())),
            chunk_cache: None,
            prefetch_enabled: true,
            quota: Arc::new(std::sync::RwLock::new(VfsStatFs::default())),
        })
    }

    pub fn with_root_folder_id(mut self, id: FileId) -> Self {
        self.root_folder_id = Some(id);
        self
    }

    pub fn with_chunk_cache(mut self, chunk_cache: Arc<BoundedChunkCache>) -> Self {
        self.chunk_cache = Some(chunk_cache);
        self
    }

    pub fn with_prefetch(mut self, enabled: bool) -> Self {
        self.prefetch_enabled = enabled;
        self
    }

    pub fn with_max_concurrent_reads(mut self, max: usize) -> Self {
        let permits = if max > 0 {
            max
        } else {
            MAX_CONCURRENT_RANGE_READS
        };
        self.read_semaphore = Arc::new(Semaphore::new(permits));
        self
    }

    pub fn chunk_cache(&self) -> Option<&Arc<BoundedChunkCache>> {
        self.chunk_cache.as_ref()
    }

    pub fn with_quota(self, total: u64, used: u64) -> Self {
        if let Ok(mut q) = self.quota.write() {
            *q = VfsStatFs::new(total, used);
        }
        self
    }

    pub fn with_quota_handle(mut self, quota: Arc<std::sync::RwLock<VfsStatFs>>) -> Self {
        self.quota = quota;
        self
    }

    pub fn set_quota(&self, total: u64, used: u64) {
        if let Ok(mut q) = self.quota.write() {
            *q = VfsStatFs::new(total, used);
        }
    }

    pub fn quota_handle(&self) -> Arc<std::sync::RwLock<VfsStatFs>> {
        Arc::clone(&self.quota)
    }

    pub fn provider(&self) -> &Arc<P> {
        &self.provider
    }

    pub fn handles(&self) -> &HandleTable {
        &self.handles
    }

    pub fn metadata(&self) -> &MetadataStore {
        &self.metadata
    }

    pub fn normalize_path(path: &str) -> Vec<&str> {
        path.split(['/', '\\'])
            .filter(|p| !p.is_empty() && *p != ".")
            .collect()
    }

    pub fn validate_path(path: &str) -> Result<Vec<String>> {
        sanitize_path(path).map_err(|_| VfsError::InvalidPath)
    }

    fn split_parent_child(path: &str) -> Result<(String, String)> {
        let segments = Self::validate_path(path)?;
        if segments.is_empty() {
            return Err(VfsError::InvalidPath);
        }
        let name = segments.last().ok_or(VfsError::InvalidPath)?.clone();
        let parent = if segments.len() == 1 {
            "/".to_string()
        } else {
            format!("/{}", segments[..segments.len() - 1].join("/"))
        };
        Ok((parent, name))
    }

    /// Falls back to canonical "root" when parent_id is None.
    fn effective_parent_id(&self, parent_id: &Option<FileId>) -> Result<FileId> {
        parent_id
            .clone()
            .or_else(|| self.root_folder_id.clone())
            .or_else(|| Some(FileId("root".into())))
            .ok_or_else(|| VfsError::Io("root folder id not configured".into()))
    }

    fn resolve_parent_id(&self, parent_path: &str) -> Result<Option<FileId>> {
        let segments = Self::validate_path(parent_path)?;
        if segments.is_empty() {
            return Ok(None);
        }
        let obj = self.resolve_path(parent_path)?.ok_or(VfsError::NotFound)?;
        if obj.kind != ObjectKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        Ok(Some(obj.id))
    }

    #[instrument(skip(self), level = "debug")]
    pub fn resolve_path(&self, path: &str) -> Result<Option<StoredObject>> {
        let segments = Self::validate_path(path)?;
        if segments.is_empty() {
            return Ok(None);
        }

        let mut current_parent: Option<FileId> = None;
        let total = segments.len();

        for (i, segment) in segments.iter().enumerate() {
            let is_last = i == total - 1;

            if !is_last {
                let obj = self
                    .metadata
                    .lookup_by_name(current_parent.as_ref(), segment)?
                    .ok_or(VfsError::NotFound)?;

                if obj.kind != ObjectKind::Directory {
                    return Err(VfsError::NotDirectory);
                }
                current_parent = Some(obj.id);
            } else {
                if let Some(obj) = self
                    .metadata
                    .lookup_by_name(current_parent.as_ref(), segment)?
                {
                    return Ok(Some(obj));
                }

                if let Some(stripped) = GoogleWorkspaceShortcut::strip_shortcut_extension(segment) {
                    let effective_stem = stripped.strip_suffix(" (shortcut)").unwrap_or(stripped);
                    if let Some(obj) = self
                        .metadata
                        .lookup_by_name(current_parent.as_ref(), effective_stem)?
                    {
                        if GoogleWorkspaceShortcut::is_workspace(&obj) {
                            return Ok(Some(obj));
                        }
                    }
                }

                return Err(VfsError::NotFound);
            }
        }

        Err(VfsError::NotFound)
    }

    #[instrument(skip(self), level = "debug")]
    pub fn getattr(&self, path: &str) -> Result<VfsAttr> {
        let segments = Self::validate_path(path)?;
        if segments.is_empty() {
            return Ok(VfsAttr::directory(ROOT_INODE, None));
        }

        let obj = self.resolve_path(path)?.ok_or(VfsError::NotFound)?;
        let ino = file_id_to_ino(&obj.id.0);
        let modified = parse_iso_timestamp(obj.modified_at.as_deref());

        if obj.kind == ObjectKind::Directory {
            Ok(VfsAttr::directory(ino, modified))
        } else if let Some(shortcut) = GoogleWorkspaceShortcut::try_from_stored(&obj) {
            Ok(VfsAttr::shortcut(
                ino,
                shortcut.payload.len() as u64,
                modified,
            ))
        } else {
            let size = obj.size_bytes.unwrap_or(0);
            Ok(VfsAttr::file(ino, size, modified))
        }
    }

    #[instrument(skip(self), level = "debug")]
    pub fn opendir(&self, path: &str) -> Result<VfsHandle> {
        let segments = Self::validate_path(path)?;
        if segments.is_empty() {
            return self
                .handles
                .allocate(None, VfsNodeType::Directory, 4096, None);
        }

        let obj = self.resolve_path(path)?.ok_or(VfsError::NotFound)?;
        if obj.kind != ObjectKind::Directory {
            return Err(VfsError::NotDirectory);
        }

        self.handles
            .allocate(Some(obj.id), VfsNodeType::Directory, 4096, None)
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn readdir(&self, handle: VfsHandle) -> Result<Vec<VfsDirEntry>> {
        let entry = self.handles.get(handle)?;
        if entry.kind != VfsNodeType::Directory {
            return Err(VfsError::NotDirectory);
        }

        let is_synced = self.metadata.is_directory_synced(entry.file_id.as_ref())?;
        if !is_synced {
            let target_parent_ref = match &entry.file_id {
                Some(id) => id.clone(),
                None => self.effective_parent_id(&None)?,
            };
            match self.provider.list_children(&target_parent_ref).await {
                Ok(remote_children) => {
                    for mut child in remote_children {
                        child.parent_id = entry.file_id.clone();
                        let _ = self.metadata.upsert_object(&child);
                    }
                    let _ = self.metadata.mark_directory_synced(entry.file_id.as_ref());
                }
                Err(e) => {
                    warn!(
                        dir_id = ?entry.file_id,
                        error = %e,
                        "failed to sync directory on-demand from provider; falling back to local metadata"
                    );
                }
            }
        }

        let children = self.metadata.list_children(entry.file_id.as_ref())?;
        let mut entries = Vec::with_capacity(children.len());

        let mut regular_names = std::collections::HashSet::new();
        for child in &children {
            if !GoogleWorkspaceShortcut::is_workspace(child) {
                regular_names.insert(child.name.to_lowercase());
            }
        }

        for child in children {
            let ino = file_id_to_ino(&child.id.0);
            if let Some(mut shortcut) = GoogleWorkspaceShortcut::try_from_stored(&child) {
                if regular_names.contains(&shortcut.display_name.to_lowercase()) {
                    let stem = match shortcut.display_name.rfind('.') {
                        Some(idx) => &shortcut.display_name[..idx],
                        None => shortcut.display_name.as_str(),
                    };
                    shortcut.display_name = format!("{stem} (shortcut).url");
                }
                entries.push(VfsDirEntry {
                    name: shortcut.display_name,
                    kind: VfsNodeType::GoogleDocShortcut,
                    size: shortcut.payload.len() as u64,
                    ino,
                });
            } else {
                let kind = match child.kind {
                    ObjectKind::Directory => VfsNodeType::Directory,
                    ObjectKind::File => VfsNodeType::File,
                };
                let size = child.size_bytes.unwrap_or(0);
                entries.push(VfsDirEntry {
                    name: child.name,
                    kind,
                    size,
                    ino,
                });
            }
        }

        Ok(entries)
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn open(&self, path: &str, flags: OpenFlags) -> Result<VfsHandle> {
        let obj = self.resolve_path(path)?.ok_or(VfsError::NotFound)?;
        if obj.kind == ObjectKind::Directory {
            return Err(VfsError::IsDirectory);
        }

        if let Some(shortcut) = GoogleWorkspaceShortcut::try_from_stored(&obj) {
            if flags.write || flags.truncate || flags.append {
                return Err(VfsError::AccessDenied);
            }
            let size = shortcut.payload.len() as u64;
            return self.handles.allocate(
                Some(obj.id),
                VfsNodeType::GoogleDocShortcut,
                size,
                Some(shortcut.payload),
            );
        }

        let size = obj.size_bytes.unwrap_or(0);
        let handle = self
            .handles
            .allocate(Some(obj.id.clone()), VfsNodeType::File, size, None)?;

        if flags.write || flags.append {
            let staging_path = self.staging.create(handle)?;
            self.handles.set_staging_path(handle, staging_path)?;

            if flags.truncate {
                self.handles.update_size(handle, 0)?;
                self.handles.mark_dirty(handle)?;
            } else if size > 0 {
                const SEED_CHUNK_SIZE: u64 = 4 * 1024 * 1024;
                let mut current_offset: u64 = 0;
                while current_offset < size {
                    let chunk_len = std::cmp::min(SEED_CHUNK_SIZE, size - current_offset);
                    let range = ByteRange::new(current_offset, chunk_len);
                    let _permit = self
                        .read_semaphore
                        .acquire()
                        .await
                        .map_err(|e| VfsError::Io(e.to_string()))?;
                    let output = self.provider.read_range(&obj.id, range).await?;
                    if output.data.is_empty() {
                        break;
                    }
                    self.staging
                        .write_at(handle, current_offset, &output.data)?;
                    current_offset += output.data.len() as u64;
                }
            }
        }

        Ok(handle)
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn create_file(&self, parent_path: &str, name: &str) -> Result<(VfsHandle, VfsAttr)> {
        validate_file_name(name).map_err(|_| VfsError::InvalidPath)?;
        let parent_id = self.resolve_parent_id(parent_path)?;

        if self
            .metadata
            .lookup_by_name(parent_id.as_ref(), name)?
            .is_some()
        {
            return Err(VfsError::AlreadyExists);
        }

        if let Ok(free_space) = self.staging.available_disk_space() {
            const MIN_HEADROOM_BYTES: u64 = 2 * 1024 * 1024 * 1024;
            if free_space < MIN_HEADROOM_BYTES {
                warn!(
                    free_space,
                    min_headroom = MIN_HEADROOM_BYTES,
                    "disk headroom threshold reached; rejecting new file creation"
                );
                return Err(VfsError::DiskFull);
            }
        }

        let file_id = FileId::new_local();
        let now_str = format_iso_timestamp(SystemTime::now());

        let stored = StoredObject {
            id: file_id.clone(),
            parent_id: parent_id.clone(),
            name: name.to_string(),
            remote_name: name.to_string(),
            kind: ObjectKind::File,
            size_bytes: Some(0),
            mime_type: None,
            created_at: Some(now_str.clone()),
            modified_at: Some(now_str),
            version: None,
            sync_status: SyncStatus::Pending,
            deleted: false,
        };
        self.metadata.insert_new_object(&stored)?;

        let ino = file_id_to_ino(&file_id.0);
        let handle = self
            .handles
            .allocate(Some(file_id.clone()), VfsNodeType::File, 0, None)?;

        let staging_path = self.staging.create(handle)?;
        let _ = self.metadata.record_staging_entry(
            handle.0,
            &file_id,
            &staging_path.to_string_lossy(),
            parent_id.as_ref(),
        );
        self.handles.set_staging_path(handle, staging_path)?;
        self.handles.mark_dirty(handle)?;

        let attr = VfsAttr::file(ino, 0, Some(SystemTime::now()));
        Ok((handle, attr))
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn mkdir(&self, path: &str) -> Result<VfsAttr> {
        let (parent_path, name) = Self::split_parent_child(path)?;
        validate_file_name(&name).map_err(|_| VfsError::InvalidPath)?;
        let parent_id = self.resolve_parent_id(&parent_path)?;

        if self
            .metadata
            .lookup_by_name(parent_id.as_ref(), &name)?
            .is_some()
        {
            return Err(VfsError::AlreadyExists);
        }

        let dir_id = FileId::new_local();
        let now_str = format_iso_timestamp(SystemTime::now());

        let stored = StoredObject {
            id: dir_id.clone(),
            parent_id: parent_id.clone(),
            name: name.clone(),
            remote_name: name,
            kind: ObjectKind::Directory,
            size_bytes: None,
            mime_type: Some("application/vnd.google-apps.folder".to_string()),
            created_at: Some(now_str.clone()),
            modified_at: Some(now_str),
            version: None,
            sync_status: SyncStatus::Pending,
            deleted: false,
        };
        self.metadata.insert_new_object(&stored)?;

        let _ = self.metadata.record_staging_entry(
            {
                use std::sync::atomic::{AtomicU64, Ordering};
                static DIR_COUNTER: AtomicU64 = AtomicU64::new(0x6000_0000_0000_0000);
                DIR_COUNTER.fetch_add(1, Ordering::Relaxed)
            },
            &dir_id,
            "",
            parent_id.as_ref(),
        );

        let ino = file_id_to_ino(&dir_id.0);
        Ok(VfsAttr::directory(ino, Some(SystemTime::now())))
    }

    #[instrument(skip(self, data), level = "debug", fields(data_len = data.len()))]
    pub fn write(&self, handle: VfsHandle, offset: u64, data: &[u8]) -> Result<usize> {
        let entry = self.handles.get(handle)?;

        if entry.kind == VfsNodeType::GoogleDocShortcut {
            return Err(VfsError::AccessDenied);
        }
        if entry.kind == VfsNodeType::Directory {
            return Err(VfsError::IsDirectory);
        }
        if !self.staging.has_staging(handle) {
            let staging_path = self.staging.create(handle)?;
            if let Some(ref fid) = entry.file_id {
                let _ = self.metadata.record_staging_entry(
                    handle.0,
                    fid,
                    &staging_path.to_string_lossy(),
                    None,
                );
            }
            self.handles.set_staging_path(handle, staging_path)?;
        }

        let written = self.staging.write_at(handle, offset, data)?;
        let new_size = self.staging.size(handle)?;
        self.handles.update_size(handle, new_size)?;
        self.handles.mark_dirty(handle)?;

        Ok(written)
    }

    #[instrument(skip(self), level = "debug")]
    pub fn set_length(&self, handle: VfsHandle, size: u64) -> Result<()> {
        let entry = self.handles.get(handle)?;
        if entry.kind == VfsNodeType::GoogleDocShortcut {
            return Err(VfsError::AccessDenied);
        }
        if entry.kind == VfsNodeType::Directory {
            return Err(VfsError::IsDirectory);
        }

        if !self.staging.has_staging(handle) {
            let staging_path = self.staging.create(handle)?;
            if let Some(ref fid) = entry.file_id {
                let _ = self.metadata.record_staging_entry(
                    handle.0,
                    fid,
                    &staging_path.to_string_lossy(),
                    None,
                );
            }
            self.handles.set_staging_path(handle, staging_path)?;
        }

        self.staging.truncate(handle, size)?;
        self.handles.update_size(handle, size)?;
        self.handles.mark_dirty(handle)?;
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn read(&self, handle: VfsHandle, offset: u64, length: usize) -> Result<Vec<u8>> {
        let entry = self.handles.get(handle)?;
        if entry.kind == VfsNodeType::Directory {
            return Err(VfsError::IsDirectory);
        }

        if self.staging.has_staging(handle) {
            return self.staging.read_at(handle, offset, length);
        }

        if offset >= entry.size {
            return Ok(Vec::new());
        }

        let to_read = std::cmp::min(length as u64, entry.size.saturating_sub(offset)) as usize;
        if to_read == 0 {
            return Ok(Vec::new());
        }

        if let Some(payload) = entry.shortcut_data {
            let start = offset as usize;
            let end = std::cmp::min(start + to_read, payload.len());
            return Ok(payload[start..end].to_vec());
        }

        let file_id = entry.file_id.ok_or(VfsError::NotFound)?;

        if file_id.is_local() {
            if let Ok(entries) = self.metadata.list_uncommitted_staging() {
                if let Some(stg) = entries.iter().find(|e| e.file_id == file_id) {
                    let path = std::path::Path::new(&stg.staging_path);
                    if path.exists() {
                        use std::io::{Read, Seek, SeekFrom};
                        if let Ok(mut f) = std::fs::File::open(path) {
                            if f.seek(SeekFrom::Start(offset)).is_ok() {
                                let mut buf = vec![0u8; to_read];
                                if let Ok(n) = f.read(&mut buf) {
                                    buf.truncate(n);
                                    return Ok(buf);
                                }
                            }
                        }
                    }
                }
            }
            return Ok(Vec::new());
        }

        if let Some(ref cache) = self.chunk_cache {
            let version = self
                .metadata
                .get_object(&file_id)
                .ok()
                .flatten()
                .and_then(|obj| obj.version.or(obj.modified_at))
                .unwrap_or_else(|| "1".into());

            let chunk_size = cache.chunk_size() as u64;
            let start_chunk = offset / chunk_size;
            let end_chunk = (offset + to_read as u64 - 1) / chunk_size;
            let mut result = Vec::with_capacity(to_read);

            for chunk_idx in start_chunk..=end_chunk {
                let key = ChunkKey::new(file_id.clone(), &version, chunk_idx);
                let chunk_data = match cache
                    .get(&key)
                    .await
                    .map_err(|e| VfsError::Io(e.to_string()))?
                {
                    Some(data) => data,
                    None => {
                        let chunk_offset = chunk_idx * chunk_size;
                        let chunk_len =
                            std::cmp::min(chunk_size, entry.size.saturating_sub(chunk_offset));
                        let range = ByteRange::new(chunk_offset, chunk_len);

                        let _permit = self
                            .read_semaphore
                            .acquire()
                            .await
                            .map_err(|e| VfsError::Io(e.to_string()))?;

                        let output = self.provider.read_range(&file_id, range).await?;
                        let _ = cache.put(key, &output.data).await;
                        output.data
                    }
                };

                let chunk_start_offset = chunk_idx * chunk_size;
                let slice_start = offset.saturating_sub(chunk_start_offset) as usize;
                let needed = to_read - result.len();
                let slice_end = std::cmp::min(slice_start + needed, chunk_data.len());
                if slice_start < chunk_data.len() {
                    result.extend_from_slice(&chunk_data[slice_start..slice_end]);
                }
            }

            let seq_count = self
                .handles
                .record_read(handle, offset, result.len() as u64)
                .unwrap_or(0);

            if seq_count >= 2 && self.prefetch_enabled {
                self.spawn_prefetch(
                    file_id.clone(),
                    version,
                    end_chunk + 1,
                    chunk_size,
                    entry.size,
                );
            }

            return Ok(result);
        }

        let range = ByteRange::new(offset, to_read as u64);

        let _permit = self
            .read_semaphore
            .acquire()
            .await
            .map_err(|e| VfsError::Io(e.to_string()))?;

        debug!(
            file_id = %file_id,
            offset = offset,
            length = to_read,
            "fetching byte range from cloud provider"
        );

        let output = self.provider.read_range(&file_id, range).await?;
        let _ = self
            .handles
            .record_read(handle, offset, output.data.len() as u64);
        Ok(output.data)
    }

    fn spawn_prefetch(
        &self,
        file_id: FileId,
        version: String,
        chunk_index: u64,
        chunk_size: u64,
        file_size: u64,
    ) {
        let cache = match self.chunk_cache {
            Some(ref c) => Arc::clone(c),
            None => return,
        };
        let chunk_offset = chunk_index * chunk_size;
        if chunk_offset >= file_size {
            return;
        }
        let chunk_len = std::cmp::min(chunk_size, file_size - chunk_offset);
        let key = ChunkKey::new(file_id.clone(), version, chunk_index);
        let provider = Arc::clone(&self.provider);
        let read_semaphore = Arc::clone(&self.read_semaphore);

        tokio::spawn(async move {
            if let Ok(Some(_)) = cache.get(&key).await {
                return;
            }

            let _permit = match read_semaphore.acquire().await {
                Ok(p) => p,
                Err(_) => return,
            };

            let range = ByteRange::new(chunk_offset, chunk_len);
            if let Ok(output) = provider.read_range(&file_id, range).await {
                let _ = cache.put(key, &output.data).await;
            }
        });
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn flush(&self, handle: VfsHandle) -> Result<()> {
        let entry = self.handles.get(handle)?;
        if !entry.is_dirty {
            return Ok(());
        }

        let file_id = entry.file_id.ok_or(VfsError::NotFound)?;

        if let Some(ref staging_path) = entry.staging_path {
            let size = self.staging.size(handle).unwrap_or(entry.size);
            let now_str = format_iso_timestamp(SystemTime::now());
            let _ = self
                .metadata
                .update_size_and_mtime(&file_id, size, Some(&now_str));
            let _ = self.metadata.record_staging_entry(
                handle.0,
                &file_id,
                &staging_path.to_string_lossy(),
                None,
            );
            self.handles.update_size(handle, size)?;
            self.handles.clear_dirty(handle)?;

            if let Some(ref cache) = self.chunk_cache {
                let _ = cache.invalidate_file(&file_id).await;
            }
        }

        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn close_async(&self, handle: VfsHandle) -> Result<()> {
        let entry = self.handles.get(handle)?;

        if entry.is_dirty {
            let _ = self.flush(handle).await;
            if let Some(ref cache) = self.chunk_cache {
                if let Some(ref file_id) = entry.file_id {
                    let _ = cache.invalidate_file(file_id).await;
                }
            }
        }

        self.staging.release(handle);
        self.handles.release(handle)?;
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    pub fn close(&self, handle: VfsHandle) -> Result<()> {
        self.staging.release(handle);
        self.handles.release(handle)?;
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn rename(
        &self,
        old_path: &str,
        new_path: &str,
        replace_if_exists: bool,
    ) -> Result<()> {
        Self::validate_path(new_path)?;
        let obj = self.resolve_path(old_path)?.ok_or(VfsError::NotFound)?;
        let (new_parent_path, new_name) = Self::split_parent_child(new_path)?;
        let new_parent_id = self.resolve_parent_id(&new_parent_path)?;

        // Cycle check: a directory cannot be moved into itself or any of its descendants
        if obj.kind == ObjectKind::Directory {
            if let Some(ref target_parent) = new_parent_id {
                if target_parent == &obj.id
                    || self.metadata.is_descendant_of(target_parent, &obj.id)?
                {
                    return Err(VfsError::InvalidPath);
                }
            }
        }

        if let Ok(Some(existing_target)) = self.resolve_path(new_path) {
            if existing_target.id != obj.id {
                if !replace_if_exists {
                    return Err(VfsError::AlreadyExists);
                }
                // Target must be replaced: if directory, it must be empty
                if existing_target.kind == ObjectKind::Directory {
                    if self.metadata.has_children(&existing_target.id)? {
                        return Err(VfsError::DirectoryNotEmpty);
                    }
                    self.rmdir(new_path).await?;
                } else {
                    self.unlink(new_path).await?;
                }
            }
        }

        let old_parent_id = obj.parent_id.clone();
        let same_parent = old_parent_id == new_parent_id;

        // Strip .url extension when renaming workspace shortcuts
        let effective_name = if GoogleWorkspaceShortcut::is_workspace(&obj) {
            GoogleWorkspaceShortcut::strip_shortcut_extension(&new_name)
                .unwrap_or(&new_name)
                .to_string()
        } else {
            new_name.clone()
        };

        if same_parent {
            if !obj.id.is_local() {
                self.provider.rename(&obj.id, &effective_name).await?;
            }
            self.metadata.rename_object(&obj.id, &effective_name)?;
        } else {
            if !obj.id.is_local() {
                let new_parent_ref = self.effective_parent_id(&new_parent_id)?;
                if effective_name != obj.remote_name {
                    self.provider.rename(&obj.id, &effective_name).await?;
                }
                self.provider.move_object(&obj.id, &new_parent_ref).await?;
            }
            self.metadata
                .move_object(&obj.id, new_parent_id.as_ref(), &effective_name)?;
        }

        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn unlink(&self, path: &str) -> Result<()> {
        let obj = self.resolve_path(path)?.ok_or(VfsError::NotFound)?;
        if obj.kind == ObjectKind::Directory {
            return Err(VfsError::IsDirectory);
        }

        self.metadata.mark_trashed(&obj.id)?;

        if obj.id.is_local() {
            if let Ok(entries) = self.metadata.list_uncommitted_staging() {
                for entry in entries {
                    if entry.file_id == obj.id {
                        let _ = self.metadata.remove_staging_entry(entry.handle_id);
                        let _ = std::fs::remove_file(&entry.staging_path);
                    }
                }
            }
        } else {
            self.metadata.enqueue_delete(&obj.id, None)?;
        }

        if let Some(ref cache) = self.chunk_cache {
            let _ = cache.invalidate_file(&obj.id).await;
        }
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn rmdir(&self, path: &str) -> Result<()> {
        let obj = self.resolve_path(path)?.ok_or(VfsError::NotFound)?;
        if obj.kind != ObjectKind::Directory {
            return Err(VfsError::NotDirectory);
        }

        if self.metadata.has_children(&obj.id)? {
            return Err(VfsError::DirectoryNotEmpty);
        }

        self.metadata.mark_trashed(&obj.id)?;

        if obj.id.is_local() {
            if let Ok(entries) = self.metadata.list_uncommitted_staging() {
                for entry in entries {
                    if entry.file_id == obj.id {
                        let _ = self.metadata.remove_staging_entry(entry.handle_id);
                    }
                }
            }
        } else {
            self.metadata.enqueue_delete(&obj.id, None)?;
        }

        Ok(())
    }

    pub fn statfs(&self) -> VfsStatFs {
        self.quota.read().map(|q| *q).unwrap_or_default()
    }

    #[instrument(skip(self), level = "info")]
    pub async fn recover_pending_uploads(&self) -> Result<usize> {
        let pending = self.metadata.list_uncommitted_staging()?;
        let preserved_paths: Vec<PathBuf> = pending
            .iter()
            .map(|e| PathBuf::from(&e.staging_path))
            .collect();

        self.staging.cleanup_orphans(&preserved_paths);

        let mut recovered = 0;
        for entry in pending {
            let path = PathBuf::from(&entry.staging_path);
            if !path.exists() {
                let _ = self.metadata.remove_staging_entry(entry.handle_id);
                continue;
            }

            info!(
                handle = entry.handle_id,
                file_id = %entry.file_id,
                path = %entry.staging_path,
                "recovering uncommitted staging file after crash"
            );

            let _ = self
                .metadata
                .update_staging_state(entry.handle_id, StagingState::Uploading);

            let metadata_clone = Arc::clone(&self.metadata);
            let handle_id = entry.handle_id;
            let on_session = Arc::new(move |session_uri: &str| {
                let _ = metadata_clone.update_staging_session(handle_id, session_uri, 0);
            });

            match self
                .provider
                .upload_file_resumable(
                    &entry.file_id,
                    &path,
                    entry.session_uri.as_deref(),
                    Some(on_session),
                )
                .await
            {
                Ok(updated_meta) => {
                    let new_size = updated_meta.size_bytes.unwrap_or(0);
                    let _ = self.metadata.update_size_mtime_version(
                        &entry.file_id,
                        new_size,
                        updated_meta.modified_at.as_deref(),
                        updated_meta.version.as_deref(),
                    );
                    if entry.file_id.is_local() {
                        let stored = StoredObject {
                            id: updated_meta.id.clone(),
                            parent_id: updated_meta
                                .parent_id
                                .clone()
                                .or_else(|| entry.parent_id.clone()),
                            name: updated_meta.name.clone(),
                            remote_name: updated_meta.name.clone(),
                            kind: updated_meta.kind,
                            size_bytes: updated_meta.size_bytes,
                            mime_type: updated_meta.mime_type.clone(),
                            created_at: updated_meta.created_at.clone(),
                            modified_at: updated_meta.modified_at.clone(),
                            version: updated_meta.version.clone(),
                            sync_status: SyncStatus::Synced,
                            deleted: false,
                        };
                        let _ = self.metadata.replace_file_id(&entry.file_id, &stored);
                    }
                    let _ = self.metadata.remove_staging_entry(entry.handle_id);
                    let _ = std::fs::remove_file(&path);
                    if let Some(ref cache) = self.chunk_cache {
                        let _ = cache.invalidate_file(&entry.file_id).await;
                    }
                    recovered += 1;
                }
                Err(e) => {
                    warn!(
                        handle = entry.handle_id,
                        file_id = %entry.file_id,
                        error = %e,
                        "failed to upload recovered staging file during crash recovery"
                    );
                    let _ = self
                        .metadata
                        .update_staging_state(entry.handle_id, StagingState::Failed);
                }
            }
        }

        Ok(recovered)
    }
}
