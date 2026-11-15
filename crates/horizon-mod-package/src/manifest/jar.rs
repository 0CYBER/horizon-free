use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

/// An original Java jar used as a passive asset/data layer, in manifest order.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(try_from = "Fields")]
pub struct JarRef {
    id: String,
    version: String,
    sha512: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    id: String,
    version: String,
    sha512: String,
}

impl TryFrom<Fields> for JarRef {
    type Error = String;

    fn try_from(fields: Fields) -> Result<Self, Self::Error> {
        Self::new(fields.id, fields.version, fields.sha512)
    }
}

/// One exact file and every Horizon mod requiring it, in load order.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(try_from = "RequirementFields")]
pub struct JarRequirement {
    pub owners: Vec<String>,
    pub jar: JarRef,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequirementFields {
    owners: Vec<String>,
    jar: JarRef,
}

impl TryFrom<RequirementFields> for JarRequirement {
    type Error = String;

    fn try_from(fields: RequirementFields) -> Result<Self, Self::Error> {
        let unique: std::collections::BTreeSet<_> = fields.owners.iter().collect();
        if fields.owners.is_empty()
            || fields.owners.iter().any(|id| !super::valid_mod_id(id))
            || unique.len() != fields.owners.len()
        {
            return Err(format!("invalid jar owners: {:?}", fields.owners));
        }
        Ok(Self {
            owners: fields.owners,
            jar: fields.jar,
        })
    }
}

/// One missing original file, shared by launcher preparation and game refusals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingJar {
    pub owners: Vec<String>,
    pub jar: JarRef,
    pub path: PathBuf,
}

impl fmt::Display for MissingJar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} {}: {}",
            self.owners.join(", "),
            self.jar.id(),
            self.jar.version(),
            self.path.display()
        )
    }
}
impl std::error::Error for MissingJar {}

/// An accepted resource root's original-jar inputs could not be checked.
#[derive(Debug)]
#[non_exhaustive]
pub enum JarInputsError {
    /// An exact original is unavailable, with every requiring mod retained.
    Missing(Box<MissingJar>),
    /// The accepted inputs or cache metadata could not be read.
    Io {
        /// The input file or cache directory being checked.
        path: PathBuf,
        /// The filesystem failure.
        source: std::io::Error,
    },
    /// The accepted input record failed schema validation.
    Invalid {
        /// The accepted input record.
        path: PathBuf,
        /// The decoding or validation failure.
        source: serde_json::Error,
    },
}
impl fmt::Display for JarInputsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(missing) => missing.fmt(f),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Invalid { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}
impl std::error::Error for JarInputsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::Missing(missing) => missing.as_ref(),
            Self::Io { source, .. } => source,
            Self::Invalid { source, .. } => source,
        })
    }
}

/// A hash cannot name two different original Java mod identities.
#[derive(Debug)]
pub struct JarConflict {
    pub first: Box<JarRef>,
    pub second: Box<JarRef>,
}

impl fmt::Display for JarConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} {} != {} {}",
            self.first.sha512(),
            self.first.id(),
            self.first.version(),
            self.second.id(),
            self.second.version()
        )
    }
}
impl std::error::Error for JarConflict {}

/// Collect exact files once, preserving all owning mod ids in input load order.
/// The returned hash order is for preparation only; pack layers stay per mod.
pub fn collect_jar_requirements<'a>(
    mods: impl IntoIterator<Item = (&'a str, &'a [JarRef])>,
) -> Result<Vec<JarRequirement>, JarConflict> {
    let mut hashes: BTreeMap<&str, JarRequirement> = BTreeMap::new();
    for (owner, jars) in mods {
        for jar in jars {
            if let Some(first) = hashes.get_mut(jar.sha512()) {
                if first.jar != *jar {
                    return Err(JarConflict {
                        first: Box::new(first.jar.clone()),
                        second: Box::new(jar.clone()),
                    });
                }
                if !first.owners.iter().any(|id| id == owner) {
                    first.owners.push(owner.to_owned());
                }
            } else {
                hashes.insert(
                    jar.sha512(),
                    JarRequirement {
                        owners: vec![owner.to_owned()],
                        jar: jar.clone(),
                    },
                );
            }
        }
    }
    Ok(hashes.into_values().collect())
}

/// Missing exact files in a jar cache, with every requiring mod retained.
/// Byte identity is verified by launcher preparation, not rehashed on a reload.
pub fn missing_jars(
    store: &Path,
    requirements: &[JarRequirement],
) -> std::io::Result<Vec<MissingJar>> {
    let mut missing = Vec::new();
    for requirement in requirements {
        let path = requirement.jar.cache_path(store);
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    path.display().to_string(),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(MissingJar {
                    owners: requirement.owners.clone(),
                    jar: requirement.jar.clone(),
                    path,
                })
            }
            Err(error) => {
                return Err(std::io::Error::new(
                    error.kind(),
                    format!("{}: {error}", path.display()),
                ));
            }
        }
    }
    Ok(missing)
}

impl JarRef {
    /// The exact file's path in the supplied jar-cache directory.
    pub fn cache_path(&self, store: &Path) -> PathBuf {
        store.join(format!("{}.jar", self.sha512))
    }

