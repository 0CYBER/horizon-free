//! `horizon-mod.toml`, a mod's manifest, and the `.hmod` file around it.
//!
//! Raw injection schema adapted from horizon-mixins (Copyright (c) 2026 levzzz),
//! MIT licensed; see `HORIZON_MIXINS_LICENSE` and `THIRD_PARTY_NOTICES.md`.
pub mod inject;
mod jar;
pub mod loops;
pub mod mixin;
mod wasm;
pub use jar::{
    JarConflict, JarInputsError, JarRef, JarRequirement, MissingJar, collect_jar_requirements,
    missing_jars,
};
pub use wasm::{MAX_WASM, Wasm, WasmError, sha512_of};

use mod_surface::uses::USES_FILE;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    io::Read,
    path::{Path, PathBuf},
};

/// The manifest's file name in a `.hmod` or a mod folder.
pub const MANIFEST: &str = "horizon-mod.toml";
/// Executable components are not content and are refused in a `.hmod`.
pub const WASM: &str = "mod.wasm";
/// The largest uses table ([`USES_FILE`]) a package may carry: what the surface crate reads.
pub const MAX_USES: u64 = mod_surface::uses::MAX_USES_BYTES as u64;
/// The content manifest API this build loads.
pub const API: &str = "0.2";
/// Namespaces no mod may claim: vanilla's, the convention tags' (mods add
/// to those, docs/modding/src/how-to/content-mods.md) and the game's own.
const RESERVED_NAMESPACES: [&str; 3] = ["minecraft", "c", "horizon"];
/// A mod's saved settings, shared by native mods and modpack overrides.
pub const STORAGE_DIR: &str = ".horizon-mod-data";
/// wasm32's whole address space in MiB (4 GiB): the most a mod may declare in `wasm_memory_mib`,
/// and an instance's group memory cap unless the player lowers it (`horizon-mods.txt`).
pub const MAX_WASM_MEMORY_MIB: u32 = 4096;

/// The largest manifest this build reads.
/// Maximum manifest bytes accepted by package readers and developer tools.
pub const MAX_MANIFEST: u64 = 32 << 20;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// `[a-z0-9_]`, 1 to 64 characters: the mod's folder, log and settings key.
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// The Mods screen's `License:` line (a name such as `MIT`); empty shows it as not given.
    #[serde(default)]
    pub license: String,
    /// Where the license text is: the screen makes its name a link.
    #[serde(default)]
    pub license_url: Option<String>,
    /// The Mods screen's `Credits:` line.
    #[serde(default)]
    pub credits: String,
    /// The mod's home page, the Mods screen's Homepage button.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Where to report problems, the Mods screen's Issues button.
    #[serde(default)]
    pub issues: Option<String>,
    /// The content API line: `0.2`.
    pub api: String,
    #[serde(default)]
    pub side: Side,
    pub capabilities: Vec<Capability>,
    /// Namespaces besides `id` that the mod's `assets/` and `data/` use, for
    /// a content mod whose files sit under other names than its id (Towns
    /// and Towers ships `kaisyn` and `towns_and_towers`). No two content mods
    /// that are on may share one (`content::clash`).
    #[serde(default)]
    pub namespaces: Vec<String>,
    /// Other native mods whose hooks must be available before this one.
    #[serde(default)]
    pub dependencies: std::collections::BTreeMap<String, Dependency>,
    /// Mods this one breaks with: the game refuses to start while one of them is loaded at a
    /// version the rule matches (NeoForge's `DependencyType.INCOMPATIBLE`).
    #[serde(default)]
    pub incompatible: std::collections::BTreeMap<String, Rule>,
    /// Mods this one works badly with: they load, with a warning (`DependencyType.DISCOURAGED`).
    #[serde(default)]
    pub discouraged: std::collections::BTreeMap<String, Rule>,
    /// Mods whose hooks run after this one's when they are loaded, without depending on them
    /// (`Ordering.BEFORE`).
    #[serde(default)]
    pub load_before: Vec<String>,
    /// Original Java jars, layered in list order below this mod's own files.
    #[serde(default)]
    pub jar: Vec<JarRef>,
    /// Exact build pins and support modules for a code mod.
    pub native: Option<Native>,
    /// A wasm mod's `mod.wasm`, its toolchain and what it was linked against.
    pub wasm: Option<Wasm>,
    /// The linear memory, in MiB (1 to [`MAX_WASM_MEMORY_MIB`]), this wasm mod or library expects
    /// its dependency group to have. The group gets the largest any member declares, up to the
    /// instance's cap; with none declared it gets 1 GiB. Native mods are not wasm groups: they
    /// share the game's memory, and the key means nothing to them.
    #[serde(default)]
    pub wasm_memory_mib: Option<u32>,
    /// Game internals used by this mod, fingerprinted against its pinned build.
    #[serde(default)]
    pub internals: Vec<Internal>,
    #[serde(default, deserialize_with = "inject::rows")]
    pub inject: Vec<inject::Inject>,
    /// Free-form values other mods read through `PackageInfo::property`.
    #[serde(default)]
    pub properties: std::collections::BTreeMap<String, Property>,
}

/// The most `[properties]` keys a manifest may declare.
pub const MAX_PROPERTIES: usize = 64;
/// The most `[properties]` bytes: keys and strings by length, 8 per number, 1 per boolean.
pub const MAX_PROPERTY_BYTES: usize = 8 << 10;

/// One `[properties]` value: a table or array is refused.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Property {
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String),
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Dependency {
    Version(semver::VersionReq),
    Detailed(DependencyDetails),
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DependencyDetails {
    pub version: semver::VersionReq,
    #[serde(default)]
    pub optional: bool,
    /// The installation on which this dependency declaration applies.
    #[serde(default = "both_sides")]
    pub side: Side,
    /// A mod-provided explanation, at most 512 bytes.
    #[serde(default)]
    pub reason: Option<String>,
    /// An HTTPS link with more information.
    #[serde(default)]
    pub url: Option<String>,
    /// Dispatch ordering only; `none` does not remove a code link requirement.
    #[serde(default)]
    pub order: mod_vocab::DependencyOrder,
}

impl Dependency {
    /// The dependency's typed declaration, with bare-version defaults filled in.
    pub fn details(&self) -> DependencyDetails {
        match self {
            Self::Version(version) => DependencyDetails {
                version: version.clone(),
                optional: false,
                side: Side::Both,
                reason: None,
                url: None,
                order: mod_vocab::DependencyOrder::After,
            },
            Self::Detailed(details) => details.clone(),
        }
    }
}

/// An `[incompatible]` or `[discouraged]` entry: the versions of the other mod it matches
/// (`"<2"`), or a table that also gives the player a reason and a link.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Rule {
    Version(semver::VersionReq),
    Detailed(RuleDetails),
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RuleDetails {
    /// Every version when left out.
    #[serde(default = "any_version")]
    pub version: semver::VersionReq,
    /// Why, shown when the rule fails: at most 512 bytes.
    #[serde(default)]
    pub reason: Option<String>,
    /// Where to read more, shown with the reason: an `https://` link.
    #[serde(default)]
    pub url: Option<String>,
    /// Where the rule holds: both sides, or only the client or the dedicated server.
    #[serde(default = "both_sides")]
    pub side: Side,
}

fn both_sides() -> Side {
    Side::Both
}

fn any_version() -> semver::VersionReq {
    semver::VersionReq::STAR
}

impl Rule {
    /// The versions, reason, link and side, the bare form's as defaults.
    pub fn details(&self) -> RuleDetails {
        match self {
            Self::Version(version) => RuleDetails {
                version: version.clone(),
                reason: None,
                url: None,
                side: Side::Both,
            },
            Self::Detailed(details) => details.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Native {
    pub game: String,
    pub horizon: String,
    pub mixins: String,
    /// Source-level edits of game functions, native only: the one form that compiles game code.
    #[serde(default)]
    pub mixin: Vec<mixin::Mixin>,
    #[serde(default)]
    pub module: Vec<Module>,
    /// Accessor and invoker bridges, forwarded as horizon-mixins' `[[bridge]]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bridge: Vec<mixin::Forwarded>,
    /// State fields added to game types, forwarded as horizon-mixins' `[[state]]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state: Vec<mixin::Forwarded>,
    /// Slots of published game loops, native only (`mod_api::loops`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub r#loop: Vec<loops::Loop>,
}

/// One declared game internal, as listed in the pinned game's `internals.json`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Internal {
    pub path: String,
    /// The catalog's member kind, required when several kinds share the path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Optional pinning: the catalog fingerprint `internals add` writes. Absent, the build takes
    /// it from the pinned game's catalog, which `[native] horizon` fixes exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// A support module merged into a game crate, forwarded as horizon-mixins'
/// `[[module]]`: self-contained Rust items, emitted into `file` as
/// `mod name`, for helpers that need more than one function.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub id: String,
    /// The game source file that receives the module, as the pin indexes it.
    pub file: String,
    /// The module's Rust name under the destination file's top level.
    pub name: String,
    /// The payload's file in this package, under `src/`.
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
}

pub use mod_vocab::Side;

/// What a content mod adds; the player sees it before turning the mod on.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// New blocks, items and mob effects from the mod's `data/` (API 0.2).
    Registries,
}
impl Capability {
    /// What the player reads before turning the mod on: "May <this>".
    pub fn describe(self) -> &'static str {
        match self {
            Capability::Registries => {
                "add blocks, items, mob effects and entity types to your worlds"
            }
        }
    }
}
impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Capability::Registries => f.write_str("registries"),
        }
    }
}

