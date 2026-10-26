//! Extras that became official mods (docs/mods.md, "Horizon's official
//! mods"). For a player who had one on, the game installs the mod from
//! Horizon's registry once, checked as the launcher checks it, turns it on
//! and carries the extra's settings over into the mod's storage. A player
//! who had it off gets nothing: the mod is in the mod browser.
use crate::{extras::Extras, mod_list::ModList};
use horizon_mods::index::{self, Index};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// Horizon's registry; `HORIZON_MOD_INDEX` points elsewhere (a local file
/// for trials, whose files may then be `file://`), as for the launcher.
pub const INDEX_URL: &str = "https://ludovic111.github.io/horizon-mods/index.json";
const MAX_INDEX: u64 = 16 << 20;

/// An extra that is an official mod now.
#[derive(Debug, PartialEq)]
pub struct Moved {
    /// The mod's id in the registry.
    pub id: &'static str,
    /// The extra's name, for the line the player reads.
    pub name: &'static str,
}

pub const MOVED: [Moved; 2] = [
    Moved {
        id: "fps_overlay",
        name: "FPS counter",
    },
    Moved {
        id: "keystrokes",
        name: "Keystrokes",
    },
];

impl Moved {
    fn was_on(&self, e: &Extras) -> bool {
        match self.id {
            "fps_overlay" => e.fps,
            _ => e.keystrokes,
        }
    }
    /// The extra's settings as the mod keeps them (its storage keys).
    pub fn settings(&self, e: &Extras) -> Vec<(&'static str, String)> {
        let flag = |b: bool| if b { "true" } else { "false" }.to_owned();
        match self.id {
            "fps_overlay" => vec![
                ("corner", e.fps_corner.name().into()),
                ("lows", flag(e.fps_lows)),
            ],
            _ => vec![
                ("corner", e.keystrokes_corner.name().into()),
                ("cps", flag(e.keystrokes_cps)),
            ],
        }
    }
    /// The line the player reads once it is installed.
    pub fn notice(&self) -> String {
        format!("{} moved to the mod browser: installed for you", self.name)
    }
}

/// The moved extras still to install: on, and not done yet.
pub fn pending(e: &Extras) -> Vec<&'static Moved> {
    MOVED
        .iter()
        .filter(|m| m.was_on(e) && !e.moved.iter().any(|d| d == m.id))
        .collect()
}

/// The moved extras that were off and not marked yet: nothing to install,
/// only to remember.
pub fn nothing_to_do(e: &Extras) -> Vec<&'static Moved> {
    MOVED
        .iter()
        .filter(|m| !m.was_on(e) && !e.moved.iter().any(|d| d == m.id))
        .collect()
}

fn location() -> String {
    std::env::var("HORIZON_MOD_INDEX")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| INDEX_URL.to_owned())
}

/// `url`'s bytes, at most `max`: `https://` from the network; `file://` only
/// with a local index (a trial registry).
fn get(url: &str, local: bool, max: u64) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    if url.starts_with("https://") {
        let response = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .get(url)
            .call()
            .map_err(|e| format!("{url}: {e}"))?;
        response
            .into_reader()
            .take(max + 1)
            .read_to_end(&mut out)
            .map_err(|e| e.to_string())?;
    } else {
        let path = match url.strip_prefix("file://") {
            Some(p) if local => p,
            _ if local && !url.contains("://") => url,
            _ => return Err(format!("{url}: not an https address")),
        };
        out = fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    }
    if out.len() as u64 > max {
        return Err(format!("{url}: over {max} bytes"));
    }
    Ok(out)
}

/// The registry's index, from the network (or `HORIZON_MOD_INDEX`).
pub fn fetch_index() -> Result<Index, String> {
    let at = location();
    let local = !at.starts_with("https://");
    let text = String::from_utf8(get(&at, local, MAX_INDEX)?).map_err(|e| e.to_string())?;
    if local {
        Index::parse_local(&text)
    } else {
        Index::parse(&text)
    }
}

/// Installs `m` into the game folder `dir` (where `options.txt` is), for a
/// player whose extras were `e`: its newest version for this game that
/// wasn't taken down, checked against the index (size, SHA-256, signature,
/// manifest), as `horizon-mods/<id>.hmod`, turned on, with the extra's
/// settings in its storage unless the mod has its own already.
pub fn install(index: &Index, m: &Moved, e: &Extras, dir: &Path) -> Result<(), String> {
    let project = index
        .project(m.id)
        .ok_or_else(|| format!("{} isn't in Horizon's registry yet", m.id))?;
    let version = project
        .versions
        .iter()
        .find(|v| {
            v.game == horizon_core::version::VERSION_ID
                && index.revoked(&project.id, &v.file.sha256).is_none()
        })
        .ok_or_else(|| format!("{} has no version for this game", m.id))?;
    let mods = dir.join(crate::mod_list::DIR);
    fs::create_dir_all(&mods).map_err(|e| format!("{}: {e}", mods.display()))?;
    let target = mods.join(format!("{}.hmod", m.id));
    let current = index::sha256_file(&target).ok();
    if current.as_deref() != Some(version.file.sha256.as_str()) {
        let local = !location().starts_with("https://");
        let bytes = get(&version.file.url, local, index::MAX_FILE)?;
        let partial: PathBuf = mods.join(format!(".{}.hmod.download", m.id));
        fs::write(&partial, bytes).map_err(|e| format!("{}: {e}", partial.display()))?;
        if let Err(e) = index::check_file(index, project, version, &partial) {
            let _ = fs::remove_file(&partial);
            return Err(format!("{}: {e}", m.id));
        }
        fs::rename(&partial, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    let storage = mods.join(horizon_mods::manifest::STORAGE_DIR).join(m.id);
    fs::create_dir_all(&storage).map_err(|e| format!("{}: {e}", storage.display()))?;
    for (key, value) in m.settings(e) {
        let path = storage.join(key);
        if !path.exists() {
            fs::write(&path, value).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    let mut list = ModList::load(dir);
    list.enabled.insert(m.id.to_owned());
    list.save(dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extras::Corner;

    #[test]
    fn only_extras_that_were_on_are_installed_and_each_once() {
        let mut e = Extras::default();
        assert!(pending(&e).is_empty());
        assert_eq!(nothing_to_do(&e).len(), 2);
        e.fps = true;
        e.fps_corner = Corner::BottomRight;
        e.fps_lows = true;
        assert_eq!(pending(&e), [&MOVED[0]]);
        assert_eq!(
            MOVED[0].settings(&e),
            [("corner", "bottom_right".into()), ("lows", "true".into())]
        );
        assert_eq!(
            MOVED[1].settings(&e),
            [("corner", "top_left".into()), ("cps", "true".into())]
        );
        e.moved.push("fps_overlay".into());
        assert!(pending(&e).is_empty());
        assert_eq!(nothing_to_do(&e), [&MOVED[1]]);
        assert_eq!(
            MOVED[0].notice(),
            "FPS counter moved to the mod browser: installed for you"
        );
    }

    #[test]
    fn a_file_url_needs_a_local_index() {
        assert!(get("file:///etc/hosts", false, 1 << 20).is_err());
        assert!(get("http://example.com/x", true, 1 << 20).is_err());
    }
}
