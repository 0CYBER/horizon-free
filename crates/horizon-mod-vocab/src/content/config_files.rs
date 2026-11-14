//! Checked, mod-bound files for config, game data and current-world data.
//! All three roots use the same paths, errors and paged file operations.

/// Bytes a config file may hold, read or written.
pub const CONFIG_FILE_MAX: u64 = 4 << 20;

/// Horizon's bulk-data bound: 64 MiB per file, for tiles and exports.
/// Applies to both `Host::data_files` and `ServerHost::world_data_files`.
pub const DATA_FILE_MAX: u64 = 64 << 20;

/// Maximum entries in one page, for config and both bulk-data roots.
pub const CONFIG_LIST_MAX: usize = 1024;

/// The calling mod's checked file service on either side. The historical
/// name also serves bulk data: its root chooses the per-file size cap.
/// Config writes are observed by the registered config reload poll; bulk
/// data is never scanned as config or settings.
pub trait ConfigFiles {
    /// The file's bytes, at most the root's file-size cap.
    fn read(&self, file: &ConfigPath) -> Result<Vec<u8>, ConfigFileError>;
    /// Replaces the file's bytes, at most the root's file-size cap, making its
    /// folders when missing. The new bytes replace the old ones at once: a
    /// reader never sees half a write.
    fn write(&self, file: &ConfigPath, bytes: &[u8]) -> Result<(), ConfigFileError>;
    /// An ordered page of entries strictly after `after` (a name, not a path).
    /// Pass `None` for the first page, then the last returned name. An empty
    /// page ends the walk. Concurrent folder edits are not a snapshot.
    /// `limit` must be 1..=CONFIG_LIST_MAX; memory use is bounded by this page.
    fn list(
        &self,
        folder: &ConfigPath,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ConfigEntry>, ConfigFileError>;
    /// What is at `path`; None where nothing is.
    fn stat(&self, path: &ConfigPath) -> Result<Option<ConfigStat>, ConfigFileError>;
}

/// A relative, `/`-separated path, checked once. Each component must satisfy
/// [`ConfigPath::is_valid_name`], identically on every platform. Bulk paths
/// omit the mod id; config paths name its owned config.
// no wire: decoding validates the checked relative path or the explicit root.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ConfigPath(String);

impl From<&ConfigPath> for String {
    fn from(path: &ConfigPath) -> Self {
        path.0.clone()
    }
}

/// A path checked as [`ConfigPath::new`] checks it.
impl TryFrom<String> for ConfigPath {
    type Error = ConfigFileError;
    fn try_from(path: String) -> Result<Self, ConfigFileError> {
        if path.split('/').all(Self::is_valid_name) {
            Ok(Self(path))
        } else {
            Err(ConfigFileError::InvalidPath)
        }
    }
}

impl ConfigPath {
    /// The calling mod's bulk-data root, for listing or stat. The shared
    /// config folder itself is not owned and refuses this path.
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Checks `path` and makes a `ConfigPath` of it, or refuses with `ConfigFileError::InvalidPath`.
    pub fn new(path: &str) -> Result<Self, ConfigFileError> {
        let valid = path.split('/').all(Self::is_valid_name);
        valid
            .then(|| Self(path.to_owned()))
            .ok_or(ConfigFileError::InvalidPath)
    }

    /// Whether a directory-entry name or listing cursor is portable and safe,
    /// without allocating a path. Windows device names and normalized aliases
    /// are refused on every platform, as are separators and control characters.
    pub fn is_valid_name(name: &str) -> bool {
        if name.is_empty()
            || name.ends_with(['.', ' '])
            || name.chars().any(|c| {
                c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
            })
        {
            return false;
        }
        let base = name.split('.').next().unwrap_or(name);
        if ["CON", "PRN", "AUX", "NUL"]
            .iter()
            .any(|reserved| base.eq_ignore_ascii_case(reserved))
        {
            return false;
        }
        let bytes = base.as_bytes();
        !(bytes.len() == 4
            && (bytes[..3].eq_ignore_ascii_case(b"COM") || bytes[..3].eq_ignore_ascii_case(b"LPT"))
            && matches!(bytes[3], b'1'..=b'9'))
    }

    /// The path as written, `/`-separated.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the path is `mod_id`'s: its folder `<mod_id>/` and anything
    /// in it, or a file beside it named `<mod_id>.<ext>` or
    /// `<mod_id>-<rest>` (NeoForge's `<mod_id>-common.toml`). `.` and `-`
    /// are not id characters, so no other mod's id starts the same way.
    pub fn owned_by(&self, mod_id: &str) -> bool {
        let (first, rest) = self.0.split_once('/').unwrap_or((&self.0, ""));
        match first.strip_prefix(mod_id) {
            Some("") => true,
            Some(tail) => rest.is_empty() && (tail.starts_with('.') || tail.starts_with('-')),
            None => false,
        }
    }
}

/// What a path holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ConfigEntryKind {
    /// A file.
    File,
    /// A folder.
    Folder,
}