/// Whether a source pin is an exact lowercase Git object id.
pub fn revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn valid_mod_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && !reserved_mod_id(id)
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn reserved_mod_id(id: &str) -> bool {
    matches!(id, "con" | "prn" | "aux" | "nul")
        || (id.len() == 4
            && (id.starts_with("com") || id.starts_with("lpt"))
            && matches!(id.as_bytes()[3], b'1'..=b'9'))
}

fn rust_ident(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !name.as_bytes()[0].is_ascii_digit()
}

impl Internal {
    fn check_all(internals: &[Self]) -> Result<(), String> {
        let mut paths = std::collections::HashSet::new();
        for internal in internals {
            if internal.path.is_empty()
                || internal.hash.as_ref().is_some_and(String::is_empty)
                || internal.kind.as_ref().is_some_and(|kind| kind.is_empty())
                || !paths.insert((&internal.path, &internal.kind))
            {
                return Err(format!(
                    "{MANIFEST}: empty internal path, kind or hash, or duplicate internal path and kind"
                ));
            }
        }
        Ok(())
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let both = || {
            format!(
                "{MANIFEST}: a mod is [native] or [wasm], not both; only [native] tables \
                 compile game code, so a wasm mod has no mixin, bridge, state or module"
            )
        };
        // A wasm manifest whose only fault is its `[native]` table is told that, not which
        // pin a half-written `[native]` lacks. Any other error stays the one reported.
        // A wasm mod's loop row is refused by its own name, whatever else the manifest holds.
        let wasm_loop = || {
            let table = toml::from_str::<toml::Table>(text).ok()?;
            table.get("wasm")?;
            let row = table.get("native")?.get("loop")?.as_array()?.first()?;
            Some(format!(
                "{MANIFEST}: {}",
                loops::LoopError::Wasm {
                    id: format!(
                        "{}:{}",
                        table.get("id").and_then(toml::Value::as_str).unwrap_or("?"),
                        row.get("id").and_then(toml::Value::as_str).unwrap_or("?")
                    ),
                    target: row
                        .get("loop")
                        .and_then(toml::Value::as_str)
                        .unwrap_or("?")
                        .to_owned()
                }
            ))
        };
        let m: Manifest = toml::from_str(text).map_err(|e| {
            // Serde knows the rejected field; this owning boundary supplies its provenance.
            if let Ok(table) = toml::from_str::<toml::Table>(text)
                && let Some(rows) = table.get("inject").and_then(toml::Value::as_array)
            {
                for row in rows {
                    if let Some(Err(error)) = row.as_table().cloned().map(inject::row) {
                        let field = |key| row.get(key).and_then(toml::Value::as_str).unwrap_or("?");
                        let id = table.get("id").and_then(toml::Value::as_str).unwrap_or("?");
                        return format!(
                            "{MANIFEST}: {id}:{}: {}: {error}",
                            field("id"),
                            field("at")
                        );
                    }
                }
            }
            let native_alone = toml::from_str::<toml::Table>(text).is_ok_and(|mut table| {
                table.contains_key("wasm")
                    && table.remove("native").is_some()
                    && table.try_into::<Manifest>().is_ok()
            });
            if native_alone {
                wasm_loop().unwrap_or_else(both)
            } else {
                format!("{MANIFEST}: {e}")
            }
        })?;
        if m.side == Side::Server {
            return Err(format!(
                "{MANIFEST}: top-level side = \"server\" is not a supported package side"
            ));
        }
        inject::Inject::check_all(&m.inject, &m.id, m.native.is_some() || m.wasm.is_some())
            .map_err(|e| e.to_string())?;
        for entry in &m.inject {
            if m.dependencies
                .keys()
                .any(|id| entry.at.split("::").next() == Some(id.replace('-', "_").as_str()))
            {
                return Err(format!(
                    "{}: target {} is another mod's function; use the library mod's own hooks and functions",
                    entry.id, entry.at
                ));
            }
        }
        Internal::check_all(&m.internals)?;
        if !m.internals.is_empty() && m.native.is_none() && m.wasm.is_none() {
            return Err(format!(
                "{MANIFEST}: internals need a [native] or [wasm] mod"
            ));
        }
        if reserved_mod_id(&m.id) {
            return Err(format!(
                "{MANIFEST}: id {:?} is a reserved Windows device name",
                m.id
            ));
        }
        if !valid_mod_id(&m.id) {
            return Err(format!(
                "{MANIFEST}: id {:?} is not [a-z0-9_]{{1,64}}",
                m.id
            ));
        }
        let rules = m.incompatible.iter().chain(&m.discouraged);
        for id in m
            .dependencies
            .keys()
            .chain(rules.clone().map(|(id, _)| id))
            .chain(&m.load_before)
        {
            if !valid_mod_id(id) {
                return Err(format!("{MANIFEST}: invalid dependency id {id:?}"));
            }
        }
        let rule_notes = rules.map(|(id, rule)| {
            let details = rule.details();
            (id, details.reason, details.url)
        });
        let dependency_notes = m.dependencies.iter().map(|(id, dependency)| {
            let details = dependency.details();
            (id, details.reason, details.url)
        });
        for (id, reason, url) in rule_notes.chain(dependency_notes) {
            if reason.as_ref().is_some_and(|reason| reason.len() > 512) {
                return Err(format!("{MANIFEST}: rule for {id}: reason over 512 bytes"));
            }
            if url.as_ref().is_some_and(|url| !url.starts_with("https://")) {
                return Err(format!("{MANIFEST}: rule for {id}: url is not https://"));
            }
        }
        if let Some(mib) = m.wasm_memory_mib
            && !(1..=MAX_WASM_MEMORY_MIB).contains(&mib)
        {
            return Err(format!(
                "{MANIFEST}: wasm_memory_mib {mib} is not 1 to {MAX_WASM_MEMORY_MIB} (wasm32's 4 GiB)"
            ));
        }
        if m.native.is_some() && m.wasm.is_some() {
            if let Some(refusal) = wasm_loop() {
                return Err(refusal);
            }
            return Err(format!(
                "{MANIFEST}: a mod is [native] or [wasm], not both; only [native] tables compile game code, so a wasm mod has no mixin, bridge, state or module"
            ));
        }
        mixin::Mixin::check_all(
            m.native
                .as_ref()
                .map(|n| n.mixin.as_slice())
                .unwrap_or_default(),
            &m.id,
        )
        .map_err(|e| format!("{MANIFEST}: {e}"))?;
        loops::Loop::check_all(
            m.native
                .as_ref()
                .map(|n| n.r#loop.as_slice())
                .unwrap_or_default(),
            &m.id,
        )
        .map_err(|e| format!("{MANIFEST}: {e}"))?;
        if let Some(entry) = m
            .native
            .as_ref()
            .map(|n| n.mixin.as_slice())
            .unwrap_or_default()
            .iter()
            .find(|entry| {
                m.dependencies.keys().any(|id| {
                    entry.target.split("::").next() == Some(id.replace('-', "_").as_str())
                })
            })
        {
            return Err(format!(
                "{MANIFEST}: {}",
                mixin::MixinError::Invalid {
                    id: format!("{}:{}", m.id, entry.id),
                    field: "target",
                    fix: "a mixin targets a game function",
                }
            ));
        }
        if let Some(native) = &m.native {
            // Native source edits share one id space.
            let mut ids: std::collections::HashSet<_> =
                m.inject.iter().map(|entry| &entry.id).collect();
            for id in native
                .mixin
                .iter()
                .map(|entry| &entry.id)
                .chain(native.bridge.iter().chain(&native.state).map(|row| &row.id))
                .chain(native.r#loop.iter().map(|row| &row.id))
            {
                if id.trim().is_empty() || !ids.insert(id) {
                    return Err(format!(
                        "{MANIFEST}: empty or duplicate mixin, bridge, state or loop id"
                    ));
                }
            }
            // A bridge's id becomes part of its accessor's name: `-` reads as `_`, and case is
            // dropped, so the id is that alphabet and no two ids may give one name.
            let mut accessors = std::collections::HashMap::new();
            for bridge in &native.bridge {
                let id = &bridge.id;
                if !id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    return Err(format!(
                        "{MANIFEST}: E_MIXIN_INVALID: {}:{id}: invalid id; a bridge id is ASCII letters, digits, `_` and `-`",
                        m.id
                    ));
                }
                if let Some(other) = accessors.insert(id.replace('-', "_").to_ascii_lowercase(), id)
                {
                    return Err(format!(
                        "{MANIFEST}: E_MIXIN_INVALID: {}:{id}: invalid id; it gives the same accessor name as bridge `{other}`",
                        m.id
                    ));
                }
            }
            for module in &native.module {
                if module.id.trim().is_empty() || !ids.insert(&module.id) {
                    return Err(format!("{MANIFEST}: empty or duplicate module id"));
                }
                if !plain_path(&module.file) {
                    return Err(format!("{MANIFEST}: module file must be a plain path"));
                }
                if !rust_ident(&module.name) {
                    return Err(format!("{MANIFEST}: module name must be an identifier"));
                }
                if !plain_path(&module.source) || !source_file(&module.source) {
                    return Err(format!(
                        "{MANIFEST}: module source must be a file under src/"
                    ));
                }
                if let Some(visibility) = &module.visibility
                    && !matches!(visibility.as_str(), "pub" | "pub(crate)")
                {
                    return Err(format!(
                        "{MANIFEST}: module visibility must be pub or pub(crate)"
                    ));
                }
            }
            if m.version.is_empty()
                || native.game.is_empty()
                || !revision(&native.horizon)
                || !revision(&native.mixins)
            {
                return Err(format!(
                    "{MANIFEST}: native requires game and exact revisions"
                ));
            }
        }
        if m.api != API {
            return Err(format!(
                "unsupported content API {}; this Horizon reads {API}",
                m.api
            ));
        }
        let property_bytes: usize = m
            .properties
            .iter()
            .map(|(key, value)| {
                key.len()
                    + match value {
                        Property::Boolean(_) => 1,
                        Property::Integer(_) | Property::Float(_) => 8,
                        Property::String(text) => text.len(),
                    }
            })
            .sum();
        if m.properties.len() > MAX_PROPERTIES || property_bytes > MAX_PROPERTY_BYTES {
            return Err(format!(
                "{MANIFEST}: [properties] holds at most {MAX_PROPERTIES} keys and {MAX_PROPERTY_BYTES} \
                 bytes; this one has {} keys and {property_bytes} bytes",
                m.properties.len()
            ));
        }
        if m.capabilities != [Capability::Registries] {
            return Err(format!(
                "{MANIFEST}: a content mod has capabilities = [\"registries\"]"
            ));
        }
        for (field, url) in [
            ("license_url", &m.license_url),
            ("homepage", &m.homepage),
            ("issues", &m.issues),
        ] {
            if let Some(url) = url
                && (url.len() > 2048
                    || !(url.starts_with("https://") || url.starts_with("http://")))
            {
                return Err(format!(
                    "{MANIFEST}: {field} is an http(s) link of at most 2048 characters"
                ));
            }
        }
        for (i, ns) in m.namespaces.iter().enumerate() {
            // Vanilla's namespace characters, at most as long as an id.
            let chars_ok = (1..=64).contains(&ns.len())
                && ns
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b));
            if !chars_ok {
                return Err(format!(
                    "{MANIFEST}: namespace {ns:?} is not [a-z0-9_.-]{{1,64}}"
                ));
            }
            if RESERVED_NAMESPACES.contains(&ns.as_str()) {
                return Err(format!(
                    "{MANIFEST}: namespace {ns} is not a mod's to claim (only to add to its tags)"
                ));
            }
            if *ns == m.id || m.namespaces[..i].contains(ns) {
                return Err(format!("{MANIFEST}: namespace {ns} is listed twice"));
            }
        }
        Ok(m)
    }
    /// The namespaces the mod's content owns: its id, then `namespaces`.
    pub fn owned_namespaces(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.id.as_str()).chain(self.namespaces.iter().map(String::as_str))
    }
}

