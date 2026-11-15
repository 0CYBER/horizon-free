//! The mod registry's index, `index.json` (docs/modding/src/reference/registry-index.md): every
//! reviewed mod and modpack, its versions, the file of each with its size,
//! SHA-256 and minisign signature, and the files taken down. The launcher
//! reads it to browse and install, the website to show it, and the
//! registry's CI writes it.
//!
//! [`check_file`] is what a file must pass before it is installed: its size
//! and hash are the ones the index lists, its signature verifies against the
//! project's pinned key, and what its own manifest says matches the entry.
use crate::{
    Capability, Package,
    modpack::{self, Modpack},
};
use mod_vocab::Side;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};

/// The index format this build reads and writes. A reader refuses a newer
/// one rather than misread it.
pub const SCHEMA: u32 = 1;
/// The biggest file the registry takes (a `.hmod` with content, a `.hpack`).
pub const MAX_FILE: u64 = 64 << 20;
/// The biggest native binary build ID, in bytes.
pub const MAX_NATIVE_BUILD: usize = 128;
/// Where files may be downloaded from: the registry's release assets.
pub const FILE_HOST: &str = "https://github.com/horizon-rust/horizon-mods/releases/download/";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub schema: u32,
    /// When CI built it, RFC 3339.
    #[serde(default)]
    pub generated: String,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub revoked: Vec<Revoked>,
    /// What [`Index::parse`] left out because it breaks a rule, with the reasons: the rest of
    /// the index still loads. Never written.
    #[serde(skip)]
    pub left_out: Vec<LeftOut>,
}

/// A project, a version or a takedown [`Index::parse`] left out, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct LeftOut {
    pub id: String,
    /// None: the whole project (or the takedown).
    pub version: Option<String>,
    pub reason: String,
}

impl LeftOut {
    fn project(p: &Project, reason: String) -> LeftOut {
        LeftOut {
            id: p.id.clone(),
            version: None,
            reason,
        }
    }
    fn version(id: &str, v: &Version, reason: String) -> LeftOut {
        LeftOut {
            id: id.to_string(),
            version: Some(v.version.clone()),
            reason,
        }
    }
}

/// What a project is. New kinds (data packs, worlds, themes) can be added
/// without a new schema: a reader keeps a kind it doesn't know as
/// [`Kind::Other`] and doesn't offer it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Kind {
    /// A `.hmod` of content only.
    Mod,
    /// A native source bundle, compiled into an isolated game build and published as a `.hmod`.
    NativeMod,
    /// A `.hpack`: registry mods played together as a profile.
    Modpack,
    /// A resource pack (`.zip` with `pack.mcmeta`) made or tested for Horizon.
    ResourcePack,
    /// A Horizon shader pack (`.hshader`, the native WGSL format of
    /// docs/shaders.md).
    Shader,
    /// A `shaders/`-folder shader pack (`.zip`, the format `horizon-shaders` loads), listed by
    /// its author under a licence that allows it.
    ShaderPack,
    /// A data pack (`.zip` with `pack.mcmeta` and `data/`), installed into a world.
    DataPack,
    Other(String),
}

impl Kind {
    pub fn name(&self) -> &str {
        match self {
            Kind::Mod => "mod",
            Kind::NativeMod => "native-mod",
            Kind::Modpack => "modpack",
            Kind::ResourcePack => "resourcepack",
            Kind::Shader => "shader",
            Kind::ShaderPack => "shaderpack",
            Kind::DataPack => "datapack",
            Kind::Other(name) => name,
        }
    }

    /// The extension of its files; None for a kind this build doesn't know.
    pub fn extension(&self) -> Option<&'static str> {
        match self {
            Kind::Mod => Some("hmod"),
            Kind::NativeMod => Some("hmod"),
            Kind::Modpack => Some("hpack"),
            Kind::ResourcePack => Some("zip"),
            Kind::Shader => Some(horizon_shader_api::EXTENSION),
            Kind::ShaderPack | Kind::DataPack => Some("zip"),
            Kind::Other(_) => None,
        }
    }
}

