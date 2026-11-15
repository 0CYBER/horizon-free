//! Which of the installed Horizon mods are on (`docs/modding/src/how-to/content-mods.md`), kept in
//! `horizon-mods.txt` beside `options.txt` as `mod.<id>:true` lines. Every
//! mod is off until the player turns it on in Options > Mods. A native mod is
//! built in, so on until turned off (`mod.<id>:false`); the launcher builds
//! only the native mods whose line is `true`.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub const FILE: &str = "horizon-mods.txt";
/// The `horizon-mods.txt` key of [`ModList::wasm_memory_cap_mib`].
const WASM_MEMORY_CAP: &str = "wasm.memory_cap_mib";
/// The folder holding `.hmod` files, beside `options.txt`.
pub const DIR: &str = "horizon-mods";
/// Where the launcher installs native mods (`<id>.hmod`), beside [`DIR`]. They run only in a
/// game the launcher built with them.
pub const NATIVE_DIR: &str = "horizon-native-mods";
/// Why a native mod installed in [`NATIVE_DIR`] doesn't run in this game.
pub const NOT_BUILT: &str = "Installed, not built into this game: build the game with it in the launcher (Settings > Native mods), then play the instance that build is in.";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModList {
    /// Ids of the mods the player turned on.
    pub enabled: BTreeSet<String>,
    /// Ids of the native mods the player turned off: they rest
    /// (`on`) until the launcher's next build leaves them out.
    pub disabled: BTreeSet<String>,
    /// The keys the player chose for mods' keys, mod id → key name → key
    /// (`key.<id>.<name>:key.keyboard.x` lines).
    pub keys: BTreeMap<String, BTreeMap<String, String>>,
    /// The player's cap on each wasm dependency group's memory, in MiB
    /// (`wasm.memory_cap_mib:<1 to 4096>`); unset, wasm32's whole 4 GiB. A group whose mods
    /// declare more (`wasm_memory_mib`) is refused at load, naming both numbers.
    pub wasm_memory_cap_mib: Option<u32>,
    /// What the Mods page shows, filled by the game from the mods it found.
    pub installed: Vec<Installed>,
    /// What the Mods screen's panel shows of each installed mod, by id.
    pub info: BTreeMap<String, ModInfo>,
}

/// What the Mods screen shows of a mod besides its row: the manifest's descriptive fields and
/// its `icon.png`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModInfo {
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub credits: String,
    pub license: String,
    pub license_url: Option<String>,
    pub homepage: Option<String>,
    pub issues: Option<String>,
    /// The PNG bytes of `icon.png`.
    pub icon: Option<std::sync::Arc<[u8]>>,
    /// Registration and dependency-policy warnings: translation key and literal arguments.
    pub warnings: Vec<(String, Vec<String>)>,
}
impl ModInfo {
    pub fn of(manifest: &crate::Manifest, icon: Option<&crate::icon::Icon>) -> Self {
        Self {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            authors: manifest.authors.clone(),
            credits: manifest.credits.clone(),
            license: manifest.license.clone(),
            license_url: manifest.license_url.clone(),
            homepage: manifest.homepage.clone(),
            issues: manifest.issues.clone(),
            icon: icon.map(|icon| icon.bytes().into()),
            warnings: Vec::new(),
        }
    }
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
    /// Loaded with `--dev-mod`: always on.
    pub dev: bool,
    /// A native mod (`game/native_hooks.rs`): built into the game, on until
    /// the player turns it off, and its settings page.
    pub native: Option<NativePage>,
}

/// A native mod's settings page: its keys as Key Binds rows, then the settings it registered
/// (`Registrar::register_setting`). The keys the player chooses are kept in `ModList::keys`, the
/// settings in the mod's data folder.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativePage {
    /// The keys the mod registers on Options > Controls, under its title.
    pub mappings: Vec<KeyMapping>,
    /// Its settings, in registration order, with the values the page shows.
    pub settings: Vec<NativeSetting>,
}

/// A setting on a native mod's page: its key within the mod, the translation keys of its label
/// and tooltip, and its value.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeSetting {
    pub key: String,
    pub label: String,
    pub tooltip: Option<String>,
    pub value: NativeSettingValue,
}

