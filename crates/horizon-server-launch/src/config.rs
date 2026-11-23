//! `server.toml`, the one file an owner edits (docs/server.md, section 4).
//!
//! It has typed sections for what owners change and `[properties]` for any other vanilla key. Before every
//! start the supervisor writes them into `server.properties`, which the game reads exactly as vanilla's
//! server does; what the game changes there itself (`/whitelist on`) is copied back afterwards.

use horizon_platform::java_properties::{Properties, date_string, now_millis};
use horizon_platform::server_files::biome_regions::BiomeRegions;
#[cfg(test)]
use horizon_platform::server_files::biome_regions::MissingRegions;
use horizon_platform::server_files::properties::{
    DIFFICULTIES, GAME_MODES, ServerProperties, SystemEntropy,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

pub const FILE: &str = "server.toml";

/// The `backups.keep` values `config set` and `server.toml` both accept.
pub const BACKUPS_KEEP: std::ops::RangeInclusive<i64> = 1..=1000;
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
max_viewed_loaded_chunks = 2048             # chunks a mod may keep loaded for one player in another level (0: none; at most 4489, higher values lowered with a warning; an older Horizon client gets at most 2048)
max_viewed_ticking_chunks = 441             # of them, ticking (one player's simulation area at distance 10)
max_viewed_loaded_chunks_per_level = 16384  # the same for all players together, per level
max_viewed_ticking_chunks_per_level = 3528  # ticking, all players together, per level
admin_feedback_to_ops = true # operators see each other's commands ([name: ...], vanilla); false: only the log
accept_older_clients = false # true: 26.2 clients join too, their packets translated (docs/hosting.md)

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
low_memory = false           # true: no transparent huge pages, ~60 MB less idle for ~1 % more tick time
parallel_tick = false        # true: the server tick shares its work among helper threads (docs/hosting.md)

[backups]
every = "6h"                 # automatic backups while the server runs: 30m, 6h, 1d...; "0" turns them off
keep = 12                    # automatic backups kept (backups made by hand are never deleted)

[updates]
keep = 1                     # other releases kept by updates, besides the one in use and the one rollback goes to

[panel]
bind = "127.0.0.1:8130"      # the web panel: this computer only, or a tailnet address (100.x); "" turns it off
allow_public = false         # true lets bind be any address; never put the panel on the open internet

[api]
bind = ""                    # the control API: "" a free port on 127.0.0.1; or a fixed one, like 127.0.0.1:8131 or a tailnet address
token = ""                   # its token (16 characters or more); "" a new random one at every start
allow_public = false         # true lets bind be any address; the token is then all that guards the server

# A proxy handing players over between servers (docs/server.md). Not vanilla's accepts-transfers (the transfer packet).
[transfer]
enabled = false              # true: take a proxy's player-* requests; needs Velocity forwarding (forwarding.secret)
expect_seconds = 10          # how long a player the proxy announced is waited for, when it names no time
max_expect_seconds = 60      # the longest the proxy may ask for
max_expected = 64            # players waited for at once
max_freeze_seconds = 30      # the longest a player stays frozen while it is handed over
max_hold_seconds = 60        # the longest a player arriving from another server is held before it is dropped
stop_wait_seconds = 10       # a stop with players on waits this long for the proxy to move them out (0 to 60; 0: no wait)

# Post the chat, joins, leaves and deaths to a Discord channel, drawn like the in-game chat (docs/hosting.md)
[discord]
webhook = ""                 # the channel's webhook URL (a secret: anyone with it can post there); "" off
events = ["chat", "join", "leave", "death"]   # what is posted; "kick" too if you like
enabled = true               # false pauses it and keeps the webhook

# Horizon's anticheat (docs/anticheat.md) is on: `combat` drops attacks no vanilla client sends and
# `ground` the false ground claims of a no-fall or a walk on water; both kick, and ban at the second kick.
# timer and prediction only write logs/anticheat.log until they're trusted. To change it:
# [anticheat]
# enabled = true             # false: the server checks only what vanilla's checks
# alerts = true              # tell online operators (alert_level = 2) in their chat; the console always is
# log = "logs/anticheat.log" # every flag, one line each
# [anticheat.timer]          # a sped-up game: more client ticks than the server's clock allows
# enabled = true             # false turns this check off
# setback = 2                # the violation level from which the player is sent back; 0: never
# alert = 5
# kick = 20
# decay = 0.02               # violation level lost each second
# ban_after = 0              # the kick (counting this check's earlier ones) that bans instead; 0: never

# The chat filter (docs/chat-filter.md) is on: a message, sign, anvil name or book title with a hate slur reaches
# nobody, the sender is told, and logs/chat-filter.log keeps it for operators. To change it:
# [chat_filter]
# enabled = true             # false: players' text goes out as they wrote it
# mute_after = 2             # the hit (within window_minutes) that mutes for mute_minutes; 0: never
# tempban_after = 3          # the hit that bans for tempban_hours; 0: never
# words_file = ""            # your own list, in the format of the built-in one; "": the built-in list

# Fights follow 26.3's rules. For 1.8.9's (no attack cooldown or sweep, 1.8 weapon damage; docs/pvp.md):
# [combat]
# legacy = true

# A world's biome regions from mods are fixed when it is made (docs/design/mod-api/mod-biome-regions.md).
# When a mod with one is gone, the server refuses to start until you choose. To change it:
# [biome_regions]
# missing = "vanilla"        # generate vanilla's biomes where the missing mods' regions were; "refuse": don't start
# adopt_installed = true     # new chunks use the installed mods' regions (the world keeps its saved weights)
# Villagers that cannot move (a trading hall's cells, or riding) skip their brain, which cuts the lag of big halls;
# they keep trading and restocking (docs/server.md). Not vanilla, so off. Mobs push every mob they overlap, as
# vanilla does; a crowded pen costs the square of its crowd, and max_collisions caps it (the most other mobs one
# mob pushes per tick, pushes it receives counted; 0 = no cap, vanilla). To turn them on:
# [entities]
# villager_lobotomize = true
# villager_lobotomize_check_interval = 100   # ticks between rechecks of whether a villager can move
# max_collisions = 8
"##;

/// Each typed setting: its section, its name, and the vanilla key it writes.
pub const TYPED: [(&str, &str, &str); 21] = [
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
    (
        "server",
        "max_viewed_loaded_chunks",
        "horizon-max-viewed-loaded-chunks",
    ),
    (
        "server",
        "max_viewed_ticking_chunks",
        "horizon-max-viewed-ticking-chunks",
    ),
    (
        "server",
        "max_viewed_loaded_chunks_per_level",
        "horizon-max-viewed-loaded-chunks-per-level",
    ),
    (
        "server",
        "max_viewed_ticking_chunks_per_level",
        "horizon-max-viewed-ticking-chunks-per-level",
    ),
    (
        "server",
        "admin_feedback_to_ops",
        "horizon-admin-feedback-to-ops",
    ),
    (
        "server",
        "accept_older_clients",
        "horizon-accept-older-clients",
    ),
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
    #[serde(default)]
    pub api: Api,
    /// A proxy's hand-overs (docs/server.md), passed to the game as `--transfers`.
    #[serde(default)]
    pub transfer: Transfer,
    /// Chat and players posted to a Discord channel (discord.rs).
    #[serde(default)]
    pub discord: Discord,
    /// The game reads this section itself (docs/anticheat.md); written only when it isn't the default.
    #[serde(default, skip_serializing_if = "anticheat_default")]
    pub anticheat: horizon_anticheat::Config,
    /// The game reads this section itself (docs/chat-filter.md); written only when it isn't the default.
    #[serde(default, skip_serializing_if = "chat_filter_default")]
    pub chat_filter: horizon_chat_filter::Config,
    /// The game reads this section itself (docs/hosting.md); written only when it allows a start.
    #[serde(default, skip_serializing_if = "MissingMods::is_default")]
    pub missing_mods: MissingMods,
    /// A one-time prune of barely visited chunks before a start (prune.rs); written only when set.
    #[serde(default, skip_serializing_if = "crate::prune::Settings::is_default")]
    pub prune_chunks: crate::prune::Settings,
    /// The game reads this section itself (docs/pvp.md); written only when it isn't the default.
    #[serde(default, skip_serializing_if = "Combat::is_default")]
    pub combat: Combat,
    /// The game reads this section itself (game/interface/dedicated/biome_regions.rs); written only when it
    /// isn't the default.
    #[serde(default, skip_serializing_if = "BiomeRegions::is_default")]
    pub biome_regions: BiomeRegions,
    /// `[entities]` (docs/server.md): mob settings passed to the game; written only when it isn't the default.
    #[serde(default, skip_serializing_if = "Entities::is_default")]
    pub entities: Entities,
    /// `[proxy]` (docs/hosting.md): the supervisor owns the public port and the game listens behind it;
    /// written only when it isn't the default.
    #[serde(default, skip_serializing_if = "Proxy::is_default")]
    pub proxy: Proxy,
}

/// `[proxy]`: the supervisor listens on `server.port` and signs players in itself (proxy/mod.rs), and the
/// game listens on 127.0.0.1 behind it, with Velocity forwarding. Off by default. Read when horizon-server
/// starts: turning it on or off needs a restart of horizon-server, not only of the game.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Proxy {
    pub enabled: bool,
    /// The port the game listens on, on 127.0.0.1; 0 takes a free one at every start of horizon-server.
    pub game_port: u16,
    /// How long a player is held through a restart before it is sent away if the game isn't back.
    pub hold_seconds: u32,
}

