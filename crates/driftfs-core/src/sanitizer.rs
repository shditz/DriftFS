use crate::error::DriftFsError;
use crate::Result;

const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

const DISALLOWED_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

pub fn is_windows_reserved(name: &str) -> bool {
    let stem = match name.split('.').next() {
        Some(s) => s.trim(),
        None => return false,
    };
    WINDOWS_RESERVED_NAMES
        .iter()
        .any(|&reserved| stem.eq_ignore_ascii_case(reserved))
}

pub fn validate_file_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(DriftFsError::Filesystem {
            message: "file name cannot be empty".into(),
            source: None,
        });
    }

    if name.len() > 255 {
        return Err(DriftFsError::Filesystem {
            message: format!("file name exceeds 255 characters limit: len={}", name.len()),
            source: None,
        });
    }

    if name.contains('\0') {
        return Err(DriftFsError::Filesystem {
            message: "file name contains null byte".into(),
            source: None,
        });
    }

    if name == "." || name == ".." {
        return Err(DriftFsError::Filesystem {
            message: format!("file name '{name}' is reserved relative path"),
            source: None,
        });
    }

    if name.ends_with(' ') || name.ends_with('.') {
        return Err(DriftFsError::Filesystem {
            message: format!("file name '{name}' cannot end with space or dot"),
            source: None,
        });
    }

    for c in name.chars() {
        if (c as u32) < 32 || DISALLOWED_CHARS.contains(&c) {
            return Err(DriftFsError::Filesystem {
                message: format!("file name contains illegal character '{c}'"),
                source: None,
            });
        }
    }

    if is_windows_reserved(name) {
        return Err(DriftFsError::Filesystem {
            message: format!("file name '{name}' is a reserved Windows device name"),
            source: None,
        });
    }

    Ok(())
}

pub fn sanitize_path(path: &str) -> Result<Vec<String>> {
    if path.contains('\0') {
        return Err(DriftFsError::Filesystem {
            message: "path contains null byte".into(),
            source: None,
        });
    }

    if path.contains(':') || path.starts_with(r"\\") {
        return Err(DriftFsError::Filesystem {
            message: "absolute volume/drive or UNC path traversal rejected".into(),
            source: None,
        });
    }

    let mut segments = Vec::new();
    for part in path.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }

        if part == ".." {
            return Err(DriftFsError::Filesystem {
                message: "path traversal ('..') detected".into(),
                source: None,
            });
        }

        validate_file_name(part)?;
        segments.push(part.to_string());
    }

    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_file_names() {
        assert!(validate_file_name("document.pdf").is_ok());
        assert!(validate_file_name("my photo 2026.png").is_ok());
        assert!(validate_file_name("archive.tar.gz").is_ok());
    }

    #[test]
    fn reject_empty_or_reserved_names() {
        assert!(validate_file_name("").is_err());
        assert!(validate_file_name(".").is_err());
        assert!(validate_file_name("..").is_err());
    }

    #[test]
    fn reject_trailing_dot_or_space() {
        assert!(validate_file_name("name.").is_err());
        assert!(validate_file_name("name ").is_err());
    }

    #[test]
    fn reject_null_byte_and_illegal_chars() {
        assert!(validate_file_name("hello\0world.txt").is_err());
        assert!(validate_file_name("hello/world.txt").is_err());
        assert!(validate_file_name("hello\\world.txt").is_err());
        assert!(validate_file_name("file*name.txt").is_err());
        assert!(validate_file_name("file?name.txt").is_err());
        assert!(validate_file_name("file:name.txt").is_err());
        assert!(validate_file_name("file<name.txt").is_err());
        assert!(validate_file_name("file>name.txt").is_err());
        assert!(validate_file_name("file|name.txt").is_err());
        assert!(validate_file_name("file\"name.txt").is_err());
    }

    #[test]
    fn reject_windows_reserved_device_names() {
        assert!(validate_file_name("CON").is_err());
        assert!(validate_file_name("con.txt").is_err());
        assert!(validate_file_name("aux.tar.gz").is_err());
        assert!(validate_file_name("NUL").is_err());
        assert!(validate_file_name("com1.log").is_err());
        assert!(validate_file_name("LPT3").is_err());
    }

    #[test]
    fn sanitize_path_valid() {
        let segs = sanitize_path("/folder/subfolder/file.txt").unwrap();
        assert_eq!(segs, vec!["folder", "subfolder", "file.txt"]);

        let segs2 = sanitize_path(r"folder\subfolder\file.txt").unwrap();
        assert_eq!(segs2, vec!["folder", "subfolder", "file.txt"]);
    }

    #[test]
    fn sanitize_path_rejects_traversal() {
        assert!(sanitize_path("/folder/../secret.txt").is_err());
        assert!(sanitize_path("../secret.txt").is_err());
        assert!(sanitize_path("folder/../../etc/passwd").is_err());
        assert!(sanitize_path("C:\\Windows\\System32").is_err());
        assert!(sanitize_path(r"\\server\share").is_err());
        assert!(sanitize_path("/folder/null\0byte").is_err());
    }
}