/// A setting's value as its row shows and changes it.
#[derive(Clone, Debug, PartialEq)]
pub enum NativeSettingValue {
    /// On or off: a click flips it.
    Toggle(bool),
    /// Option `index` of `options` (translation keys): a click moves to the next.
    Choice { options: Vec<String>, index: usize },
    /// A number from `min` to `max` in steps of `step`, on a slider.
    Range {
        min: f64,
        max: f64,
        step: f64,
        value: f64,
    },
    /// A colour, `0xRRGGBB`: a click moves to the next of the sixteen dye colours.
    Color(u32),
}

/// The sixteen dye colours (`DyeColor.getTextureDiffuseColor`, white to black), a colour
/// setting's button steps through.
pub const DYE_COLORS: [u32; 16] = [
    0xF9FFFE, 0xF9801D, 0xC74EBD, 0x3AB3DA, 0xFED83D, 0x80C71F, 0xF38BAA, 0x474F52, 0x9D9D97,
    0x169C9C, 0x8932B8, 0x3C44AA, 0x835432, 0x5E7C16, 0xB02E26, 0x1D1D21,
];

impl NativeSettingValue {
    /// A click on its button: a toggle flips, a choice and a colour move to the next (a colour
    /// that isn't a dye colour to the first one), a range doesn't change (it is a slider).
    pub fn click(&mut self) {
        match self {
            Self::Toggle(on) => *on = !*on,
            Self::Choice { options, index } => *index = (*index + 1) % options.len().max(1),
            Self::Color(rgb) => {
                let next = DYE_COLORS
                    .iter()
                    .position(|c| c == rgb)
                    .map_or(0, |i| i + 1);
                *rgb = DYE_COLORS[next % DYE_COLORS.len()];
            }
            Self::Range { .. } => {}
        }
    }
    /// A range's value as the slider's handle place, 0 to 1; 0 for the other kinds.
    pub fn fraction(&self) -> f32 {
        match self {
            Self::Range {
                min, max, value, ..
            } => ((value - min) / (max - min)).clamp(0.0, 1.0) as f32,
            _ => 0.0,
        }
    }
    /// A range moved to slider place `fraction` (0 to 1): the nearest step, kept in range.
    pub fn slide(&mut self, fraction: f32) {
        if let Self::Range {
            min,
            max,
            step,
            value,
        } = self
        {
            let raw = *min + f64::from(fraction.clamp(0.0, 1.0)) * (*max - *min);
            let steps = ((raw - *min) / *step).round();
            *value = (*min + steps * *step).clamp(*min, *max);
        }
    }
}

/// A key a native mod registers on Options > Controls, under the mod's title.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyMapping {
    /// Its id within the mod (`zoom`); its saved binding is `key.<mod id>.<id>`.
    pub id: String,
    /// The label's translation key.
    pub label: String,
    /// The default key (`key.keyboard.z`).
    pub key: String,
    /// The default modifiers: [`SHIFT`], [`CONTROL`] and [`ALT`] bits.
    pub modifiers: u32,
    pub context: KeyConflictContext,
    /// The category's translation key; empty for none.
    pub category: String,
}

/// The screens in which two mappings can conflict.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: the built-in conflict contexts
pub enum KeyConflictContext {
    #[default]
    Universal,
    InGame,
    Gui,
}
impl KeyConflictContext {
    pub fn conflicts(self, other: Self) -> bool {
        self == Self::Universal || other == Self::Universal || self == other
    }
}
/// Compare bindings, excluding unbound keys and pairs that are both at default.
pub fn bindings_conflict(
    (key, modifiers, context, at_default): (&str, u32, KeyConflictContext, bool),
    (other_key, other_modifiers, other_context, other_default): (
        &str,
        u32,
        KeyConflictContext,
        bool,
    ),
) -> bool {
    key != "key.keyboard.unknown"
        && key == other_key
        && modifiers == other_modifiers
        && context.conflicts(other_context)
        && (!at_default || !other_default)
}

#[cfg(test)]
mod conflict_tests {
    use super::{KeyConflictContext::*, bindings_conflict};

