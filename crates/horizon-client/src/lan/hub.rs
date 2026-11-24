//! The play state of a hosted world (PlayerList, ServerGamePacketListenerImpl,
//! ChunkMap, PlayerChunkSender): admits the players the connection threads
//! hand over, sends them the level (chunks, time, weather, entities, player
//! list), keeps their views up to date as blocks and entities change, and
//! turns what they send into events for the gameplay code. It runs on the
//! game thread inside the tick, only when the world is open to the network.
use super::{
    Lan, NetEvent,
    chunk_watch::{ChunkWatchKind, ChunkWatchLog},
    chunks::ChunkEncoder,
    data::SyncData,
    entity::{Entities, EntityState},
    equipment_cache,
    guest::{Guest, InboxItem, SentEffect, stack_to_network},
};
use crate::{
    chunk::Chunk,
    collisions::SceneCollisions,
    effects::{EffectInstance, PlayerEffects},
    items::ItemStack,
    text::Text,
    world::World,
};
use horizon_core::random::LegacyRandom;
use horizon_protocol::ids::play::clientbound as ids;
use horizon_protocol::{
    entity_data::{Particle, ParticleData, Value, serializer},
    packets::play::{self, Clientbound as C, Serverbound as S},
    text::{literal, translatable},
};
use std::sync::{Arc, Mutex, Weak, atomic::Ordering};
use {
    horizon_core::nbt::{JavaString, Tag},
    horizon_mod_runtime::attachments::Holder,
    horizon_world::{
        block_collision::BlockCollisionRegistry, native_load_context::LoopContext,
        world_movement::in_range_of_ground,
    },
};

mod activity;
/// GameEvent types (ClientboundGameEventPacket).
pub mod advancements;
mod block_changes;
pub mod chat;
pub mod chat_filter;
mod fake_kicks;
mod item_particles;
mod joined;
mod move_collision;
mod moves;
pub mod passage_cross;
mod pending_move;
mod raw_inbox;
mod selector_sync;
mod spawn_info;
pub(super) mod terrain;
mod view_center;
use advancements::AdvancementOwners;
use chat::yellow_names;
pub use pending_move::{
    Accepted, Hearing, MOVE_PACKET_BUDGET, MOVE_PACKET_QUEUE, MovePacket, PendingMove,
};
use raw_inbox::{DeferredPayload, PacketDrain, yield_raw};
pub use view_center::ViewCenterRefusal;
pub mod game_event {
    pub const NO_RESPAWN_BLOCK_AVAILABLE: u8 = 0;
    pub const START_RAINING: u8 = 1;
    pub const STOP_RAINING: u8 = 2;
    pub const CHANGE_GAME_MODE: u8 = 3;
    pub const RAIN_LEVEL: u8 = 7;
    pub const THUNDER_LEVEL: u8 = 8;
    pub const GUARDIAN_ELDER_EFFECT: u8 = 10;
    pub const IMMEDIATE_RESPAWN: u8 = 11;
    pub const LIMITED_CRAFTING: u8 = 12;
    pub const LEVEL_CHUNKS_LOAD_START: u8 = 13;
}

fn initial_recipe_book_packets(sync: &SyncData, book: &crate::recipe_book::Settings) -> [C; 2] {
    let settings = (0..4)
        .flat_map(|b| [u8::from(book.open[b]), u8::from(book.filtering[b])])
        .collect();
    [
        raw(ids::RECIPE_BOOK_SETTINGS, settings),
        raw(
            ids::RECIPE_BOOK_ADD,
            sync.advancements.initial_recipe_book(book, sync),
        ),
    ]
}

// exhaustive: engine-owned configuration, constructed by the game and its network fixtures;
// not a mod API value.
#[derive(Clone, Debug)]
pub struct HubConfig {
    pub default_game_mode: i32,
    pub guest_commands: bool,
    pub force_game_mode: bool,
    pub hardcore: bool,
    pub view_distance: i32,
    pub simulation_distance: i32,
    pub dimension: String,
    /// `MinecraftServer.allowFlight`: true except on a dedicated server whose
    /// `allow-flight` is off.
    pub allow_flight: bool,
    pub seed: i64,
    pub max_players: usize,
    /// An integrated server: the owner's level from `ops`, the others' from Allow Commands
    /// and the guests' command access, and the integrated-only commands. Otherwise a
    /// dedicated server: operators from `ops`, the dedicated-only commands.
    pub integrated: bool,
    /// The shapes of the world's levels (their dimension types).
    pub levels: crate::chunk::LevelShapes,
    /// The own seeds of the runtime levels a mod opened with one, by id; the world's seed for
    /// the others. Kept here, not in `levels`, whose tables are leaked.
    pub level_seeds: std::collections::BTreeMap<String, i64>,
}

/// What the other players see of a player's body besides sneaking and
/// sprinting: the pose and the rest of `Entity.DATA_SHARED_FLAGS_ID`.
#[derive(Clone, Copy, Debug, Default)]
pub struct BodyFlags {
    /// `Entity.isFallFlying`: gliding with elytra.
    pub gliding: bool,
    /// `Entity.isSwimming` (the swimming flag).
    pub swimming: bool,
    /// The swimming pose of a player crawling under a block.
    pub crawling: bool,
    /// The crouching pose, held without sneaking under a low ceiling.
    pub crouching: bool,
    /// `LivingEntity.isAutoSpinAttack`: Riptide's spin.
    pub spin_attack: bool,
    /// `Entity.isOnFire`.
    pub on_fire: bool,
    /// `Entity.isInvisible`: the invisibility effect.
    pub invisible: bool,
    /// The glowing effect.
    pub glowing: bool,
}

/// Everything the hub needs from the game for one tick.
pub struct TickCtx<'a> {
    /// The active level's generator sea level (`CommonPlayerSpawnInfo`).
    pub sea_level: i32,
    pub world: &'a mut World,
    pub entities: &'a [EntityState],
    pub next_entity: &'a mut u32,
    /// Where new players appear.
    pub spawn: [f64; 3],
    /// Rotation of the world spawn sent by PlayerList.sendLevelInfo.
    pub spawn_angle: [f32; 2],
    /// The world spawn's dimension (`RespawnData.dimension`), sent with it.
    pub spawn_dimension: &'a str,
    pub difficulty: i32,
    pub difficulty_locked: bool,
    /// The overworld clock's speed (0 when the day does not advance).
    pub clock_rate: f32,
    /// TickRateManager: the tick rate and whether the game is frozen.
    pub tick_rate: f32,
    pub tick_frozen: bool,
    /// The last step command's broadcast and the remaining ticks for joins.
    pub tick_steps: crate::clock::StepState,
    /// Block collisions, for ServerGamePacketListenerImpl's moved-wrongly
    /// check (none: moves are not checked against blocks).
    pub collision: Option<&'a BlockCollisionRegistry<'a>>,
    /// The level's world border (PlayerList.sendLevelInfo sends it).
    pub border: &'a crate::worldborder::WorldBorder,
    /// The level's columns still being generated: a column is sent once
    /// each neighbour is loaded or a decorated proto chunk (#4294). None:
    /// every column there is is loaded.
    pub streaming: Option<&'a crate::streaming::Streaming>,
}

/// What the gameplay code has to do for a guest.
pub enum HostEvent {
    /// Release input reservations only after this segment has finished in Game.
    InputCredits(Vec<crate::net::ingress::ReceiveLease<NetEvent>>),
    RawPayload(Box<crate::raw_payloads::Delivery>),
    /// A guest entered the world; load their saved state.
    Joined(u32),
    /// A guest left (state to save, entities to drop).
    Left(Box<Guest>),
    Packet(u32, Box<S>),
    /// `handleClientTickEnd`'s movement reset, queued behind the packets that came
    /// before it: a `MoveVehicle` the game dispatches after the drain sets the rider's
    /// known movement first, and the reset keeps it for that tick (vanilla runs both
    /// handlers in packet order on the main thread). The position flag and the
    /// anticheat's tick end stay in the hub's drain, where no queued move bears on them.
    TickEnd(u32),
    /// A guest sent new client information: the values before and after, for the mods.
    ClientInformation(
        u32,
        Box<horizon_protocol::packets::configuration::ClientInformation>,
        Box<horizon_protocol::packets::configuration::ClientInformation>,
    ),
    /// A changed movement report is held for the game to rule on ([`Hub::resolve_move`]).
    PlayerMove(u32),
    /// A validated movement packet, whether or not it changes anything (`player_move_packet`).
    PlayerMovePacket {
        conn: u32,
        to: [f64; 3],
        to_rotation: [f32; 2],
        on_ground: bool,
        has_position: bool,
        has_rotation: bool,
    },
    /// An accepted position started a jump, before later gameplay packets.
    Jump {
        conn: u32,
        from: [f64; 3],
        yaw: f32,
    },
    /// A guest's client finished loading the level around them.
    PlayerLoaded(u32),
    /// A guest changed recipe book settings.
    RecipeBookSettings {
        conn: u32,
        book: i32,
        open: bool,
        filtering: bool,
    },
    /// A guest changed sneaking in a player input packet, already applied.
    Sneak {
        conn: u32,
        sneaking: bool,
    },
    /// A loaded guest sent their current player input flags.
    Input {
        conn: u32,
        flags: i32,
    },
    /// A loaded guest's client answered a ping (`ServerboundPongPacket`) numbered `id`.
    Pong {
        conn: u32,
        id: i32,
    },
    /// A guest was kicked, with the reason rendered by the hub.
    Kicked {
        conn: u32,
        player: u128,
        name: String,
        reason: String,
    },
    /// A guest's client started (`flying`) or stopped flying (their abilities packet), applied to
    /// the guest already: the game tells the mods, which may refuse it.
    Flight {
        conn: u32,
        flying: bool,
    },
    /// A chat line to show on the host (already broadcast to the guests);
    /// a dedicated server logs it, `[Not Secure]` unless signed and fresh.
    Chat {
        line: String,
        secure: bool,
    },
    /// A chat message a player sent (after it was broadcast), for server mods.
    PlayerChat {
        sender: u128,
        text: String,
    },
    /// A chat message the sender's chain and the chat filter let through,
    /// not yet broadcast: with server mods loaded, the game tells them
    /// (`native_server_chat`, NeoForge's `ServerChatEvent`) and then
    /// broadcasts it with `Hub::broadcast_chat`.
    ChatSubmitted {
        message: Box<chat::ChatMessage>,
        bound: Box<play::ChatTypeBound>,
        name: String,
    },
    /// A new chat session's announcement (`PlayerInfoUpdate` with
    /// INITIALIZE_CHAT) held behind a `ChatSubmitted` of the same tick: the
    /// game broadcasts it once it has relayed that message.
    ChatSessionStarted(Box<play::PlayerInfoUpdate>),
    /// A player's text the chat filter refused (docs/chat-filter.md): relayed to nobody, for the
    /// game to log and to carry out `action`.
    ChatFiltered {
        sender: u128,
        name: String,
        place: chat_filter::Place,
        text: String,
        /// The hit's number in the filter's window.
        count: u32,
        action: horizon_server_files::chat_guard::Action,
    },
    /// A server log warning (ServerGamePacketListenerImpl's LOGGER.warn).
    Warn(String),
    /// A server log error (LOGGER.error).
    Error(String),
    /// The anticheat bans this player (`banned-players.json`, then the ban's kick).
    Ban {
        uuid: u128,
        name: String,
        reason: String,
    },
}

/// The criteria `CriteriaTriggers.TICK` granted one player.
pub struct TickGrants {
    pub player: u128,
    pub entity_id: i32,
    pub name: String,
    pub conn: u32,
    pub now_millis: i64,
    pub first_tick: bool,
    pub sync: Arc<SyncData>,
    pub grants: Vec<super::advancements::CriterionGrant>,
}

/// What one player started and stopped tracking, in order, for the game to tell the mods.
pub struct TrackingBatch {
    pub player: u128,
    pub name: String,
    pub events: Vec<super::entity::TrackEvent>,
}

/// A holder whose synced attachments one player is sent whole, because the
/// player started seeing it.
pub struct AttachmentSnapshot {
    pub player: u128,
    pub holder: Holder,
}

/// Queues `holders` as snapshots for `player`.
fn queue_snapshots(
    snapshots: &mut Vec<AttachmentSnapshot>,
    player: u128,
    holders: impl IntoIterator<Item = Holder>,
) {
    snapshots.extend(
        holders
            .into_iter()
            .map(|holder| AttachmentSnapshot { player, holder }),
    );
}

/// Moves a guest's tracking events into `out`, one batch, and the entities it
/// started tracking into `snapshots`.
fn take_tracking(
    g: &mut Guest,
    out: &mut Vec<TrackingBatch>,
    snapshots: &mut Vec<AttachmentSnapshot>,
    sync_attachments: bool,
) {
    let events = g.tracker.take_events();
    if !events.is_empty() {
        if sync_attachments {
            let started = events.iter().filter(|e| e.start);
            queue_snapshots(
                snapshots,
                g.uuid,
                started.map(|e| Holder::Entity(e.entity as u32)),
            );
        }
        out.push(TrackingBatch {
            player: g.uuid,
            name: g.name.clone(),
            events,
        });
    }
}

pub struct Hub {
    packet_drain: Option<PacketDrain>,
    deferred_payloads: std::collections::HashMap<Option<u32>, DeferredPayload>,
    /// Main-thread world identity used to bind typed network-context player handles.
    pub payload_player_epoch: Option<mod_api::WorldEpoch>,
    /// Clicks on and closings of native mods' menus, for the game.
    pub native_menu_events: Vec<super::menu::NativeMenuEvent>,
    /// The chunks players started and stopped watching, for the mods
    /// (`chunk_watch`): recorded only while a mod listens.
    pub chunk_watches: ChunkWatchLog,
    /// Kicks waiting for the game thread to notify native mods.
    pending_kicks: Vec<HostEvent>,
    pending_logouts: crate::fast_hash::FxHashMap<u128, Weak<()>>,
    /// The disk copy can be stale until this player's next successful logout save.
    failed_logout_saves: crate::fast_hash::FxHashSet<u128>,
    /// A native mod listens to `player_move`: a changed report waits for the game ([`pending_move`]).
    pub moves_listened: bool,
    /// A native mod listens to `player_move_packet`: every validated movement packet is told to
    /// the game.
    pub move_packets_listened: bool,
    /// `[transfer]` on (a dedicated server's `--transfers`): the players a proxy
    /// said would arrive (`player-expect`). None while it is off.
    pub transfers: Option<super::arrivals::Transfers>,
    /// The entities players started and stopped tracking, collected by
    /// `collect_tracking`, for the game to tell the mods.
    pub native_tracking: Vec<TrackingBatch>,
    /// The holders players started seeing, for the game to send their synced
    /// attachments (`flush_attachment_sync`).
    pub attachment_snapshots: Vec<AttachmentSnapshot>,
    /// Whether native mods can produce synced attachments.
    pub sync_attachments: bool,
    /// Criteria `CriteriaTriggers.TICK` granted to players, for the game to tell the mods.
    pub native_advancement_grants: Vec<TickGrants>,
    pub native_reload_pending: bool,
    /// Whether any native mod is loaded: `Input` and `Sneak` events only exist for them, so
    /// the hub queues none otherwise. The game sets it before each poll.
    pub mods_loaded: bool,
    /// Whether a mod listens for `player_pong`: only then is a pong answering a ping queued. The
    /// game sets it before each poll.
    pub pong_listened: bool,
    /// Players a native mod hid (`native_hide_player`), by UUID, on or off:
    /// only players of at least that permission level see them, in the
    /// player list, the world and their joining and leaving.
    pub hidden: crate::fast_hash::FxHashMap<u128, u8>,
    /// Players `refresh_player` hid from everyone: the level a mod hid them
    /// at before (`hidden`), and the tick it began.
    refreshing: Vec<(u128, u64)>,
    pub lan: Lan,
    pub config: HubConfig,
    /// What the hooks fired inside a click or a name format are given; the
    /// game replaces it on each reload.
    pub loop_context: Arc<LoopContext>,
    pub encoder: ChunkEncoder,
    pub(in crate::lan) terrain: Option<Box<super::terrain_view::Views>>,
    pub guests: super::guest_list::GuestList,
    pub(super) sync: Arc<SyncData>,
    /// The active world session rule definitions.
    pub rule_registry: Arc<crate::gamerules::RuleRegistry>,
    pub ticks: u64,
    /// `Level.soundSeedGenerator`: a fresh `nextLong` per `playSound`.
    pub(super) sound_seeds: Mutex<LegacyRandom>,
    pub(super) sound_order: Mutex<super::sound_order::SoundOrder>,
    /// The `player_movement_check` and `elytra_movement_check` game rules,
    /// set by the game before each tick.
    pub movement_checks: (bool, bool),
    /// The anticheat, when `server.toml` turns it on (dedicated servers).
    pub anticheat: Option<Box<super::anticheat::Anticheat>>,
    /// The chat filter, when `server.toml` turns it on (dedicated servers).
    pub chat_filter: Option<Box<dyn horizon_server_files::chat_guard::ChatGuard>>,
    /// The far ring, when `server.toml`'s `max_view_distance` turns it on (dedicated servers).
    pub far: Option<super::far::Far>,
    /// The `immediate_respawn` game rule, as the players were last told.
    immediate_respawn: bool,
    /// The `reduced_debug_info` game rule, as the players were last told.
    reduced_debug_info: bool,
    /// The `limited_crafting` game rule, as the players were last told.
    pub limited_crafting: bool,
    /// An integrated server's owner (`MinecraftServer.isSingleplayerOwner`),
    /// whose moves are never checked.
    pub singleplayer_owner: Option<u128>,
    /// Splitscreen seats an integrated server gave the owner's permissions
    /// (`IntegratedServer.joinSplit`): they are in `ops` for permission checks
    /// but are no `ops.json` entries, so `PlayerList.isOp` does not name them.
    pub split_players: std::collections::HashSet<u128>,
    /// The world is superflat (`ServerLevel.isFlat`, a world-wide flag, sent as
    /// `CommonPlayerSpawnInfo.isFlat` in every dimension).
    pub flat_world: bool,
    /// The world is the Debug preset (`CommonPlayerSpawnInfo.isDebug`).
    pub debug_world: bool,
    /// Who rides what, set by the game before each tick.
    pub riding: super::entity::Riding,
    /// The scoreboard's teams, set by the game each tick, for players' display names.
    pub teams: Vec<crate::scoreboard::Team>,
    /// Names a server mod shows for players in chat and the Tab list
    /// (`native_set_nickname`), by UUID. Their real name stays everywhere
    /// else: the log, commands, the hover of the shown name.
    pub nicknames: std::collections::HashMap<u128, String>,
    /// Names a server mod shows for players in the Tab list only (`player_set_list_name`),
    /// by UUID, over their nickname.
    pub list_names: std::collections::HashMap<u128, crate::text::Text>,
    /// Client-only entities a mod put in one viewer's client (`viewer_entity_spawn`), by
    /// (viewer UUID, entity id).
    pub viewer_entities: std::collections::HashSet<(u128, u32)>,
    /// The name the server's user cache knew a joining player by, by UUID, looked up before the
    /// cache learns the new one (PlayerList.placeNewPlayer's `oldName`); taken by the join line.
    pub former_names: std::collections::HashMap<u128, String>,
    last_clock: i64,
    last_rate: f32,
    /// The total and rate last told of the clock the level in place follows when it is not the
    /// Overworld's (`level_clock`); `i64::MIN` while none is.
    last_level_clock: (i64, f32),
    /// The Overworld clock's total while `World.day_time` follows another clock (the level in
    /// place has its own); None while it follows the Overworld's.
    pub overworld_day: Option<i64>,
    /// The clock `World.day_time` follows when it is not the Overworld's, with its rate.
    pub level_clock: Option<(String, f32)>,
    ticking: (f32, bool),
    /// The `tick_steps` count last broadcast.
    stepping: u32,
    weather: (bool, f32, f32),
    /// The clock and weather last sent in the levels not in place, by
    /// dimension (`set_dimension` swaps them with the fields above).
    level_sync: std::collections::HashMap<String, LevelSync>,
    /// Operators and their permission levels (dedicated servers).
    pub ops: std::collections::HashMap<u128, u8>,
    /// `entity-broadcast-range-percentage` (dedicated servers).
    pub entity_range_percent: i32,
    /// `spawn-protection` (dedicated servers): the radius around the world
    /// spawn that only operators may change.
    pub spawn_protection: i32,
    /// `chat-spam-threshold-seconds` as a TickThrottler threshold (20 a second):
    /// over it, a guest who is not an operator is kicked. 0 never kicks.
    pub chat_spam_threshold: i32,
    /// `command-spam-threshold-seconds`, as `chat_spam_threshold`.
    pub command_spam_threshold: i32,
    /// `server.toml`'s `admin_feedback_to_ops` (dedicated servers): whether
    /// the other operators get a command's `[name: ...]` line (vanilla's
    /// broadcastToAdmins). False keeps it to the console's log.
    pub admin_feedback_to_ops: bool,
    chat_type: i32,
    dimension_type: i32,
    clock_ids: Vec<i32>,
    item_ids: std::collections::HashMap<Arc<str>, i32>,
    entity_types: std::collections::HashMap<String, i32>,
    /// Each level's tracked entities, by dimension.
    entities: std::collections::HashMap<String, Entities>,
    /// Each level's entities that joined it after its tracking pass, as they
    /// were then, and the server tick they were noted in (`note_joined`).
    joined: std::collections::HashMap<String, (u64, Vec<EntityState>)>,
    /// Each level's fresh entities that joined it during the tick before its
    /// tracking pass (arrows shot by players or dispensers): paired as they
    /// joined (IN_GROUND false), then updated in the next pass.
    fresh_joined: std::collections::HashMap<String, Vec<EntityState>>,
    /// The ranges of entity ids taken this tick in the phases vanilla runs
    /// before the level's ChunkMap.tick (`Entities::synced`).
    pub synced: Vec<(i32, i32)>,
    /// What one tick of entity tracking sends each guest, kept between ticks.
    entity_out: Vec<Vec<super::Body>>,
    /// Each placed guest's entities gone this tick, where they were last sent (only while the
    /// prediction records).
    entity_gone: Vec<Vec<[f64; 3]>>,
    /// Server mods' commands, a root in the commands report's format, sent
    /// after vanilla's; the key the game last set them for.
    mod_commands: (serde_json::Value, Option<u64>),
    selector_tokens: Vec<horizon_client_core::registries::SelectorToken>,
    /// Players' areas in other levels and what their clients view of them (arismoko/horizon#3716).
    pub viewed: super::viewed_levels::ViewedLevels,
}

fn string(s: &str) -> Tag {
    Tag::String(JavaString::from(s))
}

/// A player's display name was formatted. Replace `displayname` to change it. Both sides.
#[inline(never)]
pub fn native_player_name_format(context: &LoopContext, username: &Text, displayname: &mut Text) {
    crate::native_client_hooks::player_name_format(context, username, displayname);
}

fn formatted_player_name(context: &LoopContext, name: &str, component: Tag) -> Tag {
    if !crate::native_client_hooks::has_mods() {
        return component;
    }
    let username = crate::text::Text::literal(name);
    let original = crate::scoreboard::tag_to_json(&component);
    let mut displayname = crate::scoreboard::comp_text(&original).with_component(original.clone());
    native_player_name_format(context, &username, &mut displayname);
    let result = displayname.component();
    if result == original {
        component
    } else {
        crate::scoreboard::component_tag(&result)
    }
}

/// Player.getDisplayName for a player in no team: the name, suggesting
/// `/tell NAME `, showing the player on hover and inserting the name.
pub fn display_name(context: &LoopContext, uuid: u128, name: &str) -> Tag {
    let words = [
        (uuid >> 96) as i32,
        (uuid >> 64) as i32,
        (uuid >> 32) as i32,
        uuid as i32,
    ];
    let component = Tag::compound([
        ("text", string(name)),
        (
            "click_event",
            Tag::compound([
                ("action", string("suggest_command")),
                ("command", string(&format!("/tell {name} "))),
            ]),
        ),
        (
            "hover_event",
            Tag::compound([
                ("action", string("show_entity")),
                ("id", string("minecraft:player")),
                ("uuid", Tag::IntArray(words.to_vec())),
                ("name", string(name)),
            ]),
        ),
        ("insertion", string(name)),
    ]);
    formatted_player_name(context, name, component)
}

/// ServerPlayer.getTabListDisplayName: the name the Tab list shows for
/// `player`, None to let the client choose it. Replace `display_name` to
/// change it.
#[inline(never)]
pub fn native_player_tab_list_name_format(
    context: &LoopContext,
    player: u128,
    name: &str,
    display_name: &mut Option<Text>,
) {
    crate::native_client_hooks::player_tab_list_name_format(context, player, name, display_name);
}

/// `LivingEntity.DATA_ARROW_COUNT_ID` (12) and `DATA_STINGER_COUNT_ID` (13):
/// the counts of what is stuck in a player or a mob, 0 by default. `meta`
/// gets the counts that are set and `defaults` both, so one back at zero is
/// sent as the default.
pub fn stuck_meta(
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
    (arrows, stingers): (i32, i32),
) {
    for (index, count) in [(12, arrows), (13, stingers)] {
        if count != 0 {
            meta.push((index, serializer::INT, Value::Int(count)));
        }
        defaults.push((index, serializer::INT, Value::Int(0)));
    }
}

fn frozen_meta(state: &mut EntityState, ticks: i32) {
    if ticks != 0 {
        state.meta.push((7, serializer::INT, Value::Int(ticks)));
    }
    state.defaults.push((7, serializer::INT, Value::Int(0)));
}

/// `Player.DATA_SHOULDER_PARROT_LEFT` (19) and `_RIGHT` (20), an
/// `OptionalInt`: 0 for none, else the parrot variant's id plus one.
fn shoulder_meta(state: &mut EntityState, shoulders: [Option<i32>; 2]) {
    for (index, variant) in [(19, shoulders[0]), (20, shoulders[1])] {
        if let Some(v) = variant {
            state
                .meta
                .push((index, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(v + 1)));
        }
        state
            .defaults
            .push((index, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(0)));
    }
}

/// The shoulder parrot slots that differ from what the player's client was
/// last sent, as entity data: 0 for none, else the variant's id plus one.
fn shoulder_changes(now: [Option<i32>; 2], sent: [Option<i32>; 2]) -> Vec<(u8, i32, Value)> {
    (19..)
        .zip(now.into_iter().zip(sent))
        .filter(|(_, (now, sent))| now != sent)
        .map(|(index, (now, _))| {
            let id = now.map_or(0, |v| v + 1);
            (index, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(id))
        })
        .collect()
}

