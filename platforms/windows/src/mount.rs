use std::sync::Arc;

use driftfs_filesystem::DriftFsVfs;
use driftfs_provider::CloudProvider;
use tracing::{info, instrument};
use widestring::U16CString;
use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;
use winfsp_wrs::{FileSystem, OperationGuardStrategy, Params};

use crate::error::PlatformError;
use crate::fs::WindowsVfs;

#[derive(Debug, Clone)]
pub struct MountConfig {
    pub drive_letter: Option<char>,

    pub volume_label: String,
}

impl Default for MountConfig {
    fn default() -> Self {
        Self {
            drive_letter: None,
            volume_label: "DriftFS".to_string(),
        }
    }
}

pub fn find_available_drive_letter(preferred: Option<char>) -> Result<char, PlatformError> {
    let mask = unsafe { GetLogicalDrives() };

    if let Some(letter) = preferred {
        let upper = letter.to_ascii_uppercase();
        if !upper.is_ascii_uppercase() {
            return Err(PlatformError::InvalidDriveLetter(upper));
        }

        let bit = 1u32 << (upper as u8 - b'A');
        if (mask & bit) == 0 {
            return Ok(upper);
        }
        return Err(PlatformError::DriveLetterInUse(upper));
    }

    for upper in ('G'..='Z').chain('D'..='F') {
        let bit = 1u32 << (upper as u8 - b'A');
        if (mask & bit) == 0 {
            return Ok(upper);
        }
    }

    Err(PlatformError::NoDriveLetterAvailable)
}

pub struct DriftFsMount {
    drive_letter: char,
    mount_point: String,
    file_system: Option<FileSystem>,
}

impl DriftFsMount {
    #[instrument(skip(vfs, rt_handle), level = "info")]
    pub fn mount<P: CloudProvider + 'static>(
        vfs: Arc<DriftFsVfs<P>>,
        rt_handle: tokio::runtime::Handle,
        config: MountConfig,
    ) -> Result<Self, PlatformError> {
        winfsp_wrs::init().map_err(PlatformError::InitFailed)?;

        let drive_letter = find_available_drive_letter(config.drive_letter)?;
        let mount_point = format!("{}:", drive_letter);
        let mountpoint_cstr = U16CString::from_str(&mount_point)
            .map_err(|e| PlatformError::InvalidVolumeLabel(e.to_string()))?;

        let mut params = Params::default();
        params.volume_params.set_read_only_volume(false);
        params.volume_params.set_case_preserved_names(true);
        params.volume_params.set_case_sensitive_search(false);
        params.volume_params.set_unicode_on_disk(true);
        params.volume_params.set_persistent_acls(false);
        params.volume_params.set_sector_size(4096);
        params.volume_params.set_sectors_per_allocation_unit(1);
        params.volume_params.set_max_component_length(255);

        let label_cstr = U16CString::from_str(&config.volume_label)
            .map_err(|e| PlatformError::InvalidVolumeLabel(e.to_string()))?;
        let _ = params.volume_params.set_file_system_name(&label_cstr);
        params.guard_strategy = OperationGuardStrategy::Fine;

        let win_vfs = WindowsVfs::new(vfs, rt_handle)?;
        let fs = FileSystem::start(params, Some(&mountpoint_cstr), win_vfs)
            .map_err(PlatformError::MountFailed)?;

        info!(
            mount_point = %mount_point,
            volume_label = %config.volume_label,
            "DriftFS mounted successfully onto Windows drive letter"
        );

        Ok(Self {
            drive_letter,
            mount_point,
            file_system: Some(fs),
        })
    }

    pub fn drive_letter(&self) -> char {
        self.drive_letter
    }

    pub fn mount_point(&self) -> &str {
        &self.mount_point
    }

    pub fn unmount(&mut self) {
        if let Some(fs) = self.file_system.take() {
            info!(mount_point = %self.mount_point, "Unmounting DriftFS virtual drive");
            fs.stop();
        }
    }
}

impl Drop for DriftFsMount {
    fn drop(&mut self) {
        self.unmount();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_available_drive_letter_default() {
        let res = find_available_drive_letter(None);
        assert!(res.is_ok());
        let ch = res.unwrap();
        assert!(('D'..='Z').contains(&ch));
    }

    #[test]
    fn test_find_available_drive_letter_in_use() {
        let res = find_available_drive_letter(Some('C'));
        assert!(matches!(res, Err(PlatformError::DriveLetterInUse('C'))));
    }

    #[test]
    fn test_find_available_drive_letter_invalid() {
        let res = find_available_drive_letter(Some('1'));
        assert!(matches!(res, Err(PlatformError::InvalidDriveLetter('1'))));

        let res = find_available_drive_letter(Some('?'));
        assert!(matches!(res, Err(PlatformError::InvalidDriveLetter('?'))));
    }
}
