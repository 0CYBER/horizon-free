use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

pub use super::Side;

/// An original Java jar used as a passive asset/data layer, in manifest order.
///
/// `take` lists the path prefixes of the jar the mod layers, each `assets/<namespace>` or
/// `data/<namespace>`, optionally deeper; without it the whole jar is layered. A jar whose
/// `take` names no `data/` prefix is client-only ([`JarRef::side`]).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(try_from = "Fields")]
pub struct JarRef {
    id: String,
    version: String,
    sha512: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    take: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    id: String,
    version: String,
    sha512: String,
    #[serde(default)]
    take: Option<Vec<String>>,
}

impl TryFrom<Fields> for JarRef {
    type Error = String;

    fn try_from(fields: Fields) -> Result<Self, Self::Error> {
        let jar = Self::new(fields.id, fields.version, fields.sha512)?;
        match fields.take {
            Some(take) => jar.with_take(take),
            None => Ok(jar),
        }
    }
}

/// One exact file and every Horizon mod requiring it, in load order.
///
/// `side` is the widest of its owners' jar sides: [`Side::Client`] only when every owner's
/// `take` leaves out `data/`. The jar is the exact file's identity, without any owner's `take`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(try_from = "RequirementFields")]
pub struct JarRequirement {
    pub owners: Vec<String>,
    pub jar: JarRef,
    pub side: Side,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequirementFields {
    owners: Vec<String>,
    jar: JarRef,
    #[serde(default = "both_sides")]
    side: Side,
}

fn both_sides() -> Side {
    Side::Both
}

impl JarRequirement {
    /// Whether an installation of `side` needs this file: a client needs every jar, a
    /// server (a dedicated server, the server export, a headless run) only those whose side
    /// is [`Side::Both`].
    pub fn needed_on(&self, side: Side) -> bool {
        side != Side::Server || self.side == Side::Both
    }
}

impl TryFrom<RequirementFields> for JarRequirement {
    type Error = String;