impl Hub {
    /// Player.getDisplayName: the name formatted for the player's team
    /// (PlayerTeam.formatNameForTeam), suggesting `/tell NAME `, showing the
    /// player on hover and inserting the name.
    /// A nickname (`nicknames`) takes the name's place in the text; the
    /// hover, the suggested `/tell` and the insertion keep the real name.
    pub fn player_display_name(&self, uuid: u128, name: &str) -> Tag {
        let shown = self.nicknames.get(&uuid).map_or(name, String::as_str);
        let team = self.teams.iter().find(|t| t.players.contains(name));
        if team.is_none() && shown == name {
            return display_name(&self.loop_context, uuid, name);
        }
        let mut c = match team {
            Some(team) => team.formatted_name_comp(&crate::scoreboard::literal(shown)),
            None => serde_json::json!({ "text": shown }),
        };
        c["click_event"] =
            serde_json::json!({"action": "suggest_command", "command": format!("/tell {name} ")});
        c["hover_event"] = serde_json::json!({"action": "show_entity", "id": "minecraft:player",
            "uuid": horizon_protocol::crypto::uuid_string(uuid), "name": name});
        c["insertion"] = name.into();
        formatted_player_name(
            &self.loop_context,
            name,
            crate::scoreboard::component_tag(&c),
        )
    }
}

impl Hub {
    /// PlayerList.placeNewPlayer's join line: `multiplayer.player.joined`, or
    /// `multiplayer.player.joined.renamed` with the cached old name when the
    /// player's name changed (ignoring case) since the user cache last saw it.
    fn joined_line(&mut self, uuid: u128, name: &str) -> Tag {
        let display = self.player_display_name(uuid, name);
        match self
            .former_names
            .remove(&uuid)
            .filter(|old| !old.eq_ignore_ascii_case(name))
        {
            Some(old) => yellow_names(
                "multiplayer.player.joined.renamed",
                vec![display, string(&old)],
            ),
            None => yellow_names("multiplayer.player.joined", vec![display]),
        }
    }
}

/// An entity type's registry id, remembered once looked up.
fn entity_type(
    types: &mut std::collections::HashMap<String, i32>,
    sync: &SyncData,
    kind: &str,
) -> Option<i32> {
    if let Some(id) = types.get(kind) {
        return Some(*id);
    }
    let id = sync.builtin.get("minecraft:entity_type")?.id(kind)?;
    types.insert(kind.to_owned(), id);
    Some(id)
}

/// ChunkTrackingView.contains: the chunks a player at (cx, cz) with view
/// distance `r` is sent (isWithinDistance with the neighbours' buffer).
pub(super) fn tracked(cx: i32, cz: i32, r: i32, x: i32, z: i32) -> bool {
    let dx = i64::from(((x - cx).abs() - 2).max(0));
    let dz = i64::from(((z - cz).abs() - 2).max(0));
    dx * dx + dz * dz < i64::from(r) * i64::from(r)
}

/// The view's queue around a new centre: the queued chunks still tracked, then
/// the tracked ones neither sent nor queued, closest first (a stable sort, so
/// equally close chunks keep their order).
fn requeue(
    queue: &mut Vec<(i32, i32)>,
    sent: &crate::fast_hash::FxHashSet<(i32, i32)>,
    center: (i32, i32),
    r: i32,
) {
    requeue_ring(queue, sent, center, 0, r);
}

/// `requeue` for the chunks tracked at `r` and not at `inner` (0: none): the far ring's.
fn requeue_ring(
    queue: &mut Vec<(i32, i32)>,
    sent: &crate::fast_hash::FxHashSet<(i32, i32)>,
    (cx, cz): (i32, i32),
    inner: i32,
    r: i32,
) {
    let tracked = |x, z| tracked(cx, cz, r, x, z) && !tracked(cx, cz, inner, x, z);
    queue.retain(|&(x, z)| tracked(x, z));
    // A walking player changes chunk every few ticks with hundreds of chunks
    // queued: a set, not a scan of the queue for each of the view's chunks.
    let mut queued: crate::fast_hash::FxHashSet<(i32, i32)> = queue.iter().copied().collect();
    for dz in -r - 2..=r + 2 {
        for dx in -r - 2..=r + 2 {
            let key = (cx + dx, cz + dz);
            if tracked(key.0, key.1) && !sent.contains(&key) && queued.insert(key) {
                queue.push(key);
            }
        }
    }
    queue.sort_by_key(|&(x, z)| i64::from(x - cx).pow(2) + i64::from(z - cz).pow(2));
}

/// A player's next chunks (`Hub::next_batch`), with their places in its queues: the view's, then
/// the far ring's. Missing saved columns stay queued for a later retry.
struct Batch<'w> {
    near: Vec<(usize, &'w Chunk)>,
    far: Vec<(usize, Column<'w>)>,
}

/// A far chunk to send: the level's own while it holds it, else what the save's readers made.
enum Column<'w> {
    Level(&'w Chunk),
    Saved(Arc<[u8]>),
}

/// A body in a batch: a column of `Columns`, or one read from the save.
enum Body {
    Level(usize),
    Saved(Arc<[u8]>),
}

/// The level's columns a tick's batches send, each once, with the number of batches taking it.
#[derive(Default)]
struct Columns<'w> {
    chunks: Vec<&'w Chunk>,
    uses: Vec<usize>,
    index: crate::fast_hash::FxHashMap<(i32, i32), usize>,
}

impl<'w> Columns<'w> {
    fn add(&mut self, chunk: &'w Chunk) -> usize {
        let j = *self.index.entry((chunk.x, chunk.z)).or_insert_with(|| {
            self.chunks.push(chunk);
            self.uses.push(0);
            self.chunks.len() - 1
        });
        self.uses[j] += 1;
        j
    }
}

/// What a level's players were last told of its clock and weather.
#[derive(Clone, Copy)]
struct LevelSync {
    last_clock: i64,
    last_rate: f32,
    last_level_clock: (i64, f32),
    weather: (bool, f32, f32),
}

impl Default for LevelSync {
    fn default() -> Self {
        Self {
            last_clock: i64::MIN,
            last_rate: 0.0,
            last_level_clock: (i64::MIN, 0.0),
            weather: (false, 0.0, 0.0),
        }
    }
}

impl Hub {
    pub fn new(
        lan: Lan,
        sync: Arc<SyncData>,
        encoder: ChunkEncoder,
        config: HubConfig,
        loop_context: Arc<LoopContext>,
    ) -> Self {
        let chat_type = sync
            .id("minecraft:chat_type", "minecraft:chat")
            .unwrap_or(0);
        let dimension_type = sync
            .id(
                "minecraft:dimension_type",
                config.levels.dimension_type(&config.dimension),
            )
            .unwrap_or(0);
        let clock_ids = (0..sync.dynamic.len("minecraft:world_clock") as i32).collect();
        let mut encoder = encoder;
        encoder.set_dimension(config.levels.get(&config.dimension));
        Self {
            payload_player_epoch: None,
            lan,
            config,
            loop_context,
            encoder,
            guests: Default::default(),
            sync,
            rule_registry: crate::gamerules::rule_registry(),
            ticks: 0,
            sound_seeds: Mutex::new(crate::random_source::create()),
            sound_order: Mutex::default(),
            immediate_respawn: false,
            reduced_debug_info: false,
            limited_crafting: false,
            movement_checks: (true, true),
            anticheat: None,
            chat_filter: None,
            far: None,
            singleplayer_owner: None,
            split_players: Default::default(),
            flat_world: false,
            debug_world: false,
            riding: Default::default(),
            last_clock: i64::MIN,
            last_rate: 0.0,
            last_level_clock: (i64::MIN, 0.0),
            overworld_day: None,
            level_clock: None,
            ticking: (20.0, false),
            stepping: 0,
            ops: Default::default(),
            entity_range_percent: 100,
            spawn_protection: 0,
            chat_spam_threshold: 0,
            command_spam_threshold: 0,
            admin_feedback_to_ops: true,
            weather: (false, 0.0, 0.0),
            level_sync: Default::default(),
            chat_type,
            dimension_type,
            clock_ids,
            item_ids: Default::default(),
            entity_types: Default::default(),
            entities: Default::default(),
            joined: Default::default(),
            fresh_joined: Default::default(),
            synced: Vec::new(),
            entity_out: Vec::new(),
            entity_gone: Vec::new(),
            mod_commands: (serde_json::Value::Null, None),
            selector_tokens: Vec::new(),
            native_menu_events: Vec::new(),
            chunk_watches: ChunkWatchLog::default(),
            terrain: None,
            packet_drain: None,
            deferred_payloads: Default::default(),
            pending_kicks: Vec::new(),
            pending_logouts: Default::default(),
            failed_logout_saves: Default::default(),
            moves_listened: false,
            move_packets_listened: false,
            transfers: None,
            native_tracking: Vec::new(),
            attachment_snapshots: Vec::new(),
            sync_attachments: false,
            native_advancement_grants: Vec::new(),
            native_reload_pending: false,
            mods_loaded: false,
            pong_listened: false,
            hidden: Default::default(),
            refreshing: Vec::new(),
            teams: Vec::new(),
            nicknames: std::collections::HashMap::new(),
            list_names: std::collections::HashMap::new(),
            viewer_entities: std::collections::HashSet::new(),
            former_names: std::collections::HashMap::new(),
            viewed: Default::default(),
        }
    }

    pub fn guest(&self, conn: u32) -> Option<&Guest> {
        self.guests.get(self.guests.connection_index(conn)?)
    }
    pub fn update_guest<T>(
        &mut self,
        conn: u32,
        update: impl FnOnce(&mut Guest) -> T,
    ) -> Option<T> {
        self.guests.update_connection(conn, update)
    }

    /// The network id of a name in a built-in or synchronized registry.
    pub fn registry_id(&self, registry: &str, name: &str) -> Option<i32> {
        self.sync.id(registry, name)
    }

    /// The name of a network id in a built-in registry.
    pub fn registry_name(&self, registry: &str, id: i32) -> Option<&str> {
        self.sync.builtin.get(registry)?.name(id)
    }

    pub fn item_id(&mut self, item: &Arc<str>) -> i32 {
        if let Some(id) = self.item_ids.get(item) {
            return *id;
        }
        let id = self
            .sync
            .builtin
            .get("minecraft:item")
            .and_then(|i| i.id(item))
            .unwrap_or(0);
        self.item_ids.insert(item.clone(), id);
        id
    }
    pub fn stack(&mut self, s: &Option<ItemStack>) -> Option<horizon_protocol::item::Stack> {
        let item = s.as_ref()?.item.clone();
        let id = self.item_id(&item);
        let sync = self.sync.clone();
        // The stacks inside it (a bundle's contents, a crossbow's projectiles)
        // have ids of their own.
        let item_id = |name: &str| {
            if name == &*item {
                return id;
            }
            sync.builtin
                .get("minecraft:item")
                .and_then(|i| i.id(name))
                .unwrap_or(0)
        };
        stack_to_network(item_id, |registry, name| sync.id(registry, name), s)
    }

