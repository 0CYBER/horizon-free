//! Read-only client session and local-player status.

use crate::{BlockPos, GameMode, Identifier, Text};

/// The connected server, not the reconnect target of an earlier session.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientServer {
    /// The integrated world's display name. External connections do not retain
    /// a server display name; this is `None`, never the connecting player's name.
    pub name: Option<String>,
    /// The joined address, with friend-invite secrets redacted. Memory
    /// connections have no address (`None`), not a made-up socket address.
    pub address: Option<String>,
    /// This window owns the connected integrated server. A loopback address
    /// alone does not make this true; a splitscreen guest does not own it.
    pub integrated_local: bool,
    /// Folder name relative to the instance's saves directory for the owned
    /// integrated world. Renaming the world in-game leaves this key unchanged.
    /// External connections and splitscreen guests return `None`.
    pub save_folder: Option<String>,
}

/// The local player's item use as the client sees it (`LivingEntity.isUsingItem`,
/// `getUsedItemHand`, `getTicksUsingItem`, `getUseItemRemainingTicks`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ItemUse {
    /// The hand holding the item in use.
    pub hand: crate::Hand,
    /// Ticks the item has been used so far.
    pub ticks_used: i32,
    /// Ticks left until the use completes; with `ticks_used`, the item's use duration.
    pub ticks_remaining: i32,
}

impl ItemUse {
    /// An item use of `hand`, `ticks_used` ticks old with `ticks_remaining` left.
    pub fn new(hand: crate::Hand, ticks_used: i32, ticks_remaining: i32) -> Self {
        Self {
            hand,
            ticks_used,
            ticks_remaining,
        }
    }
}

/// Existing local frame counters, sampled only when requested by a mod.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientFrameStats {
    /// The game's most recently computed frames per second (updated about once
    /// a second), not an inverse of a single frame time.
    pub fps: f32,
    /// Frames rendered by this window since it started, including menus.
    pub rendered_frames: u64,
    /// The most recent completed frame's wall interval in nanoseconds, including
    /// waits. `None` before a frame sample exists or after the chart was reset.
    pub frame_time_ns: Option<u64>,
}

/// The local player's current block-breaking operation. An unavailable player
/// returns `None` from the handle, distinct from [`Self::Idle`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BlockBreakingState {
    /// No block is currently being broken.
    Idle,
    /// The client's current breaking target and predicted progress.
    Breaking {
        /// The target block.
        position: BlockPos,
        /// The accumulated breaking fraction, as the client holds it.
        progress: f32,
        /// The visible crack stage, 0 through 9; absent while progress is zero.
        stage: Option<u8>,
    },
}

/// The player list as the Tab overlay draws it (`PlayerTabOverlay`), read with
/// `ClientSession::client_tab_list`: what this client has been sent, changing as the server
/// updates the list.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientTabList {
    /// The listed players in drawn order (`getPlayerInfos`: higher list order first, spectators
    /// last, then team name and profile name), at most the 80 the overlay draws.
    pub entries: Vec<ClientTabListEntry>,
    /// The header the server set; `None` when it is empty or was cleared.
    pub header: Option<Text>,
    /// The footer the server set; `None` when it is empty or was cleared.
    pub footer: Option<Text>,
}

impl ClientTabList {
    /// A list from its entries, header and footer.
    pub fn new(
        entries: Vec<ClientTabListEntry>,
        header: Option<Text>,
        footer: Option<Text>,
    ) -> Self {
        Self {
            entries,
            header,
            footer,
        }
    }
}

/// One listed player (`PlayerInfo`) as the Tab overlay shows it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientTabListEntry {
    /// The profile name (`GameProfile.name`).
    pub name: String,
    /// The profile UUID.
    pub uuid: u128,
    /// The game mode the server reported.
    pub game_mode: GameMode,
    /// The latency in milliseconds the ping icon shows; negative when unknown.
    pub latency: i32,
    /// The name as drawn (`getNameForDisplay`): the display name the server set, else the
    /// profile name with its team's prefix, suffix and colour; italic for a spectator.
    pub display: Text,
}

