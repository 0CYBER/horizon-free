//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
use std::path::PathBuf;
/// The host's methods both sides share of the `config` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait ConfigHost {
    /// Physical process, not the logical side of this hook.
    #[access(Metadata)]
    fn dist(&self) -> Dist;
    /// Exact Minecraft version supported by this process.
    #[access(Metadata)]
    fn game_version(&self) -> &'static str;
    /// The engine's `DataVersion` written into level data and chunks.
    /// Constant on both sides, including before a world is open; this describes this
    /// game build, not the version of a remote server or an older save.
    #[access(Metadata)]
    fn server_data_version(&self) -> i32;
    /// Horizon version printed in crash reports.
    #[access(Metadata)]
    fn horizon_version(&self) -> &'static str;
    /// Side: both.
    /// Whether the player left mod `mod_id` on in Options > Mods. A mod
    /// turned off does nothing (check this first in each hook) until the
    /// launcher's next build leaves it out.
    #[access(Metadata)]
    fn on(&self, mod_id: &str) -> bool;
    /// Side: both.
    /// A mod's setting `key` (one file per key in `native_data_dir`). A mod reads only its own:
    /// another mod's id or a key not registered by that owner gives None.
    #[access(WorldOnly)]
    fn setting(&self, mod_id: &str, key: &str) -> Option<String>;
    /// Side: both.
    /// Keeps a mod's registered setting `key`. Another mod's id or an unregistered key keeps
    /// nothing. A successful write invalidates the typed read cache for this data folder and key;
    /// a failed write leaves it unchanged. This raw write does not fire `setting_changed`.
    #[access(WorldOnly)]
    fn set_setting(&self, mod_id: &str, key: &str, value: &str);
    /// Side: both.
    /// The calling mod's own files in the game's `config/` folder, beside
    /// `options.txt` (beside `server.properties` on a dedicated server):
    /// its folder `config/<mod id>/` and the files beside it named
    /// `<mod id>.<ext>` or `<mod id>-<rest>`, as Fabric and NeoForge mods
    /// keep theirs. The service is bound to the mod whose hook or part is
    /// running, so a mod never names its id and can't reach another mod's
    /// files; a world's `serverconfig/` is read only through
    /// `config_loading`.
    #[access(WorldOnly)]
    #[static_call]
    fn config_files(&self) -> &'static dyn ConfigFiles;
    /// Side: both. The calling mod's bulk files in
    /// `horizon-mod-data/<mod id>/` in the game folder, never scanned as
    /// settings or config. Paths omit the mod id; `ConfigPath::root()` names
    /// its root. Horizon allows at most `DATA_FILE_MAX` bytes per file.
    #[access(WorldOnly)]
    #[static_call]
    fn data_files(&self) -> &'static dyn ConfigFiles;
    /// Side: both.
    /// The level in place (`minecraft:overworld`), for a hook that isn't told
    /// which it runs in.
    #[access(WorldOnly)]
    fn dimension(&self) -> &'static str;
    /// Side: both.
    /// Whether the server's current tick still has time before the next one is
    /// due (`MinecraftServer.haveTime`, NeoForge's `ServerTickEvent.hasTime` and
    /// `LevelTickEvent.hasTime`): a mod doing optional work in `server_tick` or
    /// `level_tick_pre`/`level_tick_post` skips it when this is false. Always
    /// true on a client, and outside a scheduled tick.
    #[access(WorldOnly)]
    fn server_has_time(&self) -> bool;
    /// Side: both.
    /// The instance (game) folder, which its players share: `saves/`,
    /// `resourcepacks/`, `shaderpacks/` and `horizon-mods/` are in it, and the
    /// title's screens create worlds and read packs there. A world's own
    /// `datapacks/` is in that world's folder (`saves/<world>/datapacks`).
    /// The folder the game runs in (`--player-dir` moves a player's options
    /// and servers only, not the instance). Nothing is created.
    ///
    /// Not on wasm: a path in the game's file system, which a wasm mod has none of; a wasm mod
    /// reads its own data and assets through reload listeners.
    #[access(WorldOnly)]
    fn instance_dir(&self) -> PathBuf;
}
/// The server host's methods of the `config` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait ConfigServerHost {
    /// Startup opt-in for test-only content. Test registration runs regardless.
    #[access(Metadata)]
    fn game_tests_enabled(&self) -> bool;
    /// Side: server.
    /// The entity broadcast range the server hosts with, in percent of vanilla's range
    /// (`entity-broadcast-range-percentage` of server.properties, `/serversettings`). 100 when
    /// no server is hosting a world, as vanilla's default is.
    #[access(WorldOnly)]
    fn entity_range_percent(&self) -> i32;
    /// Side: server.
    /// The server's tick rate (20 unless `/tick rate` changed it) and the
    /// milliseconds its ticks took on average lately, as `/tick query`
    /// reports them. The ticks a second it keeps up is the rate, or
    /// 1000 / milliseconds when that is lower.
    #[access(WorldOnly)]
    fn tick_stats(&self) -> (f32, f64);
    /// Side: server.
    /// The last 100 server ticks' durations in nanoseconds, as
    /// `MinecraftServer.getTickTimesNanos` lays them out: the tick numbered `n` at index
    /// `n % 100`, 0 where none was recorded yet. Sort a copy for percentiles; `tick_stats`
    /// has the average.
    #[access(WorldOnly)]
    fn tick_times(&self) -> Vec<u64>;
    /// Side: server.
    /// Times every mob's own tick (its riders inside it) from now on, or no longer, summed by
    /// level and mob type for `take_mob_costs`, as a profiler samples them: while on, two
    /// clock reads and a map update a mob; off, one thread-local read. Apart from the server's
    /// own performance report, so both can run. Off drops what was sampled. False on a client.
    #[access(WorldOnly)]
    fn mob_cost_sampling(&mut self, on: bool) -> bool;
    /// Side: server.
    /// The mob tick costs sampled since the last call (`mob_cost_sampling`), costliest first;
    /// clears them. Empty while nothing was sampled and on a client.
    #[access(WorldOnly)]
    fn take_mob_costs(&mut self) -> Vec<MobTickCost>;
    /// Side: server.
    /// Counts every player's play packets by id and direction from now on, or no longer, for
    /// `player_take_packet_counts`. While on, each packet costs its connection a lock and a
    /// map update; off, one atomic read. One switch for the whole server, shared by every
    /// mod. False on a client.
    #[access(WorldOnly)]
    fn count_packets(&mut self, on: bool) -> bool;
    /// Side: server.
    /// The server's view distance in chunks (`PlayerList.getViewDistance`, `view-distance`):
    /// how far players are sent chunks at most. None off a server open to players.
    #[access(WorldOnly)]
    fn server_view_distance(&self) -> Option<i32>;
    /// Side: server.
    /// The server's simulation distance in chunks (`PlayerList.getSimulationDistance`,
    /// `simulation-distance`): how far around each player chunks tick. None off a server open
    /// to players.
    #[access(WorldOnly)]
    fn server_simulation_distance(&self) -> Option<i32>;
    /// Side: server.
    /// Sets the server's view distance live, as `PlayerList.setViewDistance` (and the
    /// management protocol's `serversettings/view_distance/set`) do: every player is sent
    /// `ClientboundSetChunkCacheRadiusPacket` and their chunks follow, each up to what their
    /// client asked for. `server.properties` is not written, so a restart goes back to it.
    /// False for a distance outside 2 to 32, and off a dedicated server (an integrated
    /// server follows its window's option).
    #[access(WorldOnly)]
    fn server_set_view_distance(&mut self, distance: i32) -> bool;
    /// Side: server.
    /// Sets the server's simulation distance live, as `PlayerList.setSimulationDistance`
    /// does: every player is sent `ClientboundSetSimulationDistancePacket`, and the chunks
    /// around the players tick to the new distance from the next tick. `server.properties`
    /// is not written. False for a distance outside 2 to 32, and off a dedicated server.
    #[access(WorldOnly)]
    fn server_set_simulation_distance(&mut self, distance: i32) -> bool;
    /// Side: server.
    /// Sets the server's tick rate, as `/tick rate` does (`ServerTickRateManager.setTickRate`):
    /// the game ticks `rate` times a second from the next tick on, an autosave wait already
    /// running is shortened when the new rate asks for less, and `server_tick_rate_changed`
    /// fires. The players' clients follow on the next sync. False, with nothing changed, for a
    /// rate outside 1 to 10000 (the command's range), one that isn't finite, a call from
    /// inside `server_tick_rate_changed` (which would hear itself again without end), or on a
    /// client.
    #[access(WorldOnly)]
    fn set_tick_rate(&mut self, rate: f32) -> bool;
    /// Side: server.
    /// The open world's folder (`world/`, beside its `level.dat`): a server
    /// mod keeps its data in a folder of its own there, so the world's
    /// backups carry it. None when no world is open.
    ///
    /// Not on wasm: a path in the game's file system, which a wasm mod has none of; a wasm mod
    /// keeps world data in level attachments.
    #[access(WorldOnly)]
    fn world_dir(&self) -> Option<PathBuf>;
    /// Side: server. The calling mod's bulk files in the current save's
    /// `horizon-mod-data/<mod id>/`, copied, backed up and deleted with that
    /// save. Paths omit the mod id. Calls refuse with `NoWorld` before a
    /// save is open; this service never outlives its borrowed world host.
    #[access(WorldOnly)]
    fn world_data_files(&self) -> &dyn ConfigFiles;
    /// Side: server.
    /// Every current rule in the one server-wide store, by namespaced id.
    /// Readable from `server_about_to_start`, before any level exists.
    /// Level game-rule reads use this same store once their level exists.
    #[access(WorldOnly)]
    fn game_rules(&self) -> Vec<(String, GameRuleValue)>;
    /// Side: server.
    /// Read the server-wide rule from the first server hook onward.
    /// Returns `UnknownRule` for an unknown id; level reads use the same store.
    #[access(WorldOnly)]
    fn game_rule(&self, id: &str) -> Result<GameRuleValue, GameRuleReadError>;
    /// Side: server.
    /// The default value of rule `id` in the server-wide store (`GameRule.getDefault`), the value
    /// the rule has before `/gamerule` or a mod changes it. None for an unknown id, so a rule
    /// that was never registered reads as absent, not as a default.
    #[access(WorldOnly)]
    fn game_rule_default(&self, id: &str) -> Option<GameRuleValue>;
    /// Side: server.
    /// The retained server reload listeners of the last successful reload.
    #[access(WorldOnly)]
    fn reload_resources(&self) -> &dyn ServerReloadResources;
}
/// The client host's methods of the `config` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait ConfigClientHost {
    /// Side: client.
    /// The selected language's text for a translation key (`Component.translatable`'s lookup), `None`
    /// when no language layer defines it.
    #[access(ClientSetup)]
    fn translate(&self, key: &str) -> Option<String>;
    /// Side: client.
    /// The calling mod's toggle `key` (`Registrar::register_setting`): the player's choice, else
    /// its default. None when the mod registered no toggle by that key.
    #[access(WorldOnly)]
    fn setting_bool(&self, key: &str) -> Option<bool>;
    /// Side: client.
    /// The index of the option the player picked for the calling mod's choice `key`, else its
    /// default. None when the mod registered no choice by that key.
    #[access(WorldOnly)]
    fn setting_choice(&self, key: &str) -> Option<u32>;
    /// Side: client.
    /// The calling mod's range `key`: the player's value, else its default; a kept value outside
    /// the range reads as the default. None when the mod registered no range by that key.
    #[access(WorldOnly)]
    fn setting_f64(&self, key: &str) -> Option<f64>;
    /// Side: client.
    /// The calling mod's colour `key`, `0xRRGGBB`: the player's choice, else its default. None
    /// when the mod registered no colour by that key.
    #[access(WorldOnly)]
    fn setting_color(&self, key: &str) -> Option<u32>;
    /// Side: client.
    /// The calling mod's old settings from `horizon-extras.txt`, when it is the official mod
    /// that took over a built-in extra (`fps_overlay`, `keystrokes`, `minimap`, `zoom`): each
    /// of that extra's keys the file has a line for, with the value the extra used, as it wrote
    /// it (`("fpsCorner", "top_left")`). A written key doesn't mean the player changed it: the
    /// game writes every key once any extra differs from its default, so a line can hold the
    /// extra's default. Empty for any other mod (the file isn't read), outside a mod's call,
    /// for a player who never changed the extras, and for a file that isn't a regular UTF-8
    /// file of at most 1 MiB. Read-only and read from disk on each call: a port reads it once,
    /// keeps what it takes in its own settings, and remembers that it did.
    #[access(ClientSetup)]
    fn official_extra_settings(&self) -> Vec<(String, String)>;
}
