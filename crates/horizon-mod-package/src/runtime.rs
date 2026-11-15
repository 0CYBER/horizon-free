//! The native mods compiled into a game, and their executable-adjacent packages.
use crate::{
    Manifest,
    mod_list::NATIVE_DIR,
    packages::{self, AdjacentManifest, ReadError},
};
use std::path::{Path, PathBuf};

use mod_vocab::__private::native::BuiltMod;

/// Validated once before installing hooks. Vanilla explicitly supplies an empty inventory.
#[derive(Debug, Default)]
pub struct Runtime {
    built: &'static [BuiltMod],
    packages: Vec<AdjacentManifest>,
}

impl Runtime {
    pub fn built(&self) -> &'static [BuiltMod] {
        self.built
    }

    pub fn packages(&self) -> &[AdjacentManifest] {
        &self.packages
    }

    pub fn load(executable: &Path, built: &'static [BuiltMod]) -> Result<Self, RuntimeError> {
        let root = executable.with_file_name(NATIVE_DIR);
        let mut packages = Vec::with_capacity(built.len());
        let mut refused = Vec::new();
        for expected in built {
            let path = root.join(expected.id);
            match packages::read_native(&path) {
                Ok(package) => {
                    let m = &package.manifest;
                    if m.id != expected.id
                        || m.version != expected.version
                        || m.side != expected.side
                    {
                        refused.push(PackageError::Identity {
                            expected: *expected,
                            path,
                            found: Box::new(package.manifest),
                        });
                    } else {
                        packages.push(package);
                    }
                }
                Err(source) => refused.push(PackageError::Read {
                    expected: *expected,
                    source,
                }),
            }
        }
        if refused.is_empty() {
            Ok(Self { built, packages })
        } else {
            Err(RuntimeError { refused })
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum PackageError {
    Read {
        expected: BuiltMod,
        source: ReadError,
    },
    Identity {
        expected: BuiltMod,
        path: PathBuf,
        found: Box<Manifest>,
    },
}

#[derive(Debug)]
pub struct RuntimeError {
    pub refused: Vec<PackageError>,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Compiled native mods cannot be loaded:")?;
        for problem in &self.refused {
            match problem {
                PackageError::Read { expected, source } => {
                    writeln!(f, "{} {}: {source}", expected.id, expected.version)?;
                }
                PackageError::Identity {
                    expected,
                    path,
                    found,
                } => {
                    writeln!(
                        f,
                        "{} {} ({:?}) requires its code package at {}; found {} {} ({:?})",
                        expected.id,
                        expected.version,
                        expected.side,
                        path.display(),
                        found.id,
                        found.version,
                        found.side
                    )?;
                }
            }
        }
        write!(
            f,
            "Restore the packages exported with this executable, or rebuild it."
        )
    }
}
impl std::error::Error for RuntimeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_vocab::Side;

    const BUILT: &[BuiltMod] = &[
        BuiltMod {
            id: "machines",
            version: "1.2.3",
            side: Side::Both,
        },
        BuiltMod {
            id: "local",
            version: "2.0.0",
            side: Side::Client,
        },
    ];

    struct Fixture(PathBuf);
    impl Fixture {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "horizon-native-runtime-{}-{label}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn package(&self, expected: BuiltMod, native: bool) -> PathBuf {
            let path = self.0.join(NATIVE_DIR).join(expected.id);
            std::fs::create_dir_all(&path).unwrap();
            let mut text = format!(
                "id = '{}'\nname = 'A mod'\nversion = '{}'\napi = '0.2'\nside = '{}'\ncapabilities = ['registries']\n",
                expected.id,
                expected.version,
                match expected.side {
                    Side::Both => "both",
                    Side::Server => "server",
                    _ => "client",
                }
            );
            if native {
                text += &format!(
                    "[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
                    "a".repeat(40),
                    "b".repeat(40)
                );
                std::fs::write(
                    path.join("Cargo.toml"),
                    "[package]\nname = 'different_name'\nversion = '9.9.9'\n",
                )
                .unwrap();
                std::fs::create_dir_all(path.join("src")).unwrap();
                std::fs::write(path.join("src/lib.rs"), "// native code").unwrap();
            }
            std::fs::write(path.join("horizon-mod.toml"), text).unwrap();
            path
        }
        fn load(&self) -> Result<Runtime, RuntimeError> {
            Runtime::load(&self.0.join("horizon-engine"), BUILT)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn packages_follow_the_executable_and_manifest_not_cargo_identity() {
        let f = Fixture::new("identity");
        for expected in BUILT {
            f.package(*expected, true);
        }
        let runtime = f.load().unwrap();
        assert_eq!(runtime.packages.len(), 2);
        assert_eq!(runtime.packages[0].manifest.id, "machines");
        assert_eq!(runtime.packages[0].manifest.version, "1.2.3");
        assert_eq!(
            runtime.packages[0].path,
            f.0.join(NATIVE_DIR).join("machines")
        );
    }

    #[test]
    fn every_missing_package_is_named_in_one_refusal() {
        let f = Fixture::new("missing");
        let error = f.load().unwrap_err();
        assert_eq!(error.refused.len(), 2);
        let message = error.to_string();
        for expected in BUILT {
            assert!(message.contains(expected.id), "{message}");
        }
        assert!(message.contains("Restore the packages"));
    }

    #[test]
    fn changing_an_identity_version_or_side_refuses_startup() {
        for (label, changed) in [
            (
                "id",
                BuiltMod {
                    id: "other",
                    ..BUILT[0]
                },
            ),
            (
                "version",
                BuiltMod {
                    version: "1.2.4",
                    ..BUILT[0]
                },
            ),
            (
                "side",
                BuiltMod {
                    side: Side::Client,
                    ..BUILT[0]
                },
            ),
        ] {
            let f = Fixture::new(label);
            let changed_path = f.package(changed, true);
            if changed.id != BUILT[0].id {
                std::fs::rename(changed_path, f.0.join(NATIVE_DIR).join(BUILT[0].id)).unwrap();
            }
            f.package(BUILT[1], true);
            let error = f.load().unwrap_err();
            assert!(
                matches!(&error.refused[..], [PackageError::Identity { expected, .. }] if expected.id == "machines")
            );
        }
    }

    #[test]
    fn a_content_package_cannot_replace_a_compiled_code_mod() {
        let f = Fixture::new("content");
        f.package(BUILT[0], false);
        f.package(BUILT[1], true);
        assert!(matches!(
            &f.load().unwrap_err().refused[..],
            [PackageError::Read { source: ReadError::NotNative { id, .. }, .. }] if id == "machines"
        ));
    }

    #[test]
    fn a_corrupt_manifest_is_not_treated_as_an_unmodded_build() {
        let f = Fixture::new("corrupt");
        let path = f.package(BUILT[0], true);
        std::fs::write(path.join("horizon-mod.toml"), "not a manifest").unwrap();
        f.package(BUILT[1], true);
        assert!(matches!(
            &f.load().unwrap_err().refused[..],
            [PackageError::Read { .. }]
        ));
    }
}
