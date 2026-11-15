//! Vanilla's directory-and-extension resource converter.
use crate::Identifier;

/// An invalid directory or extension passed to [`FileToIdConverter`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum FileToIdConverterError {
    /// Directory.
    Directory,
    /// Extension.
    Extension,
}
impl std::fmt::Display for FileToIdConverterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Directory => "invalid resource directory",
            Self::Extension => "invalid resource extension",
        })
    }
}
impl std::error::Error for FileToIdConverterError {}

/// Maps `data/<namespace>/<directory>/<path><extension>` to `namespace:path`,
/// preserving subdirectories. Any namespace may add entries for a listener.
/// The same converter can be used for client assets.
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: transfers directory/extension only, reconstructing cached prefix through new and validating resource-path syntax.
pub struct FileToIdConverter {
    directory: String,
    prefix: String,
    extension: String,
}
/// The converter as its directory and extension, the form it crosses a wasm mod's boundary in.
impl From<&FileToIdConverter> for (String, String) {
    fn from(converter: &FileToIdConverter) -> Self {
        (converter.directory.clone(), converter.extension.clone())
    }
}
/// A converter from its directory and extension, checked as [`FileToIdConverter::new`] checks
/// them.
impl TryFrom<(String, String)> for FileToIdConverter {
    type Error = FileToIdConverterError;
    fn try_from((directory, extension): (String, String)) -> Result<Self, Self::Error> {
        Self::new(&directory, &extension)
    }
}
impl FileToIdConverter {
    /// A relative directory without trailing `/`, and a suffix such as `.json`.
    /// An empty extension keeps each relative resource path whole.
    pub fn new(directory: &str, extension: &str) -> Result<Self, FileToIdConverterError> {
        if Identifier::from_parts("minecraft", directory).is_err()
            || directory
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(FileToIdConverterError::Directory);
        }
        if !extension.is_empty()
            && (Identifier::from_parts("minecraft", extension).is_err() || extension.contains('/'))
        {
            return Err(FileToIdConverterError::Extension);
        }
        Ok(Self {
            directory: directory.into(),
            prefix: format!("{directory}/"),
            extension: extension.into(),
        })
    }
    /// Vanilla's `FileToIdConverter.json(directory)`.
    pub fn json(directory: &str) -> Result<Self, FileToIdConverterError> {
        Self::new(directory, ".json")
    }

    pub(crate) fn directory(&self) -> &str {
        &self.directory
    }

    pub(crate) fn file_to_id(&self, file: &str) -> Option<Identifier> {
        let (namespace, path) = file.split_once('/')?;
        let path = path
            .strip_prefix(&self.prefix)?
            .strip_suffix(&self.extension)?;
        // Pack entry names are untrusted; IDs never expose filesystem traversal.
        if namespace.is_empty()
            || namespace == "."
            || namespace == ".."
            || path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return None;
        }
        Identifier::from_parts(namespace, path).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removes_only_directory_and_suffix_and_keeps_other_namespaces() {
        let converter = FileToIdConverter::json("machines").unwrap();
        for (file, expected) in [
            ("my_mod/machines/press.json", "my_mod:press"),
            ("other/machines/heavy/press.json", "other:heavy/press"),
        ] {
            assert_eq!(
                converter.file_to_id(file),
                Some(Identifier::parse(expected).unwrap())
            );
        }
        for file in [
            "my_mod/machines2/press.json",
            "my_mod/machines/press.txt",
            "my_mod/machines/../private.json",
            "../machines/press.json",
        ] {
            assert!(converter.file_to_id(file).is_none(), "{file}");
        }
        assert_eq!(
            FileToIdConverter::new("machines", ".bin")
                .unwrap()
                .file_to_id("other/machines/raw.bin"),
            Some(Identifier::parse("other:raw").unwrap())
        );
    }
    #[test]
    fn validates_the_filesystem_directory_once() {
        for directory in [
            "",
            "/machines",
            "../machines",
            "machines/..",
            "machines//nested",
            "machines/",
        ] {
            assert!(FileToIdConverter::json(directory).is_err(), "{directory}");
        }
        assert!(FileToIdConverter::new("machines", "/private").is_err());
        assert_eq!(
            FileToIdConverter::new("machines", "")
                .unwrap()
                .file_to_id("other/machines/readme"),
            Some(Identifier::parse("other:readme").unwrap())
        );
    }
}