    /// `PlayerList.broadcastAll`: send a packet to every placed player.
    pub fn broadcast(&self, packet: &C) {
        self.broadcast_to(packet, self.online_players());
    }
    /// [`Self::broadcast`] to the held players as well, which keep a picture of
    /// the players and the entities while they wait for their hand-over.
    pub fn broadcast_watching(&self, packet: &C) {
        self.broadcast_to(
            packet,
            self.guests
                .simulation_iter()
                .filter(|g| g.participates_online() && (g.placed || g.held)),
        );
    }
    fn broadcast_to<'g>(&self, packet: &C, guests: impl IntoIterator<Item = &'g Guest>) {
        let mut body = None;
        for g in guests {
            if let C::RemoveEntities(p) = packet {
                self.send_ordered_removal(g, &p.ids);
            } else {
                g.sender
                    .send(body.get_or_insert_with(|| packet.encode()).clone());
            }
        }
    }
    /// MinecraftServer.onGameRuleChanged: every player is told when
    /// `immediate_respawn` changes; a joining player's Login carries it.
    pub fn set_immediate_respawn(&mut self, on: bool) {
        if self.immediate_respawn != on {
            self.immediate_respawn = on;
            self.broadcast(&C::GameEvent(play::GameEvent {
                event: game_event::IMMEDIATE_RESPAWN,
                param: if on { 1.0 } else { 0.0 },
            }));
        }
    }
    /// MinecraftServer.onGameRuleChanged: every player gets entity event 22
    /// (reduced) or 23 (full) for themselves when `reduced_debug_info`
    /// changes; a joining player's Login carries it.
    pub fn set_reduced_debug_info(&mut self, on: bool) {
        if self.reduced_debug_info != on {
            self.reduced_debug_info = on;
            for g in self.guests.iter().filter(|g| g.placed) {
                g.send(&C::EntityEvent(play::EntityEvent {
                    id: g.entity_id,
                    event: if on { 22 } else { 23 },
                }));
            }
        }
    }
    /// `MinecraftServer.onGameRuleChanged`: every player is told when
    /// `limited_crafting` changes; a joining player's Login carries it.
    pub fn set_limited_crafting(&mut self, on: bool) {
        if self.limited_crafting != on {
            self.limited_crafting = on;
            self.broadcast(&C::GameEvent(play::GameEvent {
                event: game_event::LIMITED_CRAFTING,
                param: if on { 1.0 } else { 0.0 },
            }));
        }
    }
    /// PlayerList.setViewDistance: every player is told the new radius,
    /// and each one's chunks follow it, up to what they asked for
    /// (ChunkMap.getPlayerViewDistance).
    pub fn set_view_distance(&mut self, distance: i32) {
        self.config.view_distance = distance;
        let distance = self.chunk_radius();
        self.broadcast(&C::SetChunkCacheRadius(play::Distance { distance }));
        self.guests.update_all(|g| {
            // Mth.clamp(requested, 2, serverViewDistance): Java's max then min order also
            // handles a management value below two without panicking when the bounds cross.
            g.view_distance = i32::from(g.info.view_distance)
                .max(2)
                .min(self.config.view_distance);
            g.center = None;
        });
    }
    /// The radius the players are told (Login, SetChunkCacheRadius): with the far ring on, its
    /// `max_view_distance`, so that a client keeps and draws what it is sent past the view.
    fn chunk_radius(&self) -> i32 {
        let view = self.config.view_distance;
        self.far.as_ref().map_or(view, |far| far.max.max(view))
    }
    /// How far a player is sent chunks: its view, and with the far ring on, as far as it asked for
    /// up to `max_view_distance`.
    fn reach(&self, g: &Guest) -> i32 {
        self.far.as_ref().map_or(g.view_distance, |far| {
            i32::from(g.info.view_distance)
                .min(far.max)
                .max(g.view_distance)
        })
    }
    /// Where the far ring reads the level in place from: its saves, before each of its ticks.
    pub fn far_level(&mut self, save: &Arc<crate::save::SaveDir>) {
        if let Some(far) = self.far.as_mut() {
            far.set_level(&self.config.dimension, save, &self.encoder);
        }
    }
    /// PlayerList.setSimulationDistance.
    pub fn set_simulation_distance(&mut self, distance: i32) {
        self.config.simulation_distance = distance;
        self.broadcast(&C::SetSimulationDistance(play::Distance { distance }));
    }
    /// Resolve a player attachment holder in the level in place.
    pub fn holder_guest(&self, uuid: u128, dimension: &str) -> Option<&Guest> {
        self.guests
            .iter()
            .find(|g| g.placed && g.uuid == uuid && g.participates_online())
            .or_else(|| {
                self.guests.iter().find(|g| {
                    g.placed
                        && g.uuid == uuid
                        && !g.participates_online()
                        && g.dimension == dimension
                })
            })
    }

    /// Placed network players in join order.
    pub fn online_players(&self) -> impl Iterator<Item = &Guest> {
        self.guests
            .simulation_iter()
            .filter(|g| g.placed && g.participates_online())
    }
    /// The number of network guests, including those still joining.
    pub fn online_count(&self) -> usize {
        self.guests
            .simulation_iter()
            .filter(|g| g.participates_online())
            .count()
    }
    /// Whether an online guest is placed in the level in place (`config.dimension`).
    pub fn here(&self, g: &Guest) -> bool {
        g.placed_in(&self.config.dimension)
    }
    /// How many passes the level in place's entity tracking has run (None before its first).
    pub fn tracking_ticks(&self) -> Option<u32> {
        self.entities
            .get(&self.config.dimension)
            .map(Entities::ticks)
    }
    /// Whether a guest is sent the level in place: the players placed in it, and
    /// the held ones, which look at it without being part of it.
    pub fn watches(&self, g: &Guest) -> bool {
        g.watches_level(&self.config.dimension)
    }
    /// The chunk under each player the level in place has to keep loaded: the players placed in
    /// it and the held ones, which are sent its chunks too ([`Self::watches`]), whose `loads`
    /// says so (a spectator may not load chunks). A server with only a held player must load
    /// and generate the world around it, or the player waits for ever for its world.
    pub fn loading_chunks(
        &self,
        loads: impl Fn(&Guest) -> bool,
    ) -> impl Iterator<Item = (i32, i32)> {
        self.guests
            .simulation_iter()
            .filter(move |g| self.watches(g) && loads(g))
            // A mod's view centre loads as the player's chunk does; the player's own stays loaded.
            .flat_map(|g| std::iter::once(g.chunk()).chain(g.view_center))
    }
    /// `PlayerList.broadcastAll(packet, dimension)`: the players in the level in place.
    pub fn broadcast_level(&self, packet: &C) {
        let mut body = None;
        for g in self.guests.simulation_iter().filter(|g| self.watches(g)) {
            g.sender
                .send(body.get_or_insert_with(|| packet.encode()).clone());
        }
    }
    /// `DedicatedServer.sendLowDiskSpaceWarning`: the players with
    /// `PermissionLevel.ADMINS` (3) or more get `ClientboundLowDiskSpaceWarningPacket`.
    pub fn send_low_disk_space_warning(&self) {
        let body = C::LowDiskSpaceWarning(horizon_protocol::packets::Empty).encode();
        for g in self.guests.iter().filter(|g| g.placed && g.permission >= 3) {
            g.sender.send(body.clone());
        }
    }
    pub fn broadcast_except(&self, conn: u32, packet: &C) {
        let mut body = None;
        for g in self
            .guests
            .simulation_iter()
            .filter(|g| g.placed && g.conn != conn && g.participates_online())
        {
            g.sender
                .send(body.get_or_insert_with(|| packet.encode()).clone());
        }
    }
    /// PlayerList.broadcastSystemMessage.
    pub fn system_message(&self, content: Tag) {
        self.system_message_to(content, |_| true);
    }
    /// A system message for the guests `hears` picks by name
    /// (`PlayerList.broadcastSystemToTeam` and its kin).
    pub fn system_message_to(&self, content: Tag, hears: impl Fn(&str) -> bool) {
        self.send_system(content, |g| hears(&g.name));
    }
    /// `ServerPlayer.sendSystemMessage` to the guest with `uuid`, in any dimension.
    pub fn system_message_to_player(&self, uuid: u128, content: Tag) {
        self.send_system(content, |g| g.uuid == uuid);
    }
    /// `ServerPlayer.sendSystemMessage(Component, false)` to the placed guests `to` picks: not to one whose
    /// chat is hidden (`acceptsSystemMessages`: HIDDEN, 2, takes only an overlay).
    fn send_system(&self, content: Tag, to: impl Fn(&Guest) -> bool) {
        let body = C::SystemChat(play::SystemChat {
            content,
            overlay: false,
        })
        .encode();
        for g in self
            .guests
            .iter()
            .filter(|g| g.placed && g.info.chat_visibility != 2 && to(g))
        {
            g.sender.send(body.clone());
        }
    }
    /// `ServerPlayer.sendOverlayMessage` to every guest (the action bar).
    pub fn overlay_message(&self, content: Tag) {
        self.broadcast(&C::SystemChat(play::SystemChat {
            content,
            overlay: true,
        }));
    }
    /// A chat message from the host (a disguised chat: the host has no connection to sign with).
    pub fn chat(&self, sender: &str, text: &str) {
        self.broadcast(&C::DisguisedChat(play::DisguisedChat {
            message: literal(text),
            chat_type: play::ChatTypeBound {
                chat_type: self.chat_type,
                name: literal(sender),
                target: None,
            },
        }));
    }
    /// ListPlayersCommand's reply: the players in join order (the host of a
    /// LAN world first), as display names (Player.getDisplayName, with the
    /// team's colour and affixes) or names with ids, joined by
    /// gray commas (ComponentUtils.formatList).
    pub fn list_reply(&self, uuids: bool) -> Tag {
        let players: Vec<(u128, &str)> = self
            .online_players()
            .map(|g| (g.uuid, g.name.as_str()))
            .collect();
        let format = |(uuid, name): (u128, &str)| {
            if uuids {
                Tag::compound([
                    ("translate", string("commands.list.nameAndId")),
                    (
                        "with",
                        Tag::List(vec![
                            string(name),
                            string(&horizon_protocol::crypto::uuid_string(uuid)),
                        ]),
                    ),
                ])
            } else {
                self.player_display_name(uuid, name)
            }
        };
        let names = match players.len() {
            0 => string(""),
            1 => format(players[0]),
            _ => {
                let mut extra = Vec::new();
                for (k, p) in players.iter().enumerate() {
                    if k > 0 {
                        extra.push(Tag::compound([
                            ("text", string(", ")),
                            ("color", string("gray")),
                        ]));
                    }
                    extra.push(format(*p));
                }
                Tag::compound([("text", string("")), ("extra", Tag::List(extra))])
            }
        };
        Tag::compound([
            ("translate", string("commands.list.players")),
            (
                "with",
                Tag::List(vec![
                    Tag::Int(players.len() as i32),
                    Tag::Int(self.lan.shared.max_players() as i32),
                    names,
                ]),
            ),
        ])
    }

    /// SayCommand from a source without a player: a disguised chat of type
    /// say_command named after the source.
    pub fn say(&self, source: &str, text: &str) {
        let chat_type = self
            .sync
            .id("minecraft:chat_type", "minecraft:say_command")
            .unwrap_or(self.chat_type);
        self.broadcast(&C::DisguisedChat(play::DisguisedChat {
            message: literal(text),
            chat_type: play::ChatTypeBound {
                chat_type,
                name: literal(source),
                target: None,
            },
        }));
    }
    /// Disconnect a guest with a reason (ServerGamePacketListenerImpl.disconnect), once
    /// ([`Guest::begin_disconnect`]): a guest already on the way out keeps its first reason.
    pub fn kick(&mut self, conn: u32, reason: Tag) {
        let text = self.english(&reason);
        let started = self.update_guest(conn, |g| {
            g.begin_disconnect(text.clone(), reason)
                .then(|| (g.uuid, g.name.clone()))
        });
        if let Some((player, name)) = started.flatten() {
            self.pending_kicks.push(HostEvent::Kicked {
                conn,
                player,
                name,
                reason: text,
            });
        }
    }

    /// `TransferCommand.transfer`: the guest's client is handed to another
    /// server (`ClientboundTransferPacket`, the play state's `Transfer`); the
    /// client closes the connection and logs in there. Nothing disconnects the
    /// player here: as in vanilla, the close comes from the client.
    pub fn transfer(&self, conn: u32, host: &str, port: i32) {
        self.send_to(
            conn,
            &C::Transfer(horizon_protocol::packets::configuration::Transfer {
                host: host.to_owned(),
                port,
            }),
        );
    }

    /// Disconnects the held player `uuid` and nothing else: no save (it was
    /// never placed), no word to anyone (no one was told of it). Returns whether
    /// there was one.
    pub fn drop_held(&mut self, uuid: u128) -> bool {
        let Some(index) = self
            .guests
            .iter()
            .position(|g| g.uuid == uuid && g.held && !g.placed)
        else {
            return false;
        };
        self.guests.update(index, |g| {
            // Neither a confirm nor an import may take it while its connection closes.
            g.imported = false;
            if g.left_reason.is_empty() {
                g.left_reason = "dropped before its hand-over".to_owned();
            }
            g.sender
                .disconnect(translatable("multiplayer.disconnect.generic", &[]));
            true
        })
    }

    /// Lets go of what a proxy that stopped answering left behind: the players
    /// it said would come and did not, the held players whose hand-over never
    /// finished, and the freezes no hand-over ended. None may last for ever, and
    /// a held player goes without a word to anyone, since no one was told of it.
    fn expire_arrivals(&mut self, events: &mut Vec<HostEvent>) {
        // Off: nothing to do (and no player can be frozen).
        let Some(transfers) = &self.transfers else {
            return;
        };
        for uuid in transfers.table().expire(std::time::Instant::now()) {
            events.push(HostEvent::Warn(format!(
                "player {} was expected from another server and never came",
                horizon_protocol::crypto::uuid_string(uuid)
            )));
        }
        let (max, max_hold) = (
            transfers.settings.max_freeze_ticks,
            transfers.settings.max_hold_ticks,
        );
        // Not one already kicked or dropped: its connection may take a while to close,
        // and it must not be kicked (and reported) again on every tick.
        let stale: Vec<u32> = self
            .guests
            .iter()
            .filter(|g| {
                g.held
                    && !g.placed
                    && g.left_reason.is_empty()
                    && super::arrivals::held_too_long(g.held_since, self.ticks, max_hold)
            })
            .map(|g| g.conn)
            .collect();
        for conn in stale {
            if let Some(g) = self.guest(conn) {
                events.push(HostEvent::Warn(format!(
                    "{} was held for {max_hold} ticks and the hand-over never finished: dropped",
                    g.name,
                )));
            }
            self.kick(conn, translatable("disconnect.timeout", &[]));
        }
        self.guests.update_all(|g| {
            let Some(h) = &g.handover else {
                return;
            };
            // A frozen player takes no damage, but the void and `/kill` still kill: its
            // record is then not what is left of it, and `player-release moved` must fail.
            if g.dead {
                g.handover = None;
                events.push(HostEvent::Warn(format!(
                    "{} died while frozen for a hand-over: the hand-over is over",
                    g.name,
                )));
            } else if super::arrivals::frozen_too_long(h.since, self.ticks, max) {
                g.handover = None;
                events.push(HostEvent::Warn(format!(
                    "{} was frozen for a hand-over for {max} ticks and nothing finished it: let go",
                    g.name,
                )));
            }
        });
    }

    /// Whether a player of `permission` sees the player `uuid`: not one a
    /// native mod hid from players below a higher level.
    pub fn sees(&self, permission: u8, uuid: u128) -> bool {
        // A player being sent again (`refresh_player`) is out of everyone's lists for that time; the level a
        // mod hid them at is a separate thing and stays what the mod set.
        !self.refreshing.iter().any(|r| r.0 == uuid)
            && self
                .hidden
                .get(&uuid)
                .is_none_or(|&level| permission >= level)
    }

    /// Paper's `Player.hideEntity` and `showEntity`: the entity `entity` is never paired with
    /// the client of the placed player `viewer` while hidden (`Tracker::hidden`), and is paired
    /// again once shown. `None` when no such player is placed; otherwise whether it changed. A
    /// change has the tracker check that client's pairs again on the next pass. Kept until the
    /// player leaves.
    pub fn set_entity_hidden_from(
        &mut self,
        viewer: u128,
        entity: u128,
        hidden: bool,
    ) -> Option<bool> {
        let index = (0..self.guests.len())
            .find(|&i| self.guests[i].uuid == viewer && self.guests[i].placed)?;
        Some(self.guests.update(index, |g| {
            let changed = g.tracker.set_entity_hidden(entity, hidden);
            g.moved |= changed;
            changed
        }))
    }
    /// Whether `Player.hideEntity` hid the entity `entity` from the placed player `viewer`.
    pub fn entity_hidden_from(&self, viewer: u128, entity: u128) -> Option<bool> {
        let guest = self.guests.iter().find(|g| g.uuid == viewer && g.placed)?;
        Some(guest.tracker.entity_hidden(entity))
    }

    /// The player lists after `hidden` or a permission changed: each placed
    /// player loses the players hidden from it (ClientboundPlayerInfoRemove)
    /// and gets back those it sees again (an ADD_PLAYER entry). A change
    /// checks the client's entity pairs again, so the tracker follows too.
    /// Nothing runs while nobody is hidden.
    fn sync_hidden(&mut self) {
        if self.hidden.is_empty()
            && self.refreshing.is_empty()
            && self.guests.iter().all(|g| g.tab_hidden.is_empty())
        {
            return;
        }
        let candidates: Vec<usize> = (0..self.guests.len())
            .filter(|&j| {
                let u = self.guests[j].uuid;
                self.guests[j].placed
                    && self.guests[j].participates_online()
                    && (self.hidden.contains_key(&u)
                        || self.refreshing.iter().any(|r| r.0 == u)
                        || self.guests.iter().any(|g| g.tab_hidden.contains(&u)))
            })
            .collect();
        for i in 0..self.guests.len() {
            // A held client (waiting for its hand-over) keeps a picture of the players, the refresh included.
            if !(self.guests[i].placed || self.guests[i].held)
                || !self.guests[i].participates_online()
            {
                continue;
            }
            for &j in &candidates {
                let (viewer, other) = (&self.guests[i], &self.guests[j]);
                if i == j {
                    continue;
                }
                let hide = !self.sees(viewer.permission, other.uuid);
                if hide == viewer.tab_hidden.contains(&other.uuid) {
                    continue;
                }
                let packet = if hide {
                    C::PlayerInfoRemove(play::PlayerInfoRemove {
                        uuids: vec![other.uuid],
                    })
                } else {
                    C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                        actions: Self::INFO_ACTIONS,
                        entries: vec![self.player_info(
                            other.uuid,
                            &other.name,
                            other.game_mode,
                            other.latency.load(Ordering::Relaxed),
                        )],
                    })
                };
                let uuid = other.uuid;
                self.guests.update(i, |viewer| {
                    viewer.send(&packet);
                    if hide {
                        viewer.tab_hidden.insert(uuid);
                    } else {
                        viewer.tab_hidden.remove(&uuid);
                    }
                    viewer.moved = true;
                });
            }
        }
    }

    fn player_info(&self, uuid: u128, name: &str, mode: i32, latency: i32) -> play::PlayerInfo {
        let guest = self
            .guests
            .iter()
            .find(|g| g.uuid == uuid && g.participates_online());
        play::PlayerInfo {
            uuid,
            name: Some(name.to_owned()),
            properties: guest
                .map(|g| {
                    listed_properties(
                        &g.properties,
                        &g.profile_overrides,
                        self.singleplayer_owner == Some(g.uuid),
                        self.lan.shared.config.owner_profile.as_deref(),
                    )
                })
                .unwrap_or_default(),
            chat_session: guest.and_then(|g| g.chain.session()).map(|s| s.data()),
            game_mode: Some(mode),
            listed: Some(true),
            latency: Some(latency),
            display_name: Some(self.tab_list_name(uuid, name)),
            list_order: Some(0),
            show_hat: Some(guest.is_none_or(|g| g.info.model_customisation & 0x40 != 0)),
        }
    }
    /// ServerPlayer.getTabListDisplayName: a mod's list name, else the nickname, if any, as
    /// mods format it.
    /// The Tab list name before the format hook: a mod's list name, else the nickname.
    fn tab_list_base(&self, uuid: u128) -> Option<crate::text::Text> {
        self.list_names
            .get(&uuid)
            .cloned()
            .or_else(|| self.nicknames.get(&uuid).map(crate::text::Text::literal))
    }
    fn tab_list_name(&self, uuid: u128, name: &str) -> Option<Tag> {
        let mut display_name = self.tab_list_base(uuid);
        native_player_tab_list_name_format(&self.loop_context, uuid, name, &mut display_name);
        display_name.map(|text| crate::scoreboard::component_tag(&text.component()))
    }
    /// ClientboundPlayerInfoUpdatePacket.createPlayerInitializing: every action.
    const INFO_ACTIONS: u8 = 0xFF;

    /// The player list entries to the guest at `i`, in as many packets as the frame limit needs: several
    /// players with profiles of 16 properties each can pass it together, and a frame too large closes
    /// the connection (`info_batches`).
    fn send_player_list(&self, i: usize, entries: Vec<play::PlayerInfo>) {
        for entries in info_batches(Self::INFO_ACTIONS, entries) {
            self.guests[i].send(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                actions: Self::INFO_ACTIONS,
                entries,
            }));
        }
    }

    // ---- joining -------------------------------------------------------

    fn admit(&mut self, j: super::Joined, ctx: &mut TickCtx) -> u32 {
        // The game's ids are taken by incrementing first: the current value
        // is the last id in use (the host's own, right after opening to LAN).
        *ctx.next_entity += 1;
        let entity_id = *ctx.next_entity as i32;
        let conn = j.conn;
        let mut guest = Guest::new(j, entity_id, ctx.spawn, self.config.view_distance);
        self.bind_raw_player(&guest, entity_id);
        guest.last_action_tick = self.ticks;
        guest.game_mode = self.config.default_game_mode;
        guest.abilities.update_for_mode(guest.game_mode);
        guest.permission = self.default_permission(guest.uuid);
        guest.joined_tick = self.ticks;
        guest.position = ctx.spawn;
        guest.previous_position = ctx.spawn;
        guest.dimension = self.config.dimension.clone();
        self.guests.push(guest);
        conn
    }

    /// The permission level a player gets: an integrated server gives every
    /// guest the game masters' level when commands are allowed
    /// (IntegratedServer.setGuestCommandAccess); a dedicated server its
    /// operators' level (`ops`).
    fn default_permission(&self, uuid: u128) -> u8 {
        if self.config.integrated {
            // IntegratedServer.getProfilePermissions / getCustomPermissionLevel.
            let guests = if self.config.guest_commands { 2 } else { 0 };
            self.ops.get(&uuid).copied().unwrap_or(guests)
        } else {
            self.ops.get(&uuid).copied().unwrap_or(0)
        }
    }

    /// PlayerList.placeNewPlayer for a guest whose saved state is loaded.
    pub fn place_new_player(&mut self, conn: u32, ctx: &mut TickCtx) {
        let Some(i) = self.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        if !self.guests[i].participates_online() {
            return;
        }
        let (name, uuid, entity_id, mode) = {
            let g = &self.guests[i];
            (g.name.clone(), g.uuid, g.entity_id, g.game_mode)
        };
        // A player whose login used up a proxy's expectation (`arrivals`, as the
        // login started) is held: it gets the whole join sequence, so it can build
        // its picture of the world, but no one else is told about it.
        if self.guests[i].expected {
            self.guests.update(i, |g| {
                g.held = true;
                g.held_since = self.ticks;
            });
        }
        let levels = self.level_keys();
        self.guests
            .update(i, |g| g.viewed.logged_in_with(levels.clone()));
        let held = self.guests[i].held;
        let g = &self.guests[i];
        g.send(&C::Login(play::Login {
            player_id: entity_id,
            hardcore: self.config.hardcore,
            levels,
            max_players: i32::try_from(self.lan.shared.max_players()).unwrap_or(i32::MAX),
            chunk_radius: self.chunk_radius(),
            simulation_distance: self.config.simulation_distance,
            reduced_debug_info: self.reduced_debug_info,
            show_death_screen: !self.immediate_respawn,
            limited_crafting: self.limited_crafting,
            spawn: self.spawn_info(i, 0, ctx.sea_level),
            // MinecraftServer.usesAuthentication, enforceSecureProfile.
            online_mode: self.lan.shared.config.online.is_some(),
            enforces_secure_chat: self.enforces_secure_chat(),
        }));
        g.send(&C::ChangeDifficulty(play::ChangeDifficulty {
            difficulty: ctx.difficulty,
            locked: ctx.difficulty_locked,
        }));
        g.send(&g.abilities_packet());
        g.send(&C::SetHeldSlot(play::SetHeldSlot {
            slot: g.selected as i32,
        }));
        // PlayerList.placeNewPlayer: ClientboundUpdateRecipesPacket.
        g.send(&raw(ids::UPDATE_RECIPES, self.sync.recipes.clone()));
        // OnDatapackSyncEvent.sendRecipes: the recipes the native mods asked
        // for, to a Horizon client only.
        if g.brand == "horizon" {
            super::recipes::send_content(g);
            super::recipes::advertise_viewer_channels(g);
        }
        // PlayerList.sendPlayerPermissionLevel, then the recipe book.
        self.send_permission_level(i);
        self.send_initial_recipe_book(i);
        // PlayerList.placeNewPlayer broadcasts the join line before
        // players.add: the others read it, the newcomer doesn't (nor those
        // a native mod hides it from). A held player is not announced, and
        // not placed.
        if !held {
            let joined = C::SystemChat(play::SystemChat {
                content: self.joined_line(uuid, &name),
                overlay: false,
            })
            .encode();
            for g in self
                .guests
                .iter()
                .filter(|g| g.placed && g.conn != conn && g.participates_online())
            {
                if self.sees(g.permission, uuid) {
                    g.sender.send(joined.clone());
                }
            }
            {
                let value = true;
                self.guests.update(i, |guest| guest.placed = value);
            }
        }
        self.resend_teleport(i, None);
        let g = &self.guests[i];
        g.send(&C::ServerData(play::ServerData {
            motd: literal(&self.lan.shared.motd()),
            icon: self.lan.shared.config.favicon.clone(),
        }));
        // The player list: everyone already here, then the newcomer to all.
        let mut entries = Vec::new();
        // Players a native mod hid from the newcomer stay out of its list,
        // and a hidden newcomer out of theirs.
        let permission = self.guests[i].permission;
        let mut hidden = Vec::new();
        for o in self
            .guests
            .iter()
            .filter(|o| o.conn != conn && o.placed && o.participates_online())
        {
            if !self.sees(permission, o.uuid) {
                hidden.push(o.uuid);
                continue;
            }
            entries.push(self.player_info(
                o.uuid,
                &o.name,
                o.game_mode,
                o.latency.load(Ordering::Relaxed),
            ));
        }
        self.guests.update(i, |g| g.tab_hidden.extend(hidden));
        self.send_player_list(i, entries);
        let newcomer = C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: Self::INFO_ACTIONS,
            entries: vec![self.player_info(uuid, &name, mode, 0)],
        })
        .encode();
        if held {
            // Its own list has itself in it; the others' lists do not.
            self.guests[i].sender.send(newcomer);
        } else {
            let level = self.hidden.get(&uuid).copied();
            // A held player is shown the entities around it, so it must know the players
            // who join meanwhile, or its client drops their entity (`ClientPacketListener`
            // adds a player only with its list entry).
            self.guests.update_all(|g| {
                if (g.placed || g.held) && g.participates_online() {
                    if level.is_some_and(|l| g.permission < l) {
                        g.tab_hidden.insert(uuid);
                    } else {
                        g.sender.send(newcomer.clone());
                    }
                }
            });
        }
        self.send_level_info(i, ctx, true);
        // ServerLevel.addNewPlayer: the chunk tracking starts at the player (or a mod's view
        // centre, kept through a placement in the same level).
        self.guests.update(i, |g| {
            let (cx, cz) = g.view_chunk();
            g.send(&C::SetChunkCacheCenter(play::SetChunkCacheCenter {
                x: cx,
                z: cz,
            }));
            g.center_sent = Some((cx, cz));
            g.placed_at = self.ticks;
            g.level_entered = self.ticks;
            g.last_reported = (g.position, [g.yaw, g.pitch]);
        });
        self.send_active_effects(i);
        // ServerPlayer.sendPostEffects and initInventoryMenu.
        self.send_post_effects(i);
        self.send_inventory(i);
    }

    /// What an import changed, told to the held player's client the way a join tells it: game
    /// mode, abilities, held slot, a position to confirm, effects, inventory, and the player
    /// list as it is now (who came while it was held is in it).
    ///
    /// When the import moved the player to another chunk than the one its view was built around,
    /// the view is moved at once (the chunk centre is sent and the chunks out of reach are
    /// forgotten, as the next tick would do), and this returns true: the chunks around the new
    /// place are still to come, one batch at a time, and whoever reads them must wait for them.
    pub fn send_arrival_state(&mut self, i: usize) -> bool {
        let (mode, conn) = {
            let g = &self.guests[i];
            (g.game_mode, g.conn)
        };
        // The effects the login sent from this server's own file and the import took away:
        // `send_active_effects` only adds, and the client would go on predicting with them.
        let gone: Vec<&'static str> = {
            let g = &self.guests[i];
            g.sent_effects
                .iter()
                .filter(|s| !g.effects.list.iter().any(|e| e.id == s.id))
                .map(|s| s.id)
                .collect()
        };
        for id in gone {
            if let Some(effect) = self
                .sync
                .id("minecraft:mob_effect", crate::brewing::namespaced(id))
            {
                let g = &self.guests[i];
                g.send(&C::RemoveMobEffect(play::RemoveMobEffect {
                    id: g.entity_id,
                    effect,
                }));
            }
        }
        let recentered = self.guests[i]
            .center
            .is_some_and(|c| c != self.guests[i].view_chunk());
        if recentered {
            self.update_view(i, self.chunk_watches.recording());
        }
        let g = &self.guests[i];
        g.send(&C::GameEvent(play::GameEvent {
            event: game_event::CHANGE_GAME_MODE,
            param: mode as f32,
        }));
        g.send(&g.abilities_packet());
        g.send(&C::SetHeldSlot(play::SetHeldSlot {
            slot: g.selected as i32,
        }));
        self.resend_teleport(i, None);
        self.send_active_effects(i);
        self.send_inventory(i);
        // The login sent the book of this server's file; the import may have brought another
        // (`ServerRecipeBook.sendInitialRecipeBook` replaces what the client has).
        self.send_initial_recipe_book(i);
        let permission = self.guests[i].permission;
        let entries: Vec<_> = self
            .guests
            .iter()
            .filter(|o| {
                o.conn != conn
                    && o.placed
                    && o.participates_online()
                    && self.sees(permission, o.uuid)
            })
            .map(|o| {
                self.player_info(
                    o.uuid,
                    &o.name,
                    o.game_mode,
                    o.latency.load(Ordering::Relaxed),
                )
            })
            .collect();
        self.send_player_list(i, entries);
        recentered
    }

    /// Places the held player at `i`: it is simulated, shown to the others and takes part from
    /// the next tick, when its first tick does what a new player's does (health, experience,
    /// advancements). Everyone gets its player-list entry, since everyone will be sent its entity
    /// (a client adds a player's entity only with its list entry); `announce` is the join line.
    pub fn place_arrival(&mut self, i: usize, announce: bool) {
        if !self.guests[i].held || self.guests[i].placed {
            return;
        }
        let (name, uuid, mode, conn) = {
            let g = &self.guests[i];
            (g.name.clone(), g.uuid, g.game_mode, g.conn)
        };
        if announce {
            let joined = C::SystemChat(play::SystemChat {
                content: self.joined_line(uuid, &name),
                overlay: false,
            })
            .encode();
            for g in self
                .guests
                .iter()
                .filter(|g| g.placed && g.participates_online() && g.conn != conn)
            {
                if self.sees(g.permission, uuid) {
                    g.sender.send(joined.clone());
                }
            }
        }
        self.guests.update(i, |g| {
            g.held = false;
            g.imported = false;
            g.placed = true;
            g.placed_at = self.ticks;
            g.level_entered = self.ticks;
            g.last_reported = (g.position, [g.yaw, g.pitch]);
        });
        // The player list entry of the newcomer, to everyone placed or held (itself included).
        let newcomer = C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: Self::INFO_ACTIONS,
            entries: vec![self.player_info(uuid, &name, mode, 0)],
        })
        .encode();
        let level = self.hidden.get(&uuid).copied();
        self.guests.update_all(|g| {
            if g.participates_online() && (g.placed || g.held) {
                if level.is_some_and(|l| g.permission < l) {
                    g.tab_hidden.insert(uuid);
                } else {
                    g.sender.send(newcomer.clone());
                }
            }
        });
    }

    /// The guest index of the player `uuid` placed from the hand-over `id` (its import or
    /// confirmation committed): a retry of either, whose reply was lost, is answered the same.
    pub fn placed_arrival(&self, uuid: u128, id: u128) -> Option<usize> {
        self.guests
            .iter()
            .position(|g| g.uuid == uuid && g.placed && g.arrival_handover == Some(id))
    }

    /// The guest index of the held player `uuid` whose record of the hand-over `id` was
    /// imported, and that has not been dropped or kicked meanwhile.
    pub fn imported_arrival(&self, uuid: u128, id: u128) -> Result<usize, &'static str> {
        let Some(i) = self
            .guests
            .iter()
            .position(|g| g.uuid == uuid && g.held && !g.placed)
        else {
            return Err("no player with that unique id is held");
        };
        let g = &self.guests[i];
        if !g.left_reason.is_empty() {
            Err("that player is leaving")
        } else if !g.imported {
            Err("that player is held but its record has not been imported")
        } else if g.arrival_handover != Some(id) {
            Err("that player's record is of another hand-over")
        } else {
            Ok(i)
        }
    }

    /// Places the held player `uuid` whose record was imported without placing it
    /// (`player-confirm`). Returns whether the others were told it came, as the import asked.
    ///
    /// What the client did since the import was not acted on, so it is put back where the
    /// server has it: the position to confirm again and the inventory.
    pub fn confirm_arrival(&mut self, uuid: u128, id: u128) -> Result<bool, &'static str> {
        let i = self.imported_arrival(uuid, id)?;
        let announce = self.guests[i].arrival_announce;
        self.resend_teleport(i, None);
        self.send_inventory(i);
        self.place_arrival(i, announce);
        Ok(announce)
    }

    /// `ServerPlayer.sendPostEffects`: the player's `/posteffect` chains.
    pub fn send_post_effects(&mut self, i: usize) {
        self.guests.update(i, |g| {
            g.post_effects_dirty = false;
            let effects = g.post_effects.clone();
            g.send(&C::PostEffects(play::PostEffects { effects }));
        })
    }

    /// ClientboundUpdateMobEffectPacket for one of the guest's effects.
    fn send_effect(&mut self, i: usize, e: &EffectInstance, blend: bool) -> Option<SentEffect> {
        let effect = self
            .sync
            .id("minecraft:mob_effect", crate::brewing::namespaced(e.id))?;
        let flags = PlayerEffects::flags(e);
        let g = &self.guests[i];
        g.send(&C::UpdateMobEffect(play::UpdateMobEffect {
            id: g.entity_id,
            effect,
            amplifier: e.amplifier,
            duration: e.duration,
            flags: flags | if blend { 8 } else { 0 },
        }));
        Some(SentEffect {
            id: e.id,
            amplifier: e.amplifier,
            flags,
            duration: e.duration,
            tick_count: g.effects.tick_count,
        })
    }

    /// PlayerList.sendActivePlayerEffects: every effect, unblended, when the
    /// player is placed (or respawns).
    pub fn send_active_effects(&mut self, i: usize) {
        let effects = self.guests[i].effects.list.clone();
        let sent = effects
            .iter()
            .filter_map(|e| self.send_effect(i, e, false))
            .collect();
        {
            let value = sent;
            self.guests.update(i, |guest| guest.sent_effects = value);
        }
    }

    /// ServerPlayer.onEffectAdded (blended), onEffectUpdated and
    /// onEffectsRemoved, from what changed since the client was told: an
    /// effect that only counted down is not sent again, except at every
    /// 600 ticks left (`LivingEntity.tickEffects`).
    /// `sync_effects` for every player at once: what a command changed
    /// leaves with its feedback (ServerPlayer.onEffectAdded sends at once),
    /// not after the tick's countdown.
    pub fn sync_all_effects(&mut self) {
        for i in 0..self.guests.len() {
            if self.guests[i].placed {
                self.sync_effects(i);
            }
        }
    }
    fn sync_effects(&mut self, i: usize) {
        let g = &self.guests[i];
        if g.effects.list.is_empty() && g.sent_effects.is_empty() {
            return;
        }
        let now = g.effects.tick_count;
        let mut sent = Vec::with_capacity(g.effects.list.len());
        let mut changes = Vec::new();
        for e in &g.effects.list {
            match g.sent_effects.iter().find(|s| s.id == e.id) {
                None => changes.push((e.clone(), true)),
                Some(s) => {
                    let ticked = now.wrapping_sub(s.tick_count);
                    let expected = if s.duration == -1 {
                        -1
                    } else {
                        s.duration - ticked
                    };
                    let refresh = ticked > 0 && e.duration > 0 && e.duration % 600 == 0;
                    if s.amplifier != e.amplifier
                        || s.flags != PlayerEffects::flags(e)
                        || e.duration != expected
                        || refresh
                    {
                        changes.push((e.clone(), false));
                    } else {
                        sent.push(s.clone());
                    }
                }
            }
        }
        let removed: Vec<&'static str> = g
            .sent_effects
            .iter()
            .filter(|s| g.effects.get(s.id).is_none())
            .map(|s| s.id)
            .collect();
        for id in removed {
            let Some(effect) = self
                .sync
                .id("minecraft:mob_effect", crate::brewing::namespaced(id))
            else {
                continue;
            };
            let g = &self.guests[i];
            g.send(&C::RemoveMobEffect(play::RemoveMobEffect {
                id: g.entity_id,
                effect,
            }));
        }
        for (e, added) in changes {
            sent.extend(self.send_effect(i, &e, added));
        }
        {
            let value = sent;
            self.guests.update(i, |guest| guest.sent_effects = value);
        }
    }

    /// PlayerList.sendPlayerPermissionLevel: the op level as an entity event
    /// (24 + level), then the commands that level may use.
    fn send_permission_level(&self, i: usize) {
        let g = &self.guests[i];
        g.send(&C::EntityEvent(play::EntityEvent {
            id: g.entity_id,
            event: 24 + g.permission.min(4) as i8,
        }));
        self.send_commands(i);
    }

    /// The permissions changed: tell everyone their level and commands again.
    pub fn resend_commands(&mut self) {
        self.refresh_permissions();
        for i in 0..self.guests.len() {
            if self.guests[i].placed {
                self.send_permission_level(i);
            }
        }
    }

    /// Every guest's level from the operator list and the settings, the first
    /// half of [`Hub::resend_commands`].
    pub fn refresh_permissions(&mut self) {
        for i in 0..self.guests.len() {
            {
                let value = self.default_permission(self.guests[i].uuid);
                self.guests.update(i, |guest| guest.permission = value);
            }
        }
    }

    /// The names of the root commands the tree `conn`'s player would get
    /// holds (before the mods take any out), in the tree's order.
    pub fn command_names(&self, conn: u32) -> Vec<String> {
        let Some(g) = self.guest(conn) else {
            return Vec::new();
        };
        let packet = self.commands_packet(g.permission);
        packet
            .nodes
            .get(packet.root as usize)
            .map(|root| {
                root.children
                    .iter()
                    .filter_map(|&c| packet.nodes.get(c as usize))
                    .map(|n| n.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether vanilla's command tree has a command of this name.
    pub fn vanilla_command(&self, name: &str) -> bool {
        self.sync.commands.as_ref().is_some_and(|t| t.has(name))
    }

    /// The key [`Hub::set_mod_commands`] last had: the game compares it
    /// with its server mods' each tick.
    pub fn mod_commands_key(&self) -> Option<u64> {
        self.mod_commands.1
    }
    /// Server mods' commands changed: every player gets their tree again.
    #[horizon_mod_api_macros::replaceable]
    pub fn set_mod_commands(&mut self, key: u64, commands: serde_json::Value) {
        if self.store_mod_commands(key, commands) {
            for i in 0..self.guests.len() {
                if self.guests[i].placed {
                    self.send_commands(i);
                }
            }
        }
    }

    /// [`Hub::set_mod_commands`] without sending: true when the tree changed.
    /// The caller sends the trees itself (`native_refresh_commands` first asks
    /// the mods which root commands each player keeps).
    pub fn store_mod_commands(&mut self, key: u64, commands: serde_json::Value) -> bool {
        let changed = self.mod_commands.0 != commands;
        self.mod_commands = (commands, Some(key));
        changed
    }

    /// The command tree a player of this permission level gets.
    fn commands_packet(&self, permission: u8) -> play::Commands {
        match &self.sync.commands {
            Some(tree) => {
                let dedicated = !self.config.integrated;
                tree.packet_with_features(
                    permission,
                    dedicated,
                    &self.sync.builtin,
                    &super::commands::with_session_tools(&self.mod_commands.0),
                    &self.sync.features,
                    &self.rule_registry,
                )
            }
            None => play::Commands {
                nodes: vec![play::CommandNode {
                    kind: 0,
                    executable: false,
                    restricted: false,
                    children: Vec::new(),
                    redirect: None,
                    name: String::new(),
                    parser: None,
                    suggestions: None,
                }],
                root: 0,
            },
        }
    }

    /// ServerGamePacketListenerImpl.teleport: move the client and wait for its confirmation.
    pub fn teleport(&mut self, i: usize, position: Option<[f64; 3]>) {
        // ServerPlayer.teleportTo: resetFlyingTicks.
        {
            let value = 0;
            self.guests
                .update(i, |guest| guest.above_ground_ticks = value);
        }
        self.teleport_relative(i, position, 0);
    }

    /// `ServerGamePacketListenerImpl.teleport` alone (a refused or too fast
    /// move, a teleport sent again): the flying ticks go on.
    pub(super) fn resend_teleport(&mut self, i: usize, position: Option<[f64; 3]>) {
        self.teleport_relative(i, position, 0);
    }

    /// A teleport to `position` that keeps the player's motion and look
    /// (`Relative.DELTA` and `Relative.ROTATION`: an End gateway's).
    pub fn teleport_keeping_motion(&mut self, i: usize, position: [f64; 3]) {
        {
            let value = 0;
            self.guests
                .update(i, |guest| guest.above_ground_ticks = value);
        }
        use play::relative::{DELTA_X, DELTA_Y, DELTA_Z, ROTATE_DELTA, X_ROT, Y_ROT};
        let relatives = Y_ROT | X_ROT | DELTA_X | DELTA_Y | DELTA_Z | ROTATE_DELTA;
        self.teleport_relative(i, Some(position), relatives);
    }

    /// A command's teleport to `position` (`ServerGamePacketListenerImpl.teleport`
    /// with `TeleportCommand.performTeleport`'s relatives): the motion of a
    /// relative axis is kept (`Relative.DELTA_*`, from `Relative.direction`),
    /// an absolute axis is a zero change.
    pub fn teleport_command(&mut self, i: usize, position: [f64; 3], delta: [bool; 3]) {
        use play::relative::{DELTA_X, DELTA_Y, DELTA_Z};
        let mut relatives = 0;
        if delta[0] {
            relatives |= DELTA_X;
        }
        if delta[1] {
            relatives |= DELTA_Y;
        }
        if delta[2] {
            relatives |= DELTA_Z;
        }
        {
            let value = 0;
            self.guests
                .update(i, |guest| guest.above_ground_ticks = value);
        }
        self.teleport_relative(i, Some(position), relatives);
    }

    /// A rider's own teleport when its vehicle is teleported
    /// (`Entity.teleportSameDimension`'s passenger transition): the position,
    /// the rotation the vehicle turned it by (absolute, as
    /// `ServerPlayer.teleport` sends it), and its motion kept
    /// (`Relative.DELTA`).
    pub fn teleport_rider(&mut self, i: usize, position: [f64; 3]) {
        {
            let value = 0;
            self.guests
                .update(i, |guest| guest.above_ground_ticks = value);
        }
        use play::relative::{DELTA_X, DELTA_Y, DELTA_Z};
        self.teleport_relative(i, Some(position), DELTA_X | DELTA_Y | DELTA_Z);
    }

    /// ServerGamePacketListenerImpl.teleport with `relatives`: the rotation
    /// and motion they name are sent as zero changes.
    fn teleport_relative(&mut self, i: usize, position: Option<[f64; 3]>, relatives: i32) {
        let ticks = self.ticks;
        self.guests.update(i, |g| {
            g.teleport_tick = ticks;
            g.held_ticket_chunk = Some(g.ticket_chunk()); // teleport has no move
            g.pending_move = None;
            if let Some(p) = position {
                g.position = p;
                g.previous_position = p;
            }
            g.last_reported = (g.position, [g.yaw, g.pitch]);
            g.teleport_counter = (g.teleport_counter + 1) & 0x7FFF_FFFF;
            let id = g.teleport_counter;
            g.awaiting_teleport = Some((id, g.position));
            use play::relative::{DELTA_X, DELTA_Y, DELTA_Z};
            g.awaiting_keeps_motion = relatives & (DELTA_X | DELTA_Y | DELTA_Z) != 0;
            let turn = relatives & (play::relative::Y_ROT | play::relative::X_ROT) != 0;
            // A relative axis (X, Y, Z: bits 0 to 2) goes as no change (`follow_passage`).
            let at = std::array::from_fn(|k| [g.position[k], 0.0][(relatives >> k & 1) as usize]);
            g.send(&C::PlayerPosition(play::PlayerPosition {
                teleport: id,
                position: at,
                delta: [0.0; 3],
                yaw: if turn { 0.0 } else { g.yaw },
                pitch: if turn { 0.0 } else { g.pitch },
                relatives,
            }));
        });
    }

    /// ClientboundSetDefaultSpawnPositionPacket for the level's respawn data:
    /// the world spawn with the dimension it was set in, wherever the player is.
    pub fn default_spawn(&self, dimension: &str, spawn: [f64; 3], angle: [f32; 2]) -> C {
        C::SetDefaultSpawnPosition(play::SetDefaultSpawnPosition {
            pos: play::GlobalPos {
                dimension: dimension.into(),
                pos: [
                    spawn[0].floor() as i32,
                    spawn[1].floor() as i32,
                    spawn[2].floor() as i32,
                ],
            },
            yaw: angle[0],
            pitch: angle[1],
        })
    }

    /// PlayerList.sendLevelInfo; `load_start` false holds back
    /// LEVEL_CHUNKS_LOAD_START for a player who has no position yet
    /// ([`Self::land`] sends it).
    fn send_level_info(&mut self, i: usize, ctx: &TickCtx, load_start: bool) {
        let spawn = ctx.spawn;
        let g = &self.guests[i];
        if self.sync_attachments {
            queue_snapshots(
                &mut self.attachment_snapshots,
                g.uuid,
                [Holder::Level, Holder::Player(g.uuid)],
            );
        }
        g.send(&C::InitializeBorder(super::border::initialize(ctx.border)));
        g.send(&self.time_packet(ctx.world, ctx.clock_rate));
        g.send(&self.default_spawn(ctx.spawn_dimension, spawn, ctx.spawn_angle));
        let w = &ctx.world.weather;
        if w.raining {
            g.send(&C::GameEvent(play::GameEvent {
                event: game_event::START_RAINING,
                param: 0.0,
            }));
            g.send(&C::GameEvent(play::GameEvent {
                event: game_event::RAIN_LEVEL,
                param: w.rain_level.1,
            }));
            g.send(&C::GameEvent(play::GameEvent {
                event: game_event::THUNDER_LEVEL,
                param: w.thunder_level.1,
            }));
        }
        if load_start {
            g.send(&C::GameEvent(play::GameEvent {
                event: game_event::LEVEL_CHUNKS_LOAD_START,
                param: 0.0,
            }));
        }
        // TickRateManager.updateJoiningPlayer.
        g.send(&ticking_state(ctx.tick_rate, ctx.tick_frozen));
        g.send(&C::TickingStep(play::TickingStep {
            steps: ctx.tick_steps.remaining,
        }));
    }

    /// ServerClockManager.createFullSyncPacket: every clock. Horizon keeps the
    /// overworld clock (the day time); the End's clock runs with the game
    /// time. Clocks do not advance without the advance_time rule.
    pub(in crate::lan) fn time_packet(&self, world: &World, rate: f32) -> C {
        let overworld = self
            .sync
            .id("minecraft:world_clock", "minecraft:overworld")
            .unwrap_or(0);
        let level_clock = self
            .level_clock
            .as_ref()
            .and_then(|(name, rate)| Some((self.sync.id("minecraft:world_clock", name)?, *rate)));
        C::SetTime(play::SetTime {
            game_time: world.game_time,
            clocks: self
                .clock_ids
                .iter()
                .map(|&id| {
                    if id == overworld {
                        play::ClockState {
                            clock: id,
                            total_ticks: self.overworld_day.unwrap_or(world.day_time),
                            partial_tick: 0.0,
                            rate,
                        }
                    } else if let Some((_, level_rate)) =
                        level_clock.filter(|(clock, _)| *clock == id)
                    {
                        play::ClockState {
                            clock: id,
                            total_ticks: world.day_time,
                            partial_tick: 0.0,
                            rate: level_rate,
                        }
                    } else {
                        play::ClockState {
                            clock: id,
                            total_ticks: world.end_clock,
                            partial_tick: 0.0,
                            rate: if rate > 0.0 { 1.0 } else { 0.0 },
                        }
                    }
                })
                .collect(),
        })
    }

    /// ServerPlayer.initInventoryMenu / sendAllDataToRemote for the inventory menu.
    /// ClientboundPlaceGhostRecipePacket: the recipe `id` faded into the
    /// guest's menu `container`, whose items were missing.
    pub fn send_ghost_recipe(&mut self, conn: u32, container: i32, id: &str) {
        let Some(display) = self.sync.advancements.ghost_display(id, &self.sync) else {
            return;
        };
        self.send_to(
            conn,
            &C::PlaceGhostRecipe(horizon_protocol::packets::recipes::PlaceGhostRecipe {
                container,
                display,
            }),
        );
    }
    pub fn send_inventory(&mut self, i: usize) {
        let items: Vec<Option<ItemStack>> = (0..46).map(|s| self.guests[i].menu_slot(s)).collect();
        let net: Vec<_> = items.iter().map(|s| self.stack(s)).collect();
        let carried = self.guests[i].carried.clone();
        let carried = self.stack(&carried);
        self.guests.update(i, |g| {
            g.menu_state = (g.menu_state + 1) & 0x7FFF;
            g.send(&C::ContainerSetContent(play::ContainerSetContent {
                container: 0,
                state: g.menu_state,
                items: net,
                carried,
            }));
            g.menu_sent = items;
            g.sent_selected = g.selected;
        })
    }

    // ---- per tick ------------------------------------------------------

    /// A whole tick with one level: the network, the level, then the
    /// players' latencies.
    pub fn tick(&mut self, ctx: &mut TickCtx) -> Vec<HostEvent> {
        let mut events = self.poll(ctx);
        events.extend(self.drain_packets(ctx));
        // A caller that consumes raw events must resume the inbox before
        // running level/maintenance work. Game uses the split methods.
        if !self.packets_paused() {
            events.extend(self.tick_level(ctx));
            self.end_tick();
        }
        events
    }

    /// The network since the last tick, once per server tick before any
    /// level ticks: joins (into the level in place), leaves, and each guest's
    /// packets queued for its own level's pass.
    pub fn poll(&mut self, ctx: &mut TickCtx) -> Vec<HostEvent> {
        self.ticks += 1;
        self.end_refreshes();
        if !self.pending_logouts.is_empty() {
            self.pending_logouts
                .retain(|_, guard| guard.strong_count() != 0);
        }
        self.anticheat_tick(ctx.tick_rate);
        let mut events = self.begin_payload_poll();
        self.finish_fake_kicks(&mut events);
        self.expire_arrivals(&mut events);
        let mut credits = Vec::new();
        for receipt in self.lan.poll() {
            let (event, credit) = receipt.into_parts();
            match event {
                NetEvent::RawPayload(delivery) => {
                    self.poll_raw_payload(delivery, credit, &mut events, &mut credits);
                }
                NetEvent::Joined(j) => {
                    credits.push(credit);
                    while let Some(i) = self
                        .guests
                        .iter()
                        .position(|g| g.uuid == j.uuid && !g.participates_online())
                    {
                        let guest = self.guests.remove(i);
                        events.push(self.leave_event(guest));
                    }
                    let conn = self.admit(*j, ctx);
                    events.push(HostEvent::Joined(conn));
                }
                NetEvent::Left(conn, reason) => {
                    credits.push(credit);
                    if let Some(i) = self.guests.iter().position(|g| g.conn == conn) {
                        let g = self.remove_guest(i, &reason);
                        events.push(self.leave_event(g));
                    }
                }
                NetEvent::Packet(conn, packet, bytes) => {
                    self.update_guest(conn, |g| {
                        g.sender.undrained.taken(bytes);
                        if g.left_reason.is_empty() {
                            g.inbox.push_back(InboxItem::Packet(*packet, Some(credit)));
                        }
                    });
                }
                NetEvent::Duplicate(uuid) => {
                    credits.push(credit);
                    let conns: Vec<u32> = self
                        .guests
                        .iter()
                        .filter(|g| g.uuid == uuid && g.participates_online())
                        .map(|g| g.conn)
                        .collect();
                    for conn in conns {
                        self.kick(
                            conn,
                            translatable("multiplayer.disconnect.duplicate_login", &[]),
                        );
                    }
                }
            }
        }
        // keepConnectionAlive's timeout: close a kicked client still unread a second later.
        if self.ticks.is_multiple_of(20) {
            for g in &self.guests {
                if g.participates_online() && !g.left_reason.is_empty() && g.sender.timed_out() {
                    g.sender.close();
                }
            }
            let silent: Vec<u32> = self
                .guests
                .iter()
                .filter(|g| {
                    g.participates_online() && g.left_reason.is_empty() && g.sender.timed_out()
                })
                .map(|g| g.conn)
                .collect();
            for conn in silent {
                self.kick(conn, translatable("disconnect.timeout", &[]));
            }
        }
        events.push(HostEvent::InputCredits(credits));
        events
    }

    /// Keeps server-answer packets in a frozen player's inbox (teleport, chunk batch,
    /// loaded, keep-alive, ping, tick end, client information) and drops the rest.
    /// Chat messages and signed commands are dropped, but their last-seen messages
    /// are taken first (`unpackAndApplyLastSeen`). Chat acknowledgements are handled
    /// here in arrival order: the client counts them as acknowledged, and its next
    /// message after the freeze would not validate otherwise.
    pub(crate) fn keep_acknowledgements(&mut self, i: usize, events: &mut Vec<HostEvent>) {
        let inbox = self.guests.update(i, |g| std::mem::take(&mut g.inbox));
        let mut credits = Vec::new();
        let mut inbox = inbox.into_iter();
        while let Some(entry) = inbox.next() {
            let InboxItem::Packet(packet, credit) = entry else {
                // Never filter or acknowledge past a raw callback boundary.
                self.guests.update(i, |g| {
                    g.inbox.push_back(entry);
                    g.inbox.extend(inbox);
                });
                break;
            };
            match &packet {
                S::Chat(c) => {
                    self.apply_last_seen(i, &c.last_seen, events);
                }
                S::ChatCommandSigned(c) => {
                    self.apply_last_seen(i, &c.last_seen, events);
                }
                S::ChatAck(c) => self.chat_ack(i, c.offset, events),
                S::AcceptTeleportation(_)
                | S::PlayerLoaded(_)
                | S::ChunkBatchReceived(_)
                | S::ClientInformation(_)
                | S::ClientTickEnd(_)
                | S::KeepAlive(_)
                | S::PingRequest(_)
                | S::Pong(_) => {
                    self.guests
                        .update(i, |g| g.inbox.push_back(InboxItem::Packet(packet, credit)));
                    continue;
                }
                _ => {}
            }
            credits.extend(credit);
        }
        events.push(HostEvent::InputCredits(credits));
    }

    /// The level in place's part of the tick (`ServerLevel.tick`'s chunk
    /// map and the connections of the players in it): the chunks, block
    /// changes, containers and entities they see. Player packets are drained
    /// earlier (`drain_packets`) so scheduled ticks see what they placed.
    pub fn tick_level(&mut self, ctx: &mut TickCtx) -> Vec<HostEvent> {
        let events = Vec::new();
        if !self.guests.simulation_iter().any(|g| self.watches(g)) {
            // ServerTickRateManager broadcasts a change to the players there
            // are; one who joins later is sent the state as it is then
            // (`updateJoiningPlayer`).
            if !self
                .guests
                .iter()
                .any(|g| g.placed && g.participates_online())
            {
                self.ticking = (ctx.tick_rate, ctx.tick_frozen);
                self.stepping = ctx.tick_steps.revision;
            }
            if self.viewed_pass(ctx) {
                return events;
            }
            if !self
                .guests
                .iter()
                .any(|g| g.participates_online() && g.dimension == self.config.dimension)
            {
                let _ = ctx.world.block_log.take();
                let _ = ctx.world.light_log.take();
                if let Some(far) = self.far.as_mut() {
                    far.forget_level();
                }
                ctx.world.bulk_log.clear();
            }
            return events;
        }
        if ctx.world.block_log.is_none() {
            ctx.world.block_log = Some(Vec::new());
        }
        if ctx.world.light_log.is_none() {
            ctx.world.light_log = Some(Default::default());
        }
        crate::spans::time("lan_sync_world", || self.sync_world(ctx));
        crate::spans::time("lan_chunks", || self.update_chunks(ctx));
        crate::spans::time("lan_viewed_levels", || self.send_viewed(ctx));
        crate::spans::time("lan_block_changes", || {
            self.broadcast_light_changes(ctx.world);
            self.broadcast_block_changes(ctx.world);
        });
        // ServerGamePacketListenerImpl.tick: the actions handled since the last tick are
        // acknowledged after the block changes they made were sent, so a client's predicted
        // blocks settle on the server's states.
        for g in self.guests.iter().filter(|g| self.here(g)) {
            let sequence = g.ack_up_to.replace(-1);
            if sequence >= 0 {
                g.send(&C::BlockChangedAck(play::BlockChangedAck { sequence }));
            }
        }
        crate::spans::time("lan_containers", || self.sync_containers(ctx.world));
        self.sync_hidden();
        crate::spans::time("lan_track_entities", || self.track_entities(ctx));
        crate::spans::time("lan_player_state", || {
            for i in 0..self.guests.len() {
                if self.here(&self.guests[i]) {
                    self.sync_player_state(i);
                    self.sync_effects(i);
                }
            }
        });
        events
    }

    /// After every level: `PlayerList.tick`.
    pub fn end_tick(&mut self) {
        let latencies: Vec<_> = self
            .guests
            .iter()
            .filter(|g| g.participates_online())
            .map(|g| (g.uuid, g.latency.load(Ordering::Relaxed)))
            .collect();
        // PlayerList.tick: UPDATE_LATENCY once `++sendAllPlayerInfoIn > 600`,
        // so on the 601st tick, then every 601.
        if self.ticks.is_multiple_of(601) && !latencies.is_empty() {
            let entries = latencies
                .iter()
                .map(|(uuid, latency)| play::PlayerInfo {
                    uuid: *uuid,
                    latency: Some(*latency),
                    ..Default::default()
                })
                .collect();
            self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                actions: 16,
                entries,
            }));
        }
    }

    /// The starts and stops of tracking the guests had since the last call
    /// are kept in `native_tracking`.
    pub fn collect_tracking(&mut self) {
        self.guests.update_all(|g| {
            take_tracking(
                g,
                &mut self.native_tracking,
                &mut self.attachment_snapshots,
                self.sync_attachments,
            );
        });
    }

    pub fn player_logout_pending(&self, uuid: u128) -> bool {
        self.pending_logouts
            .get(&uuid)
            .is_some_and(|guard| guard.strong_count() != 0)
    }

    pub fn player_logout_save_failed(&self, uuid: u128) -> bool {
        self.failed_logout_saves.contains(&uuid)
    }

    pub fn record_player_logout_save(&mut self, uuid: u128, saved: bool) {
        if saved {
            self.failed_logout_saves.remove(&uuid);
        } else {
            self.failed_logout_saves.insert(uuid);
        }
    }

    /// An offline reader must wait until every queued logout for this player finishes saving.
    pub fn hold_player_logout(&mut self, uuid: u128) -> Arc<()> {
        let guard = self.pending_logouts.entry(uuid).or_default();
        guard.upgrade().unwrap_or_else(|| {
            let pending = Arc::new(());
            *guard = Arc::downgrade(&pending);
            pending
        })
    }

    /// The event owns the guard even if the game drops the event without handling it.
    pub fn leave_event(&mut self, mut guest: Guest) -> HostEvent {
        if guest.participates_online() {
            guest.logout_guard = Some(self.hold_player_logout(guest.uuid));
        }
        HostEvent::Left(Box::new(guest))
    }

    pub(super) fn remove_guest(&mut self, i: usize, reason: &str) -> Guest {
        let mut g = self.guests.remove(i);
        if let Some(terrain) = self.terrain.as_mut() {
            terrain.players.remove(&g.conn);
        }
        super::chunk_watch::unwatch_all(&mut self.chunk_watches, &mut g);
        if let Some(m) = g
            .menu
            .as_ref()
            .filter(|m| m.slots == super::menu::Slots::Native)
        {
            self.native_menu_events
                .push(super::menu::NativeMenuEvent::Closed {
                    uuid: g.uuid,
                    window: m.window,
                });
        }
        // A held player a proxy dropped keeps, in the log, how its connection really ended
        // (an unreadable packet, a reset): the drop says only that it was dropped.
        if g.held
            && !g.placed
            && !reason.is_empty()
            && !g.left_reason.is_empty()
            && g.left_reason != reason
        {
            g.left_reason = format!("{} (the connection ended: {reason})", g.left_reason);
        }
        if g.left_reason.is_empty() {
            g.left_reason = reason.to_owned();
        }
        if std::env::var_os("HORIZON_NET_DEBUG").is_some() {
            log::info!("[lan] {} left: {reason}", g.name);
        }
        if !g.placed {
            return g;
        }
        // `PlayerList.remove`: the entities it tracked stop (`removePairing`).
        g.tracker.clear();
        take_tracking(
            &mut g,
            &mut self.native_tracking,
            &mut self.attachment_snapshots,
            self.sync_attachments,
        );
        // To the held players too: they were told of this one, and see its entity.
        self.broadcast_watching(&C::PlayerInfoRemove(play::PlayerInfoRemove {
            uuids: vec![g.uuid],
        }));
        self.broadcast_watching(&C::RemoveEntities(play::RemoveEntities {
            ids: vec![g.entity_id],
        }));
        let left = C::SystemChat(play::SystemChat {
            content: yellow_names(
                "multiplayer.player.left",
                vec![self.player_display_name(g.uuid, &g.name)],
            ),
            overlay: false,
        })
        .encode();
        self.guests.update_all(|o| {
            if o.placed && !o.tab_hidden.remove(&g.uuid) && !g.quiet_leave {
                o.sender.send(left.clone());
            }
        });
        g
    }

    /// Takes the frozen player `uuid` out of the world because a proxy moved it to another
    /// server, for the hand-over `id`: the others see it go (its entity and its player-list
    /// entry), but nothing is saved (the other server has the player now) and, unless
    /// `announce`, the "left the game" line is not sent. The game is told it left, marked
    /// released, so it saves nothing.
    ///
    /// Refused when the player is not in that hand-over: another one, or none because the
    /// freeze ran out (the player has played on since, and the record the other server has is
    /// older), or a player who died while frozen.
    pub fn release_moved(
        &mut self,
        uuid: u128,
        id: u128,
        announce: bool,
    ) -> Result<(), &'static str> {
        let i = self.handover_of(uuid, id)?;
        self.guests.update(i, |g| {
            g.quiet_leave = !announce;
            g.left_reason = "moved to another server".to_owned();
            g.sender
                .disconnect(translatable("multiplayer.disconnect.generic", &[]));
        });
        let mut gone = self.remove_guest(i, "moved to another server");
        gone.released = true;
        gone.handover = None;
        let left = self.leave_event(gone);
        self.pending_kicks.push(left);
        Ok(())
    }

    /// The guest index of the player `uuid` while it is frozen for the hand-over `id`.
    pub fn handover_of(&self, uuid: u128, id: u128) -> Result<usize, &'static str> {
        let Some(i) = self.guests.iter().position(|g| g.uuid == uuid) else {
            return Err("that player is not connected to this server");
        };
        let g = &self.guests[i];
        match &g.handover {
            _ if g.dead => Err("that player died during the hand-over"),
            None => Err("that player is not in a hand-over (or its freeze ran out)"),
            Some(h) if h.id != id => Err("that is not the player's current hand-over"),
            Some(_) => Ok(i),
        }
    }

    /// `ServerTickRateManager.updateStepTicks` / `updateStateToClients`:
    /// send clock changes before the block events caused by their ticks.
    pub fn sync_ticking(&mut self, rate: f32, frozen: bool, steps: crate::clock::StepState) {
        if (rate, frozen) != self.ticking {
            self.ticking = (rate, frozen);
            self.broadcast(&ticking_state(rate, frozen));
        }
        // ServerTickRateManager.updateStepTicks: a step started or stopped.
        if steps.revision != self.stepping {
            self.stepping = steps.revision;
            self.broadcast(&C::TickingStep(play::TickingStep {
                steps: steps.requested,
            }));
        }
    }

    /// Time and weather (ServerLevel.tickTime / advanceWeatherCycle broadcasts).
    fn sync_world(&mut self, ctx: &TickCtx) {
        let world = &*ctx.world;
        // MinecraftServer.tickServer: the game time every 20 ticks
        // (forceGameTimeSynchronization, no clocks).
        if self.ticks.is_multiple_of(20) {
            self.broadcast_level(&C::SetTime(play::SetTime {
                game_time: world.game_time,
                clocks: Vec::new(),
            }));
        }
        // ServerClockManager.modifyClock: a day that jumped (`time set`, `time
        // add`) or a new rate updates the overworld clock.
        let (day, rate) = (self.overworld_day.unwrap_or(world.day_time), ctx.clock_rate);
        let step = i64::from(rate.ceil() as i32).max(1);
        if self.last_clock != i64::MIN
            && ((day - self.last_clock).abs() > step || rate != self.last_rate)
        {
            let overworld = self
                .sync
                .id("minecraft:world_clock", "minecraft:overworld")
                .unwrap_or(0);
            self.broadcast_level(&C::SetTime(play::SetTime {
                game_time: world.game_time,
                clocks: vec![play::ClockState {
                    clock: overworld,
                    total_ticks: day,
                    partial_tick: 0.0,
                    rate,
                }],
            }));
        }
        self.last_clock = day;
        self.last_rate = rate;
        // The same for the clock the level follows when it is not the Overworld's: a jump
        // (`time set` in the level) or a new rate reaches its players at once.
        match self.level_clock.clone() {
            Some((name, level_rate)) => {
                let total = world.day_time;
                let (last_total, last_rate) = self.last_level_clock;
                let step = i64::from(level_rate.ceil() as i32).max(1);
                if last_total != i64::MIN
                    && ((total - last_total).abs() > step || level_rate != last_rate)
                    && let Some(clock) = self.sync.id("minecraft:world_clock", &name)
                {
                    self.broadcast_level(&C::SetTime(play::SetTime {
                        game_time: world.game_time,
                        clocks: vec![play::ClockState {
                            clock,
                            total_ticks: total,
                            partial_tick: 0.0,
                            rate: level_rate,
                        }],
                    }));
                }
                self.last_level_clock = (total, level_rate);
            }
            None => self.last_level_clock = (i64::MIN, 0.0),
        }
        self.sync_ticking(ctx.tick_rate, ctx.tick_frozen, ctx.tick_steps);
        let w = &world.weather;
        let (raining, rain, thunder) = self.weather;
        if w.raining != raining {
            self.broadcast_level(&C::GameEvent(play::GameEvent {
                event: if w.raining {
                    game_event::START_RAINING
                } else {
                    game_event::STOP_RAINING
                },
                param: 0.0,
            }));
        }
        if (w.rain_level.1 - rain).abs() > f32::EPSILON {
            self.broadcast_level(&C::GameEvent(play::GameEvent {
                event: game_event::RAIN_LEVEL,
                param: w.rain_level.1,
            }));
        }
        if (w.thunder_level.1 - thunder).abs() > f32::EPSILON {
            self.broadcast_level(&C::GameEvent(play::GameEvent {
                event: game_event::THUNDER_LEVEL,
                param: w.thunder_level.1,
            }));
        }
        self.weather = (w.raining, w.rain_level.1, w.thunder_level.1);
    }

    // ---- chunks --------------------------------------------------------

    /// Every player's chunks here: their views, then the next batch of each
    /// (PlayerChunkSender.sendNextChunks). The batches are chosen first, the
    /// columns in them encoded once each, their new sections and their light
    /// on the warm-up threads too (`ChunkEncoder::warm`, `bodies_with`), then
    /// sent: nothing changes the level in between, and each player's packets
    /// keep their order. The far ring's chunks the level doesn't hold go as
    /// the save's readers made them.
    fn update_chunks(&mut self, ctx: &TickCtx) {
        let world = &*ctx.world;
        self.prepare_terrain_pass();
        if let Some(far) = self.far.as_mut() {
            far.poll();
        }
        let watching = self.chunk_watches.recording();
        let mut batches = Vec::new();
        for i in 0..self.guests.len() {
            if self.watches(&self.guests[i]) {
                let moved = self.terrain.is_some()
                    && self.guests[i].center != Some(self.guests[i].view_chunk());
                self.update_view(i, watching);
                if moved || self.terrain.is_some() && self.ticks <= self.guests[i].placed_at + 1 {
                    self.warm_first_terrain(i, world, ctx.streaming);
                }
                if let Some(batch) = self.next_batch(i, world, ctx.streaming) {
                    batches.push((i, batch));
                }
            }
        }
        // In solo singleplayer there is only one receiver, so retaining encoded
        // wire sections and heightmaps across sends only duplicates chunk RAM.
        // Update before the empty-batch return so a departing guest releases the
        // multiplayer cache even if the remaining player stands still.
        self.encoder.set_retain_wire(self.guests.len() > 1);
        if batches.is_empty() {
            return;
        }
        // Each column once, however many players it goes to, with the
        // number of batches that take it.
        let mut columns = Columns::default();
        let mut far_bodies = crate::fast_hash::FxHashMap::default();
        let mut protected = terrain::Bodies::default();
        let protos = |x, z| ctx.streaming.and_then(|s| s.proto_sections(x, z));
        let batches: Vec<_> = batches
            .into_iter()
            .map(|(i, batch)| {
                let rules = self.terrain_rules(self.guests[i].conn);
                let (taken, mut bodies): (Vec<_>, Vec<_>) = batch
                    .near
                    .into_iter()
                    .filter_map(|(idx, chunk)| {
                        let at = (chunk.x, chunk.z);
                        match rules.as_ref() {
                            None => Some((idx, Body::Level(columns.add(chunk)))),
                            Some(rules) => match self.protected_ready(rules, world, &protos, at)? {
                                super::terrain_view::Ready::Ordinary => {
                                    Some((idx, Body::Level(columns.add(chunk))))
                                }
                                ready => self
                                    .finish_protected_body(
                                        &mut protected,
                                        (rules.generation, at, false),
                                        chunk,
                                        world,
                                        &protos,
                                        ready,
                                    )
                                    .map(|body| (idx, Body::Saved(body))),
                            },
                        }
                    })
                    .unzip();
                let mut far = Vec::new();
                for (idx, column) in batch.far {
                    let body = match column {
                        Column::Level(chunk) if rules.is_some() => {
                            let Some(body) = self.protected_body(
                                &mut protected,
                                rules.as_ref().unwrap(),
                                chunk,
                                world,
                                &protos,
                                true,
                            ) else {
                                continue;
                            };
                            body
                        }
                        Column::Level(chunk) => far_bodies
                            .entry((chunk.x, chunk.z))
                            .or_insert_with(|| {
                                Arc::<[u8]>::from(self.encoder.body_far(chunk, world))
                            })
                            .clone(),
                        Column::Saved(body) => body,
                    };
                    far.push(idx);
                    bodies.push(Body::Saved(body));
                }
                (i, taken, far, bodies)
            })
            .collect();
        let chunks = &columns.chunks;
        self.encoder.warm(chunks);
        // A column one batch takes keeps its own buffer. One that several take is
        // copied into shared storage once, and each batch queues a reference to it.
        let mut encoded: Vec<super::Body> = self
            .encoder
            .bodies_with(chunks, world, &|x, z| {
                ctx.streaming.and_then(|s| s.proto_sections(x, z))
            })
            .into_iter()
            .zip(&columns.uses)
            .map(|(body, &uses)| {
                if uses == 1 {
                    super::Body::Owned(body)
                } else {
                    super::Body::shared(body)
                }
            })
            .collect();
        self.encoder.clear_batch_wire();
        for (i, taken, far, batch) in batches {
            let g = &self.guests[i];
            let keys = taken
                .iter()
                .map(|&i| g.chunk_queue[i])
                .chain(far.iter().map(|&i| g.far_queue[i]));
            let overlays = self.terrain_overlays(g.conn, world, keys);
            let protected = self.terrain_protected(g.conn, &g.dimension);
            self.guests.update(i, |g| {
                let size = batch.len();
                // The single-player window shares the sections it is sent (handoff.rs).
                let memory = g.address == super::conn::MEMORY_ADDRESS;
                if size > 0 {
                    g.batch_quota -= size as f32;
                    g.send(&C::ChunkBatchStart(horizon_protocol::packets::Empty));
                    g.sender.send_many(batch.into_iter().map(|body| match body {
                        Body::Level(j) => {
                            if memory && !protected {
                                super::handoff::offer(chunks[j]);
                            }
                            match &mut encoded[j] {
                                super::Body::Owned(body) => {
                                    super::Body::Owned(std::mem::take(body))
                                }
                                super::Body::Shared(body) => super::Body::Shared(Arc::clone(body)),
                            }
                        }
                        Body::Saved(body) => super::Body::Shared(body),
                    }));
                    g.send(&C::ChunkBatchFinished(play::ChunkBatchFinished {
                        size: size as i32,
                    }));
                    g.unacknowledged += 1;
                }
                if !overlays.is_empty() {
                    g.sender.send_many(overlays);
                }
                if watching {
                    // ChunkMap.markChunkPendingToSend, then PlayerChunkSender.sendChunk's end:
                    // Horizon picks a chunk only as it sends it.
                    for &idx in &taken {
                        let key = g.chunk_queue[idx];
                        if g.watched.insert(key) {
                            self.chunk_watches.push(g, ChunkWatchKind::Watch, key);
                        }
                    }
                    for &idx in &taken {
                        let key = g.chunk_queue[idx];
                        self.chunk_watches.push(g, ChunkWatchKind::Sent, key);
                    }
                }
                let mut seen: Vec<_> = taken
                    .into_iter()
                    .rev()
                    .map(|idx| g.chunk_queue.remove(idx))
                    .collect();
                if self.sync_attachments {
                    queue_snapshots(
                        &mut self.attachment_snapshots,
                        g.uuid,
                        seen.iter().map(|&(x, z)| Holder::Chunk([x, z])),
                    );
                }
                g.far_cursor -= far.iter().filter(|&&idx| idx < g.far_cursor).count();
                seen.extend(far.into_iter().rev().map(|idx| {
                    let key = g.far_queue.remove(idx);
                    if self.sync_attachments && g.negotiated_attachment_sync() {
                        g.far_sent_chunks.insert(key);
                    }
                    key
                }));
                g.sent_chunks.extend(seen);
            });
        }
    }

    /// ChunkMap.move's view change: the new centre, the chunks that left
    /// the view forgotten and the ones that entered it queued, closest first;
    /// past the view, the far ring's.
    fn update_view(&mut self, i: usize, watching: bool) {
        let (cx, cz) = self.guests[i].view_chunk();
        let r = self.guests[i].view_distance;
        let reach = self.reach(&self.guests[i]);
        if self.guests[i].center != Some((cx, cz))
            && let Some(reveals) = self
                .terrain
                .as_mut()
                .and_then(|v| v.players.get_mut(&self.guests[i].conn))
                .and_then(|levels| levels.get_mut(&self.config.dimension))
                .map(|selection| &mut selection.reveals)
        {
            reveals.retain(|&(x, _, z)| tracked(cx, cz, reach, x >> 4, z >> 4));
        }
        self.guests.update(i, |g| {
            if g.center == Some((cx, cz)) {
                return;
            }
            g.center = Some((cx, cz));
            if g.center_sent.take() != Some((cx, cz)) {
                g.send(&C::SetChunkCacheCenter(play::SetChunkCacheCenter {
                    x: cx,
                    z: cz,
                }));
            }
            // Chunks that left the view.
            let forget: Vec<_> = g
                .sent_chunks
                .iter()
                .copied()
                .filter(|&(x, z)| !tracked(cx, cz, reach, x, z))
                .collect();
            for (x, z) in forget {
                g.sent_chunks.remove(&(x, z));
                g.far_sent_chunks.remove(&(x, z));
                super::chunk_watch::unwatch(&mut self.chunk_watches, g, (x, z));
                g.send(&C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [x, z] }));
            }
            // Far columns have no attachment snapshots. Resend them as near chunks
            // after they enter the view, once the level has loaded them.
            g.far_sent_chunks.retain(|&(x, z)| {
                if tracked(cx, cz, r, x, z) {
                    g.sent_chunks.remove(&(x, z));
                    false
                } else {
                    true
                }
            });
            if watching && reach > r {
                // A far column the client keeps as it comes into the view is watched from here.
                let mut entered: Vec<_> = g
                    .sent_chunks
                    .iter()
                    .copied()
                    .filter(|&(x, z)| tracked(cx, cz, r, x, z) && !g.watched.contains(&(x, z)))
                    .collect();
                entered.sort_unstable();
                for key in entered {
                    g.watched.insert(key);
                    self.chunk_watches.push(g, ChunkWatchKind::Watch, key);
                }
            }
            g.far_cursor = 0;
            requeue(&mut g.chunk_queue, &g.sent_chunks, (cx, cz), r);
            if reach > r {
                requeue_ring(&mut g.far_queue, &g.sent_chunks, (cx, cz), r, reach);
            } else {
                g.far_queue.clear();
            }
        });
    }

    /// PlayerChunkSender.sendNextChunks' choice: ready chunks, closest first,
    /// as many as the batch quota the client's acknowledgements pace allows,
    /// with their places in the queue. What the view leaves of the quota goes
    /// to the far ring, as far as its brake lets it reach. A memory connection
    /// that has not loaded yet sends every ready view column in that one
    /// batch; the quota recorded for it covers them, so the tick after
    /// `PlayerLoaded` is capped again.
    fn next_batch<'w>(
        &mut self,
        i: usize,
        world: &'w World,
        streaming: Option<&crate::streaming::Streaming>,
    ) -> Option<Batch<'w>> {
        let budget = self.guests.update(i, |g| {
            if self.ticks <= g.placed_at + 1 || g.unacknowledged >= g.max_unacknowledged {
                return None;
            }
            g.batch_quota = (g.batch_quota + g.chunks_per_tick).min(g.chunks_per_tick.max(1.0));
            if g.batch_quota < 1.0 || (g.chunk_queue.is_empty() && g.far_queue.is_empty()) {
                return None;
            }
            Some(g.batch_quota.floor() as usize)
        })?;
        let g = &self.guests[i];
        // The integrated server's own player, still on the loading screen.
        // LAN guests and a client that has loaded keep the quota above.
        let burst = g.address == super::conn::MEMORY_ADDRESS && !g.loaded;
        let near_take = if burst { g.chunk_queue.len() } else { budget };
        // ChunkStatus.LIGHT needs its neighbours at INITIALIZE_LIGHT: a
        // column whose light layers would leave one out waits for it.
        let ready = |c: &Chunk| {
            c.lit
                && streaming.is_none_or(|streaming| {
                    neighbours_ready(
                        &world.chunks,
                        |x, z| streaming.proto_sections(x, z).is_some(),
                        c,
                    )
                })
        };
        let rules = self.terrain_rules(g.conn);
        let protos = |x, z| streaming.and_then(|s| s.proto_sections(x, z));
        let encode = super::terrain_view::EncodeContext {
            dimension: &self.config.dimension,
            encoder: &self.encoder,
            world,
            protos: &protos,
            include_far: self.far.is_some(),
        };
        let mut terrain = self.terrain.as_mut();
        let near: Vec<_> = g
            .chunk_queue
            .iter()
            .enumerate()
            .filter_map(|(idx, key)| Some((idx, &**world.chunks.get(key)?)))
            .filter(|(_, c)| ready(c))
            .take(if rules.is_some() {
                near_take.saturating_add(32)
            } else {
                usize::MAX
            })
            .filter(|(_, c)| {
                rules.as_ref().is_none_or(|rules| {
                    terrain
                        .as_deref_mut()
                        .unwrap()
                        .resident
                        .get(&encode, rules, (c.x, c.z))
                        .is_some()
                })
            })
            .take(near_take)
            .collect();
        let mut far = Vec::new();
        let mut cursor = g.far_cursor;
        if let (Some(ring), Some((cx, cz))) = (self.far.as_mut(), g.center) {
            // What the view left of the vanilla quota. A first-view burst can take more than that.
            let mut left = budget.saturating_sub(near.len());
            let len = g.far_queue.len();
            let start = g.far_cursor % len.max(1);
            let mut looked = 0;
            for offset in 0..len.min(super::far::LOOKS) {
                if left == 0 {
                    break;
                }
                let idx = (start + offset) % len;
                let (x, z) = g.far_queue[idx];
                looked += 1;
                if !tracked(cx, cz, ring.limit(), x, z) {
                    continue;
                }
                let column = match world.chunks.get(&(x, z)) {
                    // Far columns carry all their own saved light layers. They need no
                    // neighbour views, including at the edge of the level's loading buffer.
                    Some(c) if c.lit => Column::Level(c),
                    Some(_) if rules.is_some() => continue,
                    _ => match if let Some(rules) = &rules {
                        ring.saved_for_view(x, z, rules, world)
                    } else {
                        ring.saved(x, z)
                    } {
                        super::far::Saved::Body(body) => Column::Saved(body),
                        super::far::Saved::Missing | super::far::Saved::Waiting => {
                            continue;
                        }
                    },
                };
                left -= 1;
                far.push((idx, column));
            }
            cursor = (start + looked) % len.max(1);
            // Removal after sending runs backwards through the queue, even when the scan wrapped.
            far.sort_unstable_by_key(|(idx, _)| *idx);
        }
        // `update_chunks` subtracts the batch. Covering the near columns keeps that from going
        // negative, so the next tick's `.min(chunks_per_tick)` cap holds again.
        self.guests.update(i, |g| {
            g.far_cursor = cursor;
            if burst {
                g.batch_quota = g.batch_quota.max(near.len() as f32);
            }
        });
        (!near.is_empty() || !far.is_empty()).then_some(Batch { near, far })
    }

    /// `ChunkMap.resendBiomesForChunks`: each player in the level is sent,
    /// in one packet, the biomes of those of `columns` it has been sent
    /// (`getPlayers(pos, false)`), in the order given.
    pub fn resend_biomes(&mut self, world: &World, columns: &[(i32, i32)]) {
        if let Some(far) = self.far.as_mut() {
            for &(x, z) in columns {
                far.changed(x, z);
            }
        }
        let data: Vec<_> = columns
            .iter()
            .filter_map(|key| Some((*key, self.encoder.biome_data(world.chunks.get(key)?))))
            .collect();
        let watching: Vec<_> = self
            .guests
            .simulation_iter()
            .filter(|g| self.watches(g))
            .collect();
        let bodies = super::biome_resend::bodies(watching.iter().map(|g| &g.sent_chunks), &data);
        for (g, body) in watching.into_iter().zip(bodies) {
            if let Some(body) = body {
                g.send_chunks_biomes_body(body);
            }
        }
    }

    /// `ChunkHolder.broadcastChanges` where `ServerChunkCache.tick` runs it,
    /// before the block events: what changed since the last time (the last
    /// tick's block events, entities and block entities, this tick's
    /// scheduled and random ticks), as it is now.
    pub fn broadcast_changes(&mut self, world: &mut World) {
        if self.guests.iter().any(|g| self.here(g)) || self.viewed_here() {
            self.broadcast_light_changes(world);
            self.broadcast_block_changes(world);
        }
    }

    // ---- entities ------------------------------------------------------

    /// `ParticleTypes.STREAM_CODEC`'s value for `options`: the type's
    /// network id and the data it carries (`state` for the block particles).
    /// None for a particle whose data isn't sent from here (items, trails).
    pub fn particle(
        &self,
        options: &crate::particles::ParticleOptions,
        state: Option<horizon_world::registry::BlockStateId>,
    ) -> Option<Particle> {
        use crate::particles::ParticleType as T;
        // A native mod's own type: no registry id, so the sentinel and the name.
        if let Some(name) = &options.mod_particle {
            return Some(Particle {
                id: horizon_protocol::entity_data::MOD_PARTICLE_ID,
                data: Some(ParticleData::Mod(name.to_string())),
            });
        }
        let name = format!("minecraft:{}", options.ty.name());
        let id = self.registry_id("minecraft:particle_type", &name)?;
        // The options' RGB as an RGB colour (`ARGB.colorFromFloat` without alpha).
        let rgb_int = |c: [f32; 3]| {
            let b = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round() as i32) & 255;
            (b(c[0]) << 16) | (b(c[1]) << 8) | b(c[2])
        };
        let c = options.color;
        let rgb = rgb_int([c[0], c[1], c[2]]);
        let data = match options.ty {
            T::Block | T::BlockMarker | T::FallingDust | T::DustPillar | T::BlockCrumble => {
                Some(ParticleData::State(state?.0 as i32))
            }
            T::Dust => Some(ParticleData::ColorScale(rgb, options.scale)),
            T::DustColorTransition => Some(ParticleData::ColorScale2(
                rgb,
                rgb_int(options.to_color),
                options.scale,
            )),
            T::Effect | T::InstantEffect => Some(ParticleData::ColorPower(rgb, options.scale)),
            T::EntityEffect | T::TintedLeaves | T::Flash => Some(ParticleData::Color(
                (((c[3].clamp(0.0, 1.0) * 255.0).round() as i32) << 24) | rgb,
            )),
            T::DragonBreath | T::SculkCharge => Some(ParticleData::Scalar(options.scale)),
            T::Shriek => Some(ParticleData::Delay(options.scale as i32)),
            T::Geyser | T::GeyserPlume => Some(ParticleData::Geyser(options.geyser?.0)),
            T::GeyserBase | T::GeyserPoof => {
                let (water_blocks, burst_impulse_base) = options.geyser?;
                Some(ParticleData::GeyserBase(water_blocks, burst_impulse_base))
            }
            T::Vibration => {
                use horizon_protocol::entity_data::PositionSource as NetworkPositionSource;
                use mod_api::client::particles::PositionSource;
                let (source, _, ticks) = options.vibration.as_ref()?;
                let source = match source {
                    PositionSource::Block(pos) => NetworkPositionSource::Block(*pos),
                    PositionSource::Entity { id, y_offset } => NetworkPositionSource::Entity {
                        id: *id,
                        y_offset: *y_offset,
                    },
                    _ => return None,
                };
                Some(ParticleData::Vibration {
                    source,
                    ticks: *ticks,
                })
            }
            T::Item | T::Trail => return item_particles::encode(self, id, options),
            _ => None,
        };
        Some(Particle { id, data })
    }

    /// `LivingEntity.updateSynchronizedMobEffectParticles`: DATA_EFFECT_PARTICLES
    /// (10) and DATA_EFFECT_AMBIENCE_ID (11) for an entity with `effects`.
    /// Nothing for an entity without effects (`removeEffectParticles`).
    pub fn effect_meta(
        &self,
        effects: impl IntoIterator<Item = crate::effects::ShownEffect>,
    ) -> Vec<(u8, i32, Value)> {
        let mut effects = effects.into_iter().peekable();
        if effects.peek().is_none() {
            return Vec::new();
        }
        let (particles, ambient) = crate::effects::effect_particles(effects);
        self.particle_meta(&particles, ambient)
    }

    /// The synchronized registry tables, available to helper threads.
    pub fn sync_data(&self) -> &SyncData {
        &self.sync
    }

    /// DATA_EFFECT_PARTICLES and DATA_EFFECT_AMBIENCE_ID for `particles` and
    /// `ambient`, each left out at its default.
    pub fn particle_meta(
        &self,
        particles: &[crate::particles::ParticleOptions],
        ambient: bool,
    ) -> Vec<(u8, i32, Value)> {
        let particles: Vec<_> = (particles.iter())
            .filter_map(|p| self.particle(p, None))
            .collect();
        let mut meta = Vec::new();
        if !particles.is_empty() {
            meta.push((10, serializer::PARTICLES, Value::Particles(particles)));
        }
        if ambient {
            meta.push((11, serializer::BOOLEAN, Value::Bool(true)));
        }
        meta
    }

    /// A player's DATA_EFFECT_PARTICLES and DATA_EFFECT_AMBIENCE_ID: vanilla's,
    /// or while a native mod listens to `effect_particle_modification`
    /// (`modified`) what the mods made of them. The game asks the mods when
    /// the effects change; until it has, the last answer goes out (nothing
    /// before the first), as vanilla's synched data keeps its last value.
    pub fn player_effect_meta(&self, g: &Guest, modified: bool) -> Vec<(u8, i32, Value)> {
        if modified {
            (g.modified_effect_particles.as_ref())
                .map(|m| m.meta.clone())
                .unwrap_or_default()
        } else {
            self.effect_meta(g.effects.list.iter().map(EffectInstance::shown))
        }
    }

    fn player_state(
        &self,
        id: i32,
        uuid: u128,
        position: [f64; 3],
        rot: (f32, f32, f32),
        on_ground: bool,
        flags: (bool, bool, bool),
        body: BodyFlags,
        living_flags: i8,
        bed: Option<[i32; 3]>,
        skin: u8,
        main_hand: i32,
        health: f32,
        equipment: Vec<(u8, Option<horizon_protocol::item::Stack>)>,
        attributes: Vec<play::AttributeSnapshot>,
        effect_meta: Vec<(u8, i32, Value)>,
    ) -> EntityState {
        let (sneaking, sprinting, sleeping) = flags;
        let shared = shared_flags(sneaking, sprinting, &body);
        // Player.getDesiredPose (and updatePlayerPose's crawl): sleeping,
        // swimming, gliding, Riptide's spin, crouching.
        let pose = if sleeping {
            2
        } else if body.swimming || body.crawling {
            3
        } else if body.gliding {
            1
        } else if body.spin_attack {
            4
        } else if sneaking || body.crouching {
            5
        } else {
            0
        };
        let mut meta = Vec::new();
        if shared != 0 {
            meta.push((0, serializer::BYTE, Value::Byte(shared)));
        }
        if pose != 0 {
            meta.push((6, serializer::POSE, Value::Id(pose)));
        }
        // Always there: the trackers send the entries that changed, and an
        // item no longer in use is a change back to 0.
        meta.push((8, serializer::BYTE, Value::Byte(living_flags)));
        // LivingEntity.SLEEPING_POS: the others draw the player lying in it,
        // a player paired later too.
        if let Some(bed) = bed.filter(|_| sleeping) {
            meta.push((14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(Some(bed))));
        }
        if (health - 1.0).abs() > f32::EPSILON && health != 20.0 {
            meta.push((9, serializer::FLOAT, Value::Float(health)));
        }
        if skin != 0 {
            meta.push((16, serializer::BYTE, Value::Byte(skin as i8)));
        }
        // Avatar.DATA_PLAYER_MAIN_HAND (Avatar.java:38): anything but 1 is LEFT
        // (HumanoidArm.java:17), and RIGHT = 1, its default, is not sent.
        let arm = if main_hand == 1 { 1 } else { 0 };
        if arm != 1 {
            meta.push((15, serializer::HUMANOID_ARM, Value::Id(arm)));
        }
        meta.extend(effect_meta);
        EntityState {
            id,
            uuid,
            kind: "minecraft:player".into(),
            position,
            yaw: rot.0,
            pitch: rot.1,
            head_yaw: rot.2,
            on_ground,
            meta,
            forced_meta: [0; 4],
            // What a client still holds goes back to its default: the effect
            // swirls to none, the hand one holds LEFT back to RIGHT (send_dirty
            // sends defaults, entity.rs), and the skin parts to 0, every outer
            // layer hidden (Avatar.DATA_PLAYER_MODE_CUSTOMISATION).
            defaults: vec![
                (10, serializer::PARTICLES, Value::Particles(Vec::new())),
                (11, serializer::BOOLEAN, Value::Bool(false)),
                (15, serializer::HUMANOID_ARM, Value::Id(1)),
                (16, serializer::BYTE, Value::Byte(0)),
            ],
            equipment,
            attributes,
            ..EntityState::default()
        }
    }

    /// Convert only equipment whose full typed source changed, including body and saddle.
    fn guest_equipment(&mut self, i: usize) -> Vec<(u8, Option<horizon_protocol::item::Stack>)> {
        for index in 0..equipment_cache::SLOTS {
            let g = &self.guests[i];
            let source = equipment_source(g, index);
            if !g.equipment_cache.is_stale(index, source) {
                continue;
            }
            let source = source.cloned();
            let converted = self.stack(&source);
            self.guests
                .update(i, |g| g.equipment_cache.store(index, source, converted));
        }
        self.guests[i].equipment_cache.equipment()
    }

    /// Whether an entity is tracked by the current dimension's level.
    pub fn tracks(&self, id: i32) -> bool {
        self.entities
            .get(&self.config.dimension)
            .is_some_and(|l| l.tracks(id))
    }

    /// All entities as one client sees them: the world's and the other players.
    pub(in crate::lan) fn track_entities(&mut self, ctx: &mut TickCtx) {
        let mut all: Vec<EntityState> = Vec::with_capacity(ctx.entities.len() + self.guests.len());
        let modified_particles =
            mod_api::__private::listened(mod_api::HookId::EffectParticleModification);
        let placed: Vec<usize> = (0..self.guests.len())
            .filter(|&i| self.here(&self.guests[i]) && self.guests[i].body_present())
            .collect();
        let guest_states: Vec<_> = placed
            .into_iter()
            .map(|i| {
                let equipment = self.guest_equipment(i);
                let g = &self.guests[i];
                let mut state = self.player_state(
                    g.entity_id,
                    g.uuid,
                    g.position,
                    (g.yaw, g.pitch, g.head_yaw),
                    g.on_ground,
                    (g.sneaking, g.sprinting, g.sleeping),
                    guest_body(g),
                    g.living_flags,
                    g.bed,
                    g.info.model_customisation,
                    g.info.main_hand,
                    g.health,
                    equipment,
                    g.motion
                        .as_ref()
                        .map(|m| super::attributes::syncable(&m.attributes))
                        .unwrap_or_default(),
                    self.player_effect_meta(g, modified_particles),
                );
                state.spectator = g.spectator();
                state.watching = g.watching;
                state.hidden = self.tracker_hidden(g.uuid);
                stuck_meta(&mut state.meta, &mut state.defaults, g.stuck);
                frozen_meta(&mut state, g.motion.as_ref().map_or(0, |m| m.ticks_frozen));
                // Entity.getDeltaMovement: the server copy's, which a glide sends.
                if let Some(v) = g.motion.as_ref().map(|m| m.velocity) {
                    state.velocity = [v.x, v.y, v.z];
                }
                shoulder_meta(&mut state, g.shoulders);
                state
            })
            .collect();
        all.extend(guest_states);
        let mut entities: Vec<&EntityState> = all.iter().chain(ctx.entities).collect();
        // ChunkMap.tick: the level's entities, once for all its players.
        let mut level = self
            .entities
            .remove(&self.config.dimension)
            .unwrap_or_default();
        let mut joined = noted_last_tick(self.joined.remove(&self.config.dimension), self.ticks);
        if let Some(fresh) = self.fresh_joined.remove(&self.config.dimension) {
            joined.extend(fresh);
        }
        let joined = level.joined_states(&entities, joined);
        Entities::starting_from(&mut entities, &joined);
        level.synced.clone_from(&self.synced);
        let mut outs = std::mem::take(&mut self.entity_out);
        let dimension = &self.config.dimension;
        // The viewers include the held players: they are shown the entities around them, but are
        // not among the entities shown; then the clients viewing the level (`viewed_levels`).
        let mut views = self.view_outs();
        let here = |g: &Guest| {
            g.participates_online() && (g.placed || g.held) && g.dimension == *dimension
        };
        outs.resize_with(self.guests.iter().filter(|g| here(g)).count(), Vec::new);
        // The prediction's replay excuses a client that may still have an entity that left it.
        let recording = self.recording();
        let mut gone = std::mem::take(&mut self.entity_gone);
        gone.resize_with(outs.len(), Vec::new);
        let mut viewers = self.guests.tracking_viewers_and_views(
            here,
            outs.iter_mut()
                .zip(gone.iter_mut())
                .map(|(out, gone)| (out, recording.then_some(gone))),
            true,
            (dimension, &mut views),
        );
        level.range_percent = Some(self.entity_range_percent);
        let (types, sync) = (&mut self.entity_types, &self.sync);
        level.tick(&entities, &self.riding, &mut viewers, |kind| {
            entity_type(types, sync, kind)
        });
        drop(viewers);
        for (g, out) in self.guests.iter().filter(|g| here(g)).zip(outs.iter_mut()) {
            self.hold_sound_removals(g.conn, out);
            g.sender.send_many(out.drain(..));
        }
        let ticks = self.ticks;
        let mut gone_rows = gone.iter_mut();
        self.guests.update_all(|g| {
            if !here(g) {
                return;
            }
            let gone = gone_rows.next().expect("one output per visible guest");
            let slack = super::prediction::slack(std::array::from_fn(|i| {
                g.position[i] - g.previous_position[i]
            }));
            for at in gone.drain(..) {
                let block = at.map(|v| v.floor() as i32);
                if let Some(record) = g.record.as_deref_mut()
                    && super::prediction::near(g.position, block, slack)
                {
                    record.entity_gone(ticks);
                }
            }
        });
        self.entity_gone = gone;
        self.entity_out = outs;
        self.send_view_entities(views);
        self.entities.insert(self.config.dimension.clone(), level);
    }

    /// ServerPlayer.stopSleepInBed: the wake-up animation goes to the player
    /// and those around, the pose is standing again and the sleep counter
    /// starts the wake-up fade (100).
    pub fn wake(&mut self, i: usize, wake_immediately: bool) {
        let Some((id, at)) = self.guests.update(i, |g| {
            let head = g.bed.take();
            if head.is_none() && !g.sleeping {
                return None;
            }
            if let Some(head) = head {
                g.woke_from = Some(head);
            }
            g.sleeping = false;
            g.sleep_timer = if wake_immediately { 0 } else { 100 };
            if g.sent_bed.take().is_some() {
                g.sent_pose = crate::pose::Pose::Standing;
                g.send(&C::SetEntityData(play::SetEntityData {
                    id: g.entity_id,
                    values: vec![
                        (6, serializer::POSE, Value::Id(0)),
                        (14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(None)),
                    ],
                }));
            }
            Some((g.entity_id, g.position))
        }) else {
            return;
        };
        // ClientboundAnimatePacket.WAKE_UP.
        self.broadcast_near(
            at,
            128.0,
            &C::Animate(play::Animate { id, action: 0 }),
            None,
        );
    }

    // ---- per-player state ------------------------------------------------

    /// ServerPlayer.doTick: health, food, experience and the inventory menu.
    fn sync_player_state(&mut self, i: usize) {
        // LivingEntity.startSleeping / stopSleeping: the pose and the bed go
        // to the player too (its client lies down in the bed from them).
        self.guests.update(i, |g| {
            if g.bed != g.sent_bed {
                g.sent_bed = g.bed;
                let pose = if g.bed.is_some() { 2 } else { 0 };
                g.sent_pose = if g.bed.is_some() {
                    crate::pose::Pose::Sleeping
                } else {
                    crate::pose::Pose::Standing
                };
                g.send(&C::SetEntityData(play::SetEntityData {
                    id: g.entity_id,
                    values: vec![
                        (6, serializer::POSE, Value::Id(pose)),
                        (14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(g.bed)),
                    ],
                }));
            }
        });
        // ServerEntity.sendDirtyEntityData on the player's first tick: the
        // health its creation set is still dirty, and goes to the player too.
        let first_tick = self.guests[i].sent_health.is_none();
        if first_tick {
            let g = &self.guests[i];
            g.send(&C::SetEntityData(play::SetEntityData {
                id: g.entity_id,
                values: vec![(9, serializer::FLOAT, Value::Float(g.health))],
            }));
            // ServerPlayer.tick's CriteriaTriggers.TICK, then flushDirty. A
            // player who has been here has had the tick's criteria already.
            if !g.advancements_loaded {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64);
                let sync = self.sync.clone();
                self.guests.update(i, |g| {
                    if self.mods_loaded {
                        self.native_advancement_grants.push(TickGrants {
                            conn: g.conn,
                            player: g.uuid,
                            entity_id: g.entity_id,
                            name: g.name.clone(),
                            now_millis: now,
                            first_tick: true,
                            grants: sync.advancements.tick_candidates(&g.advancements),
                            sync,
                        });
                    } else {
                        let first = sync.advancements.first_tick(
                            &sync,
                            now,
                            &mut g.advancements,
                            &mut g.recipe_book,
                            &mut g.recipes_unlocked,
                        );
                        if let Some(add) = first.recipe_book_add {
                            g.send(&raw(ids::RECIPE_BOOK_ADD, add));
                        }
                        g.send(&raw(ids::UPDATE_ADVANCEMENTS, first.update_advancements));
                        g.advancements_visible = first.visible;
                        g.advancements_changed.clear();
                    }
                });
            } else {
                self.flush_advancements(i, true);
            }
        }
        // ServerEntity.sendDirtyEntityData: the player's changed attribute
        // instances go to the player too (its trackers get them from
        // track_entities). On the first tick vanilla's ran before
        // ServerPlayer.tick, whose updatePlayerAttributes (and doTick's
        // removeFrost) created the instances the guest's tick just made: they
        // go out on the next tick, after the first chunk batch (#5560). This
        // server's first batch waits for the player's chunks to be ready, so
        // they wait for it too: a slow load must not put them before it.
        self.guests.update(i, |g| {
            let after_first_batch = g.unacknowledged > 0 || g.max_unacknowledged > 1;
            if let Some(m) = g
                .motion
                .as_mut()
                .filter(|_| !first_tick && after_first_batch)
            {
                let values = super::attributes::take_dirty(&mut m.attributes);
                if !values.is_empty() {
                    g.send(&C::UpdateAttributes(play::UpdateAttributes {
                        id: g.entity_id,
                        values,
                    }));
                }
            }
            // ServerEntity.sendDirtyEntityData: the air supply and the absorption
            // amount the server's body changed go to the player, whose HUD draws
            // them (bubbles, golden hearts).
            if let Some(m) = g.motion.as_ref() {
                let now = (m.air, m.health.absorption());
                let frozen = m.ticks_frozen;
                let mut values = Vec::new();
                if now.0 != g.sent_air.0 {
                    values.push((1, serializer::INT, Value::Int(now.0)));
                }
                if frozen != g.sent_frozen {
                    values.push((7, serializer::INT, Value::Int(frozen)));
                }
                if now.1.to_bits() != g.sent_air.1.to_bits() {
                    values.push((17, serializer::FLOAT, Value::Float(now.1)));
                }
                if !values.is_empty() {
                    g.sent_air = now;
                    g.sent_frozen = frozen;
                    g.send(&C::SetEntityData(play::SetEntityData {
                        id: g.entity_id,
                        values,
                    }));
                }
            }
            // ServerEntity.sendDirtyEntityData: the shared flags go to the player
            // too, so the sprint a hit stops (Player.causeExtraKnockback) stops
            // on its client.
            let shared = shared_flags(g.sneaking, g.sprinting, &guest_body(g));
            if shared != g.sent_shared {
                g.sent_shared = shared;
                g.send(&C::SetEntityData(play::SetEntityData {
                    id: g.entity_id,
                    values: vec![(0, serializer::BYTE, Value::Byte(shared))],
                }));
            }
            if let Some(pose) = g.own_pose().filter(|p| *p != g.sent_pose) {
                g.sent_pose = pose;
                g.send(&C::SetEntityData(play::SetEntityData {
                    id: g.entity_id,
                    values: vec![(6, serializer::POSE, Value::Id(pose.id()))],
                }));
            }
            // ServerEntity.sendDirtyEntityData: the player's own data goes to
            // it too (ChunkMap.java:1352-1356); 1 is RIGHT, the rest LEFT.
            let arm = if g.info.main_hand == 1 { 1 } else { 0 };
            if arm != g.sent_main_hand {
                g.sent_main_hand = arm;
                g.send(&C::SetEntityData(play::SetEntityData {
                    id: g.entity_id,
                    values: vec![(15, serializer::HUMANOID_ARM, Value::Id(arm))],
                }));
            }
            // ServerEntity.sendDirtyEntityData: the parrots on the player's
            // shoulders go to the player too, whose own model draws them.
            if g.shoulders != g.sent_shoulders {
                let values = shoulder_changes(g.shoulders, g.sent_shoulders);
                g.sent_shoulders = g.shoulders;
                g.send(&C::SetEntityData(play::SetEntityData {
                    id: g.entity_id,
                    values,
                }));
            }
            // ServerEntity.sendDirtyEntityData: the skin parts
            // (Avatar.DATA_PLAYER_MODE_CUSTOMISATION) the player's model shows, which
            // ServerPlayer.updateOptions copies from the client's information, go to the
            // player itself too. Its own entity is never tracked to it
            // (ChunkMap.TrackedEntity.updatePlayer skips the entity itself), so this is
            // the only way its client learns them and draws its outer layer (#17123).
            if g.info.model_customisation != g.sent_skin {
                g.sent_skin = g.info.model_customisation;
                g.send(&C::SetEntityData(play::SetEntityData {
                    id: g.entity_id,
                    values: vec![(16, serializer::BYTE, Value::Byte(g.sent_skin as i8))],
                }));
            }
            // ServerEntity.sendDirtyEntityData: the arrows and bee stingers stuck
            // in the player go to the player too, whose own model draws them.
            if g.stuck != g.sent_stuck {
                let values = [
                    (12, g.stuck.0, g.sent_stuck.0),
                    (13, g.stuck.1, g.sent_stuck.1),
                ]
                .into_iter()
                .filter(|(_, now, sent)| now != sent)
                .map(|(index, now, _)| (index, serializer::INT, Value::Int(now)))
                .collect();
                g.sent_stuck = g.stuck;
                g.send(&C::SetEntityData(play::SetEntityData {
                    id: g.entity_id,
                    values,
                }));
            }
            // Sent when the health or the food level changes, or the saturation
            // reaches or leaves zero; a dead player's health of 0 too.
            let health = (g.health, g.food, g.saturation == 0.0);
            if g.sent_health != Some(health) {
                g.send(&C::SetHealth(play::SetHealth {
                    health: g.health,
                    food: g.food,
                    saturation: g.saturation,
                }));
                g.sent_health = Some(health);
            }
        });
        let xp = {
            let g = &self.guests[i];
            (g.xp_progress, g.xp_level, g.xp_total)
        };
        if self.guests[i].sent_xp != Some(xp) {
            self.guests.update(i, |g| {
                g.send(&C::SetExperience(play::SetExperience {
                    progress: xp.0,
                    level: xp.1,
                    total: xp.2,
                }));
                g.sent_xp = Some(xp);
            });
        }
        self.guests.update(i, |g| {
            if g.score != g.sent_score {
                g.sent_score = g.score;
                g.send(&C::SetEntityData(play::SetEntityData {
                    id: g.entity_id,
                    values: vec![(18, serializer::INT, Value::Int(g.score))],
                }));
            }
        });
        // AbstractContainerMenu.broadcastChanges for the inventory menu: the
        // slots are compared where they lie and only the changed ones copied.
        let mut changes = Vec::new();
        for slot in 0..46 {
            let now = self.guests[i].menu_slot_ref(slot);
            if now != self.guests[i].menu_sent[slot].as_ref() {
                changes.push(slot);
            }
        }
        for slot in changes {
            let now = self.guests[i].menu_slot(slot);
            let net = self.stack(&now);
            self.guests.update(i, |g| {
                g.menu_state = (g.menu_state + 1) & 0x7FFF;
                g.send(&C::ContainerSetSlot(play::ContainerSetSlot {
                    container: 0,
                    state: g.menu_state,
                    slot: slot as i16,
                    item: net,
                }));
                g.menu_sent[slot] = now;
            });
        }
    }
    /// `ServerRecipeBook.sendInitialRecipeBook`: each book's open and
    /// filtering flags, then every known recipe.
    fn send_initial_recipe_book(&self, i: usize) {
        let g = &self.guests[i];
        for packet in initial_recipe_book_packets(&self.sync, &g.recipe_book) {
            g.send(&packet);
        }
    }
    /// `PlayerList.reloadResources` (`/reload`): every player's advancements
    /// reload against the new data, then everyone gets the tags, and each
    /// player the recipes and their recipe book again.
    pub fn reload_resources(&mut self, sync: Arc<SyncData>, owners: &mut AdvancementOwners<'_>) {
        self.lan.shared.set_sync(sync.clone());
        self.sync = sync.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        for i in 0..self.guests.len() {
            // Converted under the old registries, even if advancements are not loaded.
            self.guests
                .update(i, |g| g.equipment_cache = Default::default());
            // PlayerAdvancements.reload: isFirstPacket again, the progress
            // read against the new tree; a player whose advancements were
            // never loaded does that on their first tick.
            if !self.guests[i].advancements_loaded {
                continue;
            }
            self.guests.update(i, |g| g.advancements_visible.clear());
            let Some(state) = owners.state(&mut self.guests, i) else {
                continue;
            };
            state.changed.clear();
            if self.mods_loaded {
                sync.advancements.prune_progress(state.progress);
                let grants = sync.advancements.tick_candidates(state.progress);
                let g = &self.guests[i];
                self.native_advancement_grants.push(TickGrants {
                    conn: g.conn,
                    player: g.uuid,
                    entity_id: g.entity_id,
                    name: g.name.clone(),
                    now_millis: now,
                    first_tick: false,
                    grants,
                    sync: sync.clone(),
                });
            } else {
                let mut granted = Vec::new();
                if let Some(add) =
                    sync.advancements
                        .reload(&sync, now, state.progress, state.book, &mut granted)
                {
                    self.guests[i].send(&raw(ids::RECIPE_BOOK_ADD, add));
                }
            }
        }
        if self.mods_loaded {
            self.native_reload_pending = true;
            return;
        }
        self.finish_reload_resources(owners);
    }

    /// Finish a reload only after its tick grants and their hooks have run.
    pub fn finish_reload_resources(&mut self, owners: &mut AdvancementOwners<'_>) {
        self.native_reload_pending = false;
        let sync = self.sync.clone();
        // broadcastAll(ClientboundUpdateTagsPacket).
        self.broadcast(&C::UpdateTags(sync.tags.clone()));
        // For each player: ClientboundUpdateRecipesPacket, then sendInitialRecipeBook.
        for i in 0..self.guests.len() {
            if !self.guests[i].placed {
                continue;
            }
            self.guests[i].send(&raw(ids::UPDATE_RECIPES, sync.recipes.clone()));
            if self.guests[i].brand == "horizon" {
                super::recipes::send_content(&self.guests[i]);
            }
            if let Some(state) = owners.state(&mut self.guests, i) {
                let packets = initial_recipe_book_packets(&sync, state.book);
                for packet in packets {
                    self.guests[i].send(&packet);
                }
            }
        }
    }

    fn advancement_task_guest(&self, task: &TickGrants) -> Option<usize> {
        if !Arc::ptr_eq(&self.sync, &task.sync) {
            return None;
        }
        self.guests.iter().position(|g| {
            g.conn == task.conn && g.uuid == task.player && g.entity_id == task.entity_id
        })
    }

    /// The task's player's progress, from the same owner used for its mutation.
    pub fn advancement_task_progress<R>(
        &mut self,
        task: &TickGrants,
        owners: &mut AdvancementOwners<'_>,
        read: impl FnOnce(&crate::advancements::Progress) -> R,
    ) -> Option<R> {
        let i = self.advancement_task_guest(task)?;
        let state = owners.state(&mut self.guests, i)?;
        Some(read(state.progress))
    }

    /// Grant one queued criterion. The caller fires its hook before `reward_native_tick`.
    pub fn grant_native_tick(
        &mut self,
        task: &TickGrants,
        grant: &super::advancements::CriterionGrant,
        owners: &mut AdvancementOwners<'_>,
    ) -> Option<bool> {
        let i = self.advancement_task_guest(task)?;
        let state = owners.state(&mut self.guests, i)?;
        task.sync
            .advancements
            .grant_tick(state.progress, grant, task.now_millis)
    }

    /// Apply a completion's rewards after the hook, using the live progress.
    pub fn reward_native_tick(
        &mut self,
        task: &TickGrants,
        grant: &super::advancements::CriterionGrant,
        was_done: bool,
        owners: &mut AdvancementOwners<'_>,
    ) {
        let Some(i) = self.advancement_task_guest(task) else {
            return;
        };
        let Some(state) = owners.state(&mut self.guests, i) else {
            return;
        };
        if let Some(add) = task.sync.advancements.reward_tick(
            &task.sync,
            state.progress,
            state.book,
            grant,
            was_done,
        ) {
            self.guests[i].send(&raw(ids::RECIPE_BOOK_ADD, add));
        }
    }

    /// Send the first advancement snapshot after all its grants and hooks.
    pub fn finish_native_first_tick(
        &mut self,
        task: &TickGrants,
        fresh: &[super::advancements::CriterionGrant],
        owners: &mut AdvancementOwners<'_>,
    ) {
        if !task.first_tick {
            return;
        }
        let Some(i) = self.advancement_task_guest(task) else {
            return;
        };
        let Some(state) = owners.state(&mut self.guests, i) else {
            return;
        };
        let mut unlocked = std::mem::take(state.unlocked);
        unlocked.retain(|id| state.book.knows(id));
        let add = task.sync.advancements.recipes_added(&unlocked, &task.sync);
        let (update, visible) = task.sync.advancements.first_snapshot(
            &task.sync,
            state.progress,
            fresh,
            task.now_millis,
        );
        state.changed.clear();
        self.guests.update(i, |guest| {
            if let Some(add) = add {
                guest.send(&raw(ids::RECIPE_BOOK_ADD, add));
            }
            guest.send(&raw(ids::UPDATE_ADVANCEMENTS, update));
            guest.advancements_visible = visible;
        });
    }

    /// `PlayerAdvancements.flushDirty` and the recipes the player unlocked
    /// since (`ServerRecipeBook.addRecipes`), once the player's first tick
    /// has sent their advancements.
    pub fn flush_progress(&mut self, conn: u32) {
        if let Some(i) = self.guests.iter().position(|g| g.conn == conn)
            && self.guests[i].sent_health.is_some()
        {
            self.flush_advancements(i, true);
            // ServerPlayer.doTick: sendPostEffects when postEffectsDirty.
            if self.guests[i].post_effects_dirty {
                self.send_post_effects(i);
            }
        }
    }
    fn flush_advancements(&mut self, i: usize, show: bool) {
        let sync = self.sync.clone();
        self.guests.update(i, |g| {
            let mut unlocked = std::mem::take(&mut g.recipes_unlocked);
            // A recipe given and taken before this flush must not reappear.
            unlocked.retain(|id| g.recipe_book.knows(id));
            if let Some(add) = sync.advancements.recipes_added(&unlocked, &sync) {
                g.send(&raw(ids::RECIPE_BOOK_ADD, add));
            }
            let changed = std::mem::take(&mut g.advancements_changed);
            if let Some(update) = sync.advancements.update(
                &sync,
                &g.advancements,
                &mut g.advancements_visible,
                &changed,
                show,
            ) {
                g.send(&raw(ids::UPDATE_ADVANCEMENTS, update));
            }
        })
    }
    /// ClientboundAwardStatsPacket (`ServerStatsCounter.sendStats`): each
    /// statistic as its type and value by network id (`Stat.STREAM_CODEC`),
    /// then its count. Statistics the registries don't know are left out.
    pub fn send_stats(&self, conn: u32, stats: &[(String, String, i32)]) {
        let known: Vec<(i32, i32, i32)> = stats
            .iter()
            .filter_map(|(ty, key, count)| {
                let values = match ty.as_str() {
                    "minecraft:mined" => "minecraft:block",
                    "minecraft:killed" | "minecraft:killed_by" => "minecraft:entity_type",
                    "minecraft:custom" => "minecraft:custom_stat",
                    _ => "minecraft:item",
                };
                Some((
                    self.sync.id("minecraft:stat_type", ty)?,
                    self.sync.id(values, key)?,
                    *count,
                ))
            })
            .collect();
        let mut w = horizon_protocol::buf::Writer::new();
        w.varint(known.len() as i32);
        for (ty, value, count) in known {
            w.varint(ty);
            w.varint(value);
            w.varint(count);
        }
        self.send_to(conn, &raw(ids::AWARD_STATS, w.data));
    }
    // ---- incoming ------------------------------------------------------
    /// The squared movement past 1.0E-5F (handlePlayerKnownMovement).
    fn moved_far(&self, i: usize, to: [f64; 3]) -> bool {
        let from = self.guests[i].position;
        let d: f64 = (0..3).map(|k| (to[k] - from[k]).powi(2)).sum();
        d > f64::from(1.0E-5_f32)
    }

    /// handlePlayerCommand's START_SPRINTING and STOP_SPRINTING: `setSprinting`, whose movement
    /// speed modifier the attributes then sync to the client (which takes the modifiers as sent:
    /// one without it would slow a sprinting client to a walk).
    fn set_sprinting(&mut self, i: usize, sprinting: bool) {
        self.guests.update(i, |g| {
            g.sprinting = sprinting;
            g.client_sprinting = sprinting;
            if let Some(m) = g.motion.as_mut() {
                let _ = m.set_sprint_modifier(sprinting);
            }
        });
    }

    fn process_inbox(
        &mut self,
        i: usize,
        ctx: &mut TickCtx,
        events: &mut Vec<HostEvent>,
        begin: bool,
        held: &mut bool,
    ) -> bool {
        let mut credits = Vec::new();
        // ServerGamePacketListenerImpl.tickPlayer ran between the last
        // tick's packets and these: resetPosition, and the moves it counted.
        if begin {
            self.guests.update(i, |g| {
                g.first_good = g.position;
                g.known_moves = g.received_moves;
                // ServerPlayer.tick: tickClientLoadTimeout.
                g.load_timeout = g.load_timeout.saturating_sub(1);
            });
        }
        // An action the game runs after this pass has a move queued behind it: the
        // move waits for the next tick, but the arm swing the client sent right after
        // the action is still handled with it, as vanilla handles both in this tick
        // (the swing starts the attack strength again, #7817).
        loop {
            if self.discard_closed_input(i) || self.holds_for_move(i) {
                if self.pause_for_move(i, &mut credits, events) {
                    return true;
                }
                break;
            }
            if *held
                && !matches!(
                    self.guests[i].inbox.front(),
                    Some(InboxItem::Packet(S::Punch(_), _) | InboxItem::Raw(_, _))
                )
            {
                break;
            }
            let Some(entry) = self.guests.update(i, |g| g.inbox.pop_front()) else {
                break;
            };
            let packet = match entry {
                InboxItem::Raw(delivery, credit) => {
                    if yield_raw(delivery, credit, events, &mut credits) {
                        return true;
                    }
                    continue;
                }
                InboxItem::Packet(packet, credit) => {
                    credits.extend(credit);
                    packet
                }
            };
            let conn = self.guests[i].conn;
            // ServerPlayer.resetLastActionTime: every handler of a player's
            // own action resets it, movement only when it moved
            // (handlePlayerKnownMovement), PlayerInput once loaded.
            let acted = activity::counts_as_activity(
                &packet,
                self.guests[i].client_loaded(),
                self.guests[i].vehicle_good,
                |to| self.moved_far(i, to),
            );
            if acted {
                {
                    let value = self.ticks;
                    self.guests
                        .update(i, |guest| guest.last_action_tick = value);
                }
            }
            match packet {
                // hasClientLoaded: these handlers ignore the player's actions until
                // the client says the level around it is loaded (or 60 ticks pass),
                // and while it is dead.
                S::PlayerAction(_)
                | S::UseItemOn(_)
                | S::UseItem(_)
                | S::Attack(_)
                | S::Interact(_)
                | S::SpectatorAction(_)
                | S::PlayerCommand(_)
                    if !self.guests[i].client_loaded() => {}
                // handleAcceptTeleportPacket.
                S::AcceptTeleportation(p) => {
                    let recording = self.recording();
                    let g = &self.guests[i];
                    if p.id == g.teleport_counter && g.awaiting_teleport.is_none() {
                        self.kick(conn, invalid_movement());
                        continue;
                    }
                    if let Some((id, at)) = g.awaiting_teleport
                        && id == p.id
                    {
                        if p.x.is_nan()
                            || p.y.is_nan()
                            || p.z.is_nan()
                            || !p.yaw.is_finite()
                            || !p.pitch.is_finite()
                        {
                            self.kick(conn, invalid_movement());
                            continue;
                        }
                        self.guests.update(i, |g| {
                            g.position = at;
                            g.awaiting_teleport = None;
                            // The prediction's replay starts again where the client now is.
                            if recording {
                                let kept = g.awaiting_keeps_motion;
                                g.record.get_or_insert_default().teleported(at, kept);
                            }
                        });
                        let rot = (p.yaw, p.pitch);
                        self.position_change(
                            i,
                            at,
                            rot,
                            0,
                            (true, true),
                            Hearing::Confirmation,
                            ctx,
                            events,
                        );
                    }
                }
                // handlePlayerAbilities: the client flies only if it may.
                S::PlayerAbilities(p) => {
                    self.guests.update(i, |g| {
                        let flying = p.flags & 2 != 0 && g.abilities.may_fly;
                        // The change is the game's to announce (the mods may refuse it).
                        if flying != g.abilities.flying {
                            g.abilities.flying = flying;
                            events.push(HostEvent::Flight { conn, flying });
                        }
                    });
                }
                S::MovePlayerPos(p) => {
                    self.move_player(i, Some([p.x, p.y, p.z]), None, p.flags, ctx, events);
                }
                S::MovePlayerPosRot(p) => {
                    let (pos, rot) = ([p.x, p.y, p.z], (p.yaw, p.pitch));
                    self.move_player(i, Some(pos), Some(rot), p.flags, ctx, events);
                }
                S::MovePlayerRot(p) => {
                    self.move_player(i, None, Some((p.yaw, p.pitch)), p.flags, ctx, events);
                }
                S::MovePlayerStatus(p) => {
                    self.move_player(i, None, None, p.flags, ctx, events);
                }
                S::CustomPayload(p)
                    if p.channel == horizon_protocol::passage_sync::CROSS_CHANNEL =>
                {
                    *held = self.passage_leg(i, p, ctx, events);
                }
                // handleAcceptPlayerLoad: markClientLoaded.
                S::PlayerLoaded(_) => {
                    self.guests.update(i, |g| {
                        g.loaded = true;
                        g.loading = false;
                        g.load_timeout = 0;
                        events.push(HostEvent::PlayerLoaded(conn));
                    });
                }
                // PlayerChunkSender.onChunkBatchReceivedByClient.
                S::ChunkBatchReceived(p) => {
                    self.guests.update(i, |g| {
                        g.unacknowledged = g.unacknowledged.saturating_sub(1);
                        g.chunks_per_tick = if p.chunks_per_tick.is_nan() {
                            0.01
                        } else {
                            p.chunks_per_tick.clamp(0.01, 64.0)
                        };
                        if g.unacknowledged == 0 {
                            g.batch_quota = 1.0;
                        }
                        g.max_unacknowledged = 10;
                    });
                }
                S::Pong(p) => {
                    self.anticheat_pong(i, p.id);
                    // The event only tells the mods, and only of an answer to a ping the server
                    // sent (used up by it): nothing is queued for the rest, or with no listener.
                    // Without a listener the ledger is still kept, so one listening later is not
                    // fed stale pings.
                    let answers_a_ping = self.guests.get(i).is_some_and(|g| {
                        g.pings
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .answered(p.id, std::time::Instant::now())
                    });
                    if self.pong_listened && answers_a_ping {
                        events.push(HostEvent::Pong { conn, id: p.id });
                    }
                }
                // ServerGamePacketListenerImpl.handlePingRequest: echo on this connection.
                S::PingRequest(p) => self.send_to(conn, &C::PongResponse(p)),
                // handleAttack is the game's too: a move queued behind an attack would move
                // the player (or land them on the ground, losing a falling attack's critical
                // hit) before the game checks their reach and state.
                S::Attack(p) => {
                    if !self.anticheat_attack(i, p.id, events) {
                        events.push(HostEvent::Packet(conn, Box::new(S::Attack(p))));
                        *held = self.move_queued(i);
                    }
                }
                S::Punch(p) => {
                    self.anticheat_swing(i);
                    events.push(HostEvent::Packet(conn, Box::new(S::Punch(p))));
                }
                S::ClientTickEnd(_) => {
                    // handleClientTickEnd: no move this tick, no known movement.
                    // A rider's move came with its vehicle's packet, not its own.
                    // The movement reset is queued behind the packets already handed
                    // to the game: a MoveVehicle of this drain sets the movement when
                    // the game dispatches it, and the reset must keep that move for the
                    // tick, not zero it before the move runs (vanilla's main thread
                    // handles the two in packet order). The position flag and the
                    // anticheat tick end have no queued move to wait for.
                    self.guests.update(i, |g| {
                        g.position_this_tick = false;
                    });
                    events.push(HostEvent::TickEnd(conn));
                    self.anticheat_tick_end(i, events);
                }
                // ServerGamePacketListenerImpl.handleRecipeBookSeenRecipePacket:
                // the display the player looked at stops being a new one, so
                // the next `RecipeBookAdd` it is sent does not animate.
                S::RecipeBookSeenRecipe(p) => {
                    let seen = self
                        .sync
                        .advancements
                        .recipe_of_display(p.recipe)
                        .map(str::to_owned);
                    if let Some(id) = seen {
                        self.guests
                            .update(i, |g| g.recipe_book.remove_highlight(&id));
                    }
                }
                // ServerGamePacketListenerImpl.handleSeenAdvancements: the tab the
                // player opened, kept when it names a root with a display
                // (`PlayerAdvancements.setSelectedTab` leaves the tab empty otherwise),
                // and answered with the tab it now keeps when that changed. An id the
                // server has no advancement for changes nothing.
                S::SeenAdvancements(p) => {
                    if p.action == 0
                        && let Some(id) = p.tab
                        && let Some(keep) = self.sync.advancements.selectable(&id)
                    {
                        let tab = keep.then_some(id);
                        self.guests.update(i, |g| {
                            if g.selected_tab != tab {
                                g.selected_tab = tab.clone();
                                g.send(&C::SelectAdvancementsTab(play::SelectAdvancementsTab {
                                    tab,
                                }));
                            }
                        });
                    }
                }
                // ServerGamePacketListenerImpl.handleRecipeBookChangeSettingsPacket.
                S::RecipeBookChangeSettings(p) => {
                    if let Some(book) = crate::recipe_book::BookType::ALL.get(p.book_type as usize)
                    {
                        self.guests.update(i, |g| {
                            let settings = &mut g.recipe_book;
                            let changed = settings.open[book.index()] != p.open
                                || settings.filtering[book.index()] != p.filtering;
                            settings.open[book.index()] = p.open;
                            settings.filtering[book.index()] = p.filtering;
                            if changed {
                                events.push(HostEvent::RecipeBookSettings {
                                    conn,
                                    book: p.book_type,
                                    open: p.open,
                                    filtering: p.filtering,
                                });
                            }
                        });
                    }
                }
                S::ClientInformation(info) => {
                    let (was_hat, hat) = self.guests.update(i, |g| {
                        // PlayerModelPart.HAT (bit 6): the tab list shows the hat layer.
                        let was_hat = g.info.model_customisation & 0x40 != 0;
                        // max(2).min(view), not clamp: a live management value
                        // below 2 would cross the bounds and panic (horizon-rust/horizon#8118).
                        g.view_distance = i32::from(info.view_distance)
                            .max(2)
                            .min(self.config.view_distance);
                        let old_info = self.mods_loaded.then(|| Box::new(g.info.clone()));
                        g.info = info;
                        if let Some(old) = old_info {
                            let new = Box::new(g.info.clone());
                            events.push(HostEvent::ClientInformation(conn, old, new));
                        }
                        g.center = None;
                        let hat = g.info.model_customisation & 0x40 != 0;
                        // The status sample lists the player only if they allow it.
                        self.lan.shared.set_listed(g.uuid, g.info.allows_listing);
                        (was_hat, hat)
                    });
                    // ServerGamePacketListenerImpl.handleClientInformation: UPDATE_HAT to everyone.
                    if hat != was_hat {
                        self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                            actions: 128,
                            entries: vec![play::PlayerInfo {
                                uuid: self.guests[i].uuid,
                                show_hat: Some(hat),
                                ..Default::default()
                            }],
                        }));
                    }
                }
                S::PlayerInput(p) => {
                    self.guests.update(i, |g| {
                        g.input = p.flags;
                        if g.client_loaded() {
                            // The events only tell the mods: nothing is queued with none loaded.
                            let listening = self.mods_loaded;
                            if listening {
                                events.push(HostEvent::Input {
                                    conn,
                                    flags: i32::from(p.flags),
                                });
                            }
                            let sneaking = p.flags & 32 != 0;
                            if g.sneaking != sneaking {
                                g.sneaking = sneaking;
                                if listening {
                                    events.push(HostEvent::Sneak { conn, sneaking });
                                }
                            }
                        }
                    });
                    // handlePlayerInput only records keys and updates crouching.
                    // Sprinting persists after the sprint key is released (or
                    // starts with a double tap); PlayerCommand changes it.
                }
                // OPEN_INVENTORY (the ridden mount's inventory) and
                // START_FALL_FLYING (checked against the guest's chest slot
                // and effects) are the game's.
                // START_RIDING_JUMP and STOP_RIDING_JUMP: the vehicle's
                // PlayerRideableJumping (handleStartJump / handleStopJump).
                S::PlayerCommand(p) if matches!(p.action, 3 | 4) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::PlayerCommand(p))));
                }
                S::PlayerCommand(p) if matches!(p.action, 5 | 6) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::PlayerCommand(p))));
                }
                S::PlayerCommand(p) => match p.action {
                    1 | 2 => {
                        let sprinting = p.action == 1;
                        let changed = self.guests[i].sprinting != sprinting;
                        self.set_sprinting(i, sprinting);
                        // The game tells the mods of a change.
                        if changed {
                            events.push(HostEvent::Packet(conn, Box::new(S::PlayerCommand(p))));
                        }
                    }
                    // STOP_SLEEPING: the game wakes the guest after the mods hear it
                    // (`stopSleepInBed`), and the rest of this turn waits for that wake:
                    // a move sent after it must not run while the guest still sleeps.
                    0 => {
                        events.push(HostEvent::Packet(conn, Box::new(S::PlayerCommand(p))));
                        *held = true;
                    }
                    _ => {}
                },
                S::Chat(c) => self.handle_chat(i, c, events),
                S::ChatAck(p) => self.chat_ack(i, p.offset, events),
                S::ChatSessionUpdate(p) => self.chat_session_update(i, p, events),
                // handlePlayerAction is the game's, run after this pass, and a move
                // queued behind it waits for the next tick, as vanilla handles the
                // packets in order. For a dig starting, stopping or finishing, the
                // move is checked against the block the player just broke and falls
                // through, so it would be sent back. A drop, a RELEASE_USE_ITEM (a
                // bow's arrow, a trident, a spear) and a stab use the player's
                // position and known movement now (#8423): shot after the move, an
                // arrow left the position of the tick after the release, a tick
                // ahead of the camera's instead of a tick behind it. A STAB (8) does
                // not hold the move: held a tick, the jump the move carries
                // (handlePlayerPositionChange's jumpFromGround) ran a tick after the
                // Lunge's impulse, and the server took two moves in the next tick
                // (lunge_motion, every jump phase).
                S::PlayerAction(p) => {
                    let stab = p.action == 8;
                    events.push(HostEvent::Packet(conn, Box::new(S::PlayerAction(p))));
                    *held = !stab && self.move_queued(i);
                }
                // handleUseItem uses the player's position and known movement now:
                // a later move must not overtake a throw before the game runs it.
                S::UseItem(p) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::UseItem(p))));
                    *held = self.move_queued(i);
                }
                // handleUseItemOn is the game's too, and BlockItem.canPlace checks
                // the player's box where they clicked from (Level.isUnobstructed), as
                // their client's prediction did. After the move behind it, a player
                // jumping up a pillar was already clear of the cell their client had
                // refused: the block went down without the place sound that only the
                // placer's client plays (BlockItem.place's playSound(player, ...)).
                S::UseItemOn(p) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::UseItemOn(p))));
                    *held = self.move_queued(i);
                }
                // handleInteract is the game's too, like handleAttack above: a move queued
                // behind it waits for the next tick.
                S::Interact(p) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::Interact(p))));
                    *held = self.move_queued(i);
                }
                // ContainerInput's codec (ByIdMap, OutOfBoundsStrategy.ZERO) reads an
                // unknown click type as PICKUP.
                S::ContainerClick(mut p) => {
                    if !(0..=6).contains(&p.input) {
                        p.input = 0;
                    }
                    events.push(HostEvent::Packet(conn, Box::new(S::ContainerClick(p))));
                }
                // ServerboundSelectBundleItemPacket's constructor: no index below -1.
                S::SelectBundleItem(p) if p.index < -1 => {
                    let reason = format!("Invalid selectedItemIndex: {}", p.index);
                    self.kick(conn, literal(&reason));
                }
                other => events.push(HostEvent::Packet(conn, Box::new(other))),
            }
        }
        events.push(HostEvent::InputCredits(credits));
        false
    }

    /// `handleClientTickEnd`'s movement reset (`ServerGamePacketListenerImpl.java:2256`),
    /// applied by the game when it reaches the queued [`HostEvent::TickEnd`]: after the
    /// `MoveVehicle` packets queued before it, so a rider's move of the tick is kept and
    /// only a tick with no move of either zeroes the known movement. A guest that left
    /// mid-drain has nothing to reset.
    pub fn client_tick_end(&mut self, conn: u32) {
        let Some(i) = self.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        self.guests.update(i, |g| {
            if !g.movement_this_tick {
                g.known_movement = [0.0; 3];
            }
            g.movement_this_tick = false;
        });
    }

    /// ServerGamePacketListenerImpl.handleMovePlayer.
    fn move_player(
        &mut self,
        i: usize,
        pos: Option<[f64; 3]>,
        rot: Option<(f32, f32)>,
        flags: u8,
        ctx: &TickCtx,
        events: &mut Vec<HostEvent>,
    ) {
        let shape = (pos.is_some(), rot.is_some());
        let conn = self.guests[i].conn;
        // containsInvalidValues, on the packet's values (0 where it has none).
        let (yaw, pitch) = rot.unwrap_or_default();
        if pos.is_some_and(|p| p.iter().any(|v| v.is_nan()))
            || !yaw.is_finite()
            || !pitch.is_finite()
        {
            self.kick(conn, invalid_movement());
            return;
        }
        if self.anticheat.is_some() {
            self.anticheat_moved(i, pos, rot, flags);
        }
        let repeated = self.guests.update(i, |g| {
            if pos.is_some() {
                if g.position_this_tick {
                    return true;
                }
                g.position_this_tick = true;
            }
            false
        });
        if repeated {
            self.kick(conn, invalid_movement());
            return;
        }
        let g = &self.guests[i];
        // handleMovePlayer: nothing moves before the client has loaded.
        if g.landing || !g.client_loaded() {
            return;
        }
        // updateAwaitingTeleport: moves wait for the client to accept the
        // teleport, which is sent again after 20 ticks without an answer.
        if let Some((_, at)) = g.awaiting_teleport {
            if self.ticks - g.teleport_tick > 20 {
                self.resend_teleport(i, Some(at));
            }
            return self.snap_rotation(i, rot);
        }
        let to = pos.unwrap_or(g.position);
        let rot = rot.unwrap_or((g.yaw, g.pitch));
        self.position_change(i, to, rot, flags, shape, Hearing::Heard, ctx, events);
    }

    /// `handleMovePlayer` while a teleport awaits its accept (`updateAwaitingTeleport`): the move's
    /// position is ignored, its turn is not: `absSnapRotationTo(Mth.wrapDegrees(yRot),
    /// Mth.wrapDegrees(xRot))`, which clamps the pitch to 90 degrees either way. A move without a
    /// turn carries the player's own, so it changes nothing.
    fn snap_rotation(&mut self, i: usize, rot: Option<(f32, f32)>) {
        if let Some((yaw, pitch)) = rot {
            self.guests.update(i, |g| {
                g.yaw = wrap_degrees(yaw);
                g.pitch = wrap_degrees(pitch).clamp(-90.0, 90.0);
            });
        }
    }

    /// ServerGamePacketListenerImpl.handlePlayerPositionChange.
    fn position_change(
        &mut self,
        i: usize,
        to: [f64; 3],
        (yaw, pitch): (f32, f32),
        flags: u8,
        (has_position, has_rotation): (bool, bool),
        hearing: Hearing,
        ctx: &TickCtx,
        events: &mut Vec<HostEvent>,
    ) {
        let (frozen, (check, elytra_check)) = (self.ticking.1, self.movement_checks);
        let to = [
            to[0].clamp(-3.0E7, 3.0E7),
            to[1].clamp(-2.0E7, 2.0E7),
            to[2].clamp(-3.0E7, 3.0E7),
        ];
        let (yaw, pitch) = (wrap_degrees(yaw), wrap_degrees(pitch));
        let begin = self.guests.update(i, |g| {
            let owner = self.singleplayer_owner == Some(g.uuid);
            // A passenger only turns: its vehicle carries it (absSnapTo where it is).
            if g.vehicle.is_some() {
                (g.yaw, g.pitch) = (yaw, pitch);
                g.release_tickets();
                return std::ops::ControlFlow::Break(false);
            }
            let d = [0, 1, 2].map(|k| to[k] - g.first_good[k]);
            let moved = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            if g.sleeping {
                if moved > 1.0 {
                    (g.yaw, g.pitch) = (yaw, pitch);
                    return std::ops::ControlFlow::Break(true);
                }
                return std::ops::ControlFlow::Break(false);
            }
            // TickRateManager.runsNormally: not frozen (a frozen game's steps
            // are not counted either; Horizon's hub does not see them).
            if !frozen {
                g.received_moves += 1;
                let mut packets = g.received_moves - g.known_moves;
                if packets > 5 {
                    packets = 1;
                }
                // shouldCheckPlayerMovement (never the single-player owner).
                if !owner && check && (!g.fall_flying || elytra_check) {
                    let limit = if g.fall_flying { 300.0 } else { 100.0 };
                    // `movedDist - expectedDist` (getDeltaMovement): the velocity
                    // the server last simulated for the guest is expected
                    // movement, not a violation — a landing glide keeps its speed.
                    let expected = g.motion.as_ref().map_or(0.0, |m| {
                        let v = m.velocity;
                        v.x * v.x + v.y * v.y + v.z * v.z
                    });
                    if moved - expected > limit * f64::from(packets) {
                        events.push(HostEvent::Warn(format!(
                            "{} moved too quickly! {},{},{}",
                            g.name,
                            horizon_core::java_number::double_to_string(d[0]),
                            horizon_core::java_number::double_to_string(d[1]),
                            horizon_core::java_number::double_to_string(d[2])
                        )));
                        return std::ops::ControlFlow::Break(true);
                    }
                }
            }
            std::ops::ControlFlow::Continue(g.position)
        });
        let start = match begin {
            std::ops::ControlFlow::Continue(start) => start,
            std::ops::ControlFlow::Break(resend) => {
                if resend {
                    self.resend_teleport(i, None);
                }
                return;
            }
        };
        let (wrongly, refused, stands) = self.collide(i, to, ctx);
        if wrongly {
            let line = format!("{} moved wrongly!", self.guests[i].name);
            events.push(HostEvent::Warn(line));
        }
        if refused {
            self.guests.update(i, |g| (g.yaw, g.pitch) = (yaw, pitch));
            self.resend_teleport(i, Some(start));
            return;
        }
        // The anticheat's `ground`: a claim of the ground with nothing to
        // stand on is dropped (no-fall, walking on water).
        let client_on_ground = flags & 1 != 0;
        let (mut flags, mut corrected, mut again) = (flags, false, false);
        if flags & 1 != 0 && self.anticheat.is_some() {
            use super::anticheat::GroundRefusal;
            match self.anticheat_ground(i, to, stands, ctx, events) {
                Some(GroundRefusal::Kicked) => {
                    self.guests.update(i, |g| (g.yaw, g.pitch) = (yaw, pitch));
                    return;
                }
                Some(GroundRefusal::Dropped) => (flags, corrected) = (flags & !1, true),
                Some(GroundRefusal::Again) => (flags, again) = (flags & !1, true),
                None => {}
            }
        }
        // clientIsFloating, from the move that is accepted: no upward-or-level
        // step onto something, nothing but air around the body, and nothing
        // that lets the player hover (the server's allow-flight, mayfly,
        // levitation, gliding, riptide). A move whose ground claim the anticheat dropped never counts:
        // a false claim ends in the anticheat's own kick, logged and told to the operators, or not at all.
        // One it dropped again without a flag (standing still where one was dropped) counts.
        let floating = !corrected && ctx.collision.is_some() && {
            let g = &self.guests[i];
            to[1] - g.position[1] >= -0.03125
                && !stands
                && !g.spectator()
                && !self.config.allow_flight
                && !g.creative()
                && g.effects.get("levitation").is_none()
                && !g.fall_flying
                && !g
                    .motion
                    .as_ref()
                    .is_some_and(|m| m.auto_spin_attack_ticks > 0)
                && self.no_blocks_around(i, to, ctx.world)
        };
        // handlePlayerPositionChange: a client that says it is on the ground while the last move
        // the server resolved came down onto nothing is sent the blocks under it again, so a
        // client that missed a block change cannot stand on air for good.
        if (flags & 1 != 0 || again) && !self.guests[i].rests_on_ground {
            self.force_send_player_support_blocks(i, ctx);
        }
        let accepted = Accepted {
            to,
            yaw,
            pitch,
            flags,
            stands,
            floating,
            on_ground: client_on_ground,
            has_position,
            has_rotation,
        };
        self.accept_move(i, accepted, hearing, events);
    }

    /// `ServerGamePacketListenerImpl.noBlocksAround`: every block of the
    /// player's box grown by 0.0625 and stretched 0.55 down is air. Vanilla
    /// loads a chunk it reads; a chunk not loaded here yet (a player's first
    /// move after joining, before the spawn chunks are generated) is not
    /// known to be air, so it never makes the player float.
    fn no_blocks_around(&self, i: usize, at: [f64; 3], world: &World) -> bool {
        let d = self.guests[i].dimensions();
        let half = f64::from(d.width) / 2.0 + 0.0625;
        let lo = [at[0] - half, at[1] - 0.0625 - 0.55, at[2] - half];
        let hi = [
            at[0] + half,
            at[1] + f64::from(d.height) + 0.0625,
            at[2] + half,
        ];
        no_blocks_in(world, lo, hi, &self.encoder)
    }

    /// `ServerGamePacketListenerImpl.forceSendPlayerSupportBlocks`: the blocks just under the
    /// player's box, at the box's four corners a hundred-thousandth below its floor, each once.
    /// A client whose block below went missing without it noticing keeps standing on that block
    /// on the server, so the server tells it what is there (`ClientboundBlockUpdatePacket`, and
    /// the block entity's update tag where it has one, `BlockEntity.getUpdatePacket`). No more
    /// than once every 200 ticks of the connection, so a client that stands on air for good is
    /// not sent four packets four times a second.
    fn force_send_player_support_blocks(&mut self, i: usize, ctx: &TickCtx) {
        let tick = self.ticks.saturating_sub(self.guests[i].joined_tick);
        if tick < self.guests[i].support_blocks_sent.saturating_add(200) {
            return;
        }
        {
            let value = tick;
            self.guests
                .update(i, |guest| guest.support_blocks_sent = value);
        }
        let conn = self.guests[i].conn;
        let g = &self.guests[i];
        let d = g.dimensions();
        let half = f64::from(d.width) / 2.0;
        let at = g.position;
        // `box.minY - 9.999999747378752E-6`, which is (double) 1.0E-5f.
        let y = at[1] - f64::from(1.0E-5_f32);
        let cell = |v: f64| v.floor() as i32;
        let mut positions: Vec<[i32; 3]> = Vec::with_capacity(4);
        for (x, z) in [
            (at[0] - half, at[2] - half),
            (at[0] + half, at[2] - half),
            (at[0] - half, at[2] + half),
            (at[0] + half, at[2] + half),
        ] {
            let pos = [cell(x), cell(y), cell(z)];
            if !positions.contains(&pos) {
                positions.push(pos);
            }
        }
        for pos @ [x, y, z] in positions {
            self.send_block_correction(conn, ctx.world, pos);
            if !self.terrain_metadata_visible(conn, ctx.world, pos) {
                continue;
            }
            let state = ctx.world.block(x, y, z);
            let Some(entity) = ctx.world.block_entities.get(&(x, y, z)) else {
                continue;
            };
            let Some(tag) = self.encoder.update_tag((x, y, z), state.0, entity) else {
                continue;
            };
            let Some(kind) = self.encoder.entity_type(state.0) else {
                continue;
            };
            self.send_to(
                conn,
                &C::BlockEntityData(play::BlockEntityData { pos, kind, tag }),
            );
        }
    }

    /// Acknowledge a block interaction (ClientboundBlockChangedAckPacket).
    /// ServerGamePacketListenerImpl.ackBlockChangesUpTo: sent with the next tick's block
    /// changes (`tick`).
    pub fn ack(&self, conn: u32, sequence: i32) {
        if sequence >= 0
            && let Some(g) = self.guest(conn)
        {
            g.ack_up_to.set(g.ack_up_to.get().max(sequence));
        }
    }

    /// Set a guest's game mode and tell everyone (ServerPlayer.setGameMode).
    pub fn set_game_mode(
        &mut self,
        conn: u32,
        mode: i32,
        world: &World,
        collision: Option<&BlockCollisionRegistry<'_>>,
    ) {
        let Some(uuid) = self
            .update_guest(conn, |g| {
                g.change_game_mode(mode);
                // GameType.updatePlayerAbilities: the mode's flags, on the abilities
                // the fork keeps them in (spectator flies, creative keeps what it had).
                g.abilities.update_for_mode(mode);
                // ServerPlayerGameMode.changeGameModeForPlayer: a player left flying
                // by the change lands when the ground is within a block.
                if !g.spectator()
                    && g.abilities.flying
                    && let Some(binding) = collision
                {
                    let mut scene = SceneCollisions {
                        world,
                        binding,
                        subject: None,
                    };
                    if in_range_of_ground(&mut scene, g.bounds()) {
                        g.abilities.flying = false;
                    }
                }
                let uuid = g.uuid;
                g.send(&C::GameEvent(play::GameEvent {
                    event: game_event::CHANGE_GAME_MODE,
                    param: mode as f32,
                }));
                g.send(&g.abilities_packet());
                g.participates_online().then_some(uuid)
            })
            .flatten()
        else {
            return;
        };
        self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 4,
            entries: vec![play::PlayerInfo {
                uuid,
                game_mode: Some(mode),
                ..Default::default()
            }],
        }));
    }

    /// A player's nickname changed (None: their name again): kept for chat,
    /// and the Tab list shows it (PlayerInfo UPDATE_DISPLAY_NAME for
    /// everyone).
    /// Paper's `hidePlayer` then `showPlayer` for every other player (as
    /// SkinsRestorer refreshes a skin): the next tick takes the player out of
    /// their Tab lists and worlds (`sync_hidden`, the entity tracker), the one
    /// after brings them back with their profile as it is then. A level a mod
    /// hid them at is kept.
    pub fn refresh_player(&mut self, uuid: u128) {
        if let Some(pending) = self.refreshing.iter_mut().find(|r| r.0 == uuid) {
            pending.1 = self.ticks;
            return;
        }
        self.refreshing.push((uuid, self.ticks));
    }

    /// The permission level the entity tracker hides a player from: everyone's while the player is
    /// being sent again (`refresh_player`, so watching clients get the removal and the respawn around the
    /// profile update), else the level a mod hid them at.
    pub fn tracker_hidden(&self, uuid: u128) -> Option<u8> {
        if self.refreshing.iter().any(|r| r.0 == uuid) {
            return Some(u8::MAX);
        }
        self.hidden.get(&uuid).copied()
    }
    /// The profile properties the player list gives the guest at `i` now: those it logged in with, or for
    /// the integrated server's owner the signed-in account's once fetched, with a mod's overrides by name.
    pub fn listed_profile_properties(
        &self,
        i: usize,
    ) -> Vec<horizon_protocol::packets::login::Property> {
        let g = &self.guests[i];
        listed_properties(
            &g.properties,
            &g.profile_overrides,
            self.singleplayer_owner == Some(g.uuid),
            self.lan.shared.config.owner_profile.as_deref(),
        )
    }

    /// Shows the players `refresh_player` took out of the lists a whole tick ago again, at the level a mod
    /// hides them at (or to everyone): the refresh never touched that level.
    pub fn end_refreshes(&mut self) {
        let ticks = self.ticks;
        self.refreshing.retain(|r| r.1 + 1 >= ticks);
    }

    pub fn set_nickname(&mut self, uuid: u128, nickname: Option<String>) {
        match nickname {
            Some(n) => self.nicknames.insert(uuid, n),
            None => self.nicknames.remove(&uuid),
        };
        let Some(guest) = self
            .guests
            .iter()
            .find(|g| g.uuid == uuid && g.participates_online())
        else {
            return;
        };
        let display_name = self.tab_list_name(uuid, &guest.name);
        self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 0x20,
            entries: vec![play::PlayerInfo {
                uuid,
                display_name: Some(display_name),
                ..Default::default()
            }],
        }));
    }

    /// `viewer_entity_spawn`: an `AddEntity` of `kind` (its registry id, `entity_type`) with entity id
    /// `id` at `pos`, to `viewer`'s client alone. False when the viewer has no client or `kind` is not
    /// an entity type. The UUID is the id itself: the entity is not in the level.
    pub fn spawn_viewer_entity(
        &mut self,
        viewer: u128,
        id: u32,
        kind: &str,
        pos: [f64; 3],
    ) -> bool {
        let Some(index) = self
            .guests
            .iter()
            .position(|g| g.uuid == viewer && g.participates_online())
        else {
            return false;
        };
        let (types, sync) = (&mut self.entity_types, &self.sync);
        let Some(kind_id) = entity_type(types, sync, kind) else {
            return false;
        };
        self.guests[index].send(&C::AddEntity(play::AddEntity {
            id: id as i32,
            uuid: u128::from(id),
            kind: kind_id,
            position: pos,
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        }));
        self.viewer_entities.insert((viewer, id));
        true
    }

    /// `viewer_entity_move`: a `TeleportEntity` of the client-only entity `id` to `pos`, to `viewer`'s
    /// client alone. False when the viewer has no such entity or no client.
    pub fn move_viewer_entity(&mut self, viewer: u128, id: u32, pos: [f64; 3]) -> bool {
        if !self.viewer_entities.contains(&(viewer, id)) {
            return false;
        }
        let Some(index) = self
            .guests
            .iter()
            .position(|g| g.uuid == viewer && g.participates_online())
        else {
            return false;
        };
        self.guests[index].send(&C::TeleportEntity(play::TeleportEntity {
            id: id as i32,
            position: pos,
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            relatives: 0,
            on_ground: false,
        }));
        true
    }

    /// `viewer_entity_remove`: a `RemoveEntities` of the client-only entity `id`, to `viewer`'s client
    /// alone. False when the viewer has no such entity or no client.
    pub fn remove_viewer_entity(&mut self, viewer: u128, id: u32) -> bool {
        if !self.viewer_entities.remove(&(viewer, id)) {
            return false;
        }
        let Some(index) = self
            .guests
            .iter()
            .position(|g| g.uuid == viewer && g.participates_online())
        else {
            return false;
        };
        self.guests[index].send(&C::RemoveEntities(play::RemoveEntities {
            ids: vec![id as i32],
        }));
        true
    }

    /// Paper's `Player.setPlayerListName`: `name` in place of the player's name in the Tab
    /// list (None: the nickname or real name again), sent to everyone at once
    /// (`ClientboundPlayerInfoUpdatePacket` `UPDATE_DISPLAY_NAME`) and kept for a rejoin.
    pub fn set_list_name(&mut self, uuid: u128, name: Option<crate::text::Text>) {
        match name {
            Some(name) => self.list_names.insert(uuid, name),
            None => self.list_names.remove(&uuid),
        };
        let Some(guest) = self
            .guests
            .iter()
            .find(|g| g.uuid == uuid && g.participates_online())
        else {
            return;
        };
        let display_name = self.tab_list_name(uuid, &guest.name);
        // The held players too: a client held for its hand-over already has this entry (the login sent
        // it), and `place_arrival` sends only the newcomer's own. A viewer a mod hid the player from has no
        // entry for them and is told nothing: the update would give away their UUID and name. The player's
        // own client always has its entry.
        let packet = C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 0x20,
            entries: vec![play::PlayerInfo {
                uuid,
                display_name: Some(display_name),
                ..Default::default()
            }],
        });
        self.broadcast_to(
            &packet,
            self.guests.simulation_iter().filter(|g| {
                g.participates_online()
                    && (g.placed || g.held)
                    && (g.uuid == uuid
                        || (self.sees(g.permission, uuid) && !g.tab_hidden.contains(&uuid)))
            }),
        );
    }

    pub fn disconnect_all(&mut self) {
        let reason = translatable("multiplayer.disconnect.server_shutdown", &[]);
        for g in &self.guests {
            g.sender.disconnect(reason.clone());
        }
    }
}