/// Folders of a mod's project that are never part of its package: Git's, Cargo's build output
/// and the pinned SDK the mod tool fetches into `.horizon/`.
pub const NOT_PACKAGED: [&str; 3] = [".git", "target", ".horizon"];

/// A source file that belongs to the flat Cargo project, not its resource packs.
pub fn source_file(name: &str) -> bool {
    if name.split('/').any(|part| NOT_PACKAGED.contains(&part)) {
        return false;
    }
    name == "Cargo.toml"
        || name == "Cargo.lock"
        || name == "build.rs"
        || (name.starts_with("src/") && (name.ends_with(".rs") || name.ends_with(".toml")))
        || (!name.contains('/')
            && ["README", "LICENSE", "LICENCE", "COPYING", "NOTICE"]
                .iter()
                .any(|prefix| name.starts_with(prefix)))
}

fn plain_path(name: &str) -> bool {
    !name.starts_with('/')
        && !name.contains(['\\', ':'])
        && !name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

/// A mod as found on disk.
pub struct Package {
    pub manifest: Manifest,
    pub icon: Option<crate::icon::Icon>,
    pub layouts: crate::layouts::Layouts,
}
impl Package {
    /// The kind of this validated package, independent of its install folder.
    pub fn is_native(&self) -> bool {
        self.manifest.native.is_some()
    }

    /// Whether the package carries Rust source: a native mod, or a wasm mod built from it.
    fn has_source(&self) -> bool {
        self.manifest.native.is_some()
            || self
                .manifest
                .wasm
                .as_ref()
                .is_some_and(|w| w.built_with().is_none())
    }

    /// The package's `mod.wasm`, checked against its `[wasm]` section.
    pub fn read_wasm(path: &Path) -> Result<Vec<u8>, String> {
        let package = Self::read(path)?;
        let wasm = package
            .manifest
            .wasm
            .ok_or(WasmError::Undeclared.to_string())?;
        // Read at most one byte past the cap, so an oversized file is refused unread.
        let mut bytes = Vec::new();
        let read = if path.is_dir() {
            std::fs::File::open(path.join(WASM))
                .map_err(|e| format!("{WASM}: {e}"))?
                .take(MAX_WASM + 1)
                .read_to_end(&mut bytes)
        } else {
            #[cfg(target_arch = "wasm32")]
            return Err("archive I/O is unavailable on wasm32".into());
            #[cfg(not(target_arch = "wasm32"))]
            {
                let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
                let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
                zip.by_name(WASM)
                    .map_err(|_| WasmError::Missing.to_string())?
                    .take(MAX_WASM + 1)
                    .read_to_end(&mut bytes)
            }
        };
        read.map_err(|e| format!("{WASM}: {e}"))?;
        wasm.check(&bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    }

    /// Read a flat `.hmod` or project folder, without executing native code.
    pub fn read(path: &Path) -> Result<Self, String> {
        let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        let mut icon = None;
        let mut has_cargo = false;
        let mut has_source = false;
        let has_wasm;
        let toml = if metadata.is_dir() {
            for old in [
                "horizon-native.json",
                "horizon-mixin.toml",
                "native",
                "rust",
            ] {
                if std::fs::symlink_metadata(path.join(old)).is_ok() {
                    return Err(format!("{old}: unsupported package path"));
                }
            }
            if std::fs::symlink_metadata(path.join("Cargo.toml")).is_ok() {
                check_folder_file(path, "Cargo.toml")?;
                has_cargo = true;
            }
            if let Ok(source) = std::fs::symlink_metadata(path.join("src")) {
                if !source.is_dir() {
                    return Err("src: unsupported package path".into());
                }
                has_source = true;
            }
            let icon_path = path.join("icon.png");
            match std::fs::symlink_metadata(&icon_path) {
                Ok(metadata) => {
                    check_folder_file(path, "icon.png")?;
                    if metadata.len() > crate::icon::MAX_ICON as u64 {
                        return Err("icon.png exceeds 256 KiB".into());
                    }
                    let mut bytes = Vec::new();
                    std::fs::File::open(&icon_path)
                        .map_err(|e| format!("icon.png: {e}"))?
                        .take(crate::icon::MAX_ICON as u64 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| format!("icon.png: {e}"))?;
                    icon = Some(crate::icon::Icon::parse(bytes)?);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("icon.png: {e}")),
            }
            check_folder_file(path, MANIFEST)?;
            has_wasm = std::fs::symlink_metadata(path.join(WASM)).is_ok();
            for (name, _) in crate::content::folder_files(path)? {
                check_folder_file(path, &name)?;
            }
            let mut text = String::new();
            std::fs::File::open(path.join(MANIFEST))
                .map_err(|e| format!("{MANIFEST}: {e}"))?
                .take(MAX_MANIFEST + 1)
                .read_to_string(&mut text)
                .map_err(|e| format!("{MANIFEST}: {e}"))?;
            if text.len() as u64 > MAX_MANIFEST {
                return Err(format!("{MANIFEST}: over {} MiB", MAX_MANIFEST >> 20));
            }
            text
        } else {
            if !metadata.is_file() {
                return Err(format!(
                    "{}: unsupported file type in a content mod",
                    path.display()
                ));
            }
            #[cfg(target_arch = "wasm32")]
            return Err("archive I/O is unavailable on wasm32".into());
            #[cfg(not(target_arch = "wasm32"))]
            {
                let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
                let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
                return Self::read_zip(&mut zip);
            }
        };
        let mut package = Self::from_parts(&toml, icon, has_cargo, has_source, has_wasm)?;
        match std::fs::File::open(path.join(crate::layouts::FILE)) {
            Ok(file) => {
                check_folder_file(path, crate::layouts::FILE)?;
                package.layouts = crate::layouts::Layouts::read(file)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("layouts.json: {e}")),
        }
        if let Some(wasm) = &package.manifest.wasm {
            let bytes = std::fs::read(path.join(WASM)).map_err(|e| format!("{WASM}: {e}"))?;
            wasm.check(&bytes).map_err(|e| e.to_string())?;
        }
        Ok(package)
    }

    /// Read and validate an archive, including all selected source and resource entries.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_zip<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> Result<Self, String> {
        let mut icon = None;
        let mut has_cargo = false;
        let mut has_source = false;
        let mut has_wasm = false;
        let mut entries = Vec::new();
        let mut layouts = None;
        let mut uses = false;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let name = entry.name().map_err(|e| e.to_string())?.into_owned();
            let mode = entry.unix_mode().unwrap_or_default();
            entries.push((name.clone(), entry.size(), entry.is_dir()));
            let first = name.split('/').next().unwrap_or("");
            if matches!(
                first,
                "horizon-native.json" | "horizon-mixin.toml" | "native" | "rust"
            ) {
                return Err(format!("{name}: unsupported package path"));
            }
            has_source |= first == "src";
            if entry.is_dir() && matches!(mode & 0o170000, 0 | 0o040000) {
                continue;
            }
            has_cargo |= name == "Cargo.toml";
            // The one executable a package may carry, checked against `[wasm]` below.
            if name == WASM && matches!(mode & 0o170000, 0 | 0o100000) {
                has_wasm = true;
                continue;
            }
            if name == crate::layouts::FILE && matches!(mode & 0o170000, 0 | 0o100000) {
                if layouts.is_some() {
                    return Err("duplicate layouts.json".into());
                }
                layouts = Some(crate::layouts::Layouts::read(&mut entry)?);
                continue;
            }
            if name == USES_FILE && matches!(mode & 0o170000, 0 | 0o100000) {
                if uses {
                    return Err(format!("duplicate {USES_FILE}"));
                }
                if entry.size() > MAX_USES {
                    return Err(format!("{USES_FILE} exceeds {} MiB", MAX_USES >> 20));
                }
                uses = true;
                continue;
            }
            check_file(&name, mode, || Ok(&mut entry)).map_err(|e| e.to_string())?;
            if name == "icon.png" {
                if icon.is_some() {
                    return Err("duplicate icon.png".into());
                }
                if entry.size() > crate::icon::MAX_ICON as u64 {
                    return Err("icon.png exceeds 256 KiB".into());
                }
                let mut bytes = Vec::new();
                drop(entry);
                zip.by_index(i)
                    .map_err(|e| e.to_string())?
                    .take(crate::icon::MAX_ICON as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| format!("icon.png: {e}"))?;
                icon = Some(crate::icon::Icon::parse(bytes)?);
            }
        }
        let mut toml = String::new();
        zip.by_name(MANIFEST)
            .map_err(|e| format!("{MANIFEST}: {e}"))?
            .take(MAX_MANIFEST + 1)
            .read_to_string(&mut toml)
            .map_err(|e| format!("{MANIFEST}: {e}"))?;
        if toml.len() as u64 > MAX_MANIFEST {
            return Err(format!("{MANIFEST}: over {} MiB", MAX_MANIFEST >> 20));
        }
        let mut package = Self::from_parts(&toml, icon, has_cargo, has_source, has_wasm)?;
        if uses && package.manifest.native.is_none() && package.manifest.wasm.is_none() {
            return Err(format!("a content mod has no code, so no {USES_FILE}"));
        }
        if let Some(wasm) = &package.manifest.wasm {
            let mut bytes = Vec::new();
            zip.by_name(WASM)
                .map_err(|e| format!("{WASM}: {e}"))?
                .take(MAX_WASM + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("{WASM}: {e}"))?;
            wasm.check(&bytes).map_err(|e| e.to_string())?;
        }
        package.layouts = layouts.unwrap_or_default();
        if package.has_source() || package.manifest.wasm.is_some() {
            let mut names = std::collections::HashSet::new();
            let mut source_count = 0usize;
            let mut source_bytes = 0u64;
            let mut resource_count = 0usize;
            let mut resource_bytes = 0u64;
            for (name, size, directory) in entries {
                let path = if directory {
                    name.trim_end_matches('/')
                } else {
                    &name
                };
                if !plain_path(path) || !names.insert(name.clone()) {
                    return Err(format!("invalid or duplicate package path: {name}"));
                }
                if !directory {
                    if name != MANIFEST
                        && name != crate::layouts::FILE
                        && name != USES_FILE
                        && name != "icon.png"
                        && !(name == WASM && package.manifest.wasm.is_some())
                        && !name.starts_with("assets/")
                        && !name.starts_with("data/")
                        && !source_file(&name)
                    {
                        return Err(format!("unexpected package path: {name}"));
                    }
                    if source_file(&name) {
                        source_count += 1;
                        source_bytes = source_bytes
                            .checked_add(size)
                            .ok_or("native package too large")?;
                    } else if name.starts_with("assets/") || name.starts_with("data/") {
                        crate::content::check_path(&name)?;
                        crate::content::check_namespace(&name, &package.manifest)?;
                        if size > crate::content::MAX_FILE {
                            return Err(format!("{name} exceeds the content file size limit"));
                        }
                        resource_count += 1;
                        resource_bytes = resource_bytes
                            .checked_add(size)
                            .ok_or("content too large")?;
                    }
                }
            }
            if source_count > 511 || source_bytes > 64 << 20 {
                return Err("native package exceeds its entry or size limit".into());
            }
            if resource_count > crate::content::MAX_FILES
                || resource_bytes > crate::content::MAX_TOTAL
            {
                return Err("content exceeds its entry or size limit".into());
            }
        }
        Ok(package)
    }

    fn from_parts(
        text: &str,
        icon: Option<crate::icon::Icon>,
        has_cargo: bool,
        has_source: bool,
        has_wasm: bool,
    ) -> Result<Self, String> {
        let manifest = Manifest::parse(text)?;
        let package = Package {
            manifest,
            icon,
            layouts: Default::default(),
        };
        if has_cargo != package.has_source() || has_source != has_cargo {
            return Err("inconsistent code declaration ([native] or [wasm]) and source".into());
        }
        match (has_wasm, package.manifest.wasm.is_some()) {
            (true, false) => return Err(WasmError::Undeclared.to_string()),
            (false, true) => return Err(WasmError::Missing.to_string()),
            _ => {}
        }
        Ok(package)
    }
    /// Whether the mod at `path` (a `.hmod` or a folder) has a [`MANIFEST`].
    pub fn has_manifest(path: &Path) -> bool {
        if path.is_dir() {
            return path.join(MANIFEST).is_file();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::fs::File::open(path)
                .ok()
                .and_then(|f| zip::ZipArchive::new(f).ok())
                .is_some_and(|mut zip| zip.by_name(MANIFEST).is_ok())
        }
        #[cfg(target_arch = "wasm32")]
        false
    }
    /// Select and validate a flat project's source and resource files deterministically.
    pub fn directory_files(
        &self,
        root: &Path,
    ) -> Result<std::collections::BTreeMap<String, Vec<u8>>, String> {
        let content = crate::Content::read(root, &self.manifest)?;
        let mut files = std::collections::BTreeMap::new();
        let mut reader = content.reader()?;
        for name in &content.files {
            files.insert(name.clone(), reader.read(name)?);
        }
        if let Ok(file) = std::fs::File::open(root.join(crate::layouts::FILE)) {
            let mut bytes = Vec::new();
            file.take((1 << 20) + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            crate::layouts::Layouts::read(bytes.as_slice())?;
            files.insert(crate::layouts::FILE.into(), bytes);
        }
        if let Some(icon) = &self.icon {
            files.insert("icon.png".to_string(), icon.bytes().to_vec());
        }
        if let Some(wasm) = &self.manifest.wasm {
            let bytes = std::fs::read(root.join(WASM)).map_err(|e| format!("{WASM}: {e}"))?;
            wasm.check(&bytes).map_err(|e| e.to_string())?;
            files.insert(WASM.to_owned(), bytes);
        }
        if self.has_source() {
            fn walk(
                root: &Path,
                dir: &Path,
                files: &mut std::collections::BTreeMap<String, Vec<u8>>,
                source_bytes: &mut u64,
                source_count: &mut usize,
            ) -> Result<(), String> {
                for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let path = entry.path();
                    let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
                    let name = relative
                        .to_str()
                        .ok_or("non-UTF-8 package path")?
                        .replace('\\', "/");
                    let kind = entry.file_type().map_err(|e| e.to_string())?;
                    if kind.is_dir() {
                        if entry
                            .file_name()
                            .to_str()
                            .is_some_and(|name| NOT_PACKAGED.contains(&name))
                            || matches!(name.as_str(), "assets" | "data")
                        {
                            continue;
                        }
                        walk(root, &path, files, source_bytes, source_count)?;
                    } else if source_file(&name) {
                        check_folder_file(root, &name)?;
                        let size = entry.metadata().map_err(|e| e.to_string())?.len();
                        *source_bytes = source_bytes
                            .checked_add(size)
                            .ok_or("native package too large")?;
                        *source_count += 1;
                        if *source_bytes > 64 << 20 || *source_count > 511 {
                            return Err("native package exceeds its entry or size limit".into());
                        }
                        files.insert(name, std::fs::read(path).map_err(|e| e.to_string())?);
                    } else if name.ends_with(".rs") || (name.ends_with(".toml") && name != MANIFEST)
                    {
                        return Err(format!("unexpected package path: {name}"));
                    }
                }
                Ok(())
            }
            let (mut source_bytes, mut source_count) = (0, 0);
            walk(root, root, &mut files, &mut source_bytes, &mut source_count)?
        }
        files.insert(
            MANIFEST.to_owned(),
            std::fs::read(root.join(MANIFEST)).map_err(|e| e.to_string())?,
        );
        Ok(files)
    }

    /// Pack a flat project, selecting its source and resource files deterministically.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn pack_directory(root: &Path) -> Result<Vec<u8>, String> {
        Self::pack_directory_with(root, None)
    }

    /// The bytes of entry `name` in the `.hmod` at `path`, refused past `most` bytes; `None` when
    /// it has no such entry.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn entry(path: &Path, name: &str, most: u64) -> Result<Option<Vec<u8>>, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", path.display()))?;
        let entry = match zip.by_name(name) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::FileNotFound) => return Ok(None),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut bytes = Vec::new();
        entry
            .take(most + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{name}: {e}"))?;
        if bytes.len() as u64 > most {
            return Err(format!("{name}: over {most} bytes"));
        }
        Ok(Some(bytes))
    }

    /// [`Package::pack_directory`], carrying `uses` as the package's [`USES_FILE`] table when given.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn pack_directory_with(root: &Path, uses: Option<&[u8]>) -> Result<Vec<u8>, String> {
        let package = Self::read(root)?;
        let mut files = package.directory_files(root)?;
        if let Some(uses) = uses {
            files.insert(USES_FILE.to_owned(), uses.to_vec());
        }
        let text = String::from_utf8(files.remove(MANIFEST).expect("selected manifest"))
            .map_err(|e| e.to_string())?;
        let files: Vec<_> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        Self::write_files(&text, &files)
    }

    /// The `.hmod` bytes for `manifest_text` and selected project files.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn write_files(manifest_text: &str, files: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
        use std::io::Write;
        let manifest = Manifest::parse(manifest_text)?;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        let err = |e: zip::result::ZipError| e.to_string();
        let mut put = |name: &str, bytes: &[u8]| -> Result<(), String> {
            zip.start_file(name, options).map_err(err)?;
            zip.write_all(bytes).map_err(|e| e.to_string())
        };
        put(MANIFEST, manifest_text.as_bytes())?;
        for (name, bytes) in files {
            let code = manifest.native.is_some() || manifest.wasm.is_some();
            let source = code && source_file(name) && plain_path(name);
            if *name == crate::layouts::FILE {
                crate::layouts::Layouts::read(*bytes)?;
                put(name, bytes)?;
                continue;
            }
            if *name == WASM && manifest.wasm.is_some() {
                put(name, bytes)?;
                continue;
            }
            if *name == USES_FILE && code {
                if bytes.len() as u64 > MAX_USES {
                    return Err(format!("{USES_FILE} exceeds {} MiB", MAX_USES >> 20));
                }
                put(name, bytes)?;
                continue;
            }
            if *name != "icon.png" && !source {
                crate::content::check_path(name)?;
            }
            check_file(name, 0, || Ok(*bytes)).map_err(|e| e.to_string())?;
            put(name, bytes)?;
        }
        let bytes = zip.finish().map_err(err)?.into_inner();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).map_err(err)?;
        Self::read_zip(&mut archive)?;
        Ok(bytes)
    }
}

