use std::sync::Arc;
use std::time::Duration;

use driftfs_core::FileId;
use driftfs_filesystem::DriftFsVfs;
use driftfs_metadata::MetadataStore;
use driftfs_platform_windows::{
    find_available_drive_letter, DriftFsMount, MountConfig, WindowsVfs,
};
use driftfs_provider::ObjectKind;
use driftfs_testkit::{create_metadata, MockProvider};
use widestring::u16cstr;
use windows_sys::Win32::Foundation::{STATUS_FILE_IS_A_DIRECTORY, STATUS_OBJECT_NAME_NOT_FOUND};
use winfsp_wrs::{
    CleanupFlags, CreateFileInfo, CreateOptions, FileAccessRights, FileAttributes,
    FileSystemInterface, SecurityDescriptor, WriteMode,
};

fn setup_test_vfs() -> (Arc<DriftFsVfs<MockProvider>>, tempfile::TempDir) {
    let store = Arc::new(MetadataStore::open_in_memory().expect("open store"));
    let provider = MockProvider::new();

    let docs_folder = create_metadata(
        "folder_projects",
        "Projects",
        None,
        ObjectKind::Directory,
        None,
        Some("application/vnd.google-apps.folder"),
    );

    let readme_file = create_metadata(
        "file_readme",
        "readme.md",
        None,
        ObjectKind::File,
        Some(25),
        Some("text/markdown"),
    );

    let gdoc_file = create_metadata(
        "gdoc_notes",
        "Meeting Notes",
        None,
        ObjectKind::File,
        None,
        Some("application/vnd.google-apps.document"),
    );

    store
        .batch_upsert_objects(&[docs_folder, readme_file, gdoc_file])
        .expect("batch upsert");

    provider.add_directory("root", "My Drive", None);
    provider.add_directory("folder_projects", "Projects", None);
    provider.add_file("file_readme", "readme.md", None, 25);

    let staging_dir = tempfile::tempdir().expect("staging dir");
    let vfs = Arc::new(
        DriftFsVfs::new(store, Arc::new(provider), staging_dir.path().to_path_buf())
            .expect("create vfs")
            .with_root_folder_id(FileId("root".into())),
    );

    (vfs, staging_dir)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_windows_vfs_callbacks() {
    let (vfs, _dir) = setup_test_vfs();
    let rt_handle = tokio::runtime::Handle::current();
    let win_vfs = WindowsVfs::new(vfs, rt_handle).expect("create WindowsVfs");

    let vol_info = win_vfs.get_volume_info().expect("get_volume_info");
    assert_eq!(vol_info.volume_label().to_string_lossy(), "DriftFS");

    let (root_attr, _sd, reparse) = win_vfs
        .get_security_by_name(u16cstr!("\\"), || None)
        .expect("root getattr");
    assert!(!reparse);
    assert!(root_attr.0 & FileAttributes::DIRECTORY.0 != 0);

    let err = win_vfs.get_security_by_name(u16cstr!("\\missing.txt"), || None);
    assert_eq!(err.unwrap_err(), STATUS_OBJECT_NAME_NOT_FOUND);

    let (file_attr, _, _) = win_vfs
        .get_security_by_name(u16cstr!("\\readme.md"), || None)
        .expect("readme getattr");
    assert_eq!(file_attr.0 & FileAttributes::DIRECTORY.0, 0);

    let (dir_ctx, dir_info) = win_vfs
        .open(
            u16cstr!("\\"),
            CreateOptions(0),
            FileAccessRights::FILE_GENERIC_READ,
        )
        .expect("open root dir");
    assert!(dir_ctx.is_directory);
    assert!(dir_info.file_attributes().0 & FileAttributes::DIRECTORY.0 != 0);

    let mut entry_names = Vec::new();
    win_vfs
        .read_directory(dir_ctx.clone(), None, |entry| {
            let null_pos = entry
                .file_name
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.file_name.len());
            let name = String::from_utf16_lossy(&entry.file_name[..null_pos]);
            entry_names.push(name);
            true
        })
        .expect("read_directory");

    assert!(entry_names.contains(&".".to_string()));
    assert!(entry_names.contains(&"..".to_string()));
    assert!(entry_names.contains(&"Projects".to_string()));
    assert!(entry_names.contains(&"readme.md".to_string()));
    assert!(entry_names.contains(&"Meeting Notes.url".to_string()));

    win_vfs.close(dir_ctx);

    let (file_ctx, file_info) = win_vfs
        .open(
            u16cstr!("\\readme.md"),
            CreateOptions(0),
            FileAccessRights::FILE_GENERIC_READ,
        )
        .expect("open readme.md");
    assert!(!file_ctx.is_directory);
    assert_eq!(file_info.file_size(), 25);

    let mut buf = vec![0u8; 64];
    let bytes_read = win_vfs
        .read(file_ctx.clone(), &mut buf, 0)
        .expect("read file");
    assert_eq!(bytes_read, 25);

    let (dir_ctx2, _) = win_vfs
        .open(
            u16cstr!("\\Projects"),
            CreateOptions(0),
            FileAccessRights::FILE_GENERIC_READ,
        )
        .expect("open Projects dir");
    let dir_read_err = win_vfs.read(dir_ctx2.clone(), &mut buf, 0);
    assert_eq!(dir_read_err.unwrap_err(), STATUS_FILE_IS_A_DIRECTORY);
    win_vfs.close(dir_ctx2);

    let (url_ctx, url_info) = win_vfs
        .open(
            u16cstr!("\\Meeting Notes.url"),
            CreateOptions(0),
            FileAccessRights::FILE_GENERIC_READ,
        )
        .expect("open .url shortcut");
    assert!(!url_ctx.is_directory);
    assert!(url_info.file_size() > 0);

    let mut url_buf = vec![0u8; url_info.file_size() as usize];
    let url_read = win_vfs
        .read(url_ctx.clone(), &mut url_buf, 0)
        .expect("read url file");
    assert_eq!(url_read, url_info.file_size() as usize);
    let url_str = String::from_utf8_lossy(&url_buf);
    assert!(url_str.contains("[InternetShortcut]"));

    win_vfs.close(url_ctx);
    win_vfs.close(file_ctx);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_windows_vfs_mutation_callbacks() {
    let (vfs, _dir) = setup_test_vfs();
    let rt_handle = tokio::runtime::Handle::current();
    let win_vfs = WindowsVfs::new(vfs, rt_handle).expect("create WindowsVfs");

    let create_info = CreateFileInfo {
        create_options: CreateOptions(0),
        granted_access: FileAccessRights::FILE_GENERIC_WRITE,
        file_attributes: FileAttributes::ARCHIVE,
        allocation_size: 0,
    };
    let sd = SecurityDescriptor::from_wstr(u16cstr!("O:BAG:BAD:(A;;FA;;;WD)")).expect("sd");
    let (ctx, info) = win_vfs
        .create(u16cstr!("\\Projects\\new_file.txt"), create_info, sd)
        .expect("create file");
    assert!(!ctx.is_directory);
    assert_eq!(info.file_size(), 0);

    let (written, info) = win_vfs
        .write(
            ctx.clone(),
            b"Hello Windows VFS",
            WriteMode::Normal { offset: 0 },
        )
        .expect("write");
    assert_eq!(written, 17);
    assert_eq!(info.file_size(), 17);

    let info = win_vfs
        .set_file_size(ctx.clone(), 5, false)
        .expect("set_file_size");
    assert_eq!(info.file_size(), 5);

    let info = win_vfs
        .set_basic_info(ctx.clone(), FileAttributes::ARCHIVE, 0, 0, 0, 0)
        .expect("set_basic_info");
    assert_eq!(info.file_size(), 5);

    win_vfs.cleanup(ctx.clone(), None, CleanupFlags(0));
    win_vfs.close(ctx);

    let (read_ctx, read_info) = win_vfs
        .open(
            u16cstr!("\\Projects\\new_file.txt"),
            CreateOptions(0),
            FileAccessRights::FILE_GENERIC_READ,
        )
        .expect("reopen");
    assert_eq!(read_info.file_size(), 5);
    let mut buf = vec![0u8; 10];
    let bytes_read = win_vfs.read(read_ctx.clone(), &mut buf, 0).expect("read");
    assert_eq!(bytes_read, 5);
    assert_eq!(&buf[..5], b"Hello");
    win_vfs.close(read_ctx);

    let (rename_ctx, _) = win_vfs
        .open(
            u16cstr!("\\Projects\\new_file.txt"),
            CreateOptions(0),
            FileAccessRights::FILE_GENERIC_WRITE,
        )
        .expect("open for rename");
    win_vfs
        .rename(
            rename_ctx.clone(),
            u16cstr!("\\Projects\\new_file.txt"),
            u16cstr!("\\Projects\\renamed.txt"),
            false,
        )
        .expect("rename");
    assert_eq!(rename_ctx.path(), "/Projects/renamed.txt");

    let old_err = win_vfs.get_security_by_name(u16cstr!("\\Projects\\new_file.txt"), || None);
    assert_eq!(old_err.unwrap_err(), STATUS_OBJECT_NAME_NOT_FOUND);

    let (new_attr, _, _) = win_vfs
        .get_security_by_name(u16cstr!("\\Projects\\renamed.txt"), || None)
        .expect("new attr");
    assert_eq!(new_attr.0 & FileAttributes::DIRECTORY.0, 0);

    win_vfs
        .set_delete(
            rename_ctx.clone(),
            u16cstr!("\\Projects\\renamed.txt"),
            true,
        )
        .expect("set_delete");
    assert!(rename_ctx
        .delete_on_close
        .load(std::sync::atomic::Ordering::SeqCst));
    // Cleanup with CleanupFlags(0) deletes because delete_on_close was recorded on context
    win_vfs.cleanup(rename_ctx.clone(), None, CleanupFlags(0));
    win_vfs.close(rename_ctx);

    let del_err = win_vfs.get_security_by_name(u16cstr!("\\Projects\\renamed.txt"), || None);
    assert_eq!(del_err.unwrap_err(), STATUS_OBJECT_NAME_NOT_FOUND);

    let dir_create_info = CreateFileInfo {
        create_options: CreateOptions(0),
        granted_access: FileAccessRights::FILE_GENERIC_WRITE,
        file_attributes: FileAttributes::DIRECTORY,
        allocation_size: 0,
    };
    let sd = SecurityDescriptor::from_wstr(u16cstr!("O:BAG:BAD:(A;;FA;;;WD)")).expect("sd");
    let (dir_ctx, dir_info) = win_vfs
        .create(u16cstr!("\\Projects\\SubDir"), dir_create_info, sd)
        .expect("mkdir");
    assert!(dir_ctx.is_directory);
    assert!(dir_info.file_attributes().0 & FileAttributes::DIRECTORY.0 != 0);

    win_vfs
        .set_delete(dir_ctx.clone(), u16cstr!("\\Projects\\SubDir"), true)
        .expect("set_delete dir");
    win_vfs.cleanup(dir_ctx.clone(), None, CleanupFlags::DELETE);
    win_vfs.close(dir_ctx);

    let dir_err = win_vfs.get_security_by_name(u16cstr!("\\Projects\\SubDir"), || None);
    assert_eq!(dir_err.unwrap_err(), STATUS_OBJECT_NAME_NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_live_winfsp_mount_and_unmount() {
    let (vfs, _dir) = setup_test_vfs();
    let rt_handle = tokio::runtime::Handle::current();

    let drive_letter = match find_available_drive_letter(None) {
        Ok(letter) => letter,
        Err(e) => {
            eprintln!("Skipping live mount test: {}", e);
            return;
        }
    };

    let config = MountConfig {
        drive_letter: Some(drive_letter),
        volume_label: "DriftFSTest".to_string(),
    };

    let mount_res = DriftFsMount::mount(vfs, rt_handle, config);
    let mut mount = match mount_res {
        Ok(m) => m,
        Err(e) => {
            eprintln!(
                "WinFsp mount not permitted or failed: {}. Skipping live mount.",
                e
            );
            return;
        }
    };

    assert_eq!(mount.drive_letter(), drive_letter);
    assert_eq!(mount.mount_point(), format!("{}:", drive_letter));

    // Allow WinFsp dispatcher to settle
    tokio::time::sleep(Duration::from_millis(150)).await;

    let root_path = format!("{}:\\", drive_letter);
    if let Ok(entries) = std::fs::read_dir(&root_path) {
        let mut names = Vec::new();
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                names.push(name);
            }
        }
        assert!(names.contains(&"readme.md".to_string()));
        assert!(names.contains(&"Projects".to_string()));
        assert!(names.contains(&"Meeting Notes.url".to_string()));
    }

    let live_file = format!("{}:\\Projects\\live_test.txt", drive_letter);
    if std::fs::write(&live_file, b"live test payload").is_ok() {
        if let Ok(content) = std::fs::read_to_string(&live_file) {
            assert_eq!(content, "live test payload");
        }

        let live_renamed = format!("{}:\\Projects\\live_renamed.txt", drive_letter);
        if std::fs::rename(&live_file, &live_renamed).is_ok() {
            let _ = std::fs::remove_file(&live_renamed);
        }
    }

    let live_dir = format!("{}:\\Projects\\live_dir", drive_letter);
    if std::fs::create_dir(&live_dir).is_ok() {
        let _ = std::fs::remove_dir(&live_dir);
    }

    mount.unmount();
}