impl Default for Proxy {
    fn default() -> Self {
        Self {
            enabled: false,
            game_port: 0,
            hold_seconds: 60,
        }
    }
}

impl Proxy {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// `[entities]`: `villager_lobotomize = true` skips the brain of villagers that cannot move (stuck in a cell,
/// riding something), rechecked every `villager_lobotomize_check_interval` ticks. The game reads it from the
/// same file (game/dedicated.rs, `load_entities`).
/// `[entities]`: `dab = true` runs a mob's AI less often the farther it is from the nearest player (Pufferfish's
/// dynamic activation of brain; crates/horizon-client/src/mobs/dab.rs). Off by default: it is not vanilla's
/// behaviour. A mob farther than `dab_start_distance` blocks runs its AI every
/// `clamp(distance² >> dab_activation_dist_mod, 1, dab_max_tick_freq)` ticks. `max_collisions = N` limits how many
/// other mobs one mob pushes in a tick (pushes it receives count too); 0 is vanilla's behaviour, no cap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Entities {
    pub villager_lobotomize: bool,
    pub villager_lobotomize_check_interval: u32,
    pub dab: bool,
    pub dab_start_distance: u32,
    pub dab_max_tick_freq: u32,
    pub dab_activation_dist_mod: u32,
    pub max_collisions: u32,
}
impl Default for Entities {
    fn default() -> Self {
        Self {
            villager_lobotomize: false,
            // Must match `mobs::villager::lobotomy::DEFAULT_INTERVAL` in horizon-client (this crate cannot depend on it).
            villager_lobotomize_check_interval: 100,
            dab: false,
            dab_start_distance: 12,
            dab_max_tick_freq: 20,
            dab_activation_dist_mod: 8,
            max_collisions: 0,
        }
    }
}
impl Entities {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
    fn check(&self) -> Result<(), String> {
        if self.dab_max_tick_freq == 0 {
            return Err("entities.dab_max_tick_freq must be at least 1".into());
        }
        if self.dab_activation_dist_mod > 30 {
            return Err("entities.dab_activation_dist_mod must be 30 or less".into());
        }
        Ok(())
    }
}

/// `[combat]`: `legacy = true` resolves the players' melee hits by 1.8.9's rules (no attack cooldown or sweep,
/// 1.8.9 weapon damage). The game reads it from the same file (game/interface/dedicated/mod.rs, `load_combat`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Combat {
    pub legacy: bool,
}
impl Combat {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// `[missing_mods]`: `allow = true` opens a world saved with mods that are not loaded now instead of refusing
/// it (docs/hosting.md). The game reads it from the same file (game/interface/dedicated/missing_mods.rs), so the
/// supervisor only has to keep it; without this section it would refuse a file that has one as unknown.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MissingMods {
    pub allow: bool,
}
impl MissingMods {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The game reads this section itself too (docs/chat-filter.md).
fn chat_filter_default(c: &horizon_chat_filter::Config) -> bool {
    *c == horizon_chat_filter::Config::default()
}

fn anticheat_default(c: &horizon_anticheat::Config) -> bool {
    *c == horizon_anticheat::Config::default()
}

const EVENTS: [&str; 6] = ["chat", "join", "leave", "death", "kick", "announce"];

/// The `[discord]` table of server.toml.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Discord {
    pub enabled: bool,
    pub webhook: String,
    pub events: Vec<String>,
}