impl Hub {
    pub fn item_name(&self, id: i32) -> Option<std::sync::Arc<str>> {
        self.sync
            .builtin
            .get("minecraft:item")?
            .name(id)
            .map(std::sync::Arc::from)
    }
    pub fn enchantment_name(&self, id: i32) -> Option<String> {
        self.sync
            .dynamic
            .name("minecraft:enchantment", id)
            .map(str::to_owned)
    }
    /// A stack a client sent (`SetCreativeModeSlot`), read with this
    /// server's registries.
    pub fn stack_from_network(&self, s: &horizon_protocol::item::Stack) -> crate::items::ItemStack {
        crate::item_sync::from_network(s, &self.sync.builtin, &self.sync.dynamic)
    }

    /// ServerLevel.removePlayerImmediately, then addRespawnedPlayer: the other players
    /// lose the dead player's entity (ServerEntity.removePairing) and are sent the
    /// respawned one as a new entity at the next tick. Their clients would otherwise keep
    /// the dead body, lying down, wherever the player goes.
    pub fn respawned(&mut self, id: i32) {
        self.guests
            .update_all(|g| super::viewed_levels::entities::forget_entity(g, id));
    }

    /// A respawn point whose block is gone or blocked: the respawning player
    /// is sent NO_RESPAWN_BLOCK_AVAILABLE and says
    /// `block.minecraft.spawn.not_valid` in its chat.
    pub fn missing_respawn_block(&self, conn: u32) {
        self.send_to(
            conn,
            &C::GameEvent(play::GameEvent {
                event: game_event::NO_RESPAWN_BLOCK_AVAILABLE,
                param: 0.0,
            }),
        );
    }

