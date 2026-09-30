use std::sync::Arc;

use driftfs_core::FileId;
use driftfs_filesystem::{DriftFsVfs, OpenFlags, VfsError};
use driftfs_metadata::MetadataStore;
use driftfs_provider::ObjectKind;
use driftfs_testkit::{create_metadata, MockProvider};

fn setup_vfs() -> (DriftFsVfs<MockProvider>, tempfile::TempDir) {
    let store = Arc::new(MetadataStore::open_in_memory().expect("open store"));
    let provider = Arc::new(MockProvider::new());
    provider.add_directory("root", "My Drive", None);
    provider.add_directory("folder_docs", "Documents", None);
    provider.add_file("file_notes", "notes.txt", Some("folder_docs"), 100);

    let doc_meta = create_metadata(
        "folder_docs",
        "Documents",
        None,
        ObjectKind::Directory,
        None,
        Some("application/vnd.google-apps.folder"),
    );
    let note_meta = create_metadata(
        "file_notes",
        "notes.txt",
        Some("folder_docs"),
        ObjectKind::File,
        Some(100),
        Some("text/plain"),
    );
    store
        .batch_upsert_objects(&[doc_meta, note_meta])
        .expect("upsert");

    let staging_dir = tempfile::tempdir().expect("staging dir");
    let vfs = DriftFsVfs::new(store, provider, staging_dir.path().to_path_buf())
        .expect("create vfs")
        .with_root_folder_id(FileId("root".into()));

    (vfs, staging_dir)
}

#[tokio::test]
async fn reject_path_traversal_payloads() {
    let (vfs, _dir) = setup_vfs();

    assert_eq!(
        vfs.resolve_path("/../secret.txt"),
        Err(VfsError::InvalidPath)
    );
    assert_eq!(
        vfs.resolve_path("/Documents/../../etc/shadow"),
        Err(VfsError::InvalidPath)
    );
    assert_eq!(
        vfs.resolve_path("C:\\Windows\\System32"),
        Err(VfsError::InvalidPath)
    );
    assert_eq!(
        vfs.resolve_path(r"\\server\share\file.txt"),
        Err(VfsError::InvalidPath)
    );

    assert_eq!(vfs.getattr("/../secret.txt"), Err(VfsError::InvalidPath));
    assert_eq!(vfs.opendir("/../secret"), Err(VfsError::InvalidPath));
    assert_eq!(
        vfs.open("/../secret.txt", OpenFlags::read_only()).await,
        Err(VfsError::InvalidPath)
    );
}

#[tokio::test]
async fn reject_null_byte_injection() {
    let (vfs, _dir) = setup_vfs();

    assert_eq!(
        vfs.resolve_path("/Documents/secret\0.txt"),
        Err(VfsError::InvalidPath)
    );
    assert_eq!(vfs.getattr("/null\0file"), Err(VfsError::InvalidPath));

    let res = vfs.create_file("/Documents", "file\0exploit.txt").await;
    assert_eq!(res.unwrap_err(), VfsError::InvalidPath);

    let res2 = vfs.mkdir("/Documents/sub\0dir").await;
    assert_eq!(res2.unwrap_err(), VfsError::InvalidPath);
}

#[tokio::test]
async fn reject_windows_reserved_device_names() {
    let (vfs, _dir) = setup_vfs();

    let reserved_names = [
        "CON",
        "con.txt",
        "PRN",
        "prn.pdf",
        "AUX",
        "aux.tar.gz",
        "NUL",
        "nul.dat",
        "COM1",
        "com1.log",
        "COM9",
        "LPT1",
        "lpt3.txt",
    ];

    for name in reserved_names {
        let create_res = vfs.create_file("/Documents", name).await;
        assert_eq!(
            create_res.unwrap_err(),
            VfsError::InvalidPath,
            "Reserved name '{name}' must be rejected by create_file"
        );

        let mkdir_res = vfs.mkdir(&format!("/Documents/{name}")).await;
        assert_eq!(
            mkdir_res.unwrap_err(),
            VfsError::InvalidPath,
            "Reserved name '{name}' must be rejected by mkdir"
        );

        let rename_res = vfs
            .rename("/Documents/notes.txt", &format!("/Documents/{name}"), false)
            .await;
        assert_eq!(
            rename_res.unwrap_err(),
            VfsError::InvalidPath,
            "Rename to reserved name '{name}' must be rejected"
        );
    }
}

#[tokio::test]
async fn reject_illegal_characters_and_trailing_characters() {
    let (vfs, _dir) = setup_vfs();

    let invalid_names = [
        "file<name.txt",
        "file>name.txt",
        "file:name.txt",
        "file\"name.txt",
        "file|name.txt",
        "file?name.txt",
        "file*name.txt",
        "file.",
        "file ",
    ];

    for name in invalid_names {
        let res = vfs.create_file("/Documents", name).await;
        assert_eq!(
            res.unwrap_err(),
            VfsError::InvalidPath,
            "Name '{name}' with illegal character/format must be rejected"
        );
    }
}

#[tokio::test]
async fn verify_zero_secret_leakage_in_errors() {
    let err = driftfs_core::DriftFsError::auth("invalid bearer token ya29.a0ARrdaM-secret-key-xyz");
    let display = format!("{err}");
    assert!(!display.is_empty());

    let vfs_err: VfsError = err.into();
    assert_eq!(vfs_err, VfsError::AccessDenied);
    assert_eq!(format!("{vfs_err}"), "permission denied");
}
