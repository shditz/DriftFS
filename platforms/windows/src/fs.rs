use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use driftfs_filesystem::{DriftFsVfs, OpenFlags, VfsAttr, VfsDirEntry, VfsHandle, VfsNodeType};
use driftfs_provider::CloudProvider;
use tracing::{debug, instrument, warn};
use widestring::{u16cstr, U16CStr, U16CString};
use windows_sys::Win32::Foundation::{
    STATUS_FILE_IS_A_DIRECTORY, STATUS_NOT_A_DIRECTORY, STATUS_UNSUCCESSFUL,
};
use winfsp_wrs::{
    CleanupFlags, CreateFileInfo, CreateOptions, DirInfo, FileAccessRights, FileAttributes,
    FileInfo, FileSystemInterface, PSecurityDescriptor, SecurityDescriptor, VolumeInfo, WriteMode,
};

use crate::context::WindowsFileContext;
use crate::error::PlatformError;

const UNIX_EPOCH_FILETIME: u64 = 116_444_736_000_000_000;
const HECTONANOSECONDS_PER_SEC: u64 = 10_000_000;

pub(crate) fn system_time_to_filetime(time: Option<SystemTime>) -> u64 {
    let Some(time) = time else {
        return UNIX_EPOCH_FILETIME;
    };

    match time.duration_since(UNIX_EPOCH) {
        Ok(dur) => {
            let secs = dur.as_secs();
            let nsec = dur.subsec_nanos() as u64;
            let hecto = secs
                .saturating_mul(HECTONANOSECONDS_PER_SEC)
                .saturating_add(nsec / 100);
            UNIX_EPOCH_FILETIME.saturating_add(hecto)
        }
        Err(_) => UNIX_EPOCH_FILETIME,
    }
}

pub(crate) fn to_vfs_path(file_name: &U16CStr) -> String {
    let raw = file_name.to_string_lossy();
    let normalized = raw.replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn build_file_info(
    ino: u64,
    kind: VfsNodeType,
    size: u64,
    ctime: u64,
    mtime: u64,
) -> FileInfo {
    let mut info = FileInfo::default();

    let is_dir = kind == VfsNodeType::Directory;
    let mut flags = if is_dir {
        FileAttributes::DIRECTORY
    } else {
        FileAttributes::ARCHIVE
    };

    if kind == VfsNodeType::GoogleDocShortcut {
        flags |= FileAttributes::READONLY;
    }

    info.set_file_attributes(flags);
    info.set_file_size(size);

    let alloc_size = if is_dir {
        0
    } else {
        (size.saturating_add(4095) / 4096) * 4096
    };
    info.set_allocation_size(alloc_size);

    info.set_creation_time(ctime);
    info.set_last_access_time(mtime);
    info.set_last_write_time(mtime);
    info.set_change_time(mtime);
    info.set_index_number(ino);
    info.set_hard_links(1);

    info
}

pub(crate) fn vfs_attr_to_file_info(attr: &VfsAttr) -> FileInfo {
    let ctime = system_time_to_filetime(attr.created_at);
    let mtime = system_time_to_filetime(attr.modified_at);
    build_file_info(attr.ino, attr.kind, attr.size, ctime, mtime)
}

pub(crate) fn dir_entry_to_file_info(entry: &VfsDirEntry, parent_time: u64) -> FileInfo {
    build_file_info(entry.ino, entry.kind, entry.size, parent_time, parent_time)
}

pub struct WindowsVfs<P: CloudProvider> {
    vfs: Arc<DriftFsVfs<P>>,
    rt_handle: tokio::runtime::Handle,
    security_descriptor: SecurityDescriptor,
}

impl<P: CloudProvider + 'static> WindowsVfs<P> {
    pub fn new(
        vfs: Arc<DriftFsVfs<P>>,
        rt_handle: tokio::runtime::Handle,
    ) -> Result<Self, PlatformError> {
        let sddl = u16cstr!("O:BAG:BAD:(A;;FA;;;WD)");
        let security_descriptor =
            SecurityDescriptor::from_wstr(sddl).map_err(PlatformError::SecurityDescriptor)?;

        Ok(Self {
            vfs,
            rt_handle,
            security_descriptor,
        })
    }

    fn block_on_async<F: std::future::Future>(&self, fut: F) -> F::Output {
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::block_in_place(|| self.rt_handle.block_on(fut))
        } else {
            self.rt_handle.block_on(fut)
        }
    }

    fn apply_staging_size(&self, file_info: &mut FileInfo, handle: VfsHandle) {
        if let Ok(entry) = self.vfs.handles().get(handle) {
            if entry.staging_path.is_some() {
                file_info.set_file_size(entry.size);
                let alloc_size = (entry.size.saturating_add(4095) / 4096) * 4096;
                file_info.set_allocation_size(alloc_size);
            }
        }
    }
}