impl From<String> for Kind {
    fn from(name: String) -> Kind {
        match name.as_str() {
            "mod" => Kind::Mod,
            "native-mod" => Kind::NativeMod,
            "modpack" => Kind::Modpack,
            "resourcepack" => Kind::ResourcePack,
            "shader" => Kind::Shader,
            "shaderpack" => Kind::ShaderPack,
            "datapack" => Kind::DataPack,
            _ => Kind::Other(name),
        }
    }
}

impl From<Kind> for String {
    fn from(kind: Kind) -> String {
        kind.name().to_owned()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// The manifest's id, `[a-z0-9_]{1,64}`: unique across the index.
    pub id: String,
    pub kind: Kind,
    pub name: String,
    /// One line for lists.
    pub summary: String,
    /// Markdown, for the project's page.
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// The GitHub logins allowed to publish its updates.
    #[serde(default)]
    pub owners: Vec<String>,
    /// An SPDX expression.
    pub license: String,
    /// Where its source code is, `https://`.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// From the kind's list ([`page::categories`]), at most 3.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// Free-form, lowercase, at most 10.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// `https://` addresses of PNG or JPEG images, in order, at most 8.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screenshots: Vec<String>,
    /// The author's minisign public key: every version must be signed with
    /// it. None: unsigned (the SHA-256 still pins the reviewed file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Newest first.
    pub versions: Vec<Version>,
    /// Its files' downloads from their releases, all versions together:
    /// GitHub's counts, added by the registry's publish job (hourly).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub downloads: u64,
    /// The GitHub stars of its `source` repository, added by the registry's
    /// publish job (hourly); 0 when it has none or isn't on GitHub.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub stars: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Version {
    pub version: String,
    /// The day it was published, `YYYY-MM-DD`.
    #[serde(default)]
    pub published: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub changelog: String,
    /// A content mod's API line (`0.2`), from its manifest; a modpack's format (`1`).
    pub api: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    #[serde(default, skip_serializing_if = "Capabilities::is_empty")]
    pub capabilities: Capabilities,
    /// The Minecraft version it is for.
    pub game: String,
    /// A modpack's mods, as its manifest lists them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<Requirement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_binaries: Vec<NativeBinary>,
    pub file: File,
}

/// Registry-produced metadata; signature acceptance belongs to the binary verifier.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBinary {
    pub build: String,
    pub target: String,
    pub file: File,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeBinaryAbsent;

impl Version {
    /// Exact lookup only. Absence lets the caller prepare from source.
    pub fn native_binary(
        &self,
        build: &str,
        target: &str,
    ) -> Result<&NativeBinary, NativeBinaryAbsent> {
        self.native_binaries
            .iter()
            .find(|b| b.build == build && b.target == target)
            .ok_or(NativeBinaryAbsent)
    }
}

/// A version's capabilities as the index lists them. A name this build doesn't know (one an older
/// or a newer Horizon defined) is kept as written: that version is not installable, and the rest
/// of the index still loads.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Vec<String>", into = "Vec<String>")]
pub struct Capabilities {
    known: Vec<Capability>,
    unknown: Vec<String>,
}

impl Capabilities {
    /// The capabilities this build knows, in the index's order.
    pub fn known(&self) -> &[Capability] {
        &self.known
    }
    /// The names this build doesn't know, as the index wrote them.
    pub fn unknown(&self) -> &[String] {
        &self.unknown
    }
    pub fn is_empty(&self) -> bool {
        self.known.is_empty() && self.unknown.is_empty()
    }
    /// Why this build can't install a version with these capabilities, if it can't.
    pub fn unsupported(&self) -> Option<String> {
        (!self.unknown.is_empty()).then(|| {
            format!(
                "it asks for {}, which this Horizon doesn't have",
                self.unknown.join(", ")
            )
        })
    }
}

impl From<Vec<Capability>> for Capabilities {
    fn from(known: Vec<Capability>) -> Self {
        Self {
            known,
            unknown: Vec::new(),
        }
    }
}

impl From<Vec<String>> for Capabilities {
    fn from(names: Vec<String>) -> Self {
        let mut listed = Self::default();
        for name in names {
            match serde_json::from_value(serde_json::Value::String(name.clone())) {
                Ok(capability) => listed.known.push(capability),
                Err(_) => listed.unknown.push(name),
            }
        }
        listed
    }
}

impl From<Capabilities> for Vec<String> {
    fn from(listed: Capabilities) -> Self {
        let known = listed.known.iter().map(Capability::to_string);
        known.chain(listed.unknown).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    pub id: String,
    pub version: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct File {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
    /// The `.minisig` file's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// A file taken down: by hash, or every version of `id` when `sha256` is
/// None. The launcher refuses to install it and turns installed copies off.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Revoked {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub reason: String,
    #[serde(default)]
    pub date: String,
}

/// How far a checked file is trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// Signed with the project's pinned key.
    Verified,
    /// The project has no key: only the hash pins it.
    Unsigned,
}

impl Index {
    /// Parses an index and keeps what passes [`Index::check`]'s rules: an entry that breaks one
    /// is left out with its reason ([`Index::left_out`]). Only text that is not an index, or one
    /// of a newer schema, fails.
    pub fn parse(text: &str) -> Result<Index, String> {
        Self::parse_with(text, false)
    }

