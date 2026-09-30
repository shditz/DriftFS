use driftfs_metadata::StoredObject;

pub const SHORTCUT_EXTENSION: &str = ".url";

pub fn is_google_workspace_mime(mime: &str) -> bool {
    matches!(
        mime,
        "application/vnd.google-apps.document"
            | "application/vnd.google-apps.spreadsheet"
            | "application/vnd.google-apps.presentation"
            | "application/vnd.google-apps.form"
            | "application/vnd.google-apps.drawing"
    )
}

pub fn workspace_url(file_id: &str, mime: &str) -> Option<String> {
    match mime {
        "application/vnd.google-apps.document" => {
            Some(format!("https://docs.google.com/document/d/{file_id}/edit"))
        }
        "application/vnd.google-apps.spreadsheet" => Some(format!(
            "https://docs.google.com/spreadsheets/d/{file_id}/edit"
        )),
        "application/vnd.google-apps.presentation" => Some(format!(
            "https://docs.google.com/presentation/d/{file_id}/edit"
        )),
        "application/vnd.google-apps.form" => {
            Some(format!("https://docs.google.com/forms/d/{file_id}/edit"))
        }
        "application/vnd.google-apps.drawing" => {
            Some(format!("https://docs.google.com/drawings/d/{file_id}/edit"))
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleWorkspaceShortcut {
    pub display_name: String,
    pub payload: Vec<u8>,
}

impl GoogleWorkspaceShortcut {
    pub fn is_workspace(object: &StoredObject) -> bool {
        object
            .mime_type
            .as_deref()
            .map(is_google_workspace_mime)
            .unwrap_or(false)
    }

    pub fn try_from_stored(object: &StoredObject) -> Option<Self> {
        let mime = object.mime_type.as_deref()?;
        let url = workspace_url(&object.id.0, mime)?;
        let display_name = if object.name.ends_with(SHORTCUT_EXTENSION) {
            object.name.clone()
        } else {
            format!("{}{SHORTCUT_EXTENSION}", object.name)
        };

        let content = format!("[InternetShortcut]\r\nURL={url}\r\n");
        Some(Self {
            display_name,
            payload: content.into_bytes(),
        })
    }

    pub fn strip_shortcut_extension(name: &str) -> Option<&str> {
        if name.len() >= SHORTCUT_EXTENSION.len() {
            let (stem, ext) = name.split_at(name.len() - SHORTCUT_EXTENSION.len());
            if ext.eq_ignore_ascii_case(SHORTCUT_EXTENSION) {
                return Some(stem);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use driftfs_core::FileId;
    use driftfs_metadata::SyncStatus;
    use driftfs_provider::ObjectKind;

    #[test]
    fn synthesizes_url_shortcut_correctly() {
        let object = StoredObject {
            id: FileId("doc_123".into()),
            parent_id: None,
            name: "Team Roadmap".into(),
            remote_name: "Team Roadmap".into(),
            kind: ObjectKind::File,
            size_bytes: None,
            mime_type: Some("application/vnd.google-apps.document".into()),
            created_at: None,
            modified_at: None,
            version: None,
            sync_status: SyncStatus::Synced,
            deleted: false,
        };

        let shortcut = GoogleWorkspaceShortcut::try_from_stored(&object).expect("shortcut");
        assert_eq!(shortcut.display_name, "Team Roadmap.url");
        let text = String::from_utf8(shortcut.payload).expect("utf8");
        assert!(text.contains("[InternetShortcut]"));
        assert!(text.contains("URL=https://docs.google.com/document/d/doc_123/edit"));
    }

    #[test]
    fn ignores_non_workspace_files() {
        let object = StoredObject {
            id: FileId("file_123".into()),
            parent_id: None,
            name: "image.png".into(),
            remote_name: "image.png".into(),
            kind: ObjectKind::File,
            size_bytes: Some(500),
            mime_type: Some("image/png".into()),
            created_at: None,
            modified_at: None,
            version: None,
            sync_status: SyncStatus::Synced,
            deleted: false,
        };

        assert!(GoogleWorkspaceShortcut::try_from_stored(&object).is_none());
    }
}