    fn try_from(fields: RequirementFields) -> Result<Self, Self::Error> {
        if !matches!(fields.side, Side::Client | Side::Both) {
            return Err(format!("invalid jar side: {:?}", fields.side));
        }
        if fields.jar.take.is_some() {
            return Err(format!(
                "jar {}: a requirement names no take",
                fields.jar.id
            ));
        }
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
            side: fields.side,
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
/// Owners may take different parts of one file; the requirement's side is the widest.
pub fn collect_jar_requirements<'a>(
    mods: impl IntoIterator<Item = (&'a str, &'a [JarRef])>,
) -> Result<Vec<JarRequirement>, JarConflict> {
    collect(mods)
}

fn collect<'a, J: IntoIterator<Item = &'a JarRef>>(
    mods: impl IntoIterator<Item = (&'a str, J)>,
) -> Result<Vec<JarRequirement>, JarConflict> {
    let mut hashes: BTreeMap<&str, JarRequirement> = BTreeMap::new();
    for (owner, jars) in mods {
        for jar in jars {
            if let Some(first) = hashes.get_mut(jar.sha512()) {
                if first.jar != jar.identity() {
                    return Err(JarConflict {
                        first: Box::new(first.jar.clone()),
                        second: Box::new(jar.identity()),
                    });
                }
                if jar.side() == Side::Both {
                    first.side = Side::Both;
                }
                if !first.owners.iter().any(|id| id == owner) {
                    first.owners.push(owner.to_owned());
                }
            } else {
                hashes.insert(
                    jar.sha512(),
                    JarRequirement {
                        owners: vec![owner.to_owned()],
                        jar: jar.identity(),
                        side: jar.side(),
                    },
                );
            }
        }
    }
    Ok(hashes.into_values().collect())
}

/// The requirements an installation of `side` needs. Each declaration is filtered by
/// [`JarRef::needed_on`] before owners are gathered, so a mod whose take of a shared file
/// the installation skips neither owns that file's requirement nor conflicts over it.
pub fn jar_requirements_for<'a>(
    side: Side,
    mods: impl IntoIterator<Item = (&'a str, &'a [JarRef])>,
) -> Result<Vec<JarRequirement>, JarConflict> {
    collect(
        mods.into_iter()
            .map(move |(owner, jars)| (owner, jars.iter().filter(move |jar| jar.needed_on(side)))),
    )
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
            take: None,
        })
    }

    /// This jar layering only the files under `take`'s prefixes, validated as a manifest's
    /// `take` is: each entry is `assets/<namespace>` or `data/<namespace>`, optionally deeper,
    /// with no empty, `.` or `..` segment, no `\` or glob character, and none repeated or
    /// already covered by another entry. The list may not be empty.
    pub fn with_take(mut self, take: Vec<String>) -> Result<Self, String> {
        let id = &self.id;
        if take.is_empty() {
            return Err(format!(
                "jar {id}: take is empty; leave it out to take the whole jar"
            ));
        }
        for entry in &take {
            let segments: Vec<&str> = entry.split('/').collect();
            if !matches!(segments[0], "assets" | "data") || segments.len() < 2 {
                return Err(format!(
                    "jar {id}: take entry {entry:?} must start with assets/<namespace> or data/<namespace>"
                ));
            }
            if segments
                .iter()
                .any(|segment| segment.is_empty() || matches!(*segment, "." | ".."))
                || entry.contains(['\\', '*', '?', '[', ']', '{', '}'])
            {
                return Err(format!(
                    "jar {id}: take entry {entry:?} must be a plain path prefix: no leading, trailing or doubled '/', '.' or '..' segments, '\\' or glob characters"
                ));
            }
        }
        for (index, entry) in take.iter().enumerate() {
            for (other_index, other) in take.iter().enumerate() {
                if index == other_index {
                    continue;
                }
                if entry == other {
                    return Err(format!("jar {id}: take lists {entry:?} twice"));
                }
                if prefix_covers(other, entry) {
                    return Err(format!(
                        "jar {id}: take entry {entry:?} is already covered by {other:?}"
                    ));
                }
            }
        }
        self.take = Some(take);
        Ok(self)
    }

    /// The path prefixes this jar layers, or `None` for the whole jar.
    pub fn take(&self) -> Option<&[String]> {
        self.take.as_deref()
    }

    /// Whether the layer keeps the jar's file `path`: every file without `take`, else a file
    /// at or under one of its entries, compared whole segment by whole segment and case
    /// sensitively, so `assets/create` keeps `assets/create/lang/en_us.json` but not
    /// `assets/create_extra/...`.
    pub fn takes(&self, path: &str) -> bool {
        self.take
            .as_ref()
            .is_none_or(|take| take.iter().any(|entry| prefix_covers(entry, path)))
    }

    /// Which installations need this jar: [`Side::Client`] when its `take` names no `data/`
    /// prefix, so dedicated servers, the server export and headless runs never fetch or open
    /// it; [`Side::Both`] without `take` or with any `data/` entry.
    pub fn side(&self) -> Side {
        match &self.take {
            Some(take) if !take.iter().any(|entry| entry.starts_with("data/")) => Side::Client,
            _ => Side::Both,
        }
    }

    /// Whether a run of `side` layers this jar: see [`JarRequirement::needed_on`].
    pub fn needed_on(&self, side: Side) -> bool {
        side != Side::Server || self.side() == Side::Both
    }

    /// The exact file's identity, without the `take` of the mod listing it.
    pub fn identity(&self) -> Self {
        Self {
            take: None,
            ..self.clone()
        }
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

/// Whether `prefix` names `path` itself or a folder holding it, segment by segment.
fn prefix_covers(prefix: &str, path: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
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

    fn with_take(take: &str) -> Result<JarRef, String> {
        toml::from_str(&format!(
            "id = 'original'\nversion = '1'\nsha512 = '{}'\ntake = {take}\n",
            "a".repeat(128)
        ))
        .map_err(|e| e.to_string())
    }

    #[test]
    fn take_keeps_whole_segments_under_its_prefixes_and_roundtrips() {
        let jar =
            with_take("['assets/create', 'assets/minecraft/atlases', 'data/c/tags/item/x.json']")
                .unwrap();
        for kept in [
            "assets/create",
            "assets/create/lang/en_us.json",
            "assets/minecraft/atlases/blocks.json",
            "data/c/tags/item/x.json",
        ] {
            assert!(jar.takes(kept), "{kept}");
        }
        for left in [
            "assets/create_extra/lang/en_us.json",
            "assets/Create/lang/en_us.json",
            "assets/minecraft/models/block/stone.json",
            "assets/minecraft/atlases.json",
            "data/c/tags/item/x.json5",
            "data/create/recipe/a.json",
            "META-INF/MANIFEST.MF",
            "pack.mcmeta",
        ] {
            assert!(!jar.takes(left), "{left}");
        }
        assert_eq!(
            toml::from_str::<JarRef>(&toml::to_string(&jar).unwrap()).unwrap(),
            jar
        );
        let whole = JarRef::new("original".into(), "1".into(), "a".repeat(128)).unwrap();
        assert!(whole.takes("META-INF/MANIFEST.MF"));
        assert!(!toml::to_string(&whole).unwrap().contains("take"));
    }

    #[test]
    fn take_refuses_what_is_not_a_plain_asset_or_data_prefix() {
        for bad in [
            "[]",
            "['assets']",
            "['data/']",
            "['/assets/create']",
            "['assets/create/']",
            "['assets//create']",
            "['assets/./create']",
            "['assets/../data/create']",
            "['assets\\\\create']",
            "['assets/create/*.json']",
            "['assets/cre?te']",
            "['assets/[ab]']",
            "['assets/{a,b}']",
            "['META-INF/services']",
            "['pack.mcmeta']",
            "['Assets/create']",
            "['assets/create', 'assets/create']",
            "['assets/create', 'assets/create/lang']",
            "['assets/create/lang', 'assets/create']",
        ] {
            assert!(with_take(bad).is_err(), "{bad}");
        }
        assert!(with_take("['assets/create', 'assets/create_extra']").is_ok());
    }

    #[test]
    fn a_jar_taking_no_data_is_client_only_and_a_shared_file_takes_the_widest_side() {
        let assets = with_take("['assets/create']").unwrap();
        let data = with_take("['assets/create', 'data/create']").unwrap();
        let whole = assets.identity();
        assert_eq!(assets.side(), Side::Client);
        assert_eq!(data.side(), Side::Both);
        assert_eq!(whole.side(), Side::Both);

        let client = collect_jar_requirements([("alpha", std::slice::from_ref(&assets))]).unwrap();
        assert_eq!(client[0].side, Side::Client);
        assert_eq!(client[0].jar, whole);
        assert!(client[0].needed_on(Side::Client));
        assert!(!client[0].needed_on(Side::Server));
        assert!(
            jar_requirements_for(Side::Server, [("alpha", std::slice::from_ref(&assets))])
                .unwrap()
                .is_empty()
        );

        let shared = collect_jar_requirements([
            ("alpha", std::slice::from_ref(&assets)),
            ("beta", std::slice::from_ref(&data)),
        ])
        .unwrap();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].owners, ["alpha", "beta"]);
        assert_eq!(shared[0].side, Side::Both);
        assert!(shared[0].needed_on(Side::Server));
        let on_server = jar_requirements_for(
            Side::Server,
            [
                ("alpha", std::slice::from_ref(&assets)),
                ("beta", std::slice::from_ref(&data)),
            ],
        )
        .unwrap();
        assert_eq!(on_server.len(), 1);
        assert_eq!(on_server[0].owners, ["beta"]);
        let on_client = jar_requirements_for(
            Side::Client,
            [
                ("alpha", std::slice::from_ref(&assets)),
                ("beta", std::slice::from_ref(&data)),
            ],
        )
        .unwrap();
        assert_eq!(on_client[0].owners, ["alpha", "beta"]);

        let recorded = serde_json::to_value(&client[0]).unwrap();
        assert_eq!(recorded["side"], "client");
        assert_eq!(
            serde_json::from_value::<JarRequirement>(recorded).unwrap(),
            client[0]
        );
        let mut earlier = serde_json::to_value(&shared[0]).unwrap();
        earlier.as_object_mut().unwrap().remove("side");
        assert_eq!(
            serde_json::from_value::<JarRequirement>(earlier)
                .unwrap()
                .side,
            Side::Both
        );
        let mut server = serde_json::to_value(&shared[0]).unwrap();
        server["side"] = serde_json::json!("server");
        assert!(serde_json::from_value::<JarRequirement>(server).is_err());
    }
}