    /// [`Index::parse`] for a local test registry, whose files may be
    /// `file://` paths (trying a registry out before it is published).
    pub fn parse_local(text: &str) -> Result<Index, String> {
        Self::parse_with(text, true)
    }

    fn parse_with(text: &str, local_files: bool) -> Result<Index, String> {
        #[derive(Deserialize)]
        struct Head {
            schema: u32,
        }
        let head: Head = serde_json::from_str(text).map_err(|e| format!("not a mod index: {e}"))?;
        if head.schema > SCHEMA {
            return Err(format!(
                "the mod index is schema {}; this build reads {SCHEMA}: update Horizon",
                head.schema
            ));
        }
        let mut index: Index = serde_json::from_str(text).map_err(|e| format!("mod index: {e}"))?;
        index.keep_valid(local_files);
        Ok(index)
    }

    /// Every rule a reader relies on: unique valid ids, at least one
    /// version each, plain file names with the kind's extension, `https`
    /// URLs, hex hashes, keys and signatures that decode, and each
    /// modpack's mods present in the index with the same hash.
    pub fn check(&self) -> Result<(), String> {
        self.check_with(false)
    }

    fn check_with(&self, local_files: bool) -> Result<(), String> {
        for (i, p) in self.projects.iter().enumerate() {
            if self.projects[..i].iter().any(|q| q.id == p.id) {
                return Err(format!("{}: listed twice", p.id));
            }
            check_project(p)?;
            if p.versions.is_empty() {
                return Err(format!("{}: has no version", p.id));
            }
            for (j, v) in p.versions.iter().enumerate() {
                if p.versions[..j].iter().any(|w| w.version == v.version) {
                    return Err(format!("{} {}: empty or repeated version", p.id, v.version));
                }
                check_version(p, v, local_files)?;
                self.check_requires(p, v)?;
            }
        }
        for r in &self.revoked {
            check_revoked(r)?;
        }
        Ok(())
    }

