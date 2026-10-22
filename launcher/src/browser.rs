//! The mod browser's sources: search Modrinth (and CurseForge, once a key exists) for what
//! Horizon loads, say how well each result runs, and download a file only after its checksum
//! matches (docs/mod-browser.md).
//!
//! Only what Horizon actually loads is offered: resource packs (`resourcepacks/`), Java shader
//! packs (`shaderpacks/`, Iris and OptiFine) and modpacks (imported and ported on the Modpacks
//! page). Java mod jars don't run in Horizon (docs/mods.md), so they are never searched.
pub mod curseforge;
pub mod download;
pub mod installed;
pub mod modrinth;

use serde::{Deserialize, Serialize};

/// The Minecraft version Horizon plays. A version port updates it with the other `"26.3"` sites
/// (docs/version-ports.md).
pub const GAME: &str = "26.3";

/// What a project is, as far as Horizon is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    ResourcePack,
    ShaderPack,
    Modpack,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::ResourcePack, Kind::ShaderPack, Kind::Modpack];

    pub fn label(self) -> &'static str {
        match self {
            Kind::ResourcePack => "Resource packs",
            Kind::ShaderPack => "Shader packs",
            Kind::Modpack => "Modpacks",
        }
    }

    /// The folder beside `options.txt` the file goes in; a modpack is imported instead.
    pub fn folder(self) -> Option<&'static str> {
        match self {
            Kind::ResourcePack => Some("resourcepacks"),
            Kind::ShaderPack => Some("shaderpacks"),
            Kind::Modpack => None,
        }
    }
}

/// Where a project comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceId {
    Modrinth,
    CurseForge,
}

impl SourceId {
    pub fn label(self) -> &'static str {
        match self {
            SourceId::Modrinth => "Modrinth",
            SourceId::CurseForge => "CurseForge",
        }
    }
}

/// How well a project runs in Horizon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Support {
    /// Made for the version Horizon plays, and Horizon reads everything it uses.
    Works,
    /// It loads, with the limit said.
    Partial(String),
    /// It can't run in Horizon: never offered for install.
    No(String),
}

impl Support {
    pub fn installable(&self) -> bool {
        !matches!(self, Support::No(_))
    }

    pub fn label(&self) -> &'static str {
        match self {
            Support::Works => "Works",
            Support::Partial(_) => "Partly",
            Support::No(_) => "Doesn't run",
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Support::Works => None,
            Support::Partial(r) | Support::No(r) => Some(r),
        }
    }
}

/// A search result or a looked-up project.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub source: SourceId,
    pub id: String,
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub author: String,
    pub downloads: u64,
    /// The project's page on its source, for the player to read before installing.
    pub page: String,
    pub icon: Option<String>,
    pub license: String,
    pub kind: Kind,
    /// The Minecraft versions its files list, and the loaders (`iris`, `optifine`, `minecraft`...).
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
}

impl Project {
    pub fn support(&self) -> Support {
        support(self.kind, &self.game_versions, &self.loaders)
    }
}

/// One downloadable file of a version, with the checksums its source publishes.
#[derive(Clone, Debug, PartialEq)]
pub struct FileRef {
    pub url: String,
    pub name: String,
    pub size: u64,
    pub sha512: Option<String>,
    pub sha1: Option<String>,
    pub primary: bool,
}

impl FileRef {
    /// The strongest checksum the source gives: what the download is checked against.
    pub fn checksum(&self) -> Option<(&'static str, &str)> {
        match (&self.sha512, &self.sha1) {
            (Some(h), _) => Some(("SHA-512", h)),
            (None, Some(h)) => Some(("SHA-1", h)),
            (None, None) => None,
        }
    }
}

/// A version of a project: its files and what it needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    pub id: String,
    pub number: String,
    pub published: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub files: Vec<FileRef>,
    /// Required dependencies, as project ids on the same source.
    pub required: Vec<String>,
    /// Why this version can't be downloaded from here (CurseForge authors can forbid it).
    pub blocked: Option<String>,
}

impl Version {
    /// The file to install: the primary one, else the first.
    pub fn file(&self) -> Option<&FileRef> {
        self.files
            .iter()
            .find(|f| f.primary)
            .or_else(|| self.files.first())
    }
}

/// A search.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub text: String,
    pub kind: Kind,
    /// Off: only what runs (made for [`GAME`], Iris or OptiFine shaders). On: everything of the
    /// kind, each result marked with how well it runs.
    pub everything: bool,
    pub offset: usize,
    pub limit: usize,
}

