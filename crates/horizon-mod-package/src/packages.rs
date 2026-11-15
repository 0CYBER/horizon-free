//! Canonical native package files shared by data preparation and engine validation.
use crate::{Manifest, Package, mod_list::NATIVE_DIR};
use std::{
    fmt,
    path::{Path, PathBuf},
};

/// A schema-validated native package, not evidence that its code is compiled into a program.
#[derive(Debug)]
pub struct AdjacentManifest {
    pub path: PathBuf,
    pub manifest: Manifest,
    /// Whether the package carries an `icon.png`.
    pub has_icon: bool,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ReadError {
    Package { path: PathBuf, reason: String },
    NotNative { path: PathBuf, id: String },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Package { path, reason } => write!(f, "{}: {reason}", path.display()),
            Self::NotNative { path, id } => write!(f, "{}: {id} is not a code mod", path.display()),
        }
    }
}
impl std::error::Error for ReadError {}

/// Read data candidates beneath `executable`'s adjacent `horizon-native-mods/` directory.
/// This does not read the build record or establish compiled membership or identity. An empty
/// result is not proof of a vanilla program: the engine must validate its own embedded inventory.
pub fn adjacent_manifests(executable: &Path) -> Result<Vec<AdjacentManifest>, ReadError> {
    crate::find(&executable.with_file_name(NATIVE_DIR))
        .into_iter()
        .filter(|path| path.is_dir())
        .map(|path| read_native(&path))
        .collect()
}

/// The same package boundary for a known canonical path. The caller owns compiled-identity checks.
pub fn read_native(path: &Path) -> Result<AdjacentManifest, ReadError> {
    let package = Package::read(path).map_err(|reason| ReadError::Package {
        path: path.to_owned(),
        reason,
    })?;
    if !package.is_native() {
        return Err(ReadError::NotNative {
            path: path.to_owned(),
            id: package.manifest.id,
        });
    }
    Ok(AdjacentManifest {
        path: path.to_owned(),
        manifest: package.manifest,
        has_icon: package.icon.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let name = format!(
                "adjacent-packages-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            );
            let dir = std::env::temp_dir().join(name);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn package(&self, id: &str) -> PathBuf {
            let dir = self.0.join(NATIVE_DIR).join(id);
            fs::create_dir_all(dir.join("src")).unwrap();
            fs::write(dir.join("horizon-mod.toml"), format!("id='{id}'\nname='{id}'\nversion='1.2.3'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n", "a".repeat(40), "b".repeat(40))).unwrap();
            fs::write(
                dir.join("Cargo.toml"),
                "[package]\nname='different_crate'\nversion='9.9.9'\n",
            )
            .unwrap();
            fs::write(dir.join("src/lib.rs"), "pub fn hooks() {}").unwrap();
            dir
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn candidates_follow_the_executable_not_the_record_or_cargo_identity() {
        let home = Scratch::new();
        let program = home.0.join("horizon-engine");
        assert!(adjacent_manifests(&program).unwrap().is_empty());
        let first = home.package("first");
        home.package("not_in_the_record");
        fs::write(
            home.0.join("native-mods.json"),
            r#"{"mods":[{"id":"first","version":"wrong"}]}"#,
        )
        .unwrap();
        let candidates = adjacent_manifests(&program).unwrap();
        assert_eq!(
            candidates
                .iter()
                .map(|p| (p.manifest.id.as_str(), p.manifest.version.as_str()))
                .collect::<Vec<_>>(),
            [("first", "1.2.3"), ("not_in_the_record", "1.2.3")]
        );
        assert_eq!(candidates[0].path, first);
        fs::write(home.0.join("native-mods.json"), "not JSON").unwrap();
        assert_eq!(adjacent_manifests(&program).unwrap().len(), 2);
    }

    #[test]
    fn corrupt_and_content_only_packages_report_their_path() {
        let home = Scratch::new();
        let dir = home.package("broken");
        fs::write(dir.join("horizon-mod.toml"), "not TOML").unwrap();
        assert!(
            matches!(adjacent_manifests(&home.0.join("engine")), Err(ReadError::Package { path, .. }) if path == dir)
        );
        fs::write(
            dir.join("horizon-mod.toml"),
            "id='broken'\nname='Broken'\nversion='1.2.3'\napi='0.2'\ncapabilities=['registries']\n",
        )
        .unwrap();
        fs::remove_file(dir.join("Cargo.toml")).unwrap();
        fs::remove_dir_all(dir.join("src")).unwrap();
        assert!(
            matches!(read_native(&dir), Err(ReadError::NotNative { path, id }) if path == dir && id == "broken")
        );
    }
}