    /// What a reader keeps of an index that fails [`Index::check`]: each project, version and
    /// takedown that fails its own rule is left out with its reason ([`Index::left_out`]), and
    /// the rest loads. A modpack version whose mod was left out goes too, and so does a project
    /// left with no version. An id listed twice leaves out every project with that id.
    fn keep_valid(&mut self, local_files: bool) {
        let mut left_out = Vec::new();
        let mut listed: std::collections::HashMap<String, usize> = Default::default();
        for p in &self.projects {
            *listed.entry(p.id.clone()).or_default() += 1;
        }
        self.projects.retain(|p| {
            let reason = match listed[&p.id] {
                1 => check_project(p).err(),
                _ => Some(format!("{}: listed twice", p.id)),
            };
            if let Some(reason) = &reason {
                left_out.push(LeftOut::project(p, reason.clone()));
            }
            reason.is_none()
        });
        for p in &mut self.projects {
            let mut seen: Vec<String> = Vec::new();
            for v in std::mem::take(&mut p.versions) {
                let checked = match seen.contains(&v.version) {
                    true => Err(format!("{} {}: empty or repeated version", p.id, v.version)),
                    false => check_version(p, &v, local_files),
                };
                seen.push(v.version.clone());
                match checked {
                    Ok(()) => p.versions.push(v),
                    Err(reason) => left_out.push(LeftOut::version(&p.id, &v, reason)),
                }
            }
        }
        // A modpack names its mods by version: leaving one out leaves out the pack's version,
        // which another pack may name in turn. Repeat until nothing more goes.
        loop {
            let this: &Index = self;
            let gone: Vec<(usize, usize, String)> = this
                .projects
                .iter()
                .enumerate()
                .flat_map(|(i, p)| {
                    p.versions.iter().enumerate().filter_map(move |(j, v)| {
                        this.check_requires(p, v).err().map(|e| (i, j, e))
                    })
                })
                .collect();
            if gone.is_empty() {
                break;
            }
            for (i, j, reason) in gone.into_iter().rev() {
                let v = self.projects[i].versions.remove(j);
                left_out.push(LeftOut::version(&self.projects[i].id, &v, reason));
            }
        }
        self.projects.retain(|p| {
            if p.versions.is_empty() {
                let reason = format!("{}: has no version this Horizon can use", p.id);
                left_out.push(LeftOut::project(p, reason));
            }
            !p.versions.is_empty()
        });
        self.revoked.retain(|r| match check_revoked(r) {
            Ok(()) => true,
            Err(reason) => {
                left_out.push(LeftOut {
                    id: r.id.clone(),
                    version: None,
                    reason,
                });
                false
            }
        });
        self.left_out = left_out;
    }

    /// Each mod a modpack version names is in the index, with the same file.
    fn check_requires(&self, p: &Project, v: &Version) -> Result<(), String> {
        let at = |e: String| format!("{} {}: {e}", p.id, v.version);
        for r in &v.requires {
            let found = self
                .project(&r.id)
                .and_then(|q| q.versions.iter().find(|w| w.version == r.version));
            match found {
                Some(w) if w.file.sha256 == r.sha256 => {}
                Some(_) => {
                    return Err(at(format!(
                        "{} {}: another file than the index's",
                        r.id, r.version
                    )));
                }
                None => {
                    return Err(at(format!(
                        "needs {} {}, which is not in the index",
                        r.id, r.version
                    )));
                }
            }
        }
        Ok(())
    }

