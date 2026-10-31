//! `server.toml`, the one file an owner edits (docs/server.md, section 4).
//!
//! It has typed sections for what owners change and `[properties]` for any other vanilla key. Before every
//! start the supervisor writes them into `server.properties`, which the game reads exactly as vanilla's
//! server does; what the game changes there itself (`/whitelist on`) is copied back afterwards.

use horizon_server_files::{
    java_properties::{Properties, date_string, now_millis},
    properties::{DIFFICULTIES, GAME_MODES, ServerProperties, SystemEntropy},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

pub const FILE: &str = "server.toml";
pub const PROPERTIES: &str = "server.properties";

/// What `init` writes: every typed setting, with vanilla 26.3's defaults and what it means.
pub const TEMPLATE: &str = r##"# Horizon server settings (docs/server.md). The server writes them into server.properties before
# every start: edit this file, not that one.

[server]
motd = "A Minecraft Server"  # the line under the server's name in the server list
port = 25565                 # the port players connect to
bind = ""                    # listen on every address, or only on this one (server-ip)
max_players = 20             # how many players can be online at once
online_mode = true           # check players' Minecraft accounts (false: anyone joins under any name)
whitelist = true             # only whitelisted players and operators can join
enforce_whitelist = false    # kick online players taken off the whitelist
view_distance = 10           # chunks sent around each player
simulation_distance = 10     # chunks around each player where mobs, crops and redstone tick
max_view_distance = 0        # past view_distance, saved chunks only, up to this (never generated; 0: off)

[world]
name = "world"               # the world folder
seed = ""                    # empty: a random seed on the first start
game_mode = "survival"       # survival, creative, adventure or spectator
difficulty = "easy"          # peaceful, easy, normal or hard
hardcore = false             # one life: a player who dies becomes a spectator

# Any other server.properties key, spelled as vanilla spells it, for example:
# spawn-protection = 16
[properties]

[engine]
path = ""                    # the game program; empty: the one installed beside horizon-server
resources = ""               # the prepared game files; empty: resources/ here, else the usual places
restart_on_crash = true      # start the game again when it crashes

[backups]
every = "6h"                 # automatic backups while the server runs: 30m, 6h, 1d...; "0" turns them off
keep = 12                    # automatic backups kept (backups made by hand are never deleted)

[updates]
keep = 1                     # other releases kept by updates, besides the one in use and the one rollback goes to

[panel]
bind = "127.0.0.1:8130"      # the web panel: this computer only, or a tailnet address (100.x); "" turns it off
allow_public = false         # true lets bind be any address; never put the panel on the open internet

# Horizon's anticheat (docs/anticheat.md) is off: the server checks only what vanilla's checks. To turn it on:
# [anticheat]
# enabled = true
# alerts = true              # tell online operators (alert_level = 2) in their chat; the console always is
# log = "logs/anticheat.log" # every flag, one line each
# [anticheat.timer]          # a sped-up game: more client ticks than the server's clock allows
# setback = 2                # the violation level from which the player is sent back; 0: never
# alert = 5
# kick = 20
# decay = 0.02               # violation level lost each second
"##;

/// Each typed setting: its section, its name, and the vanilla key it writes.
pub(crate) const TYPED: [(&str, &str, &str); 15] = [
    ("server", "motd", "motd"),
    ("server", "port", "server-port"),
    ("server", "bind", "server-ip"),
    ("server", "max_players", "max-players"),
    ("server", "online_mode", "online-mode"),
    ("server", "whitelist", "white-list"),
    ("server", "enforce_whitelist", "enforce-whitelist"),
    ("server", "view_distance", "view-distance"),
    ("server", "simulation_distance", "simulation-distance"),
    ("server", "max_view_distance", "horizon-max-view-distance"),
    ("world", "name", "level-name"),
    ("world", "seed", "level-seed"),
    ("world", "game_mode", "gamemode"),
    ("world", "difficulty", "difficulty"),
    ("world", "hardcore", "hardcore"),
];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: Server,
    #[serde(default)]
    pub world: World,
    #[serde(default)]
    pub properties: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub engine: Engine,
    #[serde(default)]
    pub backups: Backups,
    #[serde(default)]
    pub updates: Updates,
    #[serde(default)]
    pub panel: Panel,
    /// The game reads this section itself (docs/anticheat.md); written only when it isn't the default (off).
    #[serde(default, skip_serializing_if = "anticheat_off")]
    pub anticheat: horizon_anticheat::Config,
}

