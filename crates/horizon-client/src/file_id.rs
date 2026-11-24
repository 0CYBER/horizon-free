//! Shared validation for names supplied to the replay and export APIs.

use std::path::{Component, Path};

/// The longest file id, in bytes: a file name most file systems still take with `.mcpr`.
const MAX_FILE_ID: usize = 250;

/// Names Windows gives to devices in every folder, whatever follows them after a dot.
const DEVICE_NAMES: [&str; 30] = [
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM1", "COM2", "COM3", "COM4", "COM5",
    "COM6", "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5",
    "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// Whether `id` can name a file or folder, judged as text alone: one plain file name, 1 to 250 bytes,
/// without a separator, drive, stream, `..`, NUL or control character, not ending in a dot or a
/// space (Windows drops those, so two ids would name one file), and not a Windows device name.
pub fn valid_id(id: &str) -> bool {
    let mut components = Path::new(id).components();
    let base = id
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ');
    !id.is_empty()
        && id.len() <= MAX_FILE_ID
        && !id.contains("..")
        && !id.ends_with(['.', ' '])
        && !id
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
        && !DEVICE_NAMES
            .iter()
            .any(|device| device.eq_ignore_ascii_case(base))
        && matches!(
            (components.next(), components.next()),
            (Some(Component::Normal(_)), None)
        )
}

/// A file's device and inode (volume and file index on Windows), independent of its path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    device: u64,
    index: u64,
}

impl FileIdentity {
    /// Read the identity from the already-open file.
    pub fn of(file: &std::fs::File) -> std::io::Result<Self> {
        use cap_fs_ext::MetadataExt;
        let metadata = cap_std::fs::File::from_std(file.try_clone()?).metadata()?;
        Ok(Self {
            device: metadata.dev(),
            index: metadata.ino(),
        })
    }
}

/// Export names must also be creatable as Windows file and directory names.
pub(crate) fn valid_export_name(name: &str) -> bool {
    valid_id(name) && !name.contains(['<', '>', '"', '|', '?', '*'])
}

/// A Windows reparse point (a link, a junction, a cloud placeholder) that `is_file` may not show.
#[cfg(windows)]
pub fn reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
pub fn reparse_point(_: &std::fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::valid_id;

    #[test]
    fn accepts_only_plain_portable_ids() {
        for invalid in [
            "",
            ".",
            "..",
            "one..two",
            "a/b",
            r"a\b",
            "C:clip",
            "clip:stream",
            "clip.",
            "clip ",
            "clip\0",
            "clip\n",
            "NuL.mp4",
            "COM¹",
            "LPT9 .png",
        ] {
            assert!(!valid_id(invalid), "{invalid:?}");
        }
        for valid in ["clip 1", "a.b", "conifer", "録画", "?", &"a".repeat(250)] {
            assert!(valid_id(valid), "{valid:?}");
        }
        assert!(!valid_id(&"a".repeat(251)));
    }
}
