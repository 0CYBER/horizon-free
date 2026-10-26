//! Which of the installed Horizon mods are on (`docs/mods.md`), kept in
//! `horizon-mods.txt` beside `options.txt` as `mod.<id>:true` lines. Every
//! mod is off until the player turns it on in Options > Mods.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub const FILE: &str = "horizon-mods.txt";
/// The folder holding `.hmod` files, beside `options.txt`.
pub const DIR: &str = "horizon-mods";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModList {
    /// Ids of the mods the player turned on.
    pub enabled: BTreeSet<String>,
    /// The keys the player chose for mods' keys, mod id → key name → key
    /// (`key.<id>.<name>:key.keyboard.x` lines).
    pub keys: BTreeMap<String, BTreeMap<String, String>>,
    /// What the Mods page shows, filled by the game from the mods it found.
    pub installed: Vec<Installed>,
}

/// One installed mod as the Mods page shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub id: String,
    /// `Name 1.0` (the file name when the manifest is unreadable).
    pub title: String,
    pub description: String,
    /// Why it is not running, when it failed.
    pub problem: Option<String>,
    /// Loaded with `--dev-mod`: always on, reloaded when rebuilt.
    pub dev: bool,
    /// The running mod set a page of widgets (capability `ui`): its row
    /// gains a Settings button.
    pub has_settings: bool,
}

impl ModList {
    pub fn load(dir: &Path) -> Self {
        fs::read_to_string(dir.join(FILE))
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }
    pub fn parse(text: &str) -> Self {
        let enabled = text
            .lines()
            .filter_map(|l| l.split_once(':'))
            .filter(|(_, v)| v.trim() == "true")
            .filter_map(|(k, _)| k.trim().strip_prefix("mod."))
            .map(str::to_owned)
            .collect();
        let mut keys: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for (k, v) in text.lines().filter_map(|l| l.split_once(':')) {
            // Mod ids have no dots; a key's name may.
            if let Some((id, name)) = k
                .trim()
                .strip_prefix("key.")
                .and_then(|r| r.split_once('.'))
            {
                let entry = keys.entry(id.to_owned()).or_default();
                entry.insert(name.to_owned(), v.trim().to_owned());
            }
        }
        Self {
            enabled,
            keys,
            installed: Vec::new(),
        }
    }
    pub fn serialize(&self) -> String {
        let on = self.enabled.iter().map(|id| format!("mod.{id}:true\n"));
        let keys = self.keys.iter().flat_map(|(id, keys)| {
            keys.iter()
                .map(move |(name, key)| format!("key.{id}.{name}:{key}\n"))
        });
        on.chain(keys).collect()
    }
    /// Mod `id`'s chosen keys, for `Mods::set_bindings`.
    pub fn keys_of(&self, id: &str) -> Vec<(&str, &str)> {
        self.keys.get(id).map_or_else(Vec::new, |k| {
            k.iter().map(|(n, k)| (n.as_str(), k.as_str())).collect()
        })
    }
    pub fn save(&self, dir: &Path) {
        let path = dir.join(FILE);
        if self.enabled.is_empty() && self.keys.is_empty() && !path.exists() {
            return;
        }
        let _ = fs::write(path, self.serialize());
    }
    pub fn is_on(&self, i: usize) -> bool {
        self.installed
            .get(i)
            .is_some_and(|m| m.dev || self.enabled.contains(&m.id))
    }
    /// Turns installed mod `i` on or off (a dev mod stays on).
    pub fn toggle(&mut self, i: usize) {
        let Some(m) = self.installed.get(i).filter(|m| !m.dev) else {
            return;
        };
        if !self.enabled.remove(&m.id) {
            self.enabled.insert(m.id.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_ignores_the_rest() {
        let list = ModList::parse("mod.coords_hud:true\nmod.other:false\nzoom:true\n");
        assert_eq!(list.enabled.iter().collect::<Vec<_>>(), ["coords_hud"]);
        assert_eq!(ModList::parse(&list.serialize()), list);
        assert_eq!(ModList::default().serialize(), "");
    }

    #[test]
    fn keeps_the_keys_players_chose() {
        let text = "mod.zoom:true\nkey.zoom.zoom.in:key.mouse.4\nkey.zoom.out:key.keyboard.x\n";
        let list = ModList::parse(text);
        assert_eq!(
            list.keys_of("zoom"),
            [("out", "key.keyboard.x"), ("zoom.in", "key.mouse.4")]
        );
        assert!(list.keys_of("other").is_empty());
        assert_eq!(ModList::parse(&list.serialize()), list);
    }

    #[test]
    fn toggles_installed_mods_but_not_dev_ones() {
        let installed = |id: &str, dev| Installed {
            id: id.into(),
            title: id.into(),
            description: String::new(),
            problem: None,
            dev,
            has_settings: false,
        };
        let mut list = ModList {
            installed: vec![installed("a", false), installed("b", true)],
            ..Default::default()
        };
        assert!(!list.is_on(0) && list.is_on(1));
        list.toggle(0);
        list.toggle(1);
        assert!(list.is_on(0) && list.is_on(1));
        assert_eq!(list.serialize(), "mod.a:true\n");
        list.toggle(0);
        assert!(!list.is_on(0));
    }
}
