//! Horizon's own registry (docs/api/mod-index.md): mods, modpacks and resource packs made for
//! Horizon, reviewed as pull requests in ludovic111/horizon-mods. One file lists them all,
//! `index.json`: this is the browser source that searches it and downloads its files, checked
//! against the SHA-256 it lists. Versions the index says were taken down are blocked.
use super::{
    details::{Details, Link},
    download, FileRef, Kind, Project, Query, Source, SourceId, Version, GAME,
};
use horizon_mods::index::{self, Index};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where the registry publishes its index (GitHub Pages of ludovic111/horizon-mods).
pub const INDEX_URL: &str = "https://ludovic111.github.io/horizon-mods/index.json";
/// The registry's pages on the website, `<SITE>/<id>`.
const SITE: &str = "https://playhorizon.dev/mods";
/// The report form of a project.
const REPORT: &str = "https://github.com/ludovic111/horizon-mods/issues/new?template=report.yml";
/// The loader name the browser gives registry versions, so `support` and `pick` know them.
pub const LOADER: &str = "horizon";
/// How long a fetched index is used before it is fetched again.
const FRESH: Duration = Duration::from_secs(600);
/// Larger than any index should be.
const MAX_INDEX: u64 = 16 << 20;
/// The last index fetched, kept in the game folder for when the registry can't be reached.
const CACHE: &str = "mod-browser/horizon-index.json";

/// Where the index comes from: `HORIZON_MOD_INDEX` (a file or an `https://` address, for a
/// test registry), else [`INDEX_URL`].
fn location() -> String {
    std::env::var("HORIZON_MOD_INDEX")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| INDEX_URL.to_owned())
}

/// True when the index is a local file: its files may then be local too (`file://`), for
/// trying a registry out before it is published.
pub fn local() -> bool {
    !location().starts_with("https://")
}

static LOADED: Mutex<Option<(Instant, Arc<Index>)>> = Mutex::new(None);

/// The index: the one fetched in the last ten minutes, else fetched again (and kept in the game
/// folder), else the copy kept last time.
pub fn index() -> Result<Arc<Index>, String> {
    if let Some((at, index)) = LOADED.lock().unwrap().as_ref() {
        if at.elapsed() < FRESH {
            return Ok(index.clone());
        }
    }
    let cache = crate::backend::game_home().join(CACHE);
    let index = match fetch_index() {
        Ok((index, text)) => {
            if let Some(dir) = cache.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = fs::write(&cache, text);
            index
        }
        Err(e) => {
            let kept = fs::read_to_string(&cache).map_err(|_| e.clone())?;
            match local() {
                true => Index::parse_local(&kept),
                false => Index::parse(&kept),
            }
            .map_err(|_| e)?
        }
    };
    let index = Arc::new(index);
    *LOADED.lock().unwrap() = Some((Instant::now(), index.clone()));
    Ok(index)
}

fn fetch_index() -> Result<(Index, String), String> {
    use std::io::Read;
    let at = location();
    let text = if at.starts_with("https://") {
        let response = download::agent().get(&at).call().map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("The mod registry answered {code}."),
            ureq::Error::Transport(t) => format!("The mod registry can't be reached: {t}"),
        })?;
        let mut text = String::new();
        response
            .into_reader()
            .take(MAX_INDEX)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        text
    } else {
        fs::read_to_string(&at).map_err(|e| format!("{at}: {e}"))?
    };
    let index = match local() {
        true => Index::parse_local(&text)?,
        false => Index::parse(&text)?,
    };
    Ok((index, text))
}

/// The browser's kind for a registry kind; None for one the launcher doesn't install yet
/// (Horizon modpacks and shader packs, kinds newer than this launcher).
fn kind_of(kind: &index::Kind) -> Option<Kind> {
    match kind {
        index::Kind::Mod => Some(Kind::Mod),
        index::Kind::ResourcePack => Some(Kind::ResourcePack),
        index::Kind::Modpack | index::Kind::Shader | index::Kind::Other(_) => None,
    }
}

fn project_of(p: &index::Project) -> Option<Project> {
    let mut game_versions: Vec<String> = p.versions.iter().map(|v| v.game.clone()).collect();
    game_versions.dedup();
    Some(Project {
        source: SourceId::Horizon,
        id: p.id.clone(),
        slug: p.id.clone(),
        title: p.name.clone(),
        summary: p.summary.clone(),
        author: p.authors.join(", "),
        downloads: 0,
        page: format!("{SITE}/{}", p.id),
        icon: p.icon.clone(),
        license: p.license.clone(),
        kind: kind_of(&p.kind)?,
        game_versions,
        loaders: vec![LOADER.into()],
    })
}