/// A content entry that cannot be safely loaded as data.
#[derive(Debug)]
pub enum ContentFileError {
    UnsupportedType(String),
    Executable(String),
    Read {
        name: String,
        source: std::io::Error,
    },
}

impl std::fmt::Display for ContentFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedType(name) => {
                write!(f, "{name}: unsupported file type in a content mod")
            }
            Self::Executable(name) => {
                write!(f, "{name}: unsupported executable file in a content mod")
            }
            Self::Read { name, source } => write!(f, "{name}: {source}"),
        }
    }
}

impl std::error::Error for ContentFileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// The shared content-file boundary. Refuse type/mode before opening a file,
/// then inspect its header even when its name has no executable extension.
pub(crate) fn check_file<R: Read>(
    name: &str,
    mode: u32,
    open: impl FnOnce() -> std::io::Result<R>,
) -> Result<(), ContentFileError> {
    if !matches!(mode & 0o170000, 0 | 0o100000) {
        return Err(ContentFileError::UnsupportedType(name.into()));
    }
    let extension = name
        .rsplit(['/', '\\'])
        .next()
        .and_then(|n| n.rsplit_once('.'));
    let executable = extension.is_some_and(|(_, ext)| {
        [
            "wasm", "exe", "dll", "so", "dylib", "class", "jar", "com", "bat", "cmd", "ps1",
        ]
        .iter()
        .any(|kind| ext.eq_ignore_ascii_case(kind))
    });
    if executable || mode & 0o111 != 0 {
        return Err(ContentFileError::Executable(name.into()));
    }
    let error = |source| ContentFileError::Read {
        name: name.into(),
        source,
    };
    let mut file = open().map_err(error)?;
    let mut header = [0; 4];
    let mut read = 0;
    while read < header.len() {
        let size = file.read(&mut header[read..]).map_err(error)?;
        if size == 0 {
            break;
        }
        read += size;
    }
    let executable = matches!(
        &header,
        b"\0asm"
            | b"\x7fELF"
            | b"\xfe\xed\xfa\xce"
            | b"\xce\xfa\xed\xfe"
            | b"\xfe\xed\xfa\xcf"
            | b"\xcf\xfa\xed\xfe"
            | b"\xca\xfe\xba\xbe"
            | b"\xbe\xba\xfe\xca"
    ) || header.starts_with(b"MZ")
        || (header.starts_with(b"#!") && header[2] != b'[');
    if executable {
        return Err(ContentFileError::Executable(name.into()));
    }
    Ok(())
}