    /// Validate the supported loader ID syntax and the exact file's SHA-512.
    /// The metadata reader, not this loader-independent manifest, checks the
    /// original loader's more specific ID rule. IDs are never normalized.
    pub fn new(id: String, version: String, sha512: String) -> Result<Self, String> {
        let bytes = id.as_bytes();
        let valid_id = (2..=64).contains(&bytes.len())
            && bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(b))
            && (!bytes.contains(&b'-') || bytes[0].is_ascii_lowercase());
        if !valid_id {
            return Err(format!(
                "jar id {id:?} is not a supported NeoForge or Fabric ID"
            ));
        }
        if version.is_empty() {
            return Err(format!("jar {id}: version is empty"));
        }
        super::wasm::sha512_hex(&sha512).map_err(|e| format!("jar {id}: sha512 {e}"))?;
        Ok(Self {
            id,
            version,
            sha512,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn sha512(&self) -> &str {
        &self.sha512
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_root_requirements_validate_owners_at_the_read_boundary() {
        let valid = serde_json::json!({
            "owners": ["zeta", "alpha"],
            "jar": {"id":"original", "version":"1", "sha512":"a".repeat(128)}
        });
        let requirement: JarRequirement = serde_json::from_value(valid.clone()).unwrap();
        assert_eq!(requirement.owners, ["zeta", "alpha"]);
        for owners in [
            serde_json::json!([]),
            serde_json::json!(["alpha", "alpha"]),
            serde_json::json!(["UPPER"]),
            serde_json::json!(["../alpha"]),
        ] {
            let mut invalid = valid.clone();
            invalid["owners"] = owners;
            assert!(serde_json::from_value::<JarRequirement>(invalid).is_err());
        }
        let mut invalid = valid.clone();
        invalid["jar"]["sha512"] = serde_json::json!("A".repeat(128));
        assert!(serde_json::from_value::<JarRequirement>(invalid).is_err());
        let mut invalid = valid;
        invalid["extra"] = serde_json::json!(true);
        assert!(serde_json::from_value::<JarRequirement>(invalid).is_err());
    }

    #[test]
    fn shared_jar_requirements_keep_all_owners_in_load_order() {
        let jar = JarRef::new("original".into(), "1".into(), "a".repeat(128)).unwrap();
        let requirements = collect_jar_requirements([
            ("zeta", std::slice::from_ref(&jar)),
            ("alpha", [jar.clone(), jar.clone()].as_slice()),
            ("beta", std::slice::from_ref(&jar)),
        ])
        .unwrap();
        assert_eq!(requirements.len(), 1);
        assert_eq!(requirements[0].jar, jar);
        assert_eq!(requirements[0].owners, ["zeta", "alpha", "beta"]);
    }

    #[test]
    fn manifest_jar_list_defaults_empty_and_survives_package_roundtrip_in_order() {
        use super::super::{Manifest, Package};
        let base = "id = 'port'\nname = 'Port'\nversion = '1'\napi = '0.2'\ncapabilities = ['registries']\n";
        assert!(Manifest::parse(base).unwrap().jar.is_empty());
        let text = format!(
            "{base}[[jar]]\nid = 'first_jar'\nversion = '1'\nsha512 = '{}'\n[[jar]]\nid = 'fabric-api'\nversion = '2'\nsha512 = '{}'\n",
            "0".repeat(128),
            "1".repeat(128)
        );
        let manifest = Manifest::parse(&text).unwrap();
        assert_eq!(
            manifest.jar.iter().map(JarRef::id).collect::<Vec<_>>(),
            ["first_jar", "fabric-api"]
        );
        let bytes = Package::write_files(&text, &[]).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(Package::read_zip(&mut archive).unwrap().manifest, manifest);
        assert!(Manifest::parse(&format!("{text}loader = 'fabric'\n")).is_err());
    }

    #[test]
    fn manifest_refs_preserve_original_ids_and_refuse_invalid_boundary_values() {
        let hash = "0123456789abcdef".repeat(8);
        for id in ["fabric-api", "neo_mod", "_1", "1mod", &"a".repeat(64)] {
            let text = format!("id = {id:?}\nversion = '1.2+build'\nsha512 = {hash:?}\n");
            let jar: JarRef = toml::from_str(&text).unwrap();
            assert_eq!(jar.id, id);
            assert_eq!(
                toml::from_str::<JarRef>(&toml::to_string(&jar).unwrap()).unwrap(),
                jar
            );
            assert!(toml::from_str::<JarRef>(&format!("{text}loader = 'fabric'\n")).is_err());
        }
        for id in [
            "",
            "a",
            "Fabric",
            "-bad",
            "1-mod",
            "bad.id",
            "bad/id",
            &"a".repeat(65),
        ] {
            assert!(
                JarRef::new(id.into(), "1".into(), hash.clone()).is_err(),
                "{id}"
            );
        }
        assert!(JarRef::new("good".into(), String::new(), hash.clone()).is_err());
        for bad_hash in [
            "a".repeat(127),
            "a".repeat(129),
            "A".repeat(128),
            "g".repeat(128),
        ] {
            let text = format!("id = 'good'\nversion = '1'\nsha512 = {bad_hash:?}\n");
            assert!(toml::from_str::<JarRef>(&text).is_err());
        }
    }
}