    #[test]
    fn contexts_modifiers_unbound_and_defaults_are_respected() {
        let binding = ("key.keyboard.z", 0, InGame, false);
        assert!(bindings_conflict(
            binding,
            ("key.keyboard.z", 0, Universal, true)
        ));
        assert!(!bindings_conflict(
            binding,
            ("key.keyboard.z", 0, Gui, false)
        ));
        assert!(!bindings_conflict(
            binding,
            ("key.keyboard.z", 1, InGame, false)
        ));
        assert!(!bindings_conflict(
            ("key.keyboard.z", 0, InGame, true),
            ("key.keyboard.z", 0, InGame, true)
        ));
        assert!(!bindings_conflict(
            ("key.keyboard.unknown", 0, Universal, false),
            ("key.keyboard.unknown", 0, Universal, false)
        ));
    }
}

/// The modifier bits of a key binding, in GLFW's input-event encoding.
pub const SHIFT: u32 = 1;
pub const CONTROL: u32 = 2;
pub const ALT: u32 = 4;
static MODIFIER_NAMES: [(u32, &str); 3] = [(SHIFT, "shift"), (CONTROL, "ctrl"), (ALT, "alt")];

/// A binding's value in `horizon-mods.txt`: its modifiers' names, then its key, joined by `+`.
fn encode_binding(key: &str, modifiers: u32) -> String {
    let mut value: String = MODIFIER_NAMES
        .iter()
        .filter(|(bit, _)| modifiers & *bit != 0)
        .map(|(_, name)| format!("{name}+"))
        .collect();
    value.push_str(key);
    value
}

/// Reads a binding's value back; a value without `+` is a bare key with no modifiers.
fn decode_binding(value: &str) -> (String, u32) {
    let (names, key) = value.rsplit_once('+').unwrap_or(("", value));
    let modifiers = names
        .split('+')
        .filter_map(|name| MODIFIER_NAMES.iter().find(|(_, n)| *n == name))
        .fold(0u32, |bits, (bit, _)| bits | bit);
    (key.to_owned(), modifiers)
}

impl ModList {
    /// Retain every applied load notice on both affected installed rows, including
    /// failed/off rows and code rows whose descriptive metadata is unavailable.
    pub fn apply_dependency_notices(&mut self, notices: &[crate::dependencies::Notice]) {
        let installed: BTreeMap<_, _> = self
            .installed
            .iter()
            .map(|row| (row.id.as_str(), row))
            .collect();
        let names: BTreeMap<_, _> = self
            .installed
            .iter()
            .map(|row| {
                let name = self
                    .info
                    .get(&row.id)
                    .map(|info| info.name.as_str())
                    .filter(|name| !name.is_empty())
                    .or_else(|| (!row.title.is_empty()).then_some(row.title.as_str()))
                    .unwrap_or(&row.id);
                (row.id.clone(), name.to_owned())
            })
            .collect();
        for notice in notices {
            let (key, mut args) = notice.message();
            args[0] = names.get(&notice.owner).unwrap_or(&notice.owner).clone();
            args[1] = names.get(&notice.target).unwrap_or(&notice.target).clone();
            // Line layout belongs only to the Mods page, never to refusal/log text.
            let mut note = String::new();
            for text in [notice.reason.as_deref(), notice.url.as_deref()]
                .into_iter()
                .flatten()
            {
                note.push('\n');
                note.push_str(text.trim_end());
            }
            args.push(note);
            let message = (key, args);
            for (index, id) in [&notice.owner, &notice.target].into_iter().enumerate() {
                if index == 1 && notice.owner == notice.target {
                    continue;
                }
                let Some(row) = installed.get(id.as_str()) else {
                    continue;
                };
                let info = self.info.entry(id.clone()).or_insert_with(|| ModInfo {
                    name: row.title.clone(),
                    ..ModInfo::default()
                });
                info.warnings.push(message.clone());
            }
        }
    }