    /// A project's newest version for Minecraft `game` that wasn't taken down and asks for no
    /// capability this build doesn't know.
    pub fn installable<'a>(&self, project: &'a Project, game: &str) -> Option<&'a Version> {
        project.versions.iter().find(|v| {
            v.game == game
                && v.capabilities.unknown().is_empty()
                && self.revoked(&project.id, &v.file.sha256).is_none()
        })
    }
    pub fn project(&self, id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    /// Why the file of `id` with this hash was taken down, if it was.
    pub fn revoked(&self, id: &str, sha256: &str) -> Option<&Revoked> {
        self.revoked.iter().find(|r| {
            r.id == id
                && r.sha256
                    .as_ref()
                    .is_none_or(|h| h.eq_ignore_ascii_case(sha256))
        })
    }
}

/// A project's own fields: a valid id, a name and a licence, `https` addresses and a key that
/// decodes.
fn check_project(p: &Project) -> Result<(), String> {
    let at = |e: String| format!("{}: {e}", p.id);
    if !valid_id(&p.id) {
        return Err(format!("project id {:?} is not [a-z0-9_]{{1,64}}", p.id));
    }
    if p.name.trim().is_empty() || p.license.trim().is_empty() {
        return Err(at("needs a name and a licence".into()));
    }
    https(&p.source).map_err(at)?;
    if let Some(icon) = &p.icon {
        https(icon).map_err(at)?;
    }
    if p.screenshots.len() > page::MAX_SCREENSHOTS {
        return Err(at(format!(
            "{} screenshots, at most {}",
            p.screenshots.len(),
            page::MAX_SCREENSHOTS
        )));
    }
    for s in &p.screenshots {
        https(s).map_err(at)?;
    }
    if let Some(key) = &p.key {
        minisign_verify::PublicKey::from_base64(key.trim()).map_err(|e| at(format!("key: {e}")))?;
    }
    Ok(())
}

/// A version's own rules: a name, valid native binaries, a plain file name with the kind's
/// extension, an `https` address, a hex hash, a size in bounds and a signature as the project's
/// key asks.
fn check_version(p: &Project, v: &Version, local_files: bool) -> Result<(), String> {
    let at = |e: String| format!("{} {}: {e}", p.id, v.version);
    if v.version.trim().is_empty() {
        return Err(at("empty or repeated version".into()));
    }
    let mut pairs = std::collections::HashSet::new();
    for b in &v.native_binaries {
        if b.build.len() > MAX_NATIVE_BUILD
            || b.build.bytes().any(|c| c.is_ascii_control())
            || b.build.is_empty()
            || b.build.trim() != b.build
            || b.target.split('-').count() < 3
            || !b
                .target
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"-_.".contains(&c))
            || b.target.split('-').any(str::is_empty)
        {
            return Err(at("invalid native build or canonical target triple".into()));
        }
        if !pairs.insert((&b.build, &b.target)) {
            return Err(at("duplicate native build and target".into()));
        }
        let f = &b.file;
        if !plain_name(&f.name) || f.size == 0 || f.size > MAX_FILE {
            return Err(at("invalid native binary file".into()));
        }
        if !(local_files && f.url.starts_with("file:///")) {
            https(&f.url).map_err(at)?;
        }
        hex64(&f.sha256).map_err(at)?;
        let signature = f
            .signature
            .as_deref()
            .ok_or_else(|| at("native binary needs a signature".into()))?;
        minisign_verify::Signature::decode(signature)
            .map_err(|e| at(format!("native signature: {e}")))?;
    }
    let f = &v.file;
    let extension_ok = p
        .kind
        .extension()
        .is_none_or(|e| f.name.ends_with(&format!(".{e}")));
    if !plain_name(&f.name) || !extension_ok {
        return Err(at(format!(
            "{:?} is not a plain file name for a {}",
            f.name,
            p.kind.name()
        )));
    }
    if !(local_files && f.url.starts_with("file:///")) {
        https(&f.url).map_err(at)?;
    }
    hex64(&f.sha256).map_err(at)?;
    if f.size == 0 || f.size > MAX_FILE {
        return Err(at(format!(
            "size {} is not 1 byte to {} MiB",
            f.size,
            MAX_FILE >> 20
        )));
    }
    match (&p.key, &f.signature) {
        (Some(_), None) => {
            return Err(at("unsigned, but the project has a key".into()));
        }
        (None, Some(_)) => return Err(at("signed, but the project has no key".into())),
        (Some(_), Some(sig)) => {
            minisign_verify::Signature::decode(sig).map_err(|e| at(format!("signature: {e}")))?;
        }
        (None, None) => {}
    }
    Ok(())
}