fn check_folder_file(root: &Path, name: &str) -> Result<(), String> {
    let path = root.join(name);
    let metadata = std::fs::symlink_metadata(&path).map_err(|e| format!("{name}: {e}"))?;
    let mode = if metadata.is_file() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            metadata.mode()
        }
        #[cfg(not(unix))]
        {
            0
        }
    } else {
        0o120000
    };
    check_file(name, mode, || std::fs::File::open(&path)).map_err(|e| e.to_string())
}

/// The `.hmod` files and mod folders in `dir`, by name.
pub fn find(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "hmod") || p.join(MANIFEST).is_file())
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injections_at_dependency_functions_are_refused_by_name() {
        let text = format!(
            "{EXAMPLE}[native]\ngame='26.3'\nmixins='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'\nhorizon='0123456789012345678901234567890123456789'\n[dependencies]\nchemlib='1'\n[[inject]]\nid='listen'\nat='chemlib::transfer'\nkind='head'\nwith='listen'\n"
        );
        let error = Manifest::parse(&text).unwrap_err();
        assert!(
            error.contains("chemlib::transfer") && error.contains("own hooks and functions"),
            "{error}"
        );
    }

    #[test]
    fn declared_wasm_memory_is_one_mib_to_wasm32s_four_gib() {
        let declare = |value: &str| {
            Manifest::parse(&EXAMPLE.replace(
                "capabilities = [\"registries\"]\n",
                &format!("capabilities = [\"registries\"]\nwasm_memory_mib = {value}\n"),
            ))
        };
        assert_eq!(Manifest::parse(EXAMPLE).unwrap().wasm_memory_mib, None);
        assert_eq!(declare("1").unwrap().wasm_memory_mib, Some(1));
        assert_eq!(declare("4096").unwrap().wasm_memory_mib, Some(4096));
        for value in ["0", "4097"] {
            assert!(
                declare(value)
                    .unwrap_err()
                    .contains(&format!("wasm_memory_mib {value} is not 1 to 4096")),
                "{value}"
            );
        }
    }

    #[test]
    fn windows_device_mod_ids_are_refused() {
        for id in ["con", "prn", "aux", "nul", "com1", "com9", "lpt1", "lpt9"] {
            assert!(
                Manifest::parse(&EXAMPLE.replace("ruby", id))
                    .unwrap_err()
                    .contains("reserved Windows device name"),
                "{id}"
            );
        }
        for id in ["console", "com0", "com10", "lpt0", "con_tool"] {
            assert!(
                Manifest::parse(&EXAMPLE.replace("ruby", id)).is_ok(),
                "{id}"
            );
        }
    }

    const EXAMPLE: &str = r#"