impl Query {
    pub fn new(text: &str, kind: Kind) -> Self {
        Self {
            text: text.trim().to_owned(),
            kind,
            everything: false,
            offset: 0,
            limit: 20,
        }
    }
}

/// A place projects come from.
pub trait Source: Send + Sync {
    fn id(&self) -> SourceId;
    fn search(&self, query: &Query) -> Result<Vec<Project>, String>;
    fn project(&self, id: &str) -> Result<Project, String>;
    /// Versions, newest first.
    fn versions(&self, project: &Project) -> Result<Vec<Version>, String>;
    /// Downloads one of its files into `folder`, checked ([`download::fetch`]).
    fn fetch(
        &self,
        file: &FileRef,
        folder: &std::path::Path,
        progress: &dyn Fn(f32),
    ) -> Result<std::path::PathBuf, String> {
        download::fetch(file, folder, progress)
    }
}

/// The sources this launcher can use: Modrinth always, CurseForge when it has an API key.
pub fn sources() -> Vec<Box<dyn Source>> {
    let mut all: Vec<Box<dyn Source>> = vec![Box::new(modrinth::Modrinth)];
    if let Some(cf) = curseforge::CurseForge::from_env() {
        all.push(Box::new(cf));
    }
    all
}

/// The source of that id, when this launcher can use it.
pub fn source(id: SourceId) -> Option<Box<dyn Source>> {
    sources().into_iter().find(|s| s.id() == id)
}

/// The loaders a Java shader pack may name that Horizon translates.
const SHADER_LOADERS: [&str; 2] = ["iris", "optifine"];

/// How well a project of `kind` made for `game_versions` and `loaders` runs in Horizon.
pub fn support(kind: Kind, game_versions: &[String], loaders: &[String]) -> Support {
    match kind {
        Kind::ResourcePack => {
            if game_versions.iter().any(|v| v == GAME) {
                Support::Works
            } else if let Some(newest) = newest_release(game_versions) {
                Support::Partial(format!(
                    "Made for {newest}: it loads with vanilla's compatibility warning, and \
                     what changed since may look wrong."
                ))
            } else {
                Support::Partial("Lists no Minecraft version.".into())
            }
        }
        Kind::ShaderPack => {
            if loaders.iter().any(|l| SHADER_LOADERS.contains(&l.as_str())) {
                Support::Partial(
                    "Java shader packs are translated: their full-screen passes draw, the world \
                     and shadow passes don't yet (BSL and Complementary tested)."
                        .into(),
                )
            } else {
                let named = loaders.first().map(String::as_str).unwrap_or("unknown");
                Support::No(format!(
                    "A shader pack for {named}: Horizon runs Iris and OptiFine packs only."
                ))
            }
        }
        Kind::Modpack => Support::Partial(
            "Imported on the Modpacks page: Horizon runs the mods it can port, from your own \
             downloads."
                .into(),
        ),
    }
}

/// The newest plain release (`1.21.4`, `26.2`) in a list of Minecraft versions.
fn newest_release(versions: &[String]) -> Option<&str> {
    versions
        .iter()
        .filter(|v| {
            v.split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        })
        .max_by_key(|v| {
            v.split('.')
                .map(|p| p.parse::<u32>().unwrap_or(0))
                .collect::<Vec<_>>()
        })
        .map(String::as_str)
}

/// The version to install: the newest one that runs, or with `everything` the newest at all.
/// `versions` is newest first, as [`Source::versions`] gives them.
pub fn pick(kind: Kind, versions: &[Version], everything: bool) -> Option<&Version> {
    let mut usable = versions
        .iter()
        .filter(|v| v.blocked.is_none() && v.file().is_some());
    let runs = |v: &&Version| match support(kind, &v.game_versions, &v.loaders) {
        Support::Works => true,
        // A modpack's version is for its own Minecraft; any shader pack Horizon translates.
        Support::Partial(_) => kind != Kind::ResourcePack,
        Support::No(_) => false,
    };
    if let Some(v) = usable.clone().find(runs) {
        return Some(v);
    }
    usable.find(|v| everything && support(kind, &v.game_versions, &v.loaders).installable())
}

/// A file name from a source, kept only when it is one plain name: no folders, no `..`, no
/// hidden file, nothing a file system would read as something else.
pub fn safe_name(name: &str) -> Option<&str> {
    let ok = !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && !name.ends_with(['.', ' '])
        && name.chars().all(|c| {
            !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        });
    ok.then_some(name)
}

#[cfg(test)]
mod tests;