/// A file's or folder's size and age (`java.io.File.length`/`lastModified`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ConfigStat {
    /// Whether the path is a file or a folder.
    pub kind: ConfigEntryKind,
    /// Bytes, for a file; 0 for a folder.
    pub len: u64,
    /// Milliseconds since the Unix epoch; 0 where the system doesn't say.
    pub modified_millis: u64,
}

/// One entry of a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ConfigEntry {
    /// The entry's name, without its folder.
    pub name: String,
    /// The entry's size and age.
    pub stat: ConfigStat,
}

/// Why a config file call was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ConfigFileError {
    /// Called outside any mod's hook or part, so no mod owns the call.
    NoCallingMod,
    /// The path is another mod's, or `config/` itself.
    NotOwned,
    /// Not a path `ConfigPath::new` takes.
    InvalidPath,
    /// No current world save folder is open.
    NoWorld,
    /// Page size is zero or exceeds `CONFIG_LIST_MAX`, or the cursor is not a name.
    InvalidPage,
    /// More bytes than the selected root's per-file size cap.
    TooLarge,
    /// Nothing is at that path.
    NotFound,
    /// A folder where a file was asked for.
    NotAFile,
    /// A file where a folder was asked for.
    NotAFolder,
    /// A symbolic link on the path, which could lead outside the bound root.
    Symlink,
    /// Any other failure of the file system, as it describes itself.
    Io(String),
}

impl std::fmt::Display for ConfigFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCallingMod => f.write_str("no mod is calling"),
            Self::NotOwned => f.write_str("the path is not the calling mod's"),
            Self::InvalidPath => f.write_str("not a checked relative file path"),
            Self::NoWorld => f.write_str("no current world save is open"),
            Self::InvalidPage => f.write_str("invalid file-list page size or cursor"),
            Self::TooLarge => f.write_str("too large"),
            Self::NotFound => f.write_str("not found"),
            Self::NotAFile => f.write_str("not a file"),
            Self::NotAFolder => f.write_str("not a folder"),
            Self::Symlink => f.write_str("a symbolic link is on the path"),
            Self::Io(error) => f.write_str(error),
        }
    }
}

impl std::error::Error for ConfigFileError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_stays_inside_config() {
        for good in ["fd.json", "fd/a/b.toml", "fd-common.toml", "fd/éá"] {
            assert!(ConfigPath::new(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "/fd.json",
            "fd/",
            "fd//a",
            "./fd",
            "fd/../x",
            "..",
            "fd\\a",
            "C:fd",
            "fd\0",
            "fd/.. ",
            "fd/a.",
            "fd/a ",
            "fd/NUL",
            "fd/con.txt",
            "fd/COM1.bin",
            "fd/a\nb",
            "fd/a?b",
            "fd/a*b",
            "fd/a|b",
            "fd/a<b",
            "fd/a>b",
            "fd/a\"b",
        ] {
            assert_eq!(
                ConfigPath::new(bad),
                Err(ConfigFileError::InvalidPath),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_mod_owns_its_folder_and_the_files_named_after_it() {
        let owned = |path: &str| ConfigPath::new(path).unwrap().owned_by("fd");
        assert!(owned("fd"));
        assert!(owned("fd/a/b.json"));
        assert!(owned("fd.json"));
        assert!(owned("fd-common.toml"));
        // Another mod whose id starts the same, and names under another folder.
        assert!(!owned("fdx.json"));
        assert!(!owned("fd_extra/a.json"));
        assert!(!owned("fdx/a.json"));
        assert!(!owned("fd.json/a"));
        assert!(!owned("other/fd.json"));
    }
}

#[doc(hidden)]
pub(crate) mod built {
    #[allow(unused_imports)]
    use super::*;
    // exhaustive: the game builds it with a literal; mods only read it
    #[derive(horizon_mod_api_macros::Wire)]
    pub struct ConfigStat {
        pub kind: ConfigEntryKind,
        pub len: u64,
        pub modified_millis: u64,
    }
    impl From<ConfigStat> for super::ConfigStat {
        fn from(v: ConfigStat) -> Self {
            let ConfigStat {
                kind,
                len,
                modified_millis,
            } = v;
            Self {
                kind,
                len,
                modified_millis,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    #[derive(horizon_mod_api_macros::Wire)]
    pub struct ConfigEntry {
        pub name: String,
        pub stat: super::ConfigStat,
    }
    impl From<ConfigEntry> for super::ConfigEntry {
        fn from(v: ConfigEntry) -> Self {
            let ConfigEntry { name, stat } = v;
            Self { name, stat }
        }
    }
}