fn version_of(index: &Index, p: &index::Project, v: &index::Version) -> Version {
    Version {
        id: v.version.clone(),
        number: v.version.clone(),
        published: v.published.clone(),
        game_versions: vec![v.game.clone()],
        loaders: vec![LOADER.into()],
        files: vec![FileRef {
            url: v.file.url.clone(),
            name: v.file.name.clone(),
            size: v.file.size,
            sha512: None,
            sha256: Some(v.file.sha256.clone()),
            sha1: None,
            primary: true,
        }],
        required: v.requires.iter().map(|r| r.id.clone()).collect(),
        blocked: index
            .revoked(&p.id, &v.file.sha256)
            .map(|r| format!("taken down: {}", r.reason)),
    }
}

/// The words of a query found in a project's id, name, summary or authors.
fn matches(p: &index::Project, text: &str) -> bool {
    let hay = format!("{} {} {} {}", p.id, p.name, p.summary, p.authors.join(" ")).to_lowercase();
    text.split_whitespace()
        .all(|word| hay.contains(&word.to_lowercase()))
}

/// The projects of the query's kind whose words match, for this game unless `everything`.
fn search(index: &Index, query: &Query) -> Vec<Project> {
    index
        .projects
        .iter()
        .filter(|p| kind_of(&p.kind) == Some(query.kind) && matches(p, &query.text))
        .filter(|p| query.everything || p.versions.iter().any(|v| v.game == GAME))
        .filter_map(project_of)
        .skip(query.offset)
        .take(query.limit)
        .collect()
}

/// The registry as a browser source.
pub struct Registry;

impl Source for Registry {
    fn id(&self) -> SourceId {
        SourceId::Horizon
    }

    fn search(&self, query: &Query) -> Result<Vec<Project>, String> {
        Ok(search(&*index()?, query))
    }

    fn project(&self, id: &str) -> Result<Project, String> {
        let index = index()?;
        let p = index
            .project(id)
            .ok_or_else(|| format!("{id} isn't in Horizon's registry."))?;
        project_of(p).ok_or_else(|| {
            format!(
                "{} is a {} in Horizon's registry: this launcher can't install those yet.",
                p.name,
                p.kind.name()
            )
        })
    }

    fn versions(&self, project: &Project) -> Result<Vec<Version>, String> {
        let index = index()?;
        let p = index
            .project(&project.id)
            .ok_or_else(|| format!("{} isn't in Horizon's registry.", project.title))?;
        Ok(p.versions
            .iter()
            .map(|v| version_of(&index, p, v))
            .collect())
    }

    fn details(&self, project: &Project) -> Result<Details, String> {
        let index = index()?;
        let p = index
            .project(&project.id)
            .ok_or_else(|| format!("{} isn't in Horizon's registry.", project.title))?;
        let newest = p.versions.first();
        let mut body = p.description.clone();
        if let Some(v) = newest.filter(|v| !v.capabilities.is_empty()) {
            body.push_str("\n\n## What it may do\n\n");
            for c in &v.capabilities {
                body.push_str(&format!("- May {}\n", c.describe()));
            }
        }
        Ok(Details {
            body,
            links: vec![
                Link {
                    label: "Source".into(),
                    url: p.source.clone(),
                },
                Link {
                    label: "Page".into(),
                    url: project.page.clone(),
                },
                Link {
                    label: "Report".into(),
                    url: format!("{REPORT}&title={}", p.id),
                },
            ],
            authors: p.authors.clone(),
            published: p
                .versions
                .last()
                .map(|v| v.published.clone())
                .unwrap_or_default(),
            updated: newest.map(|v| v.published.clone()).unwrap_or_default(),
            ..Details::default()
        })
    }

    fn fetch(
        &self,
        file: &FileRef,
        folder: &Path,
        progress: &dyn Fn(f32),
    ) -> Result<PathBuf, String> {
        fetch(file, folder, local(), progress)
    }
}

/// Downloads a registry file, checked; `file://` files only from a local index.
fn fetch(
    file: &FileRef,
    folder: &Path,
    local: bool,
    progress: &dyn Fn(f32),
) -> Result<PathBuf, String> {
    match file.url.strip_prefix("file://").filter(|_| local) {
        Some(path) => {
            let reader = fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
            download::save_from_anywhere(file, reader, folder, progress)
        }
        None => download::fetch(file, folder, progress),
    }
}

#[cfg(test)]
mod tests;