    /// PlayerList.respawn: back to `at` with a fresh start. The vanilla client waits on its
    /// level loading screen from the Respawn packet until the level info's "level chunks load
    /// start" event and the chunk under the player, so the packets follow vanilla's order.
    /// `rotation` is the facing a mod's `player_respawn_position` chose; none
    /// is the level's respawn direction.
    pub fn respawn(&mut self, i: usize, at: [f64; 3], rotation: Option<[f32; 2]>, ctx: &TickCtx) {
        let info = self.spawn_info(i, 0, ctx.sea_level);
        self.close_container_for(i);
        let id = self.guests.update(i, |g| {
            // The old player leaves its level with its chunks (ChunkMap.removeEntity).
            super::chunk_watch::unwatch_all(&mut self.chunk_watches, g);
            // PlayerList.respawn: into the level in place (the respawn level).
            g.dimension = self.config.dimension.clone();
            // A new player: a mod's collision exclusions were the dead one's (arismoko/horizon#3717).
            g.collision_exclusions.clear();
            // restartClientLoadTimerAfterRespawn: the client loads a whole new level before it
            // reports it loaded.
            g.load_timeout = super::guest::CLIENT_LOADED_TIMEOUT;
            g.loading = true;
            // ServerLevel.removePlayerImmediately: the dead player's
            // Entity.setRemoved (KILLED) stops its riding; the new player rides
            // nothing.
            g.stop_riding();
            // ServerLevel.removePlayerImmediately: every entity paired with the dead player is
            // removed (ServerEntity.removePairing, one packet each). Its chunks are dropped
            // without ForgetLevelChunk (PlayerChunkSender.dropChunk skips dead players): the
            // client keeps them and they are all sent again.
            for id in g.tracker.ids() {
                g.send(&C::RemoveEntities(play::RemoveEntities { ids: vec![id] }));
            }
            g.tracker.clear();
            g.sent_chunks.clear();
            g.far_sent_chunks.clear();
            g.chunk_queue.clear();
            g.far_queue.clear();
            g.viewed.forget_views();
            g.entity_id
        });
        self.respawned(id);
        self.guests.update(i, |g| {
            g.sent_pose = crate::pose::Pose::Standing;
            // PlayerList.respawn: the new tracker sends the hand again
            // (ServerEntity.java:311-312); the default is what a fresh one holds.
            g.sent_main_hand = 1;
            // ClientPacketListener.handleRespawn without KEEP_ENTITY_DATA: the client's new player is
            // built from zero data, so the skin parts its dead one held are gone too. Pairing this
            // reset with the pose's keeps the outer layer sent again on the next tick; without it the
            // mask counted as sent and the layer stayed missing until the skin screen (#17123).
            g.sent_skin = 0;
            // ServerPlayer.restoreFrom: the game mode again (onUpdateAbilities).
            g.send(&g.abilities_packet());
            // The new player faces the level's respawn direction (snapTo), or
            // the one its TeleportTransition carries.
            [g.yaw, g.pitch] = rotation.unwrap_or(ctx.spawn_angle);
            g.place(at);
            g.send(&C::Respawn(play::Respawn {
                spawn: info,
                keep: 0,
            }));
            // ClientPacketListener.handleRespawn without KEEP_ENTITY_DATA: the client's new player
            // starts with wasSprinting false. (A level change keeps it: the new player sprints on, or
            // says STOP.)
            g.client_sprinting = false;
            // The respawned player is a new one, without effects; so is the
            // client's (ClientPacketListener.handleRespawn).
            g.effects = Default::default();
            g.sent_effects.clear();
            g.center = None;
            g.sent_chunks.clear();
            g.far_sent_chunks.clear();
            g.chunk_queue.clear();
            g.far_queue.clear();
            g.tracker.clear();
        });
        self.resend_teleport(i, None);
        let spawn = self.default_spawn(ctx.spawn_dimension, ctx.spawn, ctx.spawn_angle);
        self.guests.update(i, |g| {
            g.send(&spawn);
            g.send(&C::ChangeDifficulty(play::ChangeDifficulty {
                difficulty: ctx.difficulty,
                locked: ctx.difficulty_locked,
            }));
            let xp = (g.xp_progress, g.xp_level, g.xp_total);
            g.send(&C::SetExperience(play::SetExperience {
                progress: xp.0,
                level: xp.1,
                total: xp.2,
            }));
            g.sent_xp = Some(xp);
            // ServerPlayer.sendPostEffects.
            let list = g.post_effects.clone();
            g.post_effects_dirty = false;
            g.send(&C::PostEffects(play::PostEffects { effects: list }));
        });
        self.send_level_info(i, ctx, true);
        self.send_permission_level(i);
        // ServerLevel.addRespawnedPlayer: the chunk tracking starts again at the player; a mod's
        // view centre ends with the player it was set for.
        self.guests.update(i, |g| {
            g.view_center = None;
            let (cx, cz) = g.chunk();
            g.send(&C::SetChunkCacheCenter(play::SetChunkCacheCenter {
                x: cx,
                z: cz,
            }));
            g.center = None;
            g.center_sent = Some((cx, cz));
            // ServerPlayer.initInventoryMenu.
        });
        self.send_inventory(i);
    }