impl Default for Discord {
    fn default() -> Self {
        Self {
            enabled: true,
            webhook: String::new(),
            events: ["chat", "join", "leave", "death"].map(String::from).into(),
        }
    }
}

impl Discord {
    pub fn check(&self) -> Result<(), String> {
        let hook = self.webhook.trim();
        let discord = [
            "discord.com",
            "discordapp.com",
            "ptb.discord.com",
            "canary.discord.com",
        ];
        if !hook.is_empty()
            && !discord.iter().any(|host| {
                hook.strip_prefix("https://")
                    .and_then(|h| h.strip_prefix(host))
                    .is_some_and(|path| path.starts_with("/api/webhooks/"))
            })
        {
            return Err("discord.webhook must be a Discord webhook URL (https://discord.com/api/webhooks/...)".into());
        }
        match self.events.iter().find(|e| !EVENTS.contains(&e.as_str())) {
            Some(e) => Err(format!(
                "discord.events has \"{e}\": use {}",
                EVENTS.join(", ")
            )),
            None => Ok(()),
        }
    }
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
    /// Viewed levels (docs/hosting.md): the chunks mods may keep loaded for one player in one other
    /// level (at most 4489 columns; a higher value is lowered with a warning, and an older
    /// Horizon client gets at most 2048), the ticking ones among them, and the same for all
    /// players together in one level.
    pub max_viewed_loaded_chunks: u32,
    pub max_viewed_ticking_chunks: u32,
    pub max_viewed_loaded_chunks_per_level: u32,
    pub max_viewed_ticking_chunks_per_level: u32,
    /// Whether operators get each other's command feedback (`[name: ...]`, vanilla's broadcastToAdmins);
    /// false leaves it to the console's log, so nobody in game sees an operator's commands.
    pub admin_feedback_to_ops: bool,
    /// Whether 26.2 clients may join, their packets translated (horizon-protocol's `ClientSession`); off, they
    /// are turned away as vanilla turns away any other release.
    pub accept_older_clients: bool,
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
    /// The game without transparent huge pages (docs/hosting.md): less memory, slightly slower ticks.
    pub low_memory: bool,
    /// The game's parallel tick phases (`HORIZON_PARALLEL_TICK=1`), for hosts that cannot set the variable.
    pub parallel_tick: bool,
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
        listen_address("panel", &self.bind, self.allow_public, 8130)
    }
}

/// The control API's fixed address and token (docs/api/server.md); by default neither: a free port on
/// 127.0.0.1 and a random token, both written to `.horizon/api.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Api {
    pub bind: String,
    pub token: String,
    pub allow_public: bool,
}

impl Api {
    /// Where the API listens: `bind`, or a free port on 127.0.0.1.
    pub fn address(&self) -> Result<std::net::SocketAddr, String> {
        Ok(listen_address("api", &self.bind, self.allow_public, 8131)?
            .unwrap_or_else(|| (std::net::Ipv4Addr::LOCALHOST, 0).into()))
    }

    fn check(&self) -> Result<(), String> {
        self.address()?;
        let token = self.token.trim();
        if !token.is_empty() && (token.len() < 16 || token.chars().any(|c| !c.is_ascii_graphic())) {
            return Err(
                "api.token must be 16 characters or more, without spaces (or \"\": a random one)"
                    .into(),
            );
        }
        Ok(())
    }
}

/// A `bind` setting: `None` when empty; anything but this computer or the tailnet needs `allow_public`.
fn listen_address(
    what: &str,
    bind: &str,
    allow_public: bool,
    example_port: u16,
) -> Result<Option<std::net::SocketAddr>, String> {
    if bind.trim().is_empty() {
        return Ok(None);
    }
    let address: std::net::SocketAddr = bind.trim().parse().map_err(|_| {
        format!(
            "{what}.bind is \"{bind}\": use an address and a port, like 127.0.0.1:{example_port}"
        )
    })?;
    if !allow_public && !private(address.ip()) {
        return Err(format!(
            "{what}.bind is \"{bind}\": the {what} listens only on this computer (127.0.0.1) or a tailnet \
             address (100.64.0.0/10, fd7a:115c:a1e0::/48); reach it from elsewhere over Tailscale or SSH"
        ));
    }
    Ok(Some(address))
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

/// `[transfer]`: a proxy moving players between this server and others (docs/server.md, the game's
/// `lan/arrivals.rs`). Off by default. The supervisor passes it to the game as `--transfers`, not as a
/// server.properties key, which stays vanilla's; and it is not vanilla's `accepts-transfers`, which is about the
/// transfer packet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Transfer {
    pub enabled: bool,
    pub expect_seconds: u32,
    pub max_expect_seconds: u32,
    pub max_expected: u32,
    pub max_freeze_seconds: u32,
    pub max_hold_seconds: u32,
    pub stop_wait_seconds: u32,
}

impl Default for Transfer {
    fn default() -> Self {
        Self {
            enabled: false,
            expect_seconds: 10,
            max_expect_seconds: 60,
            max_expected: 64,
            max_freeze_seconds: 30,
            max_hold_seconds: 60,
            stop_wait_seconds: 10,
        }
    }
}

impl Transfer {
    fn check(&self) -> Result<(), String> {
        for (name, n) in [
            ("expect_seconds", self.expect_seconds),
            ("max_expect_seconds", self.max_expect_seconds),
            ("max_expected", self.max_expected),
            ("max_freeze_seconds", self.max_freeze_seconds),
            ("max_hold_seconds", self.max_hold_seconds),
        ] {
            if n == 0 {
                return Err(format!("transfer.{name} must be above zero"));
            }
        }
        if self.expect_seconds > self.max_expect_seconds {
            return Err("transfer.expect_seconds is above transfer.max_expect_seconds".into());
        }
        // The supervisor kills a game that has not stopped within STOP_TIMEOUT: the wait must leave
        // time for the saves (the game caps the wait with a freeze at 90 s).
        if self.stop_wait_seconds > 60 {
            return Err("transfer.stop_wait_seconds is above 60".into());
        }
        // The game cuts the wait at 90 s: past it, no hand-over could start during a stop.
        if self.stop_wait_seconds > 0
            && u64::from(self.stop_wait_seconds) + u64::from(self.max_freeze_seconds) > 90
        {
            return Err(
                "transfer.stop_wait_seconds and transfer.max_freeze_seconds add up to more than 90: no hand-over could start during a stop (lower one, or set stop_wait_seconds = 0)"
                    .into(),
            );
        }
        Ok(())
    }

