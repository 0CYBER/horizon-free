//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The server host's methods of the `services` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait ServicesServerHost {
    /// The commands service.
    /// Not on wasm: a frame resource; its methods are the imports.
    #[access(WorldOnly)]
    #[service]
    fn commands(&mut self) -> &mut dyn Commands;
    /// The levels service.
    /// Not on wasm: a frame resource; its methods are the imports.
    #[access(WorldOnly)]
    #[service]
    fn levels(&self) -> &dyn Levels;
    /// The messages service.
    /// Not on wasm: a frame resource; its methods are the imports.
    #[access(WorldOnly)]
    #[service]
    fn messages(&self) -> &dyn Messages;
    /// The players service.
    /// Not on wasm: a frame resource; its methods are the imports.
    #[access(WorldOnly)]
    #[service]
    fn players(&self) -> &dyn Players;
    /// The profiles service.
    /// Not on wasm: a frame resource; its methods are the imports.
    #[access(WorldOnly)]
    #[service]
    fn profiles(&mut self) -> &mut dyn Profiles;
    /// The runtime_levels service.
    /// Not on wasm: a frame resource; its methods are the imports.
    #[access(WorldOnly)]
    #[service]
    fn runtime_levels(&mut self) -> &mut dyn RuntimeLevels;
}
/// The client host's methods of the `services` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait ServicesClientHost {
    /// Persistent model groups.
    /// Not on wasm: a frame resource; its methods are the imports.
    #[access(PublishedResources)]
    #[service]
    fn instance_groups(&self) -> &dyn InstanceGroups;
    /// The client_session service.
    /// Not on wasm: a frame resource; its methods are the imports.
    #[access(WorldOnly)]
    #[service]
    fn client_session(&self) -> &dyn ClientSession;
    /// The render_assets service.
    /// Not on wasm: a frame resource; its methods are the imports.
    #[access(PublishedResources)]
    #[service]
    fn render_assets(&self) -> &dyn RenderAssets;
}
/// The client_session service, reached through its host.
pub trait ClientSession {
    /// Side: client.
    /// The connected server, not a stale reconnect target. None outside a
    /// logged-in client world. External display names are not retained; memory
    /// connections have no address. Friend-invite secrets are redacted.
    fn client_server(&self) -> Option<ClientServer> {
        crate::world::phase::unprovided(
            "ClientSession::client_server",
            "client_server",
            crate::world::phase::LoadAccess::ClientSetup,
        )
    }
    /// Side: client.
    /// The applied language code, not a pending options selection. None outside
    /// a logged-in client world. Read is_paused for the applied pause state.
    fn client_language(&self) -> Option<String> {
        crate::world::phase::unprovided(
            "ClientSession::client_language",
            "client_language",
            crate::world::phase::LoadAccess::ClientSetup,
        )
    }
    /// Side: client.
    /// The local player's latency in milliseconds as the tab list draws it
    /// (`PlayerInfo.getLatency`): the server's smoothed keep-alive average, refreshed by its own
    /// broadcast. 0 before the server's first measurement, as vanilla; None outside a logged-in
    /// client world or before the server lists this player. Sends nothing.
    fn client_latency(&self) -> Option<u32> {
        crate::world::phase::unprovided(
            "ClientSession::client_latency",
            "client_latency",
            crate::world::phase::LoadAccess::ClientSetup,
        )
    }
    /// Side: client.
    /// The player list as the Tab overlay draws it: each listed player's drawn name, profile
    /// name, UUID, game mode and latency in drawn order, at most the 80 it draws, with the
    /// header and footer. What this client has been sent; None outside a logged-in client
    /// world. Sends nothing.
    fn client_tab_list(&self) -> Option<ClientTabList> {
        crate::world::phase::unprovided(
            "ClientSession::client_tab_list",
            "client_tab_list",
            crate::world::phase::LoadAccess::ClientSetup,
        )
    }
    /// Side: client.
    /// The locator bar's tracked waypoints: each waypoint's identifier, target, icon style and
    /// icon colour, in the order the server sent them. What this client has been sent; None
    /// outside a logged-in client world. Sends nothing.
    fn client_waypoints(&self) -> Option<ClientWaypoints> {
        crate::world::phase::unprovided(
            "ClientSession::client_waypoints",
            "client_waypoints",
            crate::world::phase::LoadAccess::ClientSetup,
        )
    }
    /// Side: client.
    /// Side: client.
    /// The title, subtitle and action bar on screen now. Each is None once its timer has run
    /// out; the subtitle shows only with a title. What this client has been sent; None outside
    /// a logged-in client world. Sends nothing.
    fn client_titles(&self) -> Option<ClientTitles> {
        crate::world::phase::unprovided(
            "ClientSession::client_titles",
            "client_titles",
            crate::world::phase::LoadAccess::ClientSetup,
        )
    }
    /// Side: client.
    /// The boss bars the window shows, in the order the server added them: each bar's name and
    /// progress (0 to 1, as drawn). What this client has been sent; None outside a logged-in
    /// client world. Sends nothing.
    fn client_boss_bars(&self) -> Option<Vec<ClientBossBarEntry>> {
        crate::world::phase::unprovided(
            "ClientSession::client_boss_bars",
            "client_boss_bars",
            crate::world::phase::LoadAccess::ClientSetup,
        )
    }
    /// Side: client.
    /// Existing local frame counters; None outside a logged-in client world.
    /// This read collects no history and sends nothing anywhere.
    fn client_frame_stats(&self) -> Option<ClientFrameStats> {
        crate::world::phase::unprovided(
            "ClientSession::client_frame_stats",
            "client_frame_stats",
            crate::world::phase::LoadAccess::ClientSetup,
        )
    }
}
/// The commands service, reached through its host.
pub trait Commands {
    /// Side: server.
    /// A mod changed the commands it registers (`register_commands`):
    /// at the start of the next server tick every mod is asked again, the
    /// commands players may run are rebuilt, and every player on the server is
    /// sent their tree again, after `player_commands_sent`. Nothing
    /// happens where no world is open to players, and nothing for a server
    /// whose mods never call it. API @API:refresh_commands@.
    fn refresh_commands(&mut self) {
        crate::world::phase::unprovided(
            "Commands::refresh_commands",
            "refresh_commands",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// A command's success feedback, routed to its originating source using
    /// vanilla's silence, send-command-feedback and block/cart track-output rules.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn feedback(&mut self, line: Text) -> Result<(), TextFormError> {
        crate::world::phase::unprovided(
            "Commands::feedback",
            "feedback",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// Runs `line` (no slash) as the server's console does: permission
    /// level 4, from the overworld's spawn, with no output. A teleport to
    /// another level (`execute in minecraft:the_nether run tp Alex 0 70 0`)
    /// happens at the next tick's start, as the console's. Whether it ran
    /// without failing. During a guest's turn, live players (including the
    /// acting player) are reachable; displaced outer players are excluded
    /// from name, UUID and selector resolution on this path only.
    fn run_command(&mut self, line: &str) -> bool {
        crate::world::phase::unprovided(
            "Commands::run_command",
            "run_command",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// `run_command` with its feedback kept: the same line, source and run
    /// path (one leading slash is dropped here), answering with whether it
    /// ran without failing (`success`, what `run_command` returns) and what the
    /// console would have printed for it as plain text (`feedback`, as a mod
    /// that answers a remote console wants; Paper's `Server.dispatchCommand`
    /// with a capturing `CommandSender`). The feedback of the command alone is
    /// kept, never the lines other code logs meanwhile; a call made while
    /// another one runs captures its own lines and the outer capture goes on
    /// unchanged. At most 256 lines of 4096 bytes each
    /// come back (a longer line is cut at a character boundary). A command that ran and failed
    /// is `success: false` with its message in `feedback`. A line run where no server
    /// is open to players, or empty once its whitespace and slash are
    /// dropped, runs nothing and is `success: false` with no feedback, exactly as
    /// `run_command` answers `false` for it; no refusal type tells those
    /// apart from a failed command, as `run_command` has none either. The
    /// dedicated console's own commands (`stop`, `op`, `whitelist`, `ban`...)
    /// are not reached: they answer as an unknown command.
    fn run_command_with_output(&mut self, line: &str) -> CommandResult {
        crate::world::phase::unprovided(
            "Commands::run_command_with_output",
            "run_command_with_output",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// The command `label` names (Paper's `Server.getCommandMap().getCommand(label)`), a leading
    /// slash dropped: a usage a native mod registered (`register_commands`), else vanilla's root
    /// command, an alias resolved to its target (`tp` gives `teleport`, with `aliases` naming
    /// `tp`). `level` is the lowest a mod's usages ask, or vanilla's permission for the root.
    /// None when neither has the name. Reads only; it runs nothing. Horizon has no permission
    /// nodes: the level is the answer.
    fn command_info(&mut self, label: &str) -> Option<CommandInfo> {
        crate::world::phase::unprovided(
            "Commands::command_info",
            "command_info",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// The root names of every command the server answers, mods' and vanilla's (aliases
    /// included), sorted, each once. Empty without a server world.
    fn command_names(&mut self) -> Vec<String> {
        crate::world::phase::unprovided(
            "Commands::command_names",
            "command_names",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
}
/// The levels service, reached through its host.
pub trait Levels {
    /// Side: server.
    /// Loaded server levels, in vanilla dimension order. Empty without a server world.
    fn list_levels(&self) -> Vec<LevelRef> {
        crate::world::phase::unprovided(
            "Levels::list_levels",
            "list_levels",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
}
/// The messages service, reached through its host.
pub trait Messages {
    /// Side: server.
    /// Whether the server's chat filter (`server.toml` `[chat_filter]`)
    /// lets `text` through: true without a filter, None inside a player's
    /// hook (where the server is out of reach).
    fn text_allowed(&self, text: &str) -> Option<bool> {
        crate::world::phase::unprovided(
            "Messages::text_allowed",
            "text_allowed",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// A system line in every player's chat and the server's log.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn broadcast(&self, line: Text) -> Result<(), TextFormError> {
        crate::world::phase::unprovided(
            "Messages::broadcast",
            "broadcast",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// Writes `line` to the server console and its log at `level` (`Logger.info`, `warning`,
    /// `severe`), prefixed with `[mod_id] ` so the line names the mod that wrote it; `mod_id` is
    /// the id the mod gives, as the other calls taking a mod id. The line is cut to its first
    /// 4096 bytes at a character boundary; control characters in it (line breaks included) are
    /// replaced by spaces, so one call is always one log line. Refuses with
    /// `ConsoleLogRefusal::EmptyModId`, `ModIdTooLong` (over 64 bytes) or `InvalidModId` (a
    /// control character in the id) and then logs nothing.
    /// It is also the reply a mod gives a console command after the command has returned: the
    /// console sender of `run_command_with_output` is the console itself, so a delayed answer
    /// goes here. Other console listeners (`console_line`) hear the line too.
    fn console_log(
        &self,
        mod_id: &str,
        level: ConsoleLevel,
        line: &str,
    ) -> Result<(), ConsoleLogRefusal> {
        crate::world::phase::unprovided(
            "Messages::console_log",
            "console_log",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
}
/// The players service, reached through its host.
pub trait Players {
    /// Side: server.
    /// The players on this server, in the order they joined; in single player that
    /// includes the world's owner, who joins its server like any player. Empty on a client.
    fn server_players(&self) -> Vec<PlayerRef> {
        crate::world::phase::unprovided(
            "Players::server_players",
            "server_players",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// Find a connected player by their account name, case-insensitively.
    fn player_by_name(&self, name: &str) -> Option<PlayerRef> {
        crate::world::phase::unprovided(
            "Players::player_by_name",
            "player_by_name",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// The player whose action or command a hook hears, as their state is
    /// swapped in (`run_mod_command`, `right_click_block`,
    /// `native_use_on`, `break_block`...): None outside such a
    /// hook.
    fn acting_player(&self) -> Option<PlayerRef> {
        crate::world::phase::unprovided(
            "Players::acting_player",
            "acting_player",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
}
/// The profiles service, reached through its host: the game's own Minecraft profile resolver,
/// the one behind dynamic player heads and `/fetchprofile`. Unlike `host.profile_by_uuid` and
/// `host.profile_by_name`, which answer from the server's online players and user cache without
/// asking Mojang, these lookups ask the session server and can resolve an account the server has
/// never seen, whatever its online mode.
pub trait Profiles {
    /// Side: server.
    /// Resolve a Minecraft account name (`Notch`) through the game's profile resolver: the
    /// session server's profile, with the signed `textures` property a skin mod replaces a
    /// player's skin with. The resolver caches answers and paces a mod's new lookups at 60 a
    /// minute, so it stays within Mojang's rate limit; the game's own lookups are not paced. It
    /// reaches Mojang's services whatever the server's online mode, and a name that is not a
    /// valid player name is [`ProfileResolution::Done`] with `None` without a lookup.
    ///
    /// The lookup runs in the background: the first call answers
    /// [`ProfileResolution::Pending`], and a later call (poll it from `server_tick`) the answer,
    /// which is also cached for the game's own uses. `Pending` can also mean the per-minute
    /// budget is refilling; poll again later.
    fn resolve_name(&mut self, name: &str) -> ProfileResolution {
        crate::world::phase::unprovided(
            "Profiles::resolve_name",
            "resolve_name",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// Resolve an account's UUID through the game's profile resolver, as `resolve_name` does
    /// by name. The profile's name is the account's current one, and new lookups share the same
    /// per-minute budget.
    fn resolve_id(&mut self, id: u128) -> ProfileResolution {
        crate::world::phase::unprovided(
            "Profiles::resolve_id",
            "resolve_id",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
}

/// The runtime_levels service, reached through its host.
/// Success from `open` or `unload` admits a queued request, not its completion.
/// Apply failures are logged with the requesting mod and level id. A failed open
/// leaves the id unpublished and free to retry; a failed unload keeps it open for retry.
pub trait RuntimeLevels {
    /// Side: server.
    /// Open the level `id` (`mymod:instance/ab`), in the calling mod's namespace, as `level`
    /// describes. The request is checked now and applied after this server tick's levels have
    /// ticked, never inside a level's tick: the level is not open, and `is_open` says false, until
    /// then. The handle names this open of the level and never a later one. A folder that
    /// exists under the id is reopened as it was saved. `level_load` fires when it opens.
    /// Refused with [`RuntimeLevelError`] for an id outside the mod's namespace, one already
    /// open or waiting, an unknown dimension type or generator, a noise generator, and when
    /// the server's cap of open runtime levels (`horizon-max-runtime-levels`, 64) is reached.
    fn open(&mut self, id: &str, level: RuntimeLevel) -> Result<LevelRef, RuntimeLevelError> {
        crate::world::phase::unprovided(
            "RuntimeLevels::open",
            "open",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// Save and close the level `id` the calling mod opened, keeping its folder, once this server
    /// tick's levels have ticked: `level_unload` fires then. Refused with
    /// [`RuntimeLevelError::PlayersInside`] while a player or an arrival not yet landed is in the
    /// level, now and again when it is applied: the mod moves its players out first
    /// (`entity_teleport_to_level`).
    fn unload(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        crate::world::phase::unprovided(
            "RuntimeLevels::unload",
            "unload",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// Remove the level `id` the calling mod owns, folder and all, once this server tick's levels
    /// have ticked. A level that is open is saved and closed first (`level_unload` fires), as by
    /// `unload`; a level that is not open, such as the leftovers of a crash, only has its files
    /// removed, and a level with no files is nothing to delete. The structure references the
    /// game keeps beside the world for the level go with its folder, unless another id would
    /// share the file (`a_b:c` and `a:b_c` do) and that level exists. Links under `dimensions` or
    /// `data` are never followed: the removal fails and is logged. Refused, now and again when
    /// it is applied, with [`RuntimeLevelError::PlayersInside`] while a player or an arrival not
    /// yet landed is in the level, with [`RuntimeLevelError::LevelsBelow`] while a level below
    /// the id exists (`mymod:instance/ab` below `mymod:instance`: the folders nest), and with
    /// [`RuntimeLevelError::AlreadyOpen`] for a level still waiting to open. An `open` of a level
    /// that is open and being deleted is `AlreadyOpen` until the delete is applied; one of a
    /// level that was not open, made after the `delete`, is applied after it and starts empty.
    /// If the level closed but its files could not all be removed, it stays closed and
    /// `is_open` is false, `open` of the id is [`RuntimeLevelError::Removing`] and an `open`
    /// already queued is dropped with a log line, until a second `delete` has removed them.
    fn delete(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        crate::world::phase::unprovided(
            "RuntimeLevels::delete",
            "delete",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// Whether the level `id` is open: false until a queued `open` is applied, true until a
    /// queued `unload` or `delete` is. Takes `&mut self` as the service's other methods do: a frame
    /// resource is reached one way.
    fn is_open(&mut self, id: &str) -> bool {
        crate::world::phase::unprovided(
            "RuntimeLevels::is_open",
            "is_open",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// Regenerate the Overworld, the Nether and the End with `seed`, once this server tick's levels have ticked: their chunk files, entities, structure references, raids, the dragon fight and the world spawn are replaced by a fresh world's; game rules, the clock, the weather, the scoreboard, the world border settings and the players' files are kept. The request is checked now and again when it is applied. Refused with [`LevelResetError::PlayersInside`] while a player or an arrival not yet landed is in one of the three levels (the mod moves its players out first, for example into a runtime level it opened: `entity_teleport_to_level`), with [`LevelResetError::NoHoldingLevel`] when the mod has no runtime level open, and with [`LevelResetError::NotPermitted`] unless the mod's manifest declares `[native] level_reset`. Maps are kept. `levels_reset` fires once the old terrain is gone and not when a removal failed; the levels open again from the new seed when somebody next enters them.
    fn reset_vanilla(&mut self, seed: i64) -> Result<(), LevelResetError> {
        crate::world::phase::unprovided(
            "RuntimeLevels::reset_vanilla",
            "reset_vanilla",
            crate::world::phase::LoadAccess::WorldOnly,
        )
    }
}
/// The render_assets service, reached through its host.
pub trait RenderAssets {
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// Resolve a registered standalone model once in `client_setup` and again in
    /// `client_resources_reloaded`, not each frame. Resource-pack overrides and other
    /// namespaces use the normal shared model store; no resource is read or baked here.
    fn model(&self, id: &Identifier) -> Result<ModelRef, ModelError> {
        crate::world::phase::unprovided(
            "RenderAssets::model",
            "model",
            crate::world::phase::LoadAccess::PublishedResources,
        )
    }
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// Resolve a pipeline registered in `register_render_pipelines` once in `client_setup` and
    /// again in `client_resources_reloaded`, not each frame. `Unknown` for an id no mod
    /// registered, `Invalid` for one whose shader the current resources don't give or that
    /// didn't compile. Nothing is read or compiled here.
    fn render_pipeline(&self, id: &Identifier) -> Result<RenderPipelineRef, RenderPipelineError> {
        crate::world::phase::unprovided(
            "RenderAssets::render_pipeline",
            "render_pipeline",
            crate::world::phase::LoadAccess::PublishedResources,
        )
    }
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// The keyframe animation `id`, from `assets/<namespace>/neoforge/animations/entity/<path>.json`
    /// in the mod's or a pack's resources (NeoForge's `Model.getAnimation`), as the latest resource
    /// load read it; `None` when no file gives it or the file failed to load. Get and
    /// [`AnimationHolder::bake`] it (or [`AnimationHolder::bake_with`] when it names registered
    /// types) once in `client_setup` and again in `client_resources_reloaded`, not each frame.
    fn animation(&self, id: &Identifier) -> Option<AnimationHolder> {
        crate::world::phase::unprovided(
            "RenderAssets::animation",
            "animation",
            crate::world::phase::LoadAccess::PublishedResources,
        )
    }
}

/// Client-side persistent copies of resolved models.
pub trait InstanceGroups {
    /// Allocate up to 16,384 copies, initially length zero. Each mod may reserve 65,536
    /// copies total (an empty group reserves one slot). Only visible copies in the drawn
    /// length, not capacity, consume the view budget. At most 16,384 groups per view.
    /// Create at setup/reload, not each frame. Visible GPU-routed copies upload each drawn frame.
    /// Reload, world close, disabling the owning mod, or explicit destroy retires the group.
    /// Re-enabling a mod never revives its old handles; recreate groups after publication.
    fn create_group(&self, model: ModelRef, capacity: u32) -> Result<InstanceGroupRef, GroupError>;
}