    /// The level the players are in: the server holds one at a time, and the
    /// hub's packets name it.
    pub fn set_dimension(&mut self, dimension: &str) {
        if self.config.dimension == dimension {
            return;
        }
        // What this level's players were told stays with it.
        let leaving = LevelSync {
            last_clock: self.last_clock,
            last_rate: self.last_rate,
            last_level_clock: self.last_level_clock,
            weather: self.weather,
        };
        let entering = self.level_sync.remove(dimension).unwrap_or_default();
        self.level_sync
            .insert(std::mem::take(&mut self.config.dimension), leaving);
        (
            self.last_clock,
            self.last_rate,
            self.last_level_clock,
            self.weather,
        ) = (
            entering.last_clock,
            entering.last_rate,
            entering.last_level_clock,
            entering.weather,
        );
        self.config.dimension = dimension.to_owned();
        self.dimension_type = self
            .sync
            .id(
                "minecraft:dimension_type",
                self.config.levels.dimension_type(dimension),
            )
            .unwrap_or(0);
        self.encoder
            .set_dimension(self.config.levels.get(dimension));
    }

    /// ServerPlayer.teleport(TeleportTransition) into the level the hub now names
    /// (`set_dimension`): ClientboundRespawnPacket keeping all data, then vanilla's packet order.
    /// Nothing tracked in the old level is removed one by one: the client drops its whole level.
    ///
    /// Through a `portal`, vanilla finds the exit (an End platform, a Nether
    /// portal, a spawn column) before the teleport; here it is found once
    /// the destination has loaded around `at`. Until [`Self::land`] the
    /// client gets no position, so it stays on its level loading screen
    /// instead of showing the player in the air at `at`.
    pub fn change_level(
        &mut self,
        i: usize,
        at: [f64; 3],
        yaw: f32,
        pitch: Option<f32>,
        portal: bool,
        portal_cooldown: i32,
        ctx: &TickCtx,
    ) {
        let info = self.spawn_info(i, portal_cooldown, ctx.sea_level);
        self.close_container_for(i);
        self.before_travel(i);
        let left = self.guests[i].dimension.clone();
        self.guests.update(i, |g| {
            // Out of the old level with its chunks (ChunkMap.removeEntity).
            super::chunk_watch::unwatch_all(&mut self.chunk_watches, g);
            g.dimension = self.config.dimension.clone();
            g.level_entered = self.ticks;
            // A mod's collision exclusions were the old level's (arismoko/horizon#3717).
            g.collision_exclusions.clear();
            // The load timer is not restarted: vanilla resets the client's loaded state on death and
            // respawn only, and covers a level change with its separate `isChangingDimension` term.
            g.center = None;
            // A mod's view centre named a chunk of the level left.
            g.view_center = None;
            g.yaw = yaw;
            if let Some(pitch) = pitch {
                g.pitch = pitch;
            }
            g.place(at);
            g.send(&C::Respawn(play::Respawn {
                spawn: info,
                // ClientboundRespawnPacket.KEEP_ALL_DATA.
                keep: 3,
            }));
        });
        // Its chunks, queues and entities left behind, or kept as a viewed level (arismoko/horizon#3716).
        self.leave_level(i, &left);
        self.guests[i].send(&C::ChangeDifficulty(play::ChangeDifficulty {
            difficulty: ctx.difficulty,
            locked: ctx.difficulty_locked,
        }));
        self.send_permission_level(i);
        if portal {
            // A teleport still pending from the old level is void: its late
            // acceptance must not move the player back there.
            self.guests.update(i, |g| {
                g.landing = true;
                g.awaiting_teleport = None;
                g.teleport_counter = (g.teleport_counter + 1) & 0x7FFF_FFFF;
            });
        } else {
            self.resend_teleport(i, None);
        }
        let g = &self.guests[i];
        g.send(&g.abilities_packet());
        self.send_level_info(i, ctx, !portal);
        // PlayerList.sendAllPlayerInfo: the inventory and the held slot.
        self.send_inventory(i);
        let g = &self.guests[i];
        g.send(&C::SetHeldSlot(play::SetHeldSlot {
            slot: g.selected as i32,
        }));
        self.send_active_effects(i);
        self.guests.update(i, |g| {
            // ServerPlayer.sendPostEffects.
            let list = g.post_effects.clone();
            g.post_effects_dirty = false;
            g.send(&C::PostEffects(play::PostEffects { effects: list }));
            // lastSentExp, lastSentHealth and lastSentFood; the client's new
            // player starts with the default air and no absorption.
            g.sent_health = None;
            g.sent_xp = None;
            g.sent_score = 0;
            g.sent_air = (300, 0.0);
            g.sent_frozen = 0;
            // Respawn KEEP_ALL_DATA copies the old player's tracked values,
            // including shared flags. Preserve the publication baseline so a
            // flag cleared after the level change is still sent to that client.
            g.sent_shoulders = [None; 2];
            g.sent_stuck = (0, 0);
            let (cx, cz) = g.chunk();
            g.send(&C::SetChunkCacheCenter(play::SetChunkCacheCenter {
                x: cx,
                z: cz,
            }));
            g.center_sent = Some((cx, cz));
        });
    }