    /// The game's `--transfers` value, when the section turns transfers on.
    pub fn engine_arg(&self) -> Option<String> {
        self.enabled.then(|| {
            format!(
                "expect_seconds={},max_expect_seconds={},max_expected={},max_freeze_seconds={},max_hold_seconds={},stop_wait_seconds={}",
                self.expect_seconds,
                self.max_expect_seconds,
                self.max_expected,
                self.max_freeze_seconds,
                self.max_hold_seconds,
                self.stop_wait_seconds
            )
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Backups {
    pub every: String,
    pub keep: u32,
}

impl Backups {
    /// The automatic backup period, or `None` when disabled.
    pub fn interval(&self) -> Result<Option<std::time::Duration>, String> {
        let every = self.every.trim();
        if every == "0" || every.is_empty() {
            return Ok(None);
        }
        let unit_at = every.char_indices().last().map_or(0, |(i, _)| i);
        let (number, unit) = every.split_at(unit_at);
        let n: u64 = number.trim().parse().map_err(|_| {
            format!("backups.every is {every:?}: use a number and m, h or d, like \"6h\"")
        })?;
        let secs = match unit {
            "m" => 60,
            "h" => 3600,
            "d" => 86_400,
            _ => {
                return Err(format!(
                    "backups.every is {every:?}: use a number and m, h or d, like \"6h\""
                ));
            }
        };
        let total = n
            .checked_mul(secs)
            .ok_or_else(|| format!("backups.every is {every:?}: too long"))?;
        Ok((n > 0).then(|| std::time::Duration::from_secs(total)))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Updates {
    /// Releases kept in `releases/` after an update besides the one in use and the one `rollback` goes
    /// back to (the supervisor's update pruning policy).
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
            max_viewed_loaded_chunks: 2048,
            max_viewed_ticking_chunks: 441,
            max_viewed_loaded_chunks_per_level: 2048 * 8,
            max_viewed_ticking_chunks_per_level: 441 * 8,
            admin_feedback_to_ops: true,
            accept_older_clients: false,
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
            low_memory: false,
            parallel_tick: false,
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

    pub(crate) fn check(&self) -> Result<(), String> {
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
        // Not the server folder itself, nor one of the supervisor's own folders in it.
        if w.name.is_empty()
            || w.name.contains(['/', '\\'])
            || matches!(
                w.name.as_str(),
                "." | ".." | "backups" | "logs" | ".horizon"
            )
        {
            return Err(format!(
                "world.name \"{}\" must be a folder name, without slashes",
                w.name
            ));
        }
        self.backups.interval()?;
        self.panel.address()?;
        self.api.check()?;
        self.transfer.check()?;
        if self.entities.villager_lobotomize_check_interval == 0 {
            return Err("entities.villager_lobotomize_check_interval must be above zero".into());
        }
        self.discord.check()?;
        self.anticheat.check()?;
        self.chat_filter.check()?;
        self.prune_chunks.check()?;
        self.entities.check()?;
        if !(1..=3600).contains(&self.proxy.hold_seconds) {
            return Err("proxy.hold_seconds must be from 1 to 3600".into());
        }
        if self.proxy.enabled && self.proxy.game_port == self.server.port {
            return Err("proxy.game_port is server.port: the game listens behind the proxy, on its own port".into());
        }
        // The game opens these as given, appending player text or rewriting them, so they stay
        // in the server folder: not absolute, not up a level, no drive (`C:x`) on Windows.
        for (what, path) in [
            ("chat_filter.log", &self.chat_filter.log),
            ("chat_filter.words_file", &self.chat_filter.words_file),
            ("anticheat.log", &self.anticheat.log),
            ("anticheat.strikes", &self.anticheat.strikes),
        ] {
            let p = Path::new(path);
            if p.is_absolute()
                || p.has_root()
                || path.contains(':')
                || path.split(['/', '\\']).any(|part| part == "..")
            {
                return Err(format!(
                    "{what} {path:?} must be a path inside the server folder"
                ));
            }
        }
        if self.server.port == 0 {
            return Err("server.port must be a port number from 1 to 65535".into());
        }
        // `config set` refuses a `backups.keep` outside this range (`BACKUPS_KEEP`); a file edited by
        // hand must not get past it with 0, which deletes every automatic backup right after it is made
        // (`backup::prune`), the new one included. `backups.every = "0"` is how automatic backups go off.
        let range = BACKUPS_KEEP;
        if !range.contains(&i64::from(self.backups.keep)) {
            return Err(format!(
                "backups.keep is {}: use {} to {} (to stop automatic backups, set backups.every = \"0\")",
                self.backups.keep,
                range.start(),
                range.end()
            ));
        }
        // The game reads these properties as Java ints; a larger number fails to parse and is silently
        // replaced by the default (20, 10). Smaller or larger values it clamps itself, so only these are refused.
        let server = &self.server;
        for (name, value) in [
            ("max_players", server.max_players),
            ("view_distance", server.view_distance),
            ("simulation_distance", server.simulation_distance),
            ("max_view_distance", server.max_view_distance),
        ] {
            if i32::try_from(value).is_err() {
                return Err(format!(
                    "server.{name} is {value}: use at most {}",
                    i32::MAX
                ));
            }
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
        // Behind the proxy the game listens where only it goes (`game_port` is set once the proxy runs).
        if self.proxy.enabled && self.proxy.game_port != 0 {
            for (key, value) in [
                ("server-ip".to_owned(), "127.0.0.1".to_owned()),
                ("server-port".to_owned(), self.proxy.game_port.to_string()),
                ("online-mode".to_owned(), "false".to_owned()),
            ] {
                match out.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1 = value,
                    None => out.push((key, value)),
                }
            }
        }
        out
    }

    /// Writes this configuration into `server.properties`, keeping the keys it doesn't set. Returns what it
    /// wrote, for [`sync_back`]. A file (or link) that exists but can't be read or parsed is left as it is
    /// and is an error: writing over it would lose the owner's other keys. (The game refuses an unparsable
    /// file too.)
    pub fn write_properties(&self, dir: &Path) -> std::io::Result<Vec<(String, String)>> {
        let path = dir.join(PROPERTIES);
        let mut properties = match std::fs::read(&path) {
            Ok(bytes) => load_properties(&path, &bytes).map_err(|why| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{why} (left as it is)"),
                )
            })?,
            // Only a file that isn't there at all: a link to a volume that is away reads as missing too,
            // and the rename would replace the link.
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound
                    && std::fs::symlink_metadata(&path)
                        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                Properties::default()
            }
            Err(e) => {
                let why = if e.kind() == std::io::ErrorKind::NotFound {
                    "it links to a file that isn't there".to_owned()
                } else {
                    e.to_string()
                };
                return Err(std::io::Error::new(
                    e.kind(),
                    format!("{}: {why} (left as it is)", path.display()),
                ));
            }
        };
        let written = self.vanilla();
        for (key, value) in &written {
            properties.put(key, value.clone());
        }
        let text = properties.store(
            Some("Minecraft server properties"),
            &date_string(now_millis()),
            false,
        );
        replace(&path, &text)?;
        Ok(written)
    }

    /// The level `op` gives a new operator: `op-permission-level` as the game reads it at its next start,
    /// that is `server.properties` with this configuration's keys written over it (as
    /// [`Config::write_properties`] does). 4, vanilla's default, when the keys cannot be read.
    pub fn op_permission_level(&self, dir: &Path) -> i32 {
        let mut properties = read_properties(&dir.join(PROPERTIES)).unwrap_or_default();
        for (key, value) in self.vanilla() {
            properties.put(&key, value);
        }
        ServerProperties::new(properties, &mut SystemEntropy::default())
            .map_or(4, |p| p.op_permission_level)
    }

    /// A key as the game reads it at its next start: `server.properties` with this configuration's keys
    /// written over it (as [`Config::write_properties`] does), so a key only the file sets counts too.
    pub fn effective_property(&self, dir: &Path, key: &str) -> Option<String> {
        let mut properties = read_properties(&dir.join(PROPERTIES)).unwrap_or_default();
        for (k, value) in self.vanilla() {
            properties.put(&k, value);
        }
        properties.get(key).map(str::to_owned)
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
        ("engine", "low_memory"),
        ("engine", "parallel_tick"),
        ("backups", "every"),
        ("backups", "keep"),
        ("updates", "keep"),
        ("panel", "bind"),
        ("panel", "allow_public"),
        ("api", "bind"),
        ("api", "token"),
        ("api", "allow_public"),
        ("transfer", "enabled"),
        ("transfer", "expect_seconds"),
        ("transfer", "max_expect_seconds"),
        ("transfer", "max_expected"),
        ("transfer", "max_freeze_seconds"),
        ("transfer", "max_hold_seconds"),
        ("transfer", "stop_wait_seconds"),
    ] {
        set(&mut doc, section, field, &table[section][field]);
    }
    let mut text = doc.to_string();
    for section in [
        "anticheat",
        "chat_filter",
        "missing_mods",
        "prune_chunks",
        "combat",
        "biome_regions",
        "entities",
        "proxy",
    ] {
        if let Some(settings) = table.get(section) {
            let mut only = toml::Table::new();
            only.insert(section.into(), settings.clone());
            text.push('\n');
            text.push_str(&toml::to_string(&only).expect("a section's settings are a table"));
        }
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
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
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
    replace(&path, &updated).map_err(|e| e.to_string())?;
    Ok(changed.iter().map(|(k, _)| (*k).to_owned()).collect())
}

/// Held while `server.toml` is read, changed and written back, so two of this process's writers
/// (management clients, the API, the supervisor's [`sync_back`]) don't drop each other's change.
static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Writes `text` to `path` through a new file renamed over it: a reader (or a crash) sees the old
/// file or the new one, never an empty or half-written one. An empty `server.toml` reads as every
/// default, and the next start would write those into `server.properties`. Another process (the
/// CLI's `config set`) has its own temporary file.
pub(crate) fn replace(path: &Path, text: &str) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .map_or_else(Default::default, |n| n.to_string_lossy());
    let tmp = path.with_file_name(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut file, text.as_bytes())?;
        file.sync_all()?;
        if let Ok(meta) = std::fs::metadata(path) {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Sets one typed setting in `server.toml` (`whitelist on`), keeping the owner's comments.
pub fn update(dir: &Path, section: &str, field: &str, value: toml::Value) -> Result<(), String> {
    update_all(dir, &[(section.to_owned(), field.to_owned(), value)])
}

/// Sets settings in `server.toml`, keeping the owner's comments; nothing is written unless the result is
/// a valid configuration.
pub fn update_all(dir: &Path, changes: &[(String, String, toml::Value)]) -> Result<(), String> {
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir.join(FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    for (section, field, value) in changes {
        set(&mut doc, section, field, value);
    }
    let updated = doc.to_string();
    Config::parse(&updated)?;
    replace(&path, &updated).map_err(|e| e.to_string())
}

pub fn read_properties(path: &Path) -> Result<Properties, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    load_properties(path, &bytes)
}

/// The properties file at `path`, read as `bytes`; when they don't load, an error naming the file and the
/// line of the entry that breaks it.
fn load_properties(path: &Path, bytes: &[u8]) -> Result<Properties, String> {
    Properties::load_file_locating(bytes)
        .map_err(|(e, line)| format!("{}, line {line}: {e}", path.display()))
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

    /// `server.toml` is checked as `config set` checks `backups.keep`: 0 would make the supervisor delete
    /// every automatic backup (the one it just made too), so it is refused with the same range.
    #[test]
    fn backups_keep_is_checked_as_config_set_checks_it() {
        let with = |keep: u32| {
            Config::parse(&format!(
                "[backups]
keep = {keep}
"
            ))
        };
        let err = with(0).unwrap_err();
        assert!(
            err.contains("backups.keep is 0: use 1 to 1000"),
            "got {err:?}"
        );
        assert!(with(1001).is_err());
        assert!(with(1).is_ok() && with(1000).is_ok());
        assert_eq!(Config::default().backups.keep, 12);
    }

    /// The game parses these as Java ints and replaces what does not fit with its default, unannounced.
    #[test]
    fn server_numbers_the_game_cannot_read_are_refused() {
        let with = |key: &str, n: u32| Config::parse(&format!("[server]\n{key} = {n}\n"));
        for key in [
            "max_players",
            "view_distance",
            "simulation_distance",
            "max_view_distance",
        ] {
            let err = with(key, 3_000_000_000).unwrap_err();
            assert!(
                err.contains(&format!("server.{key} is 3000000000")),
                "{err}"
            );
            assert!(with(key, i32::MAX as u32).is_ok());
        }
        // The game clamps these itself, so a file that worked keeps working.
        assert!(with("view_distance", 64).is_ok() && with("max_players", 5000).is_ok());
    }

    #[test]
    fn intervals() {
        let interval = |every: &str| {
            Backups {
                every: every.into(),
                ..Backups::default()
            }
            .interval()
        };
        assert_eq!(
            interval("6h").unwrap(),
            Some(std::time::Duration::from_secs(21_600))
        );
        assert_eq!(
            interval("30m").unwrap(),
            Some(std::time::Duration::from_secs(1800))
        );
        assert_eq!(interval("0").unwrap(), None);
        assert!(interval("6 hours").is_err());
        assert!(interval("6é").is_err());
        assert!(interval("é").is_err());
        assert!(interval(&format!("{}d", u64::MAX)).is_err());
    }

    #[test]
    fn template_is_the_default() {
        assert_eq!(Config::parse(TEMPLATE).unwrap(), Config::default());
        assert_eq!(render(&Config::default()), TEMPLATE);
    }

    /// docs/hosting.md shows `server.toml` with its defaults in the `toml` block that starts with `[server]`:
    /// that block loads, and loads as the defaults.
    #[test]
    fn the_hosting_guides_server_toml_block_is_the_default() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("docs/hosting.md");
        let docs = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
        let block = docs
            .split("```toml\n[server]\n")
            .nth(1)
            .and_then(|rest| rest.split("```").next())
            .expect("hosting.md has a toml block that starts with [server]");
        let shown = Config::parse(&format!("[server]\n{block}")).unwrap();
        assert_eq!(shown, Config::default());
    }

    /// `[transfer]` is off by default, passes nothing to the game then, and is checked when it is read.
    #[test]
    fn transfers_are_off_by_default_and_checked() {
        assert_eq!(Config::default().transfer.engine_arg(), None);
        let on = TEMPLATE.replace(
            "enabled = false              # true: take",
            "enabled = true               # true: take",
        );
        let config = Config::parse(&on).unwrap();
        assert_eq!(
            config.transfer.engine_arg().as_deref(),
            Some(
                "expect_seconds=10,max_expect_seconds=60,max_expected=64,max_freeze_seconds=30,max_hold_seconds=60,stop_wait_seconds=10"
            )
        );
        assert_eq!(Config::parse(&render(&config)).unwrap(), config);
        for (from, to) in [
            ("expect_seconds = 10 ", "expect_seconds = 0  "),
            ("max_expect_seconds = 60", "max_expect_seconds = 5 "),
            ("max_freeze_seconds = 30", "max_freeze_seconds = 0 "),
            ("stop_wait_seconds = 10", "stop_wait_seconds = 61"),
            ("max_freeze_seconds = 30", "max_freeze_seconds = 81"),
        ] {
            let bad = on.replace(from, to);
            assert_ne!(bad, on, "{from} is in the template");
            assert!(Config::parse(&bad).is_err(), "{to} was taken");
        }
    }

    #[test]
    fn the_chat_filter_is_on_unless_its_section_turns_it_off() {
        assert!(Config::default().chat_filter.enabled);
        assert!(!TEMPLATE.lines().any(|l| l.trim() == "[chat_filter]"));
        let text =
            format!("{TEMPLATE}\n[chat_filter]\nmute_after = 3\nwords_file = \"slurs.txt\"\n");
        let c = Config::parse(&text).unwrap();
        assert_eq!(c.chat_filter.mute_after, 3);
        assert_eq!(Config::parse(&render(&c)).unwrap(), c);
        // The game reads the same section from the same file.
        assert_eq!(
            horizon_chat_filter::Config::from_server_toml(&text).unwrap(),
            c.chat_filter
        );
        let off = Config::parse(&format!("{TEMPLATE}\n[chat_filter]\nenabled = false\n")).unwrap();
        assert!(!off.chat_filter.enabled);
        assert_eq!(Config::parse(&render(&off)).unwrap(), off);
        assert!(Config::parse("[chat_filter]\nmute = 2\n").is_err());
    }

    #[test]
    fn anticheat_is_on_unless_its_section_turns_it_off() {
        assert!(Config::default().anticheat.enabled);
        assert!(!TEMPLATE.lines().any(|l| l.trim() == "[anticheat]"));
        let text =
            format!("{TEMPLATE}\n[anticheat]\nenabled = true\n[anticheat.timer]\nkick = 20\n");
        let c = Config::parse(&text).unwrap();
        assert!(c.anticheat.enabled);
        assert_eq!(c.anticheat.timer.kick, 20.0);
        let off = Config::parse(&format!("{TEMPLATE}\n[anticheat]\nenabled = false\n")).unwrap();
        assert!(!off.anticheat.enabled);
        assert_eq!(Config::parse(&render(&off)).unwrap(), off);
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
    fn the_collision_cap_is_off_unless_its_section_sets_it() {
        assert_eq!(Config::default().entities.max_collisions, 0);
        assert!(!TEMPLATE.lines().any(|l| l.trim() == "[entities]"));
        let on = Config::parse(&format!("{TEMPLATE}\n[entities]\nmax_collisions = 8\n")).unwrap();
        assert_eq!(on.entities.max_collisions, 8);
        assert_eq!(Config::parse(&render(&on)).unwrap(), on);
        let off = render(&Config::default());
        assert!(!off.lines().any(|l| l.trim() == "[entities]"), "{off}");
        assert!(Config::parse("[entities]\nmax_collisions = -1\n").is_err());
        assert!(Config::parse("[entities]\nmax_collision = 8\n").is_err());
    }

    #[test]
    fn dab_is_off_unless_its_section_turns_it_on() {
        assert!(!Config::default().entities.dab);
        let off = render(&Config::default());
        assert!(!off.lines().any(|l| l.trim() == "[entities]"), "{off}");
        let on = Config::parse("[entities]\ndab = true\ndab_max_tick_freq = 10\n").unwrap();
        assert!(on.entities.dab);
        assert_eq!(on.entities.dab_start_distance, 12);
        assert_eq!(on.entities.dab_activation_dist_mod, 8);
        // Kept when the file is written again.
        assert_eq!(Config::parse(&render(&on)).unwrap(), on);
        assert!(Config::parse("[entities]\ndab_max_tick_freq = 0\n").is_err());
        assert!(Config::parse("[entities]\ndab_activation_dist_mod = 99\n").is_err());
        assert!(Config::parse("[entities]\ndap = true\n").is_err());
    }

    #[test]
    fn legacy_combat_is_off_unless_its_section_turns_it_on() {
        assert!(!Config::default().combat.legacy);
        assert!(!TEMPLATE.lines().any(|l| l.trim() == "[combat]"));
        let on = Config::parse(&format!("{TEMPLATE}\n[combat]\nlegacy = true\n")).unwrap();
        assert!(on.combat.legacy);
        // Kept when the file is written again, and left out while off.
        assert_eq!(Config::parse(&render(&on)).unwrap(), on);
        let off = render(&Config::default());
        assert!(!off.lines().any(|l| l.trim() == "[combat]"), "{off}");
        assert!(Config::parse("[combat]\nlegacy = 1\n").is_err());
        assert!(Config::parse("[combat]\nlegcy = true\n").is_err());
    }

    #[test]
    fn a_world_opens_without_its_mods_only_when_the_section_says_so() {
        // docs/hosting.md: the switch that lets a server open a world whose mods are gone, as the game
        // reads it. The supervisor keeps the section rather than refusing the file as unknown.
        assert!(!Config::default().missing_mods.allow);
        let set = Config::parse(&format!("{TEMPLATE}\n[missing_mods]\nallow = true\n")).unwrap();
        assert!(set.missing_mods.allow);
        // Kept when the file is written again, and left out while default.
        assert_eq!(Config::parse(&render(&set)).unwrap(), set);
        assert_eq!(
            Config::parse(&format!("{TEMPLATE}\n[missing_mods]\nallow = false\n"))
                .unwrap()
                .missing_mods,
            Config::default().missing_mods
        );
        let off = render(&Config::default());
        assert!(!off.lines().any(|l| l.trim() == "[missing_mods]"), "{off}");
        assert!(Config::parse("[missing_mods]\nalow = true\n").is_err());
    }

    #[test]
    fn biome_regions_refuse_missing_mods_unless_their_section_says() {
        assert_eq!(
            Config::default().biome_regions.missing,
            MissingRegions::Refuse
        );
        assert!(!Config::default().biome_regions.adopt_installed);
        let set = Config::parse(&format!(
            "{TEMPLATE}\n[biome_regions]\nmissing = \"vanilla\"\nadopt_installed = true\n"
        ))
        .unwrap();
        assert_eq!(set.biome_regions.missing, MissingRegions::Vanilla);
        assert!(set.biome_regions.adopt_installed);
        // Kept when the file is written again, and left out while default.
        assert_eq!(Config::parse(&render(&set)).unwrap(), set);
        let off = render(&Config::default());
        assert!(!off.lines().any(|l| l.trim() == "[biome_regions]"), "{off}");
        assert!(Config::parse("[biome_regions]\nmissing = \"skip\"\n").is_err());
    }

    #[test]
    fn villager_lobotomize_is_off_unless_its_section_turns_it_on() {
        let off = Config::default().entities;
        assert!(!off.villager_lobotomize);
        assert_eq!(off.villager_lobotomize_check_interval, 100);
        assert!(!TEMPLATE.lines().any(|l| l.trim() == "[entities]"));
        let on = Config::parse(&format!(
            "{TEMPLATE}\n[entities]\nvillager_lobotomize = true\nvillager_lobotomize_check_interval = 40\n"
        ))
        .unwrap();
        assert!(on.entities.villager_lobotomize);
        assert_eq!(on.entities.villager_lobotomize_check_interval, 40);
        // Kept when the file is written again, and left out while off.
        assert_eq!(Config::parse(&render(&on)).unwrap(), on);
        let text = render(&Config::default());
        assert!(!text.lines().any(|l| l.trim() == "[entities]"), "{text}");
        assert!(Config::parse("[entities]\nvillager_lobotomize = 1\n").is_err());
        assert!(Config::parse("[entities]\nvillager_lobotomise = true\n").is_err());
        assert!(Config::parse("[entities]\nvillager_lobotomize_check_interval = 0\n").is_err());
    }

    #[test]
    fn defaults_are_vanilla() {
        // Every typed default is what vanilla 26.3 writes for a missing key.
        let vanilla =
            ServerProperties::new(Properties::default(), &mut SystemEntropy::default()).unwrap();
        for (key, value) in Config::default().vanilla() {
            // Horizon's own, which vanilla doesn't have: their defaults behave as vanilla does
            // (the far ring off, operators told of each other's commands, other releases turned away,
            // and caps on viewed areas, which only a mod asks for).
            if key.starts_with("horizon-") {
                let vanilla_like = match key.as_str() {
                    "horizon-admin-feedback-to-ops" => "true",
                    "horizon-accept-older-clients" => "false",
                    "horizon-max-viewed-loaded-chunks" => "2048",
                    "horizon-max-viewed-ticking-chunks" => "441",
                    "horizon-max-viewed-loaded-chunks-per-level" => "16384",
                    "horizon-max-viewed-ticking-chunks-per-level" => "3528",
                    _ => "0",
                };
                assert_eq!(value, vanilla_like, "{key}");
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
        for folder in [".", "..", "backups", "logs", ".horizon"] {
            let text = format!("[world]\nname = \"{folder}\"\n");
            assert!(Config::parse(&text).is_err(), "{folder}");
        }
    }

    #[test]
    fn settings_changed_at_once_are_all_kept() {
        let dir =
            std::env::temp_dir().join(format!("horizon-server-at-once-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), TEMPLATE).unwrap();
        // Eight writers at once, as eight management clients each setting another key.
        std::thread::scope(|scope| {
            for n in 0..8i64 {
                let dir = &dir;
                scope.spawn(move || {
                    update(
                        dir,
                        "properties",
                        &format!("horizon-test-{n}"),
                        toml::Value::Integer(n),
                    )
                    .unwrap()
                });
            }
        });
        let c = Config::load(&dir).unwrap();
        for n in 0..8i64 {
            assert_eq!(
                c.properties.get(&format!("horizon-test-{n}")),
                Some(&toml::Value::Integer(n)),
                "horizon-test-{n} kept"
            );
        }
        // No temporary file is left beside it.
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from(FILE)]);
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn the_proxy_is_off_by_default_and_moves_the_game_behind_it_when_on() {
        let config = Config::default();
        assert!(!config.proxy.enabled);
        let key = |c: &Config, k: &str| c.vanilla().into_iter().find(|(key, _)| key == k);
        // Off, the game keeps the owner's port and online mode.
        assert_eq!(key(&config, "server-port").unwrap().1, "25565");
        assert_eq!(key(&config, "online-mode").unwrap().1, "true");

        let on = Config::parse("[proxy]\nenabled = true\ngame_port = 25999\n").unwrap();
        assert!(on.proxy.enabled);
        assert_eq!(key(&on, "server-port").unwrap().1, "25999");
        assert_eq!(key(&on, "server-ip").unwrap().1, "127.0.0.1");
        // The proxy signs players in: the game trusts what it forwards.
        assert_eq!(key(&on, "online-mode").unwrap().1, "false");
        // The owner's own port is where the proxy listens, and the file keeps it.
        assert_eq!(on.server.port, 25565);
        assert!(render(&on).contains("[proxy]"));
        assert!(!render(&config).contains("[proxy]"));
    }

    #[test]
    fn the_games_port_behind_the_proxy_is_not_the_public_one() {
        let err = Config::parse("[proxy]\nenabled = true\ngame_port = 25565\n").unwrap_err();
        assert!(err.contains("proxy.game_port"), "{err}");
        assert!(Config::parse("[proxy]\nunknown = 1\n").is_err());
    }
    /// One bad escape in an owner's server.properties: the start refuses with the file and line instead of
    /// writing server.toml's keys over a file that lost every other key (the game refuses the file too).
    #[test]
    fn a_malformed_server_properties_is_left_as_it_is() {
        let dir = std::env::temp_dir().join(format!(
            "horizon-server-config-malformed-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let owner = "#Minecraft server properties\nrcon.port=25576\nlevel-name=castle\nmotd=caf\\uZZZZ\nview-distance=16\n";
        std::fs::write(dir.join(PROPERTIES), owner).unwrap();
        let e = Config::default().write_properties(&dir).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
        let e = e.to_string();
        assert!(e.contains(PROPERTIES) && e.contains("line 4"), "{e}");
        assert_eq!(
            std::fs::read_to_string(dir.join(PROPERTIES)).unwrap(),
            owner
        );
        // The line is counted as the properties reader counts lines.
        for (text, line) in [
            ("motd=ok\rbad=\\uZZZZ\r", 2),
            ("motd=ok\r\nbad=\\uZZZZ\r\n", 2),
            ("motd=\\u12\\\n34\nbad=\\uZZZZ\n", 3),
        ] {
            std::fs::write(dir.join(PROPERTIES), text).unwrap();
            let e = Config::default().write_properties(&dir).unwrap_err();
            assert!(
                e.to_string().contains(&format!("line {line}:")),
                "{text:?}: {e}"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A server.properties that exists but can't be read is not replaced either; only a missing one is
    /// written from server.toml alone.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_server_properties_is_left_as_it_is() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "horizon-server-config-unreadable-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(PROPERTIES);
        let owner = "rcon.port=25576\nlevel-name=castle\n";
        std::fs::write(&path, owner).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads it anyway: nothing to check there.
        if std::fs::read(&path).is_err() {
            let e = Config::default().write_properties(&dir).unwrap_err();
            assert!(e.to_string().contains(PROPERTIES), "{e}");
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), owner);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A server.properties linked to a volume that isn't mounted reads as missing, but it is the owner's
    /// file: the link is kept, not replaced by a file of server.toml's keys.
    #[cfg(unix)]
    #[test]
    fn a_server_properties_link_to_a_missing_file_is_left_as_it_is() {
        let dir = std::env::temp_dir().join(format!(
            "horizon-server-config-dangling-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(PROPERTIES);
        let target = dir.join("away").join(PROPERTIES);
        std::os::unix::fs::symlink(&target, &path).unwrap();
        let e = Config::default().write_properties(&dir).unwrap_err();
        assert!(e.to_string().contains(PROPERTIES), "{e}");
        assert_eq!(std::fs::read_link(&path).unwrap(), target);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A key only `server.properties` sets reaches whoever asks as the game would read it, and server.toml
    /// wins where both set it.
    #[test]
    fn an_effective_property_includes_keys_only_the_file_sets() {
        let dir =
            std::env::temp_dir().join(format!("horizon-server-effective-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(PROPERTIES),
            "enable-status=false\nnetwork-compression-threshold=64\n",
        )
        .unwrap();
        let config = Config::parse("[properties]\nnetwork-compression-threshold = 512\n").unwrap();
        assert_eq!(
            config.effective_property(&dir, "enable-status").as_deref(),
            Some("false")
        );
        assert_eq!(
            config
                .effective_property(&dir, "network-compression-threshold")
                .as_deref(),
            Some("512")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn written_files_stay_in_the_server_folder() {
        for bad in [
            "/home/mc/.bashrc",
            "../outside.log",
            "logs/../../x",
            "logs\\..\\..\\x",
            "C:x.log",
            "C:\\Users\\mc\\x.log",
        ] {
            for set in [
                |c: &mut Config, p: &str| c.chat_filter.log = p.into(),
                |c: &mut Config, p: &str| c.chat_filter.words_file = p.into(),
                |c: &mut Config, p: &str| c.anticheat.log = p.into(),
                |c: &mut Config, p: &str| c.anticheat.strikes = p.into(),
            ] {
                let mut c = Config::default();
                set(&mut c, bad);
                let e = c.check().unwrap_err();
                assert!(e.contains("inside the server folder"), "{bad:?}: {e}");
            }
        }
        let mut c = Config::default();
        c.chat_filter.log = "logs/chat-filter.log".into();
        c.anticheat.strikes = String::new();
        c.check().unwrap();
    }
}