    pub fn load(dir: &Path) -> Self {
        fs::read_to_string(dir.join(FILE))
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }
    pub fn parse(text: &str) -> Self {
        let ids = |value: &str| -> BTreeSet<String> {
            text.lines()
                .filter_map(|l| l.split_once(':'))
                .filter(|(_, v)| v.trim() == value)
                .filter_map(|(k, _)| k.trim().strip_prefix("mod."))
                .map(str::to_owned)
                .collect()
        };
        let (enabled, disabled) = (ids("true"), ids("false"));
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
        let wasm_memory_cap_mib = text
            .lines()
            .filter_map(|l| l.split_once(':'))
            .filter(|(k, _)| k.trim() == WASM_MEMORY_CAP)
            .filter_map(|(_, v)| v.trim().parse().ok())
            .rfind(|mib| (1..=crate::manifest::MAX_WASM_MEMORY_MIB).contains(mib));
        Self {
            enabled,
            disabled,
            keys,
            wasm_memory_cap_mib,
            installed: Vec::new(),
            info: BTreeMap::new(),
        }
    }
    pub fn serialize(&self) -> String {
        let on = self.enabled.iter().map(|id| format!("mod.{id}:true\n"));
        let off = self.disabled.iter().map(|id| format!("mod.{id}:false\n"));
        let keys = self.keys.iter().flat_map(|(id, keys)| {
            keys.iter()
                .map(move |(name, key)| format!("key.{id}.{name}:{key}\n"))
        });
        let cap = self
            .wasm_memory_cap_mib
            .map(|mib| format!("{WASM_MEMORY_CAP}:{mib}\n"));
        on.chain(off).chain(keys).chain(cap).collect()
    }
    /// The cap on each wasm group's memory this instance allows, in MiB.
    pub fn wasm_memory_cap(&self) -> u32 {
        self.wasm_memory_cap_mib
            .unwrap_or(crate::manifest::MAX_WASM_MEMORY_MIB)
    }
    /// Mod `id`'s chosen keys, for `Mods::set_bindings`.
    pub fn keys_of(&self, id: &str) -> Vec<(&str, &str)> {
        self.keys.get(id).map_or_else(Vec::new, |k| {
            k.iter().map(|(n, k)| (n.as_str(), k.as_str())).collect()
        })
    }
    /// Mod `id`'s key `name` as the player bound it (its key and modifiers), else `default`.
    pub fn binding(&self, id: &str, name: &str, default: (&str, u32)) -> (String, u32) {
        match self.keys.get(id).and_then(|k| k.get(name)) {
            Some(value) => decode_binding(value),
            None => (default.0.to_owned(), default.1),
        }
    }
    /// Binds mod `id`'s key `name` to `key` with `modifiers`.
    pub fn set_binding(&mut self, id: &str, name: &str, key: &str, modifiers: u32) {
        let value = encode_binding(key, modifiers);
        self.keys
            .entry(id.to_owned())
            .or_default()
            .insert(name.to_owned(), value);
    }
    /// Drops the player's binding of mod `id`'s key `name`, so it is the default again.
    pub fn reset_binding(&mut self, id: &str, name: &str) {
        if let Some(keys) = self.keys.get_mut(id) {
            keys.remove(name);
        }
    }
    pub fn save(&self, dir: &Path) {
        let path = dir.join(FILE);
        if self.enabled.is_empty()
            && self.disabled.is_empty()
            && self.keys.is_empty()
            && self.wasm_memory_cap_mib.is_none()
            && !path.exists()
        {
            return;
        }
        let _ = fs::write(path, self.serialize());
    }
    pub fn is_on(&self, i: usize) -> bool {
        self.installed.get(i).is_some_and(|m| {
            m.dev
                || if m.native.is_some() {
                    !self.disabled.contains(&m.id)
                } else {
                    self.enabled.contains(&m.id)
                }
        })
    }
    /// Turn the installed mod `id` on (nothing when it isn't installed).
    pub fn turn_on(&mut self, id: &str) {
        let Some(m) = self.installed.iter().find(|m| m.id == id && !m.dev) else {
            return;
        };
        self.disabled.remove(&m.id);
        self.enabled.insert(m.id.clone());
    }
    /// Turns installed mod `i` on or off (a dev mod stays on). A native mod's
    /// line says `false` when off, so that the launcher's next build leaves
    /// it out, and `true` when on again.
    pub fn toggle(&mut self, i: usize) {
        let Some(m) = self.installed.get(i).filter(|m| !m.dev) else {
            return;
        };
        if m.native.is_some() {
            if self.disabled.remove(&m.id) {
                self.enabled.insert(m.id.clone());
            } else {
                self.enabled.remove(&m.id);
                self.disabled.insert(m.id.clone());
            }
        } else if !self.enabled.remove(&m.id) {
            // A wasm mod paused in the previous process is not loaded yet; selecting its
            // package again must clear the same code-dispatch switch used while it ran.
            self.disabled.remove(&m.id);
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
    fn the_player_may_lower_the_wasm_memory_cap() {
        assert_eq!(ModList::parse("").wasm_memory_cap(), 4096);
        let list = ModList::parse("mod.a:true\nwasm.memory_cap_mib:512\n");
        assert_eq!(list.wasm_memory_cap(), 512);
        assert_eq!(ModList::parse(&list.serialize()), list);
        // Outside 1 to 4096 MiB (wasm32's whole space) the line is ignored.
        for text in [
            "wasm.memory_cap_mib:0\n",
            "wasm.memory_cap_mib:8192\n",
            "wasm.memory_cap_mib:lots\n",
        ] {
            assert_eq!(ModList::parse(text).wasm_memory_cap(), 4096, "{text}");
        }
        // A cap alone is still saved, in a folder with no list yet.
        let dir = tempfile::tempdir().unwrap();
        let only_cap = ModList {
            wasm_memory_cap_mib: Some(64),
            ..ModList::default()
        };
        only_cap.save(dir.path());
        assert_eq!(ModList::load(dir.path()).wasm_memory_cap(), 64);
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
    fn a_binding_with_modifiers_round_trips_in_its_keys_line() {
        let mut list = ModList::default();
        list.set_binding("zoom", "in", "key.keyboard.z", SHIFT | ALT);
        assert_eq!(list.serialize(), "key.zoom.in:shift+alt+key.keyboard.z\n");
        let again = ModList::parse(&list.serialize());
        assert_eq!(
            again.binding("zoom", "in", ("key.keyboard.q", 0)),
            ("key.keyboard.z".to_owned(), SHIFT | ALT)
        );
    }

    #[test]
    fn a_plain_key_line_is_a_binding_without_modifiers() {
        let list = ModList::parse("key.zoom.in:key.keyboard.x\n");
        assert_eq!(
            list.binding("zoom", "in", ("key.keyboard.q", CONTROL)),
            ("key.keyboard.x".to_owned(), 0)
        );
        assert_eq!(
            list.binding("zoom", "out", ("key.keyboard.q", CONTROL)),
            ("key.keyboard.q".to_owned(), CONTROL)
        );
    }

    #[test]
    fn reset_restores_the_default_and_drops_the_line() {
        let mut list = ModList::default();
        list.set_binding("zoom", "in", "key.keyboard.x", CONTROL);
        list.reset_binding("zoom", "in");
        assert_eq!(
            list.binding("zoom", "in", ("key.keyboard.q", CONTROL)),
            ("key.keyboard.q".to_owned(), CONTROL)
        );
        assert_eq!(list.serialize(), "");
    }

    #[test]
    fn every_dependency_notice_reaches_both_rows_and_self_pairs_are_not_duplicated() {
        use crate::dependencies::{Notice, NoticeKind};
        let mut list = ModList::default();
        for (id, title, problem) in [
            ("owner", "Owner", Some("refused")),
            ("target", "Target fallback", None),
        ] {
            list.installed.push(Installed {
                id: id.into(),
                title: title.into(),
                description: String::new(),
                problem: problem.map(str::to_owned),
                dev: false,
                native: None,
            });
        }
        list.info.insert(
            "owner".into(),
            ModInfo {
                name: "Owner".into(),
                warnings: vec![("registration.warning".into(), Vec::new())],
                ..ModInfo::default()
            },
        );
        list.apply_dependency_notices(&[
            Notice {
                owner: "owner".into(),
                target: "target".into(),
                version: Some("1".into()),
                reason: Some("reason.".into()),
                url: Some("https://example.test/details".into()),
                kind: NoticeKind::Discouraged,
            },
            Notice {
                owner: "owner".into(),
                target: "target".into(),
                version: Some("1".into()),
                reason: Some("override.".into()),
                url: None,
                kind: NoticeKind::OverrideDropped,
            },
            Notice {
                owner: "owner".into(),
                target: "owner".into(),
                version: Some("1".into()),
                reason: None,
                url: None,
                kind: NoticeKind::OverrideDiscouraged,
            },
        ]);
        assert_eq!(list.info["owner"].warnings.len(), 4);
        assert_eq!(list.info["owner"].warnings[0].0, "registration.warning");
        assert_eq!(list.info["target"].name, "Target fallback");
        assert_eq!(list.info["target"].warnings.len(), 2);
        assert_eq!(
            list.info["target"].warnings[0].0,
            "horizon.mods.warning.discouraged"
        );
        assert_eq!(
            list.info["target"].warnings[0].1,
            [
                "Owner",
                "Target fallback",
                "1",
                "\nreason.\nhttps://example.test/details"
            ]
        );
        assert_eq!(
            list.info["target"].warnings[1].0,
            "horizon.mods.warning.override_dropped"
        );
        assert_eq!(
            list.info["target"].warnings[1].1,
            ["Owner", "Target fallback", "\noverride."]
        );
    }

    #[test]
    fn selecting_a_paused_package_clears_its_previous_code_switch() {
        let mut list = ModList::parse("mod.wasm_mod:false\n");
        list.installed.push(Installed {
            id: "wasm_mod".into(),
            title: "wasm_mod".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: None,
        });
        list.toggle(0);
        assert!(list.enabled.contains("wasm_mod"));
        assert!(!list.disabled.contains("wasm_mod"));
        list.enabled.clear();
        list.disabled.insert("wasm_mod".into());
        list.turn_on("wasm_mod");
        assert!(list.enabled.contains("wasm_mod"));
        assert!(!list.disabled.contains("wasm_mod"));
    }

    #[test]
    fn toggles_installed_and_native_mods_but_not_dev_ones() {
        let installed = |id: &str, dev| Installed {
            id: id.into(),
            title: id.into(),
            description: String::new(),
            problem: None,
            dev,
            native: None,
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
        list.installed.push(Installed {
            native: Some(NativePage::default()),
            ..installed("c", false)
        });
        // On until turned off; off, the launcher leaves it out.
        assert!(list.is_on(2));
        list.toggle(2);
        assert!(!list.is_on(2));
        assert_eq!(list.serialize(), "mod.c:false\n");
        assert_eq!(ModList::parse(&list.serialize()).disabled, list.disabled);
        list.toggle(2);
        assert!(list.is_on(2));
        assert_eq!(list.serialize(), "mod.c:true\n");
    }
}

#[cfg(test)]
mod native_setting_tests {
    use super::*;

    #[test]
    fn a_setting_button_steps_its_value_and_a_slider_snaps_to_its_step() {
        let mut toggle = NativeSettingValue::Toggle(false);
        toggle.click();
        assert_eq!(toggle, NativeSettingValue::Toggle(true));
        let mut choice = NativeSettingValue::Choice {
            options: vec!["a".into(), "b".into()],
            index: 1,
        };
        choice.click();
        assert_eq!(
            choice,
            NativeSettingValue::Choice {
                options: vec!["a".into(), "b".into()],
                index: 0
            }
        );
        let mut color = NativeSettingValue::Color(0x12_3456);
        color.click();
        assert_eq!(color, NativeSettingValue::Color(DYE_COLORS[0]));
        let mut color = NativeSettingValue::Color(DYE_COLORS[15]);
        color.click();
        assert_eq!(color, NativeSettingValue::Color(DYE_COLORS[0]));
        let mut range = NativeSettingValue::Range {
            min: 1.0,
            max: 3.0,
            step: 0.5,
            value: 2.0,
        };
        assert_eq!(range.fraction(), 0.5);
        range.slide(0.6);
        assert_eq!(
            range,
            NativeSettingValue::Range {
                min: 1.0,
                max: 3.0,
                step: 0.5,
                value: 2.0
            }
        );
        range.slide(0.9);
        assert_eq!(range.fraction(), 1.0, "2.8 snaps to 3.0");
        range.slide(-4.0);
        assert_eq!(range.fraction(), 0.0);
    }
}