    /// The rest of a portal's [`Self::change_level`] once its exit is found:
    /// the position, LEVEL_CHUNKS_LOAD_START (the client's loading screen
    /// closes once its chunk is in) and TeleportTransition.PLAY_PORTAL_SOUND.
    pub fn land(&mut self, i: usize, at: [f64; 3]) {
        {
            let value = false;
            self.guests.update(i, |guest| guest.landing = value);
        }
        self.guests.update(i, |g| g.place(at)); // hold the exit chunk
        self.teleport(i, Some(at));
        let g = &self.guests[i];
        g.send(&C::GameEvent(play::GameEvent {
            event: game_event::LEVEL_CHUNKS_LOAD_START,
            param: 0.0,
        }));
        g.send(&C::LevelEvent(play::LevelEvent {
            kind: 1032,
            pos: [0, 0, 0],
            data: 0,
            global: false,
        }));
    }
}

/// A packet Horizon sends as raw bytes (no fields worth a type).
/// multiplayer.disconnect.invalid_player_movement.
/// Mth.wrapDegrees.
fn wrap_degrees(degrees: f32) -> f32 {
    let mut wrapped = degrees % 360.0;
    if wrapped >= 180.0 {
        wrapped -= 360.0;
    }
    if wrapped < -180.0 {
        wrapped += 360.0;
    }
    wrapped
}