impl<P: CloudProvider + 'static> FileSystemInterface for WindowsVfs<P> {
    type FileContext = Arc<WindowsFileContext>;

    const GET_VOLUME_INFO_DEFINED: bool = true;
    const GET_SECURITY_BY_NAME_DEFINED: bool = true;
    const GET_SECURITY_DEFINED: bool = true;
    const OPEN_DEFINED: bool = true;
    const CLOSE_DEFINED: bool = true;
    const GET_FILE_INFO_DEFINED: bool = true;
    const READ_DIRECTORY_DEFINED: bool = true;
    const READ_DEFINED: bool = true;
    const CREATE_DEFINED: bool = true;
    const WRITE_DEFINED: bool = true;
    const CLEANUP_DEFINED: bool = true;
    const SET_DELETE_DEFINED: bool = true;
    const RENAME_DEFINED: bool = true;
    const SET_FILE_SIZE_DEFINED: bool = true;
    const SET_BASIC_INFO_DEFINED: bool = true;
    const OVERWRITE_DEFINED: bool = true;

    #[instrument(skip(self), level = "trace")]
    fn get_volume_info(&self) -> Result<VolumeInfo, i32> {
        let stat = self.vfs.statfs();
        let label = U16CString::from_str("DriftFS").map_err(|_| STATUS_UNSUCCESSFUL)?;
        VolumeInfo::new(stat.total_space, stat.free_space, label.as_ustr())
            .map_err(|_| STATUS_UNSUCCESSFUL)
    }

    #[instrument(skip(self, _find_reparse_point), level = "trace")]
    fn get_security_by_name(
        &self,
        file_name: &U16CStr,
        _find_reparse_point: impl Fn() -> Option<FileAttributes>,
    ) -> Result<(FileAttributes, PSecurityDescriptor, bool), i32> {
        let path = to_vfs_path(file_name);
        match self.vfs.getattr(&path) {
            Ok(attr) => {
                let mut flags = if attr.kind == VfsNodeType::Directory {
                    FileAttributes::DIRECTORY
                } else {
                    FileAttributes::ARCHIVE
                };
                if attr.kind == VfsNodeType::GoogleDocShortcut {
                    flags |= FileAttributes::READONLY;
                }
                Ok((flags, self.security_descriptor.as_ptr(), false))
            }
            Err(e) => Err(e.ntstatus()),
        }
    }

    #[instrument(skip(self), level = "trace")]
    fn get_security(&self, _file_context: Self::FileContext) -> Result<PSecurityDescriptor, i32> {
        Ok(self.security_descriptor.as_ptr())
    }

    #[instrument(skip(self, _security_descriptor), level = "debug")]
    fn create(
        &self,
        file_name: &U16CStr,
        create_file_info: CreateFileInfo,
        _security_descriptor: SecurityDescriptor,
    ) -> Result<(Self::FileContext, FileInfo), i32> {
        let path = to_vfs_path(file_name);
        let is_directory = create_file_info.file_attributes.0 & FileAttributes::DIRECTORY.0 != 0;

        if is_directory {
            let attr = self
                .block_on_async(self.vfs.mkdir(&path))
                .map_err(|e| e.ntstatus())?;

            let file_info = vfs_attr_to_file_info(&attr);
            let handle = self.vfs.opendir(&path).map_err(|e| e.ntstatus())?;

            let context = Arc::new(WindowsFileContext::new(handle, true, attr.ino, path));

            Ok((context, file_info))
        } else {
            let segments = DriftFsVfs::<P>::normalize_path(&path);
            if segments.is_empty() {
                return Err(STATUS_UNSUCCESSFUL);
            }
            let name = segments.last().ok_or(STATUS_UNSUCCESSFUL)?.to_string();
            let parent_path = if segments.len() == 1 {
                "/".to_string()
            } else {
                format!("/{}", segments[..segments.len() - 1].join("/"))
            };

            let (handle, attr) = self
                .block_on_async(self.vfs.create_file(&parent_path, &name))
                .map_err(|e| e.ntstatus())?;

            let file_info = vfs_attr_to_file_info(&attr);
            let context = Arc::new(WindowsFileContext::new(handle, false, attr.ino, path));

            Ok((context, file_info))
        }
    }

    #[instrument(skip(self, _create_options, granted_access), level = "debug")]
    fn open(
        &self,
        file_name: &U16CStr,
        _create_options: CreateOptions,
        granted_access: FileAccessRights,
    ) -> Result<(Self::FileContext, FileInfo), i32> {
        let path = to_vfs_path(file_name);
        let attr = self.vfs.getattr(&path).map_err(|e| e.ntstatus())?;

        let is_write = (granted_access.0
            & (FileAccessRights::FILE_WRITE_DATA.0 | FileAccessRights::FILE_APPEND_DATA.0))
            != 0;

        let flags = if is_write {
            OpenFlags {
                read: true,
                write: true,
                truncate: false,
                create: false,
                append: (granted_access.0 & FileAccessRights::FILE_APPEND_DATA.0) != 0,
            }
        } else {
            OpenFlags::read_only()
        };

        let (handle, is_directory) = if attr.kind == VfsNodeType::Directory {
            let h = self.vfs.opendir(&path).map_err(|e| e.ntstatus())?;
            (h, true)
        } else {
            let h = self
                .block_on_async(self.vfs.open(&path, flags))
                .map_err(|e| e.ntstatus())?;
            (h, false)
        };

        let file_info = vfs_attr_to_file_info(&attr);
        let context = Arc::new(WindowsFileContext::new(
            handle,
            is_directory,
            attr.ino,
            path,
        ));

        Ok((context, file_info))
    }

    #[instrument(
        skip(self, _file_attributes, _replace_file_attributes),
        level = "debug"
    )]
    fn overwrite(
        &self,
        file_context: Self::FileContext,
        _file_attributes: FileAttributes,
        _replace_file_attributes: bool,
        _allocation_size: u64,
    ) -> Result<FileInfo, i32> {
        if file_context.is_directory {
            return Err(STATUS_FILE_IS_A_DIRECTORY);
        }

        self.vfs
            .set_length(file_context.handle, 0)
            .map_err(|e| e.ntstatus())?;

        let path = file_context.path();
        let attr = self.vfs.getattr(&path).map_err(|e| e.ntstatus())?;
        let mut file_info = vfs_attr_to_file_info(&attr);
        file_info.set_file_size(0);
        file_info.set_allocation_size(0);
        Ok(file_info)
    }

    #[instrument(skip(self, file_context), level = "debug")]
    fn close(&self, file_context: Self::FileContext) {
        if let Err(e) = self.vfs.close(file_context.handle) {
            debug!(
                handle = file_context.handle.0,
                error = %e,
                "close returned non-fatal error"
            );
        }
    }

    #[instrument(skip(self, file_context, _file_name, flags), level = "debug")]
    fn cleanup(
        &self,
        file_context: Self::FileContext,
        _file_name: Option<&U16CStr>,
        flags: CleanupFlags,
    ) {
        let is_delete = (flags.0 & CleanupFlags::DELETE.0) != 0
            || file_context
                .delete_on_close
                .load(std::sync::atomic::Ordering::SeqCst);
        let path = file_context.path();

        if is_delete {
            let result = if file_context.is_directory {
                self.block_on_async(self.vfs.rmdir(&path))
            } else {
                self.block_on_async(self.vfs.unlink(&path))
            };

            if let Err(e) = result {
                warn!(
                    path = %path,
                    error = %e,
                    "cleanup delete failed"
                );
            }
            return;
        }

        if !file_context.is_directory {
            if let Err(e) = self.block_on_async(self.vfs.flush(file_context.handle)) {
                warn!(
                    path = %path,
                    error = %e,
                    "cleanup flush failed"
                );
            }
        }
    }

    #[instrument(skip(self, file_context), level = "trace")]
    fn get_file_info(&self, file_context: Self::FileContext) -> Result<FileInfo, i32> {
        let path = file_context.path();
        let attr = self.vfs.getattr(&path).map_err(|e| e.ntstatus())?;
        let mut file_info = vfs_attr_to_file_info(&attr);
        if !file_context.is_directory {
            self.apply_staging_size(&mut file_info, file_context.handle);
        }
        Ok(file_info)
    }

    #[instrument(skip(self, file_context), level = "debug")]
    fn set_delete(
        &self,
        file_context: Self::FileContext,
        _file_name: &U16CStr,
        delete_file: bool,
    ) -> Result<(), i32> {
        if file_context.is_directory && delete_file {
            if let Some(id) = self
                .vfs
                .handles()
                .get(file_context.handle)
                .ok()
                .and_then(|e| e.file_id)
            {
                if self.vfs.metadata().has_children(&id).unwrap_or(false) {
                    return Err(driftfs_filesystem::VfsError::DirectoryNotEmpty.ntstatus());
                }
            }
        }

        file_context
            .delete_on_close
            .store(delete_file, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    #[instrument(skip(self, file_context, add_dir_info), level = "debug")]
    fn read_directory(
        &self,
        file_context: Self::FileContext,
        marker: Option<&U16CStr>,
        mut add_dir_info: impl FnMut(DirInfo) -> bool,
    ) -> Result<(), i32> {
        if !file_context.is_directory {
            return Err(STATUS_NOT_A_DIRECTORY);
        }

        let path = file_context.path();
        let dir_attr = self.vfs.getattr(&path).map_err(|e| e.ntstatus())?;
        let dir_time = system_time_to_filetime(dir_attr.modified_at);
        let mut children = self
            .block_on_async(self.vfs.readdir(file_context.handle))
            .map_err(|e| e.ntstatus())?;

        children.sort_unstable_by_key(|a| a.name.to_lowercase());

        let parent_path = match path.as_str() {
            "/" => "/".to_string(),
            p => match p.rfind('/') {
                Some(0) | None => "/".to_string(),
                Some(pos) => p[..pos].to_string(),
            },
        };
        let parent_attr = self
            .vfs
            .getattr(&parent_path)
            .unwrap_or_else(|_| dir_attr.clone());

        let dot_info = DirInfo::from_str(vfs_attr_to_file_info(&dir_attr), ".");
        let dot_dot_info = DirInfo::from_str(vfs_attr_to_file_info(&parent_attr), "..");

        let mut all_entries = Vec::with_capacity(children.len() + 2);
        all_entries.push((".", dot_info));
        all_entries.push(("..", dot_dot_info));

        let child_entries: Vec<(String, DirInfo)> = children
            .into_iter()
            .map(|c| {
                let info = DirInfo::from_str(dir_entry_to_file_info(&c, dir_time), &c.name);
                (c.name, info)
            })
            .collect();

        for (name, info) in &child_entries {
            all_entries.push((name.as_str(), *info));
        }

        let start_index = match marker {
            Some(marker_cstr) => {
                let marker_str = marker_cstr.to_string_lossy();
                let marker_lower = marker_str.to_lowercase();
                if let Some(pos) = all_entries
                    .iter()
                    .position(|(n, _)| n.eq_ignore_ascii_case(&marker_str))
                {
                    pos + 1
                } else {
                    all_entries
                        .iter()
                        .position(|(n, _)| n.to_lowercase() > marker_lower)
                        .unwrap_or(all_entries.len())
                }
            }
            None => 0,
        };

        for (_, info) in all_entries.into_iter().skip(start_index) {
            if !add_dir_info(info) {
                break;
            }
        }

        Ok(())
    }

    #[instrument(skip(self, file_context, buffer), level = "trace")]
    fn read(
        &self,
        file_context: Self::FileContext,
        buffer: &mut [u8],
        offset: u64,
    ) -> Result<usize, i32> {
        if file_context.is_directory {
            return Err(STATUS_FILE_IS_A_DIRECTORY);
        }

        if buffer.is_empty() {
            return Ok(0);
        }

        let bytes = self
            .block_on_async(self.vfs.read(file_context.handle, offset, buffer.len()))
            .map_err(|e| e.ntstatus())?;

        let to_copy = bytes.len().min(buffer.len());
        buffer[..to_copy].copy_from_slice(&bytes[..to_copy]);
        Ok(to_copy)
    }

    #[instrument(skip(self, file_context, buffer), level = "trace")]
    fn write(
        &self,
        file_context: Self::FileContext,
        buffer: &[u8],
        mode: WriteMode,
    ) -> Result<(usize, FileInfo), i32> {
        if file_context.is_directory {
            return Err(STATUS_FILE_IS_A_DIRECTORY);
        }

        let offset = match mode {
            WriteMode::Normal { offset } => offset,
            WriteMode::ConstrainedIO { offset } => offset,
            WriteMode::WriteToEOF => {
                let entry = self
                    .vfs
                    .handles()
                    .get(file_context.handle)
                    .map_err(|e| e.ntstatus())?;
                entry.size
            }
        };

        let written = self
            .vfs
            .write(file_context.handle, offset, buffer)
            .map_err(|e| e.ntstatus())?;

        let path = file_context.path();
        let attr = self.vfs.getattr(&path).map_err(|e| e.ntstatus())?;

        let mut file_info = vfs_attr_to_file_info(&attr);
        if let Ok(entry) = self.vfs.handles().get(file_context.handle) {
            file_info.set_file_size(entry.size);
            let alloc_size = (entry.size.saturating_add(4095) / 4096) * 4096;
            file_info.set_allocation_size(alloc_size);
        }

        Ok((written, file_info))
    }

    #[instrument(skip(self, file_context), level = "debug")]
    fn rename(
        &self,
        file_context: Self::FileContext,
        _file_name: &U16CStr,
        new_file_name: &U16CStr,
        replace_if_exists: bool,
    ) -> Result<(), i32> {
        let new_path = to_vfs_path(new_file_name);
        let current_path = file_context.path();
        self.block_on_async(self.vfs.rename(&current_path, &new_path, replace_if_exists))
            .map_err(|e| e.ntstatus())?;
        file_context.set_path(new_path);
        Ok(())
    }

    #[instrument(skip(self, file_context), level = "debug")]
    fn set_file_size(
        &self,
        file_context: Self::FileContext,
        new_size: u64,
        set_allocation_size: bool,
    ) -> Result<FileInfo, i32> {
        if file_context.is_directory {
            return Err(STATUS_FILE_IS_A_DIRECTORY);
        }

        if !set_allocation_size {
            self.vfs
                .set_length(file_context.handle, new_size)
                .map_err(|e| e.ntstatus())?;
        }

        let path = file_context.path();
        let attr = self.vfs.getattr(&path).map_err(|e| e.ntstatus())?;
        let mut file_info = vfs_attr_to_file_info(&attr);
        if let Ok(entry) = self.vfs.handles().get(file_context.handle) {
            file_info.set_file_size(entry.size);
            let alloc_size = (entry.size.saturating_add(4095) / 4096) * 4096;
            file_info.set_allocation_size(alloc_size);
        }
        Ok(file_info)
    }

    #[instrument(skip(self, file_context), level = "debug")]
    fn set_basic_info(
        &self,
        file_context: Self::FileContext,
        _file_attributes: FileAttributes,
        _creation_time: u64,
        _last_access_time: u64,
        _last_write_time: u64,
        _change_time: u64,
    ) -> Result<FileInfo, i32> {
        let path = file_context.path();
        let attr = self.vfs.getattr(&path).map_err(|e| e.ntstatus())?;
        let mut file_info = vfs_attr_to_file_info(&attr);
        if !file_context.is_directory {
            self.apply_staging_size(&mut file_info, file_context.handle);
        }
        Ok(file_info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_to_vfs_path_normalization() {
        assert_eq!(to_vfs_path(u16cstr!("\\")), "/");
        assert_eq!(to_vfs_path(u16cstr!("")), "/");
        assert_eq!(to_vfs_path(u16cstr!("\\folder")), "/folder");
        assert_eq!(to_vfs_path(u16cstr!("\\folder\\")), "/folder");
        assert_eq!(to_vfs_path(u16cstr!("\\a\\b\\c.txt")), "/a/b/c.txt");
    }

    #[test]
    fn test_system_time_to_filetime() {
        assert_eq!(system_time_to_filetime(None), UNIX_EPOCH_FILETIME);
        assert_eq!(
            system_time_to_filetime(Some(UNIX_EPOCH)),
            UNIX_EPOCH_FILETIME
        );

        let one_sec_later = UNIX_EPOCH + Duration::from_secs(1);
        assert_eq!(
            system_time_to_filetime(Some(one_sec_later)),
            UNIX_EPOCH_FILETIME + HECTONANOSECONDS_PER_SEC
        );
    }

    #[test]
    fn test_security_descriptor_creation() {
        let sddl = u16cstr!("O:BAG:BAD:(A;;FA;;;WD)");
        let sd = SecurityDescriptor::from_wstr(sddl);
        assert!(sd.is_ok(), "Security descriptor should parse successfully");
    }
}