impl ClientTabListEntry {
    /// An entry from its parts.
    pub fn new(name: String, uuid: u128, game_mode: GameMode, latency: i32, display: Text) -> Self {
        Self {
            name,
            uuid,
            game_mode,
            latency,
            display,
        }
    }
}

/// The locator bar's tracked waypoints (`ClientWaypointManager`), read with
/// `ClientSession::client_waypoints`: what this client has been sent, changing as the server
/// tracks and untracks waypoints.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientWaypoints {
    /// The tracked waypoints in the order the server sent them.
    pub entries: Vec<ClientWaypointEntry>,
}

impl ClientWaypoints {
    /// Waypoints from their entries.
    pub fn new(entries: Vec<ClientWaypointEntry>) -> Self {
        Self { entries }
    }
}

/// One tracked waypoint (`TrackedWaypoint`) as the locator bar holds it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientWaypointEntry {
    /// The waypoint's identifier: the tracked entity's UUID, or its name when it has no entity.
    pub id: ClientWaypointId,
    /// What the waypoint points at: a block, a chunk, a bearing, or nothing.
    pub target: ClientWaypointTarget,
    /// The waypoint's icon style (`Waypoint.Icon.style`, a `waypoint_style` asset id such as
    /// `minecraft:default`).
    pub style: Identifier,
    /// The waypoint's icon colour as `0xRRGGBB`, as `living_set_waypoint_color` takes it;
    /// `None` when the server sent none (the bar then colours the dot from the waypoint's id).
    pub color: Option<u32>,
}

impl ClientWaypointEntry {
    /// A waypoint from its parts.
    pub fn new(
        id: ClientWaypointId,
        target: ClientWaypointTarget,
        style: Identifier,
        color: Option<u32>,
    ) -> Self {
        Self {
            id,
            target,
            style,
            color,
        }
    }
}

/// The title, subtitle and action bar on screen now, read with
/// `ClientSession::client_titles`: what this client has been sent, changing as the server
/// sends titles.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientTitles {
    /// The title; `None` once its timer has run out.
    pub title: Option<Text>,
    /// The subtitle; shown only with a title.
    pub subtitle: Option<Text>,
    /// The action bar; `None` once its timer has run out.
    pub action_bar: Option<Text>,
}

impl ClientTitles {
    /// Titles from their parts.
    pub fn new(title: Option<Text>, subtitle: Option<Text>, action_bar: Option<Text>) -> Self {
        Self {
            title,
            subtitle,
            action_bar,
        }
    }
}

/// A tracked waypoint's identifier (`Either<UUID, String>`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ClientWaypointId {
    /// The tracked entity's UUID (a player's waypoint).
    Uuid(u128),
    /// The waypoint's name (a named waypoint with no entity).
    Name(String),
}

/// What a tracked waypoint points at (`TrackedWaypoint`'s `Type` and contents).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ClientWaypointTarget {
    /// An untracked waypoint (`TrackedWaypoint.empty`).
    Empty,
    /// A block position (`Vec3iWaypoint`).
    Block(BlockPos),
    /// A chunk (`ChunkWaypoint`).
    Chunk([i32; 2]),
    /// A bearing in radians (`AzimuthWaypoint`).
    Azimuth(f32),
}

/// One boss bar the window shows, read with `ClientSession::client_boss_bars`. Unlike
/// `ClientBossBar`, the bar a `customize_boss_event_progress` event hands a mod, this is the
/// bar as the window holds it: name and progress only.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientBossBarEntry {
    /// The bar's name.
    pub name: Text,
    /// 0 to 1, as drawn (the lerped value).
    pub progress: f32,
}

impl ClientBossBarEntry {
    /// A boss bar entry from its parts.
    pub fn new(name: Text, progress: f32) -> Self {
        Self { name, progress }
    }
}