id = "ruby"
name = "Ruby"
version = "0.1.0"
api = "0.2"
capabilities = ["registries"]
"#;

    #[test]
    fn refuses_bad_ids_and_retired_manifests() {
        let bad_id = EXAMPLE.replace("ruby", "Ruby!");
        assert!(Manifest::parse(&bad_id).unwrap_err().contains("is not"));
        for api in ["0.1", "0.3", "0.4"] {
            let text = EXAMPLE.replace("api = \"0.2\"", &format!("api = \"{api}\""));
            assert!(
                Manifest::parse(&text)
                    .unwrap_err()
                    .contains("unsupported content API")
            );
        }
        for caps in ["[]", "[\"hud\"]", "[\"registries\", \"registries\"]"] {
            let text = EXAMPLE.replace("[\"registries\"]", caps);
            assert!(Manifest::parse(&text).is_err());
        }
        for field in ["network = true", "experimental = []", "side = \"server\""] {
            assert!(Manifest::parse(&format!("{EXAMPLE}{field}\n")).is_err());
        }
        assert_eq!(Manifest::parse(EXAMPLE).unwrap().side, Side::Client);
        assert_eq!(
            Manifest::parse(&format!("{EXAMPLE}side = \"both\"\n"))
                .unwrap()
                .side,
            Side::Both
        );
    }

    #[test]
    fn native_manifest_requires_exact_pins_and_known_fields() {
        let native = format!(
            "[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        Manifest::parse(&format!("{EXAMPLE}{native}")).unwrap();
        assert!(
            Manifest::parse(&format!(
                "{EXAMPLE}{}",
                native.replace(&"a".repeat(40), "main")
            ))
            .is_err()
        );
        for unsupported in ["unexpected = true", "[[native.post_build]]\nbin = 'bake'"] {
            assert!(Manifest::parse(&format!("{EXAMPLE}{native}{unsupported}\n")).is_err());
        }
    }

    #[test]
    fn native_unknown_fields_are_refused_at_the_owning_boundary() {
        let pin = "a".repeat(40);
        let base = format!("{EXAMPLE}[native]\ngame='26.3'\nhorizon='{pin}'\nmixins='{pin}'\n");
        let mut accepted = Vec::new();
        for field in [
            "injection=[]",
            "[[native.injection]]\nid='tick'\ntarget='game::tick'\nkind='HEAD'\nraw='tick'",
            "internals=[]",
            "[[native.internals]]\npath='Game'\nhash='h'",
            "unexpected=true",
        ] {
            let text = format!("{base}{field}\n");
            match Manifest::parse(&text) {
                Ok(_) => accepted.push(field),
                Err(error) => assert!(error.contains("unknown field"), "{error}"),
            }
            // The same owning serde boundary applies outside Manifest::parse.
            if toml::from_str::<Native>(text.split("[native]\n").nth(1).unwrap()).is_ok() {
                accepted.push(field);
            }
        }
        assert!(
            accepted.is_empty(),
            "accepted unknown native fields: {accepted:?}"
        );
    }

    #[test]
    fn internal_kinds_distinguish_members_and_round_trip() {
        let base = format!(
            "{EXAMPLE}[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let declaration = "internals=[{path='Round::listening',kind='field',hash='1234'}, {path='Round::listening',kind='method',hash='5678'}]\n";
        let manifest =
            Manifest::parse(&base.replace("[native]", &format!("{declaration}[native]"))).unwrap();
        assert_eq!(manifest.internals[1].kind.as_deref(), Some("method"));
        let row = toml::to_string(&manifest.internals[1]).unwrap();
        assert_eq!(
            toml::from_str::<Internal>(&row).unwrap(),
            manifest.internals[1]
        );
        let native = manifest.native.unwrap();
        let text = toml::to_string(&native).unwrap();
        assert_eq!(toml::from_str::<Native>(&text).unwrap(), native);
        for kind in ["field", ""] {
            let invalid = declaration.replace("kind='method'", &format!("kind='{kind}'"));
            assert!(
                Manifest::parse(&base.replace("[native]", &format!("{invalid}[native]"))).is_err()
            );
        }
        let untyped = Internal {
            path: "Game".into(),
            kind: None,
            hash: Some("1234".into()),
        };
        assert!(!toml::to_string(&untyped).unwrap().contains("kind"));
    }

    #[test]
    fn native_modules_are_validated() {
        let native = format!(
            "[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let module = "[[native.module]]\nid = 'runtime'\nfile = 'crates/horizon-client/src/game.rs'\nname = 'runtime'\nsource = 'src/runtime.rs'\n";
        let manifest = Manifest::parse(&format!("{EXAMPLE}{native}{module}")).unwrap();
        let module = &manifest.native.as_ref().unwrap().module[0];
        assert_eq!(module.name, "runtime");
        assert_eq!(module.visibility, None);
        // Support modules still forward whole to horizon-mixins.
        let forwarded = toml::to_string(manifest.native.as_ref().unwrap()).unwrap();
        assert!(forwarded.contains("[[module]]"));
        assert!(forwarded.contains("id = \"runtime\""));
        let valid = "[[native.module]]\nid = 'runtime'\nfile = 'crates/horizon-client/src/game.rs'\nname = 'runtime'\nsource = 'src/runtime.rs'\n";
        for bad in [
            // escapes the package's src/
            valid.replace("source = 'src/runtime.rs'", "source = '../escape.rs'"),
            // unknown field
            format!("{valid}order = 1\n"),
            // duplicate module id
            format!("{valid}{valid}"),
            format!(
                "{valid}[[native.module]]\nid='runtime'\nfile='game.rs'\nname='other'\nsource='src/other.rs'\n"
            ),
            // not an identifier
            valid.replace("name = 'runtime'", "name = 'run time'"),
            // file must stay a plain path
            valid.replace(
                "file = 'crates/horizon-client/src/game.rs'",
                "file = '/abs.rs'",
            ),
            // horizon-mixins accepts exactly pub and pub(crate)
            valid.to_owned() + "visibility = 'public'\n",
        ] {
            assert!(
                Manifest::parse(&format!("{EXAMPLE}{native}{bad}")).is_err(),
                "accepted: {bad}"
            );
        }
        let public = valid.to_owned() + "visibility = 'pub(crate)'\n";
        assert!(Manifest::parse(&format!("{EXAMPLE}{native}{public}")).is_ok());
    }

    #[test]
    fn properties_are_typed_and_bounded() {
        let properties = |rows: &str| Manifest::parse(&format!("{EXAMPLE}[properties]\n{rows}"));
        let manifest = properties("tier = 3\nratio = 0.5\nlabel = 'x'\nfast = true\n").unwrap();
        assert_eq!(manifest.properties["tier"], Property::Integer(3));
        assert_eq!(manifest.properties["ratio"], Property::Float(0.5));
        assert_eq!(manifest.properties["label"], Property::String("x".into()));
        assert_eq!(manifest.properties["fast"], Property::Boolean(true));
        let many: String = (0..65).map(|i| format!("k{i} = {i}\n")).collect();
        let error = properties(&many).unwrap_err();
        assert!(error.contains("at most 64 keys"), "{error}");
        assert!(properties(&format!("text = '{}'\n", "a".repeat(8 << 10))).is_err());
        assert!(properties("list = [1]\n").is_err());
    }

    #[test]
    fn dependencies_are_typed_for_content_and_code_mods() {
        let deps =
            "[dependencies]\ncreate = '^6.0'\naddon = { version = '^1.2', optional = true }\n";
        let content = Manifest::parse(&format!("{EXAMPLE}{deps}")).unwrap();
        assert_eq!(content.dependencies.len(), 2);
        let native = format!(
            "[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let text = format!("{EXAMPLE}{deps}{native}");
        let manifest = Manifest::parse(&text).unwrap();
        assert!(
            matches!(&manifest.dependencies["addon"], Dependency::Detailed(details) if details.optional)
        );
        assert!(matches!(
            &manifest.dependencies["create"],
            Dependency::Version(_)
        ));
        for bad in [
            text.replace("^6.0", "not semver"),
            text.replace("create =", "\"not-valid\" ="),
            text.replace("optional = true", "unexpected = true"),
        ] {
            assert!(Manifest::parse(&bad).is_err());
        }
    }

    #[test]
    fn a_code_mod_carries_one_bounded_uses_table_and_a_content_mod_none() {
        let native = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let cargo = b"[package]\nname = 'ruby'\n".as_slice();
        let lib = b"pub fn f() {}\n".as_slice();
        let table = br#"{"schema":1,"made_for":"x","entries":[]}"#.as_slice();
        let bytes = Package::write_files(
            &native,
            &[
                ("Cargo.toml", cargo),
                ("src/lib.rs", lib),
                (USES_FILE, table),
            ],
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        Package::read_zip(&mut zip).unwrap();
        let mut read = Vec::new();
        zip.by_name(USES_FILE)
            .unwrap()
            .read_to_end(&mut read)
            .unwrap();
        assert_eq!(read, table);
        let large = vec![b' '; MAX_USES as usize + 1];
        assert!(
            Package::write_files(
                &native,
                &[
                    ("Cargo.toml", cargo),
                    ("src/lib.rs", lib),
                    (USES_FILE, &large)
                ],
            )
            .unwrap_err()
            .contains(USES_FILE)
        );
        // A content mod has no code to scan, so no table either, written or read.
        assert!(Package::write_files(EXAMPLE, &[(USES_FILE, table)]).is_err());
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in [(MANIFEST, EXAMPLE.as_bytes()), (USES_FILE, table)] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        let mut archive = zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
        let error = Package::read_zip(&mut archive)
            .err()
            .expect("a content package's uses.json is refused");
        assert!(error.contains(USES_FILE), "{error}");
    }

    #[test]
    fn layout_metadata_round_trips_and_malformed_archives_are_refused() {
        let layouts = br#"{"ruby":{"ruby::T":"0123456789abcdef"}}"#;
        let bytes = Package::write_files(EXAMPLE, &[("layouts.json", layouts)]).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let package = Package::read_zip(&mut zip).unwrap();
        let changed =
            crate::layouts::Layouts::read(br#"{"ruby":{"ruby::T":"1123456789abcdef"}}"#.as_slice())
                .unwrap();
        assert_eq!(
            package.layouts.first_difference("ruby", &changed),
            Some("ruby::T")
        );
        assert!(
            Package::write_files(EXAMPLE, &[("layouts.json", b"{")])
                .unwrap_err()
                .contains("layouts.json")
        );
    }

    #[test]
    fn flat_native_archive_round_trips_through_shared_writer() {
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let files: &[(&str, &[u8])] = &[
            (
                "Cargo.toml",
                b"[package]\nname = 'other'\nversion = '2.0.0'\n",
            ),
            ("src/lib.rs", b"pub fn hooks() {}"),
        ];
        let bytes = Package::write_files(&manifest, files).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let package = Package::read_zip(&mut zip).unwrap();
        assert_eq!(package.manifest.id, "ruby");
        assert_eq!(package.manifest.version, "0.1.0");
        assert_eq!(package.manifest.native.unwrap().game, "26.3");
        assert!(
            Package::write_files(&manifest, &[("Cargo.toml", b""), ("src/../bad.rs", b"")])
                .is_err()
        );
        assert!(
            Package::write_files(
                &manifest,
                &[
                    ("Cargo.toml", b""),
                    ("src/lib.rs", b""),
                    ("src/lib.rs", b"")
                ]
            )
            .is_err()
        );
    }

    #[test]
    fn flat_project_packs_only_selected_safe_files() {
        let root = std::env::temp_dir().join(format!("horizon-flat-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        std::fs::create_dir_all(root.join("target/generated")).unwrap();
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::write(root.join(MANIFEST), manifest).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = 'other'\nversion = '2.0.0'\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn hooks() {}\n").unwrap();
        let registry_lock = "version = 4\n[[package]]\nname = 'itoa'\nversion = '1.0.18'\n";
        std::fs::write(root.join("Cargo.lock"), registry_lock).unwrap();
        std::fs::write(root.join(".git/hooks/pre-commit"), "#!/bin/sh\n").unwrap();
        std::fs::write(root.join("target/generated/bad.rs"), "#!/bin/sh\n").unwrap();
        // The SDK the mod tool fetched: the API's own crate, which is never the mod's.
        let sdk = root.join(".horizon/sdk/api/crates/horizon-mod-api/src");
        std::fs::create_dir_all(&sdk).unwrap();
        std::fs::write(sdk.join("lib.rs"), "pub fn api() {}\n").unwrap();
        std::fs::write(root.join(".horizon/sdk/sdk.json"), "{}").unwrap();
        let bytes = Package::pack_directory(&root).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(Package::read_zip(&mut zip).unwrap().manifest.id, "ruby");
        assert!(zip.by_name("Cargo.toml").is_ok());
        assert!(zip.by_name("src/lib.rs").is_ok());
        let mut packed_lock = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("Cargo.lock").unwrap(), &mut packed_lock)
            .unwrap();
        assert_eq!(packed_lock, registry_lock);
        assert!(zip.by_name(".git/hooks/pre-commit").is_err());
        assert!(zip.by_name("target/generated/bad.rs").is_err());
        assert!(
            zip.file_names_raw()
                .all(|name| !name.starts_with(b".horizon")),
            "{:?}",
            zip.file_names_raw()
                .map(String::from_utf8_lossy)
                .collect::<Vec<_>>()
        );
        assert!(!source_file(
            ".horizon/sdk/api/crates/horizon-mod-api/src/lib.rs"
        ));
        std::fs::write(root.join("src/lib.rs"), b"MZ executable").unwrap();
        assert!(
            Package::pack_directory(&root)
                .unwrap_err()
                .contains("unsupported executable")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn package_kind_requires_matching_source_and_native_declaration() {
        let root =
            std::env::temp_dir().join(format!("horizon-package-kind-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(MANIFEST), EXAMPLE).unwrap();
        assert!(Package::read(&root).is_ok());
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = 'unrelated'\nversion = '7.0.0'\n",
        )
        .unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        assert!(Package::read(&root).is_err());
        let native = format!(
            "[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::write(root.join(MANIFEST), format!("{EXAMPLE}{native}")).unwrap();
        assert_eq!(Package::read(&root).unwrap().manifest.id, "ruby");
        std::fs::create_dir(root.join("native")).unwrap();
        assert!(Package::read(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checks_extra_namespaces() {
        let with = |list: &str| Manifest::parse(&format!("{EXAMPLE}namespaces = [{list}]\n"));
        let m = with("\"kaisyn\", \"towns_and_towers\"").unwrap();
        let owned: Vec<&str> = m.owned_namespaces().collect();
        assert_eq!(owned, ["ruby", "kaisyn", "towns_and_towers"]);
        for reserved in ["minecraft", "c", "horizon"] {
            let e = with(&format!("\"{reserved}\"")).unwrap_err();
            assert!(e.contains("not a mod's to claim"), "{e}");
        }
        assert!(with("\"ruby\"").unwrap_err().contains("twice"));
        assert!(with("\"a\", \"a\"").unwrap_err().contains("twice"));
        assert!(with("\"Kaisyn\"").unwrap_err().contains("is not"));
        assert!(with("\"\"").unwrap_err().contains("is not"));
    }

    #[test]
    fn the_mods_screen_fields_are_optional_and_links_must_be_http() {
        let m = Manifest::parse(EXAMPLE).unwrap();
        assert!(m.license.is_empty() && m.credits.is_empty());
        assert!(m.license_url.is_none() && m.homepage.is_none() && m.issues.is_none());
        let full = format!(
            "{EXAMPLE}license = \"MIT\"
credits = \"Alex\"
homepage = \"https://example.org\"
issues = \"http://example.org/issues\"
"
        );
        let m = Manifest::parse(&full).unwrap();
        assert_eq!((m.license.as_str(), m.credits.as_str()), ("MIT", "Alex"));
        assert_eq!(m.homepage.as_deref(), Some("https://example.org"));
        for field in ["license_url", "homepage", "issues"] {
            let e = Manifest::parse(&format!(
                "{EXAMPLE}{field} = \"file:///etc/passwd\"
"
            ))
            .unwrap_err();
            assert!(e.contains(field), "{e}");
        }
    }

    #[test]
    fn a_wasm_mod_carries_exactly_its_declared_module() {
        // A minimal valid shared member: a `dylink.0` section and the three guest exports.
        let module = wat::parse_str(
            r#"(module (@custom "dylink.0" "\01\04\00\00\00\00")
                (func (export "horizon_layout")) (func (export "horizon_in")) (func (export "horizon_hook")))"#,
        )
        .unwrap();
        let text = format!(
            "{}\n[wasm]\nsha512 = \"{}\"\ntoolchain = \"1.99.1\"\nbuilt_with = \"test\"\n",
            EXAMPLE.trim_end(),
            sha512_of(&module)
        );
        let bytes = Package::write_files(&text, &[(WASM, &module)]).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        assert!(Package::read_zip(&mut zip).unwrap().manifest.wasm.is_some());
        let error = Package::write_files(&text, &[(WASM, b"\0asm\x02\0\0\0")]).unwrap_err();
        assert_eq!(error, WasmError::Hash.to_string());
        assert_eq!(
            Package::write_files(&text, &[]).unwrap_err(),
            WasmError::Missing.to_string()
        );
        let both = format!(
            "{text}\n[native]\ngame = \"g\"\nhorizon = \"{0}\"\nmixins = \"{0}\"\n",
            "a".repeat(40)
        );
        assert!(
            Manifest::parse(&both)
                .unwrap_err()
                .contains("a mod is [native] or [wasm], not both; only [native] tables compile game code, so a wasm mod has no mixin, bridge, state or module")
        );
    }

    #[test]
    fn wasm_packages_refuse_unknown_native_fields() {
        use sha2::{Digest, Sha512};
        use std::io::Write;
        let module = b"\0asm\x01\0\0\0";
        let base = format!(
            "{EXAMPLE}[wasm]\nsha512='{:x}'\ntoolchain='1.99.1'\n",
            Sha512::digest(module)
        );
        let foreign = "[native]\ngame='26.3'\nhorizon='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'\nmixins='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'\n[[native.injection]]\nid='non_raw'\ntarget='game::tick'\nkind='HEAD'\nhelper='src/count.rs'\n";
        let missing_raw = foreign.replace("helper='src/count.rs'\n", "");
        assert!(
            Manifest::parse(&format!("{base}{missing_raw}"))
                .unwrap_err()
                .contains("a mod is [native] or [wasm], not both")
        );
        for unified in [""] {
            assert!(Manifest::parse(&format!("{base}{unified}")).is_ok());
            // Build an input archive directly: the public writer rightly refuses its manifest.
            let text = format!("{base}{foreign}{unified}");
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            for (name, bytes) in [(MANIFEST, text.as_bytes()), (WASM, module.as_slice())] {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(bytes).unwrap();
            }
            let mut archive = zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
            let error = Package::read_zip(&mut archive)
                .err()
                .expect("unknown native field refused before package classification");
            assert!(
                error.contains("a mod is [native] or [wasm], not both"),
                "{error}"
            );
        }
    }

    #[test]
    fn content_round_trips_and_executable_files_are_refused() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("horizon-mods-{}", std::process::id()));
        let dir = root.join("folder");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), EXAMPLE).unwrap();
        let archive = root.join("ruby.hmod");
        std::fs::write(&archive, Package::write_files(EXAMPLE, &[]).unwrap()).unwrap();
        assert_eq!(Package::read(&archive).unwrap().manifest.id, "ruby");
        assert_eq!(Package::read(&dir).unwrap().manifest.id, "ruby");
        let cases: &[(&str, &[u8])] = &[
            ("mod.wasm", b""),
            ("./mod.wasm", b"\0asm"),
            ("MOD.WASM", b""),
            ("sub/mod.wasm", b""),
            ("assets/ruby/CODE.WaSm", b""),
            ("assets/ruby/model.bin", b"\0asm"),
            ("assets/ruby/native.bin", b"\x7fELF"),
            ("assets/ruby/windows.bin", b"MZ"),
            ("assets/ruby/mac.bin", b"\xcf\xfa\xed\xfe"),
            ("assets/ruby/script", b"#!/bin/sh"),
            ("assets/ruby/program.EXE", b""),
        ];
        for (name, bytes) in cases {
            let file = dir.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, bytes).unwrap();
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file(MANIFEST, options).unwrap();
            zip.write_all(EXAMPLE.as_bytes()).unwrap();
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
            zip.finish().unwrap();
            let error = Package::read(&archive).err().unwrap();
            // The one place a module may sit is refused for want of `[wasm]`.
            let expected = if *name == WASM {
                WasmError::Undeclared.to_string()
            } else {
                "unsupported executable file".to_string()
            };
            assert!(error.contains(&expected), "{name}: {error}");
            if name.starts_with("assets/") {
                let error = Package::read(&dir).err().unwrap();
                assert!(
                    error.contains("unsupported executable file"),
                    "{name}: {error}"
                );
            } else if std::fs::symlink_metadata(dir.join(WASM)).is_ok() {
                // On disk `./mod.wasm` is the top-level module too, and so is `MOD.WASM` where the
                // filesystem ignores case (Windows, macOS).
                let undeclared = WasmError::Undeclared.to_string();
                assert_eq!(
                    Package::read(&dir).err(),
                    Some(undeclared),
                    "unpackaged {name}"
                );
            } else {
                assert!(Package::read(&dir).is_ok(), "unpackaged {name}");
            }
            std::fs::remove_file(file).unwrap();
        }
        assert!(Package::write_files(EXAMPLE, &[("assets/ruby/model.bin", b"\0asm")]).is_err());
        // Native bundle sources can start with Rust inner attributes, not a script shebang.
        assert!(
            Package::write_files(
                EXAMPLE,
                &[("assets/ruby/source.txt", b"#![allow(dead_code)]")]
            )
            .is_ok()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn project_folder_loads_without_packaging_git_hooks() {
        let root = std::env::temp_dir().join(format!("horizon-mod-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(root.join(MANIFEST), EXAMPLE).unwrap();
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        std::fs::write(root.join(".git/hooks/pre-commit"), b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                root.join(".git/hooks/pre-commit"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        std::fs::create_dir_all(root.join("assets/ruby")).unwrap();
        std::fs::write(root.join("assets/ruby/icon.png"), b"image").unwrap();
        assert_eq!(Package::read(&root).unwrap().manifest.id, "ruby");
        std::fs::write(root.join("assets/ruby/icon.png"), b"\0asm").unwrap();
        assert!(
            Package::read(&root)
                .err()
                .unwrap()
                .contains("unsupported executable")
        );
        std::fs::remove_file(root.join("assets/ruby/icon.png")).unwrap();
        std::fs::create_dir_all(root.join("assets/minecraft/lang")).unwrap();
        std::fs::write(root.join("assets/minecraft/lang/en_us.json"), "{}").unwrap();
        let package = Package::read(&root).unwrap();
        assert!(crate::Content::read(&root, &package.manifest).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn structure_nbt_is_data_but_not_an_executable_exception() {
        let name = "data/ruby/structure/hut.nbt";
        // An uncompressed, named NBT root containing an empty compound.
        let nbt = [10u8, 0, 0, 0];
        assert!(check_file(name, 0o100644, || Ok(std::io::Cursor::new(nbt))).is_ok());
        assert!(matches!(
            check_file(name, 0o100755, || Ok(std::io::Cursor::new(nbt))),
            Err(ContentFileError::Executable(_))
        ));
        for header in [b"\x7fELF", b"MZ!!", b"\0asm"] {
            assert!(matches!(
                check_file(name, 0o100644, || Ok(std::io::Cursor::new(header))),
                Err(ContentFileError::Executable(_))
            ));
        }
    }

    #[test]
    fn archive_links_and_executable_modes_are_refused() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("horizon-mods-kinds-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("ruby.hmod");
        for link in [false, true] {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            // Made by a Unix tool on every OS: the writer's default on Windows is DOS attributes,
            // which hold no mode to refuse.
            let options = zip::write::SimpleFileOptions::default().system(zip::System::Unix);
            zip.start_file(MANIFEST, options).unwrap();
            zip.write_all(EXAMPLE.as_bytes()).unwrap();
            if link {
                zip.add_symlink("assets/ruby/link", "missing", options)
                    .unwrap();
            } else {
                zip.start_file("assets/ruby/program", options.unix_permissions(0o755))
                    .unwrap();
                zip.write_all(b"ordinary bytes").unwrap();
            }
            zip.finish().unwrap();
            let error = Package::read(&path).err().unwrap();
            assert!(
                error.contains(if link {
                    "unsupported file type"
                } else {
                    "unsupported executable"
                }),
                "{error}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn folder_links_and_executable_modes_are_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root =
            std::env::temp_dir().join(format!("horizon-mods-folder-kinds-{}", std::process::id()));
        let dir = root.join("folder");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), EXAMPLE).unwrap();
        std::fs::create_dir_all(dir.join("assets/ruby")).unwrap();
        let file = dir.join("assets/ruby/program");
        std::fs::write(&file, b"ordinary bytes").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = Package::read(&dir).err().unwrap();
        assert!(error.contains("unsupported executable"), "{error}");
        std::fs::remove_file(&file).unwrap();
        for name in ["assets/ruby/mod.wasm", "assets/ruby/nested/link"] {
            let link = dir.join(name);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            symlink("missing", &link).unwrap();
            let error = Package::read(&dir).err().unwrap();
            assert!(error.contains("unsupported file type"), "{error}");
            std::fs::remove_file(link).unwrap();
        }
        std::fs::remove_dir_all(dir.join("assets")).unwrap();
        symlink("missing", dir.join("assets")).unwrap();
        assert!(
            Package::read(&dir)
                .err()
                .unwrap()
                .contains("unsupported file type")
        );
        let link = root.join("linked-folder");
        symlink(&dir, &link).unwrap();
        let error = Package::read(&link).err().unwrap();
        assert!(error.contains("unsupported file type"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }
}