/// The packets of one player handled in a tick; the rest wait for the next.
const PACKETS_PER_TICK: usize = 400;
/// Legacy ordinary-packet transport counter, still used by conn::Undrained.
/// Hub retention uses provider byte credit instead; raw input never uses this cap.
pub(crate) const INBOX_LIMIT: usize = 20 * PACKETS_PER_TICK;
/// Legacy ordinary-packet transport wire counter, not a Hub inbox quota.
pub(crate) const INBOX_BYTES: usize = 16 << 20;

/// The `minecraft:gravity` attribute of a player.
const PLAYER_GRAVITY: f64 = 0.08;

/// `ServerGamePacketListenerImpl.getMaximumFlyingTicks`: 80 ticks, more where
/// gravity is weaker, none at all (never kicked) without gravity.
/// `ServerGamePacketListenerImpl.noBlocksAround` over a box: every block from `lo` to `hi` is
/// `isAir()` (`air`'s state table), in loaded chunks (a chunk not loaded yet is not known to be air).
pub fn no_blocks_in(world: &World, lo: [f64; 3], hi: [f64; 3], air: &ChunkEncoder) -> bool {
    let cell = |v: f64| v.floor() as i32;
    (cell(lo[0])..=cell(hi[0])).all(|x| {
        (cell(lo[2])..=cell(hi[2])).all(|z| {
            world.loaded(x, z)
                && (cell(lo[1])..=cell(hi[1])).all(|y| air.is_air(world.block(x, y, z).0))
        })
    })
}

fn maximum_flying_ticks(gravity: f64) -> i32 {
    if gravity < f64::from(1.0E-5_f32) {
        return i32::MAX;
    }
    let modifier = 0.08 / gravity;
    // Mth.ceil is (int) with a floor correction; a double past int range
    // saturates like the Java cast.
    (80.0 * modifier.max(1.0)).ceil() as i32
}

fn invalid_movement() -> Tag {
    translatable("multiplayer.disconnect.invalid_player_movement", &[])
}

fn raw(id: i32, payload: Vec<u8>) -> C {
    C::Unknown(id, payload)
}

/// The joined states `Hub::note_joined` kept, if it kept them in the last
/// server tick: a level nobody tracked since (its guests gone, or none in it)
/// would otherwise pair its entities from states long past.
fn noted_last_tick(noted: Option<(u64, Vec<EntityState>)>, ticks: u64) -> Vec<EntityState> {
    match noted {
        Some((tick, joined)) if tick + 1 == ticks => joined,
        _ => Vec::new(),
    }
}

/// ClientboundTickingStatePacket: the tick rate and whether the game is frozen.
fn ticking_state(rate: f32, frozen: bool) -> C {
    let mut w = horizon_protocol::buf::Writer::new();
    w.f32(rate);
    w.bool(frozen);
    raw(ids::TICKING_STATE, w.data)
}

/// What the others see of a guest's body, from its server-side state.
pub fn guest_body(g: &Guest) -> BodyFlags {
    let m = g.motion.as_deref();
    BodyFlags {
        gliding: g.fall_flying || g.glide_end_unsent,
        swimming: m.is_some_and(|m| m.swimming),
        crawling: m.is_some_and(|m| m.pose == crate::pose::Pose::Swimming && !m.swimming),
        crouching: m.is_some_and(|m| m.pose == crate::pose::Pose::Crouching),
        spin_attack: m.is_some_and(|m| m.auto_spin_attack_ticks > 0),
        on_fire: m.is_some_and(|m| m.fire_ticks > 0),
        // ServerPlayer.updateInvisibilityStatus: a spectator is always invisible.
        invisible: g.effects.get("invisibility").is_some() || g.spectator(),
        glowing: g.effects.get("glowing").is_some(),
    }
}

/// Entity.DATA_SHARED_FLAGS_ID of a player: on fire 0x01, crouching 0x02,
/// sprinting 0x08, swimming 0x10, invisible 0x20, glowing 0x40, gliding 0x80.
fn shared_flags(sneaking: bool, sprinting: bool, body: &BodyFlags) -> i8 {
    let mut shared = 0i8;
    if body.on_fire {
        shared |= 0x01;
    }
    if sneaking {
        shared |= 0x02;
    }
    if sprinting {
        shared |= 0x08;
    }
    if body.swimming {
        shared |= 0x10;
    }
    if body.invisible {
        shared |= 0x20;
    }
    if body.glowing {
        shared |= 0x40;
    }
    if body.gliding {
        shared |= -128;
    }
    shared
}

/// What the entries of one `PlayerInfoUpdate` may add up to: the frame limit less room for the packet
/// id, the actions byte and the entry count.
const INFO_BATCH_LIMIT: usize = horizon_protocol::frame::MAX_FRAME - 1024;

/// `entries` split, in order, into packets whose frames stay under the frame limit: a batch ends before
/// the entry that would pass `INFO_BATCH_LIMIT` (an entry is measured as it is encoded, which is
/// above the frame's compressed size). One entry alone is under the limit (a profile holds at most 16
/// properties of at most 32767 characters), so a batch is never empty, except for no entries at all:
/// that gives one empty batch, as the join always sent its list.
fn info_batches(actions: u8, entries: Vec<play::PlayerInfo>) -> Vec<Vec<play::PlayerInfo>> {
    use horizon_protocol::buf::{Codec, Writer};
    let mut batches = vec![Vec::new()];
    let mut size = 0;
    for entry in entries {
        let mut one = play::PlayerInfoUpdate {
            actions,
            entries: vec![entry],
        };
        let mut w = Writer::new();
        play::PlayerInfoUpdate::write(&one, &mut w);
        let entry_size = w.data.len();
        let entry = one.entries.pop().expect("the entry just put in");
        let batch = batches.last_mut().expect("a batch is always open");
        if !batch.is_empty() && size + entry_size > INFO_BATCH_LIMIT {
            batches.push(vec![entry]);
            size = entry_size;
        } else {
            batch.push(entry);
            size += entry_size;
        }
    }
    batches
}

/// `ExtraCodecs.PROPERTY_MAP`'s limit: a client rejects a player list update with more properties.
pub const MAX_LISTED_PROPERTIES: usize = 16;

/// The profile properties the player list gives a player: those it logged
/// in with, or for the integrated server's owner (who logs in with none) the
/// signed-in account's, once fetched (`IntegratedServer.getSingleplayerProfile`),
/// with the properties a mod set (`overrides`) over them by name. An override neither takes the
/// account's other properties away nor, set before the fetch finished, keeps the fetched skin
/// from showing. At most 16 are listed (a client refuses more): the overrides win, and the
/// properties that are not overridden fill the slots left, in their order.
fn listed_properties(
    logged_in: &[horizon_protocol::packets::login::Property],
    overrides: &[horizon_protocol::packets::login::Property],
    owner: bool,
    profile: Option<&super::OwnerProfile>,
) -> Vec<horizon_protocol::packets::login::Property> {
    type Property = horizon_protocol::packets::login::Property;
    let base: &[Property] = match profile {
        Some(profile) if owner => profile.properties(),
        _ => logged_in,
    };
    // The overrides, one a name (the later wins), at most 16.
    let mut wins: Vec<&Property> = Vec::new();
    for property in overrides {
        match wins.iter_mut().find(|p| p.name == property.name) {
            Some(old) => *old = property,
            None => wins.push(property),
        }
    }
    wins.truncate(MAX_LISTED_PROPERTIES);
    let mut room = MAX_LISTED_PROPERTIES - wins.len();
    let mut listed: Vec<Property> = Vec::with_capacity(MAX_LISTED_PROPERTIES);
    for property in base {
        match wins.iter().find(|p| p.name == property.name) {
            Some(win) => {
                if !listed.iter().any(|p| p.name == win.name) {
                    listed.push((*win).clone());
                }
            }
            None if room > 0 => {
                room -= 1;
                listed.push(property.clone());
            }
            None => {}
        }
    }
    for win in wins {
        if !listed.iter().any(|p| p.name == win.name) {
            listed.push(win.clone());
        }
    }
    listed
}

/// The source in wire slot order: hands, armor feet..head, body, saddle.
fn equipment_source(g: &Guest, index: usize) -> Option<&ItemStack> {
    match index {
        0 => g.inventory.get(g.selected)?.as_ref(),
        1 => g.offhand.as_ref(),
        2..=5 => g.armor[5 - index].as_ref(),
        6 => g.equipment[horizon_items::equipment::BODY].as_ref(),
        7 => g.equipment[horizon_items::equipment::SADDLE].as_ref(),
        _ => None,
    }
}

/// The original neighbour check of `next_batch`'s readiness: each of the nine
/// columns around `c` (itself included) is a full chunk in `chunks` or a proto
/// chunk `proto` reports.
fn neighbours_scanned(
    chunks: &crate::chunk_map::ChunkMap,
    proto: impl Fn(i32, i32) -> bool,
    c: &Chunk,
) -> bool {
    (-1..=1).all(|dz| {
        (-1..=1).all(|dx| {
            let (x, z) = (c.x + dx, c.z + dz);
            chunks.contains_key(&(x, z)) || proto(x, z)
        })
    })
}

/// `neighbours_scanned`, with the resident-neighbourhood index as a fast path:
/// a column whose nine columns are all resident (`ChunkMap::surrounded`) is ready
/// without the nine lookups. Anything else takes the scan, so the answer is the
/// scan's in every case.
fn neighbours_ready(
    chunks: &crate::chunk_map::ChunkMap,
    proto: impl Fn(i32, i32) -> bool,
    c: &Chunk,
) -> bool {
    chunks.surrounded((c.x, c.z)) || neighbours_scanned(chunks, proto, c)
}

#[cfg(test)]
mod tests;