/// A takedown's hash, when it names one, is hex.
fn check_revoked(r: &Revoked) -> Result<(), String> {
    match &r.sha256 {
        Some(h) => hex64(h).map_err(|e| format!("revoked {}: {e}", r.id)),
        None => Ok(()),
    }
}

/// `[a-z0-9_]{1,64}`, the manifests' id rule.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn https(url: &str) -> Result<(), String> {
    match url.strip_prefix("https://") {
        Some(rest) if !rest.is_empty() && !url.chars().any(char::is_whitespace) => Ok(()),
        _ => Err(format!("{url:?} is not an https:// address")),
    }
}

fn hex64(h: &str) -> Result<(), String> {
    if h.len() == 64
        && h.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err(format!("{h:?} is not a lowercase hex SHA-256"))
    }
}

/// One plain file name: no folder, no hidden file, nothing a file system
/// reads as something else.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
}

/// The SHA-256 of a file, lowercase hex.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks a minisign signature (`.minisig` text) of `path` against `key`.
pub fn verify_signature(key: &str, signature: &str, path: &Path) -> Result<(), String> {
    let key =
        minisign_verify::PublicKey::from_base64(key.trim()).map_err(|e| format!("key: {e}"))?;
    let sig =
        minisign_verify::Signature::decode(signature).map_err(|e| format!("signature: {e}"))?;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut verifier = key
        .verify_stream(&sig)
        .map_err(|e| format!("signature: {e}"))?;
    let mut reader = std::io::BufReader::new(file);
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        verifier.update(&buf[..n]);
    }
    verifier
        .finalize()
        .map_err(|_| "the signature doesn't match the project's key".to_string())
}

/// Checks the file at `path` against its entry before it is installed:
/// size, SHA-256, revocation, signature, and that its manifest says what the
/// index says (id, version, API, side, capabilities; a modpack's mods).
pub fn check_file(
    index: &Index,
    project: &Project,
    version: &Version,
    path: &Path,
) -> Result<Trust, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if size != version.file.size {
        return Err(format!(
            "{} bytes, the index lists {}",
            size, version.file.size
        ));
    }
    let hash = sha256_file(path)?;
    if hash != version.file.sha256 {
        return Err("its SHA-256 isn't the one the index lists: nothing was installed".into());
    }
    if let Some(r) = index.revoked(&project.id, &hash) {
        return Err(format!("{} was taken down: {}", project.name, r.reason));
    }
    let trust = match (&project.key, &version.file.signature) {
        (Some(key), Some(sig)) => {
            verify_signature(key, sig, path)?;
            Trust::Verified
        }
        (None, None) => Trust::Unsigned,
        _ => return Err("its signature and the project's key don't go together".into()),
    };
    if matches!(project.kind, Kind::Other(_)) {
        return Err(format!(
            "this Horizon can't install a {} yet",
            project.kind.name()
        ));
    }
    inspect(&project.kind, path)?.matches(project, version)?;
    Ok(trust)
}

/// What a file says about itself, read by its kind's own check
/// ([`inspect`]). The registry's CI writes these into the index; the
/// launcher compares them with the index before it installs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    /// A mod's or a modpack's manifest id; None for a resource pack.
    pub id: Option<String>,
    pub version: Option<String>,
    /// A mod's API line, a modpack's format, a resource pack's pack format.
    pub api: String,
    pub side: Option<Side>,
    /// In the manifest's order.
    pub capabilities: Vec<Capability>,
    /// The Minecraft version, when the file names one (a modpack does).
    pub game: Option<String>,
    pub requires: Vec<Requirement>,
}