fn anticheat_off(c: &horizon_anticheat::Config) -> bool {
    *c == horizon_anticheat::Config::default()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Server {
    pub motd: String,
    pub port: u16,
    pub bind: String,
    pub max_players: u32,
    pub online_mode: bool,
    pub whitelist: bool,
    pub enforce_whitelist: bool,
    pub view_distance: u32,
    pub simulation_distance: u32,
    /// Horizon's far ring (docs/hosting.md): saved chunks sent past `view_distance`, up to this; 0 is off.
    pub max_view_distance: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct World {
    pub name: String,
    /// A number or any text (vanilla hashes text); empty for a random seed.
    #[serde(deserialize_with = "text_or_number")]
    pub seed: String,
    pub game_mode: String,
    pub difficulty: String,
    pub hardcore: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Engine {
    pub path: String,
    pub resources: String,
    pub restart_on_crash: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Panel {
    pub bind: String,
    pub allow_public: bool,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8130".into(),
            allow_public: false,
        }
    }
}

impl Panel {
    /// Where the panel listens; `None` when it is off.
    pub fn address(&self) -> Result<Option<std::net::SocketAddr>, String> {
        if self.bind.trim().is_empty() {
            return Ok(None);
        }
        let address: std::net::SocketAddr = self.bind.trim().parse().map_err(|_| {
            format!(
                "panel.bind is \"{}\": use an address and a port, like 127.0.0.1:8130",
                self.bind
            )
        })?;
        if !self.allow_public && !private(address.ip()) {
            return Err(format!(
                "panel.bind is \"{}\": the panel listens only on this computer (127.0.0.1) or a tailnet \
                 address (100.64.0.0/10, fd7a:115c:a1e0::/48); reach it from elsewhere over Tailscale or SSH",
                self.bind
            ));
        }
        Ok(Some(address))
    }
}

/// Loopback, or a Tailscale address: reachable only from this computer or the owner's tailnet.
pub fn private(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback() || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64)
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || v6.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0]
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Backups {
    pub every: String,
    pub keep: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Updates {
    /// Releases kept in `releases/` after an update besides the one in use and the one `rollback` goes
    /// back to (crate::update::Install::prune).
    pub keep: u32,
}

impl Default for Updates {
    fn default() -> Self {
        Self { keep: 1 }
    }
}

impl Default for Server {
    fn default() -> Self {
        Self {
            motd: "A Minecraft Server".into(),
            port: 25565,
            bind: String::new(),
            max_players: 20,
            online_mode: true,
            whitelist: true,
            enforce_whitelist: false,
            view_distance: 10,
            simulation_distance: 10,
            max_view_distance: 0,
        }
    }
}
impl Default for World {
    fn default() -> Self {
        Self {
            name: "world".into(),
            seed: String::new(),
            game_mode: "survival".into(),
            difficulty: "easy".into(),
            hardcore: false,
        }
    }
}
impl Default for Engine {
    fn default() -> Self {
        Self {
            path: String::new(),
            resources: String::new(),
            restart_on_crash: true,
        }
    }
}

impl Default for Backups {
    fn default() -> Self {
        Self {
            every: "6h".into(),
            keep: 12,
        }
    }
}

fn text_or_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Seed {
        Text(String),
        Number(i64),
    }
    Ok(match Seed::deserialize(d)? {
        Seed::Text(s) => s,
        Seed::Number(n) => n.to_string(),
    })
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let config: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        config.check()?;
        Ok(config)
    }

    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join(FILE);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("{}: {e} (run `horizon-server init` first)", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn check(&self) -> Result<(), String> {
        let w = &self.world;
        if !GAME_MODES.contains(&w.game_mode.as_str()) {
            return Err(format!(
                "world.game_mode is \"{}\": use {}",
                w.game_mode,
                GAME_MODES.join(", ")
            ));
        }
        if !DIFFICULTIES.contains(&w.difficulty.as_str()) {
            return Err(format!(
                "world.difficulty is \"{}\": use {}",
                w.difficulty,
                DIFFICULTIES.join(", ")
            ));
        }
        if w.name.is_empty() || w.name.contains(['/', '\\']) || w.name == ".." {
            return Err(format!(
                "world.name \"{}\" must be a folder name, without slashes",
                w.name
            ));
        }
        crate::backup::interval(&self.backups.every)?;
        self.panel.address()?;
        self.anticheat.check()?;
        if self.server.port == 0 {
            return Err("server.port must be a port number from 1 to 65535".into());
        }
        if let Some((section, field, key)) =
            TYPED.iter().find(|t| self.properties.contains_key(t.2))
        {
            return Err(format!(
                "[properties] sets {key}: set {section}.{field} instead"
            ));
        }
        Ok(())
    }

    /// The vanilla keys and values this configuration sets, typed settings first.
    pub fn vanilla(&self) -> Vec<(String, String)> {
        let table = toml::Table::try_from(self).expect("a configuration is a table");
        let mut out: Vec<(String, String)> = TYPED
            .iter()
            .map(|(section, field, key)| {
                ((*key).to_owned(), property_text(&table[*section][*field]))
            })
            .collect();
        out.extend(
            self.properties
                .iter()
                .map(|(k, v)| (k.clone(), property_text(v))),
        );
        out
    }

    /// Writes this configuration into `server.properties`, keeping the keys it doesn't set. Returns what it
    /// wrote, for [`sync_back`].
    pub fn write_properties(&self, dir: &Path) -> std::io::Result<Vec<(String, String)>> {
        let path = dir.join(PROPERTIES);
        let mut properties = read_properties(&path).unwrap_or_default();
        let written = self.vanilla();
        for (key, value) in &written {
            properties.put(key, value.clone());
        }
        let text = properties.store(
            Some("Minecraft server properties"),
            &date_string(now_millis()),
            false,
        );
        std::fs::write(path, text)?;
        Ok(written)
    }

    /// A configuration from a vanilla `server.properties`: typed settings from their keys, and every other
    /// key the file sets to something other than vanilla's default under `[properties]`.
    pub fn import(source: &Properties) -> Result<Self, String> {
        let parsed = ServerProperties::new(source.clone(), &mut SystemEntropy::default())?;
        let defaults = ServerProperties::new(Properties::default(), &mut SystemEntropy::default())?;
        let mut table = toml::Table::try_from(Self::default()).map_err(|e| e.to_string())?;
        for (section, field, key) in TYPED {
            let (Some(text), Some(slot)) = (
                parsed.raw.get(key),
                table
                    .get_mut(section)
                    .and_then(|s| s.as_table_mut())
                    .and_then(|s| s.get_mut(field)),
            ) else {
                continue;
            };
            if let Some(value) = typed_value(text, slot) {
                *slot = value;
            }
        }
        let mut extra = toml::Table::new();
        let mut keys: Vec<&String> = source.map.keys().collect();
        keys.sort();
        for key in keys {
            let value = parsed.raw.get(key).unwrap_or_default();
            if TYPED.iter().any(|t| t.2 == key) || defaults.raw.get(key) == Some(value) {
                continue;
            }
            extra.insert(key.clone(), loose_value(value));
        }
        table.insert("properties".into(), toml::Value::Table(extra));
        let config: Self = table.try_into().map_err(|e| e.to_string())?;
        config.check()?;
        Ok(config)
    }
}

/// `server.toml` for a configuration: the template's comments with this configuration's values.
pub fn render(config: &Config) -> String {
    let mut doc: toml_edit::DocumentMut = TEMPLATE.parse().expect("the template parses");
    let table = toml::Table::try_from(config).expect("a configuration is a table");
    for (section, field, _) in TYPED {
        set(&mut doc, section, field, &table[section][field]);
    }
    for (key, value) in &config.properties {
        set(&mut doc, "properties", key, value);
    }
    for (section, field) in [
        ("engine", "path"),
        ("engine", "resources"),
        ("engine", "restart_on_crash"),
        ("backups", "every"),
        ("backups", "keep"),
        ("updates", "keep"),
        ("panel", "bind"),
        ("panel", "allow_public"),
    ] {
        set(&mut doc, section, field, &table[section][field]);
    }
    let mut text = doc.to_string();
    if let Some(anticheat) = table.get("anticheat") {
        let mut only = toml::Table::new();
        only.insert("anticheat".into(), anticheat.clone());
        text.push('\n');
        text.push_str(&toml::to_string(&only).expect("the anticheat's settings are a table"));
    }
    text
}

/// After the game stopped: copies into `server.toml` the keys it wrote that the game changed in
/// `server.properties` (`/whitelist on` stores `white-list`), keeping the owner's comments. Returns the keys
/// copied.
pub fn sync_back(dir: &Path, written: &[(String, String)]) -> Result<Vec<String>, String> {
    let Ok(now) = read_properties(&dir.join(PROPERTIES)) else {
        return Ok(Vec::new());
    };
    let changed: Vec<(&str, &str)> = written
        .iter()
        .filter_map(|(key, value)| {
            now.get(key)
                .filter(|v| v != value)
                .map(|v| (key.as_str(), v))
        })
        .collect();
    if changed.is_empty() {
        return Ok(Vec::new());
    }
    let path = dir.join(FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let defaults = toml::Table::try_from(Config::default()).map_err(|e| e.to_string())?;
    for (key, value) in &changed {
        match TYPED.iter().find(|t| t.2 == *key) {
            Some((section, field, _)) => {
                if let Some(v) = typed_value(value, &defaults[*section][*field]) {
                    set(&mut doc, section, field, &v);
                }
            }
            None => set(&mut doc, "properties", key, &loose_value(value)),
        }
    }
    let updated = doc.to_string();
    Config::parse(&updated)?;
    std::fs::write(&path, updated).map_err(|e| e.to_string())?;
    Ok(changed.iter().map(|(k, _)| (*k).to_owned()).collect())
}

/// Sets one typed setting in `server.toml` (`whitelist on`), keeping the owner's comments.
pub fn update(dir: &Path, section: &str, field: &str, value: toml::Value) -> Result<(), String> {
    update_all(dir, &[(section.to_owned(), field.to_owned(), value)])
}

/// Sets settings in `server.toml`, keeping the owner's comments; nothing is written unless the result is
/// a valid configuration.
pub fn update_all(dir: &Path, changes: &[(String, String, toml::Value)]) -> Result<(), String> {
    let path = dir.join(FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    for (section, field, value) in changes {
        set(&mut doc, section, field, value);
    }
    let updated = doc.to_string();
    Config::parse(&updated)?;
    std::fs::write(&path, updated).map_err(|e| e.to_string())
}

pub fn read_properties(path: &Path) -> Result<Properties, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Properties::load_file(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Sets one value in a TOML document, keeping the comment after the old value.
fn set(doc: &mut toml_edit::DocumentMut, section: &str, key: &str, value: &toml::Value) {
    let Some(table) = doc
        .entry(section)
        .or_insert_with(toml_edit::table)
        .as_table_mut()
    else {
        return;
    };
    let mut new = match value {
        toml::Value::String(s) => toml_edit::value(s.as_str()),
        toml::Value::Integer(n) => toml_edit::value(*n),
        toml::Value::Float(f) => toml_edit::value(*f),
        toml::Value::Boolean(b) => toml_edit::value(*b),
        other => toml_edit::value(other.to_string()),
    };
    if let (Some(old), Some(v)) = (
        table.get(key).and_then(|i| i.as_value()),
        new.as_value_mut(),
    ) {
        *v.decor_mut() = old.decor().clone();
    }
    table.insert(key, new);
}

/// How a TOML value reads in server.properties.
fn property_text(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A server.properties value as the TOML type of `like`.
fn typed_value(text: &str, like: &toml::Value) -> Option<toml::Value> {
    Some(match like {
        toml::Value::Boolean(_) => toml::Value::Boolean(text.eq_ignore_ascii_case("true")),
        toml::Value::Integer(_) => toml::Value::Integer(text.trim().parse().ok()?),
        _ => toml::Value::String(text.to_owned()),
    })
}

/// A server.properties value for `[properties]`: a boolean or a number when it reads back the same.
pub fn loose_value(text: &str) -> toml::Value {
    match text {
        "true" => toml::Value::Boolean(true),
        "false" => toml::Value::Boolean(false),
        _ => match text.parse::<i64>() {
            Ok(n) if n.to_string() == text => toml::Value::Integer(n),
            _ => toml::Value::String(text.to_owned()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn properties(pairs: &[(&str, &str)]) -> Properties {
        let mut p = Properties::default();
        for (k, v) in pairs {
            p.put(k, (*v).to_owned());
        }
        p
    }

    #[test]
    fn template_is_the_default() {
        assert_eq!(Config::parse(TEMPLATE).unwrap(), Config::default());
        assert_eq!(render(&Config::default()), TEMPLATE);
    }

    #[test]
    fn anticheat_is_off_unless_its_section_turns_it_on() {
        assert!(!Config::default().anticheat.enabled);
        assert!(!TEMPLATE.lines().any(|l| l.trim() == "[anticheat]"));
        let text =
            format!("{TEMPLATE}\n[anticheat]\nenabled = true\n[anticheat.timer]\nkick = 0\n");
        let c = Config::parse(&text).unwrap();
        assert!(c.anticheat.enabled);
        assert_eq!(c.anticheat.timer.kick, 0.0);
        // The game reads the same section from the same file.
        assert_eq!(
            horizon_anticheat::Config::from_server_toml(&text).unwrap(),
            c.anticheat
        );
        assert_eq!(Config::parse(&render(&c)).unwrap(), c);
        assert!(Config::parse("[anticheat]\nenable = true\n").is_err());
        assert!(Config::parse("[anticheat.timer]\nalert = -1\n").is_err());
    }

    #[test]
    fn defaults_are_vanilla() {
        // Every typed default is what vanilla 26.3 writes for a missing key.
        let vanilla =
            ServerProperties::new(Properties::default(), &mut SystemEntropy::default()).unwrap();
        for (key, value) in Config::default().vanilla() {
            // Horizon's own (off by default), which vanilla doesn't have.
            if key.starts_with("horizon-") {
                assert_eq!(value, "0", "{key}");
                continue;
            }
            assert_eq!(vanilla.raw.get(&key), Some(value.as_str()), "{key}");
        }
    }

    #[test]
    fn import_keeps_what_differs() {
        let source = properties(&[
            ("motd", "Hello"),
            ("server-port", "25600"),
            ("white-list", "false"),
            ("gamemode", "1"),
            ("level-seed", "12345"),
            ("spawn-protection", "0"),
            ("allow-flight", "false"),
            ("rate-limit", "10"),
        ]);
        let c = Config::import(&source).unwrap();
        assert_eq!(c.server.motd, "Hello");
        assert_eq!(c.server.port, 25600);
        assert!(!c.server.whitelist);
        assert_eq!(c.world.game_mode, "creative");
        assert_eq!(c.world.seed, "12345");
        // allow-flight=false is vanilla's default: dropped.
        assert_eq!(
            c.properties,
            BTreeMap::from([
                ("rate-limit".to_owned(), toml::Value::Integer(10)),
                ("spawn-protection".to_owned(), toml::Value::Integer(0)),
            ])
        );
        let text = render(&c);
        assert!(text.contains("spawn-protection = 0"), "{text}");
        assert!(text.contains("port = 25600"), "{text}");
        assert_eq!(Config::parse(&text).unwrap(), c);
    }

    #[test]
    fn the_panel_stays_private() {
        let panel = |bind: &str, allow_public| Panel {
            bind: bind.into(),
            allow_public,
        };
        assert!(panel("127.0.0.1:8130", false).address().unwrap().is_some());
        assert!(panel("[::1]:8130", false).address().unwrap().is_some());
        assert!(
            panel("100.101.102.103:8130", false)
                .address()
                .unwrap()
                .is_some()
        );
        assert!(
            panel("[fd7a:115c:a1e0::1]:8130", false)
                .address()
                .unwrap()
                .is_some()
        );
        assert_eq!(panel("", false).address().unwrap(), None);
        assert!(panel("0.0.0.0:8130", false).address().is_err());
        assert!(panel("100.128.0.1:8130", false).address().is_err());
        assert!(panel("192.168.1.2:8130", false).address().is_err());
        assert!(panel("192.168.1.2:8130", true).address().unwrap().is_some());
        assert!(panel("localhost", false).address().is_err());
    }

    #[test]
    fn seed_may_be_a_number() {
        let c = Config::parse("[world]\nseed = -42\n").unwrap();
        assert_eq!(c.world.seed, "-42");
    }

    #[test]
    fn rejects_mistakes() {
        assert!(Config::parse("[world]\ngame_mode = \"creatve\"\n").is_err());
        assert!(Config::parse("[server]\nprot = 1\n").is_err());
        assert!(Config::parse("[properties]\nmotd = \"x\"\n").is_err());
        assert!(Config::parse("[world]\nname = \"../x\"\n").is_err());
    }

    #[test]
    fn writes_properties_and_syncs_back() {
        let dir =
            std::env::temp_dir().join(format!("horizon-server-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Config::default();
        c.properties
            .insert("spawn-protection".into(), toml::Value::Integer(0));
        std::fs::write(dir.join(FILE), render(&c)).unwrap();
        std::fs::write(dir.join(PROPERTIES), "rcon.port=25576\nmotd=old\n").unwrap();
        let written = c.write_properties(&dir).unwrap();
        let p = read_properties(&dir.join(PROPERTIES)).unwrap();
        assert_eq!(p.get("motd"), Some("A Minecraft Server"));
        assert_eq!(p.get("rcon.port"), Some("25576"), "other keys are kept");
        assert_eq!(p.get("spawn-protection"), Some("0"));
        // The game turns the whitelist off and changes spawn protection.
        let mut p = p;
        p.put("white-list", "false".into());
        p.put("spawn-protection", "4".into());
        std::fs::write(dir.join(PROPERTIES), p.store(None, "date", false)).unwrap();
        let mut changed = sync_back(&dir, &written).unwrap();
        changed.sort();
        assert_eq!(changed, ["spawn-protection", "white-list"]);
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(
            text.contains("whitelist = false             # only whitelisted"),
            "the comment stays: {text}"
        );
        let back = Config::parse(&text).unwrap();
        assert!(!back.server.whitelist);
        assert_eq!(back.properties["spawn-protection"], toml::Value::Integer(4));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