impl Facts {
    /// Whether the file is the one `version` of `project` describes.
    pub fn matches(&self, project: &Project, version: &Version) -> Result<(), String> {
        let (mut caps, mut listed) = (
            self.capabilities.clone(),
            version.capabilities.known().to_vec(),
        );
        caps.sort();
        listed.sort();
        let differs = self.id.as_ref().is_some_and(|id| *id != project.id)
            || self.version.as_ref().is_some_and(|v| *v != version.version)
            || self.api != version.api
            || self.side != version.side
            || caps != listed
            || !version.capabilities.unknown().is_empty()
            || self.game.as_ref().is_some_and(|g| *g != version.game)
            || self.requires != version.requires;
        if !differs {
            return Ok(());
        }
        Err(match project.kind {
            Kind::Mod => format!(
                "its horizon-mod.toml ({} {}, API {}, {:?}, {}) isn't what the index lists",
                self.id.as_deref().unwrap_or("?"),
                self.version.as_deref().unwrap_or("?"),
                self.api,
                self.side,
                names(&self.capabilities)
            ),
            Kind::Modpack => format!("its {} isn't what the index lists", modpack::MANIFEST),
            Kind::Shader => format!(
                "its {} ({} {}, shader API {}) isn't what the index lists",
                horizon_shader_api::MANIFEST,
                self.id.as_deref().unwrap_or("?"),
                self.version.as_deref().unwrap_or("?"),
                self.api
            ),
            _ => format!("its pack format {} isn't what the index lists", self.api),
        })
    }
}

/// Reads and checks a file as its kind: a mod's manifest and content, a
/// modpack's manifest and overrides, a resource pack's `pack.mcmeta` and
/// paths, a shader pack's `horizon_shader_api::check` (the game's own). Kinds
/// this build doesn't know are refused: their checks come with their formats.
pub fn inspect(kind: &Kind, path: &Path) -> Result<Facts, String> {
    match kind {
        Kind::NativeMod => {
            let bundle = native::Bundle::read(path)?;
            crate::Content::read(path, &bundle.manifest)?;
            Ok(bundle.facts())
        }
        Kind::Mod => {
            let package = Package::read(path)?;
            if package.is_native() {
                return Err("it has native code: it is a native-mod".into());
            }
            let m = package.manifest;
            crate::Content::read(path, &m)?;
            Ok(Facts {
                capabilities: m.capabilities,
                id: Some(m.id),
                version: Some(m.version),
                api: m.api,
                side: Some(m.side),
                ..Facts::default()
            })
        }
        Kind::Modpack => {
            let m = Modpack::read(path)?.manifest;
            Ok(Facts {
                requires: m.mods.iter().map(modpack::PackMod::requirement).collect(),
                id: Some(m.id),
                version: Some(m.version),
                api: m.format.to_string(),
                game: Some(m.game),
                ..Facts::default()
            })
        }
        Kind::ResourcePack => Ok(Facts {
            api: check_resource_pack(path)?,
            ..Facts::default()
        }),
        Kind::Shader => {
            let pack = horizon_shader_api::Pack::read(path)?;
            if let Err(problems) = horizon_shader_api::check(&pack) {
                let lines: Vec<String> = problems.iter().map(ToString::to_string).collect();
                return Err(format!(
                    "the shader pack fails its check:\n{}",
                    lines.join("\n")
                ));
            }
            let m = pack.manifest;
            Ok(Facts {
                id: Some(m.id),
                version: Some(m.version),
                api: horizon_shader_api::API.to_owned(),
                ..Facts::default()
            })
        }
        Kind::ShaderPack => Ok(Facts {
            api: check_shader_folder_pack(path)?,
            ..Facts::default()
        }),
        Kind::DataPack => Ok(Facts {
            api: check_data_pack(path)?,
            ..Facts::default()
        }),
        Kind::Other(name) => Err(format!("this Horizon can't read a {name} yet")),
    }
}

/// A resource pack: a zip with `pack.mcmeta` at its root holding a `pack`
/// object, and only plain paths. Returns its pack format (`pack_format`, or
/// `min_format` in newer packs), empty when it names none.
fn check_resource_pack(path: &Path) -> Result<String, String> {
    let (mut zip, _) = plain_zip(path)?;
    pack_format(&mut zip)
}

/// A zip whose paths are all plain, and those paths.
fn plain_zip(path: &Path) -> Result<(zip::ZipArchive<std::fs::File>, Vec<String>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut names = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let name = zip
            .by_index_raw(i)
            .map_err(|e| e.to_string())?
            .name()
            .map_err(|e| e.to_string())?
            .into_owned();
        if name.starts_with('/') || name.split('/').any(|part| part == "..") || name.contains('\\')
        {
            return Err(format!("{name:?} is not a plain path"));
        }
        names.push(name);
    }
    Ok((zip, names))
}

/// A data pack: a resource pack's rules, and a `data/` folder at the root.
fn check_data_pack(path: &Path) -> Result<String, String> {
    let (mut zip, names) = plain_zip(path)?;
    if !names.iter().any(|n| n.starts_with("data/")) {
        return Err("no data/ folder at the zip's root: not a data pack".into());
    }
    pack_format(&mut zip)
}

/// A `shaders/`-folder shader pack: plain paths and a `shaders/` folder at the root. Its format
/// has no version of its own: the facts' `api` is empty.
fn check_shader_folder_pack(path: &Path) -> Result<String, String> {
    let (_, names) = plain_zip(path)?;
    if !names.iter().any(|n| n.starts_with("shaders/")) {
        return Err("no shaders/ folder at the zip's root: not a shader pack".into());
    }
    Ok(String::new())
}

/// What a zip's file is, by its content: a `shaders/` folder makes a shader pack, a `data/` folder
/// without `assets/` a data pack, anything else a resource pack (checked as one).
pub fn zip_kind(path: &Path) -> Kind {
    let Ok((_, names)) = plain_zip(path) else {
        return Kind::ResourcePack;
    };
    let root = |dir: &str| names.iter().any(|n| n.starts_with(dir));
    if root("shaders/") {
        Kind::ShaderPack
    } else if root("data/") && !root("assets/") {
        Kind::DataPack
    } else {
        Kind::ResourcePack
    }
}

/// `pack.mcmeta`'s pack format (`pack_format`, or `min_format` in newer packs), empty when it
/// names none; refused when it has no `pack` object.
fn pack_format(zip: &mut zip::ZipArchive<std::fs::File>) -> Result<String, String> {
    let mut meta = String::new();
    zip.by_name("pack.mcmeta")
        .map_err(|_| "pack.mcmeta: missing".to_string())?
        .take(1 << 20)
        .read_to_string(&mut meta)
        .map_err(|e| format!("pack.mcmeta: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&meta).map_err(|e| format!("pack.mcmeta: {e}"))?;
    let pack = &json["pack"];
    if !pack.is_object() {
        return Err("pack.mcmeta: no \"pack\" object".into());
    }
    let format = [&pack["pack_format"], &pack["min_format"]]
        .into_iter()
        .find(|f| !f.is_null())
        .map(|f| match f {
            serde_json::Value::Array(parts) => parts
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("."),
            other => other.to_string(),
        })
        .unwrap_or_default();
    Ok(format)
}

/// The kinds the registry takes submissions for; other kinds wait for their
/// formats.
pub fn open(kind: &Kind) -> Result<(), String> {
    match kind {
        Kind::Mod
        | Kind::NativeMod
        | Kind::Modpack
        | Kind::ResourcePack
        | Kind::Shader
        | Kind::ShaderPack
        | Kind::DataPack => Ok(()),
        Kind::Other(name) => Err(format!("the registry doesn't take {name} yet")),
    }
}

fn names(caps: &[Capability]) -> String {
    let names: Vec<String> = caps.iter().map(ToString::to_string).collect();
    format!("[{}]", names.join(", "))
}

pub mod native;
pub mod page;
pub mod registry;
#[cfg(test)]
mod tests;
