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
use std::sync::{Arc, Mutex, atomic::Ordering};
use {
    horizon_core::nbt::{JavaString, Tag},
    horizon_mod_runtime::attachments::Holder,
    horizon_world::{
        block_collision::BlockCollisionRegistry, native_load_context::LoopContext,
        world_movement::in_range_of_ground,
    },
};

/// GameEvent types (ClientboundGameEventPacket).
pub mod advancements;
mod block_changes;
pub mod chat;
pub mod chat_filter;
mod item_particles;
mod joined;
mod move_collision;
mod moves;
pub mod passage_cross;
mod pending_move;
mod raw_inbox;
mod selector_sync;
mod spawn_info;
use advancements::AdvancementOwners;
use chat::yellow_names;
pub use pending_move::{Accepted, Hearing, PendingMove};
use raw_inbox::{PacketDrain, yield_raw};
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
    /// A guest sent new client information: the values before and after, for the mods.
    ClientInformation(
        u32,
        Box<horizon_protocol::packets::configuration::ClientInformation>,
        Box<horizon_protocol::packets::configuration::ClientInformation>,
    ),
    /// A changed movement report is held for the game to rule on ([`Hub::resolve_move`]).
    PlayerMove(u32),
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
    /// Main-thread world identity used to bind typed network-context player handles.
    pub payload_player_epoch: Option<mod_api::WorldEpoch>,
    /// Clicks on and closings of native mods' menus, for the game.
    pub native_menu_events: Vec<super::menu::NativeMenuEvent>,
    /// The chunks players started and stopped watching, for the mods
    /// (`chunk_watch`): recorded only while a mod listens.
    pub chunk_watches: ChunkWatchLog,
    /// Kicks waiting for the game thread to notify native mods.
    pending_kicks: Vec<HostEvent>,
    /// A native mod listens to `player_move`: a changed report waits for the game ([`pending_move`]).
    pub moves_listened: bool,
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
    /// Players a native mod hid (`native_hide_player`), by UUID, on or off:
    /// only players of at least that permission level see them, in the
    /// player list, the world and their joining and leaving.
    pub hidden: crate::fast_hash::FxHashMap<u128, u8>,
    pub lan: Lan,
    pub config: HubConfig,
    /// What the hooks fired inside a click or a name format are given; the
    /// game replaces it on each reload.
    pub loop_context: Arc<LoopContext>,
    pub encoder: ChunkEncoder,
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
    entity_out: Vec<Vec<Vec<u8>>>,
    /// Each placed guest's entities gone this tick, where they were last sent (only while the
    /// prediction records).
    entity_gone: Vec<Vec<[f64; 3]>>,
    /// Server mods' commands, a root in the commands report's format, sent
    /// after vanilla's; the key the game last set them for.
    mod_commands: (serde_json::Value, Option<u64>),
    selector_tokens: Vec<horizon_client_core::registries::SelectorToken>,
    /// Players' areas in other levels and what their clients view of them (#3716).
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
            packet_drain: None,
            pending_kicks: Vec::new(),
            moves_listened: false,
            transfers: None,
            native_tracking: Vec::new(),
            attachment_snapshots: Vec::new(),
            sync_attachments: false,
            native_advancement_grants: Vec::new(),
            native_reload_pending: false,
            mods_loaded: false,
            hidden: Default::default(),
            teams: Vec::new(),
            nicknames: std::collections::HashMap::new(),
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
            .map(Guest::chunk)
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
        self.broadcast(&C::SystemChat(play::SystemChat {
            content,
            overlay: false,
        }));
    }
    /// A system message for the guests `hears` picks by name
    /// (`PlayerList.broadcastSystemToTeam` and its kin).
    pub fn system_message_to(&self, content: Tag, hears: impl Fn(&str) -> bool) {
        let body = C::SystemChat(play::SystemChat {
            content,
            overlay: false,
        })
        .encode();
        for g in self.guests.iter().filter(|g| g.placed && hears(&g.name)) {
            g.sender.send(body.clone());
        }
    }
    /// `ServerPlayer.sendSystemMessage` to the guest with `uuid`, in any dimension.
    pub fn system_message_to_player(&self, uuid: u128, content: Tag) {
        let body = C::SystemChat(play::SystemChat {
            content,
            overlay: false,
        })
        .encode();
        for g in self.guests.iter().filter(|g| g.placed && g.uuid == uuid) {
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
        self.hidden
            .get(&uuid)
            .is_none_or(|&level| permission >= level)
    }

    /// The player lists after `hidden` or a permission changed: each placed
    /// player loses the players hidden from it (ClientboundPlayerInfoRemove)
    /// and gets back those it sees again (an ADD_PLAYER entry). A change
    /// checks the client's entity pairs again, so the tracker follows too.
    /// Nothing runs while nobody is hidden.
    fn sync_hidden(&mut self) {
        if self.hidden.is_empty() && self.guests.iter().all(|g| g.tab_hidden.is_empty()) {
            return;
        }
        let candidates: Vec<usize> = (0..self.guests.len())
            .filter(|&j| {
                let u = self.guests[j].uuid;
                self.guests[j].placed
                    && self.guests[j].participates_online()
                    && (self.hidden.contains_key(&u)
                        || self.guests.iter().any(|g| g.tab_hidden.contains(&u)))
            })
            .collect();
        for i in 0..self.guests.len() {
            if !self.guests[i].placed || !self.guests[i].participates_online() {
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
    /// ServerPlayer.getTabListDisplayName: the nickname, if any, as mods format it.
    fn tab_list_name(&self, uuid: u128, name: &str) -> Option<Tag> {
        let mut display_name = self.nicknames.get(&uuid).map(crate::text::Text::literal);
        native_player_tab_list_name_format(&self.loop_context, uuid, name, &mut display_name);
        display_name.map(|text| crate::scoreboard::component_tag(&text.component()))
    }
    /// ClientboundPlayerInfoUpdatePacket.createPlayerInitializing: every action.
    const INFO_ACTIONS: u8 = 0xFF;

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
            max_players: self.config.max_players as i32,
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
        self.guests[i].send(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: Self::INFO_ACTIONS,
            entries,
        }));
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
        // ServerLevel.addNewPlayer: the chunk tracking starts at the player.
        self.guests.update(i, |g| {
            let (cx, cz) = g.chunk();
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
            .is_some_and(|c| c != self.guests[i].chunk());
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
        self.guests[i].send(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: Self::INFO_ACTIONS,
            entries,
        }));
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

    /// ClientboundSetDefaultSpawnPositionPacket for the level's respawn data
    /// (the world spawn, in the Overworld wherever the player is).
    pub fn default_spawn(&self, spawn: [f64; 3], angle: [f32; 2]) -> C {
        C::SetDefaultSpawnPosition(play::SetDefaultSpawnPosition {
            pos: play::GlobalPos {
                dimension: "minecraft:overworld".into(),
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
        g.send(&self.default_spawn(spawn, ctx.spawn_angle));
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
        self.anticheat_tick(ctx.tick_rate);
        let mut events = std::mem::take(&mut self.pending_kicks);
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
                        events.push(HostEvent::Left(Box::new(self.guests.remove(i))));
                    }
                    let conn = self.admit(*j, ctx);
                    events.push(HostEvent::Joined(conn));
                }
                NetEvent::Left(conn, reason) => {
                    credits.push(credit);
                    if let Some(i) = self.guests.iter().position(|g| g.conn == conn) {
                        let g = self.remove_guest(i, &reason);
                        events.push(HostEvent::Left(Box::new(g)));
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
        // keepConnectionAlive's timeout, checked once a second here. Kicked,
        // the client gets the reason; still here a second later (it reads
        // nothing, so its disconnect waits unwritten), its connection is closed.
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

    pub(super) fn remove_guest(&mut self, i: usize, reason: &str) -> Guest {
        let mut g = self.guests.remove(i);
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
        self.pending_kicks.push(HostEvent::Left(Box::new(gone)));
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
        if let Some(far) = self.far.as_mut() {
            far.poll();
        }
        let watching = self.chunk_watches.recording();
        let mut batches = Vec::new();
        for i in 0..self.guests.len() {
            if self.watches(&self.guests[i]) {
                self.update_view(i, watching);
                if let Some(batch) = self.next_batch(i, world, ctx.streaming) {
                    batches.push((i, batch));
                }
            }
        }
        if batches.is_empty() {
            return;
        }
        // Each column once, however many players it goes to, with the
        // number of batches that take it.
        let mut columns = Columns::default();
        let mut far_bodies = crate::fast_hash::FxHashMap::default();
        let batches: Vec<_> = batches
            .into_iter()
            .map(|(i, batch)| {
                let (taken, mut bodies): (Vec<_>, Vec<_>) = batch
                    .near
                    .into_iter()
                    .map(|(idx, chunk)| (idx, Body::Level(columns.add(chunk))))
                    .unzip();
                let mut far = Vec::new();
                for (idx, column) in batch.far {
                    far.push(idx);
                    let body = match column {
                        Column::Level(chunk) => far_bodies
                            .entry((chunk.x, chunk.z))
                            .or_insert_with(|| {
                                Arc::<[u8]>::from(self.encoder.body_far(chunk, world))
                            })
                            .clone(),
                        Column::Saved(body) => body,
                    };
                    bodies.push(Body::Saved(body));
                }
                (i, taken, far, bodies)
            })
            .collect();
        self.encoder.warm(&columns.chunks);
        let mut encoded = self.encoder.bodies_with(&columns.chunks, world, &|x, z| {
            ctx.streaming.and_then(|s| s.proto_sections(x, z))
        });
        let uses = &mut columns.uses;
        let chunks = &columns.chunks;
        for (i, taken, far, batch) in batches {
            self.guests.update(i, |g| {
                let size = batch.len();
                // The single-player window shares the sections it is sent (handoff.rs).
                let memory = g.address == super::conn::MEMORY_ADDRESS;
                if size > 0 {
                    g.batch_quota -= size as f32;
                    g.send(&C::ChunkBatchStart(horizon_protocol::packets::Empty));
                    g.sender.send_many(batch.into_iter().map(|body| match body {
                        Body::Level(j) => {
                            if memory {
                                super::handoff::offer(chunks[j]);
                            }
                            uses[j] -= 1;
                            if uses[j] == 0 {
                                std::mem::take(&mut encoded[j])
                            } else {
                                encoded[j].clone()
                            }
                        }
                        Body::Saved(body) => body.to_vec(),
                    }));
                    g.send(&C::ChunkBatchFinished(play::ChunkBatchFinished {
                        size: size as i32,
                    }));
                    g.unacknowledged += 1;
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
                    if self.sync_attachments && g.brand == "horizon" {
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
        let (cx, cz) = self.guests[i].chunk();
        let r = self.guests[i].view_distance;
        let reach = self.reach(&self.guests[i]);
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
                    (-1..=1).all(|dz| {
                        (-1..=1).all(|dx| {
                            let (x, z) = (c.x + dx, c.z + dz);
                            world.chunks.contains_key(&(x, z))
                                || streaming.proto_sections(x, z).is_some()
                        })
                    })
                })
        };
        let near: Vec<_> = g
            .chunk_queue
            .iter()
            .enumerate()
            .filter_map(|(idx, key)| Some((idx, &**world.chunks.get(key)?)))
            .filter(|(_, c)| ready(c))
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
                    _ => match ring.saved(x, z) {
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
            T::EntityEffect | T::TintedLeaves => Some(ParticleData::Color(
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
            velocity: [0.0; 3],
            data: 0,
            meta,
            forced_meta: [0; 4],
            // The effect swirls go back to none.
            defaults: vec![
                (10, serializer::PARTICLES, Value::Particles(Vec::new())),
                (11, serializer::BOOLEAN, Value::Bool(false)),
            ],
            equipment,
            attributes,
            precise: false,
            vertical_collision: false,
            horizontal_collision: false,
            needs_sync: false,
            sync_position: false,
            stepped: false,
            idle: false,
            bare: false,
            minecart_steps: None,
            leash: None,
            spectator: false,
            watching: false,
            hidden: None,
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
                state.hidden = self.hidden.get(&g.uuid).copied();
                stuck_meta(&mut state.meta, &mut state.defaults, g.stuck);
                frozen_meta(&mut state, g.motion.as_ref().map_or(0, |m| m.ticks_frozen));
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
                        );
                        if let Some(add) = first.recipe_book_add {
                            g.send(&raw(ids::RECIPE_BOOK_ADD, add));
                        }
                        g.send(&raw(ids::UPDATE_ADVANCEMENTS, first.update_advancements));
                        g.advancements_visible = sync.advancements.visibility(&g.advancements);
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
            let pose = g.pose();
            if matches!(
                pose,
                crate::pose::Pose::Standing | crate::pose::Pose::Crouching
            ) && pose != g.sent_pose
            {
                g.sent_pose = pose;
                g.send(&C::SetEntityData(play::SetEntityData {
                    id: g.entity_id,
                    values: vec![(6, serializer::POSE, Value::Id(pose.id()))],
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
        // AbstractContainerMenu.broadcastChanges for the inventory menu.
        let mut changes = Vec::new();
        for slot in 0..46 {
            let now = self.guests[i].menu_slot(slot);
            if now != self.guests[i].menu_sent[slot] {
                changes.push((slot, now));
            }
        }
        for (slot, now) in changes {
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
            let acted = match &packet {
                S::AcceptTeleportation(_)
                | S::PlayerAbilities(_)
                | S::MovePlayerStatus(_)
                | S::MovePlayerRot(_)
                | S::PlayerLoaded(_)
                | S::ChunkBatchReceived(_)
                | S::ClientInformation(_)
                | S::RecipeBookChangeSettings(_)
                | S::RecipeBookSeenRecipe(_)
                | S::ClientTickEnd(_)
                | S::KeepAlive(_)
                | S::Pong(_)
                | S::PingRequest(_) => false,
                S::MovePlayerPos(p) => self.moved_far(i, [p.x, p.y, p.z]),
                S::MovePlayerPosRot(p) => self.moved_far(i, [p.x, p.y, p.z]),
                S::PlayerInput(_) => self.guests[i].client_loaded(),
                _ => true,
            };
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
                        self.position_change(i, at, rot, 0, Hearing::Confirmation, ctx, events);
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
                S::Pong(p) => self.anticheat_pong(i, p.id),
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
                    self.guests.update(i, |g| {
                        if !g.position_this_tick {
                            g.known_movement = [0.0; 3];
                        }
                        g.position_this_tick = false;
                    });
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
                    let (was_hat, hat, uuid) = self.guests.update(i, |g| {
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
                        (was_hat, hat, g.uuid)
                    });
                    // ServerGamePacketListenerImpl.handleClientInformation: UPDATE_HAT to everyone.
                    if hat != was_hat {
                        self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                            actions: 128,
                            entries: vec![play::PlayerInfo {
                                uuid,
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
            return;
        }
        let to = pos.unwrap_or(g.position);
        let rot = rot.unwrap_or((g.yaw, g.pitch));
        self.position_change(i, to, rot, flags, Hearing::Heard, ctx, events);
    }

    /// ServerGamePacketListenerImpl.handlePlayerPositionChange.
    fn position_change(
        &mut self,
        i: usize,
        to: [f64; 3],
        (yaw, pitch): (f32, f32),
        flags: u8,
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
        let (mut flags, mut corrected) = (flags, false);
        if flags & 1 != 0 && self.anticheat.is_some() {
            match self.anticheat_ground(i, to, stands, ctx, events) {
                Some(true) => {
                    self.guests.update(i, |g| (g.yaw, g.pitch) = (yaw, pitch));
                    return;
                }
                Some(false) => (flags, corrected) = (flags & !1, true),
                None => {}
            }
        }
        // clientIsFloating, from the move that is accepted: no upward-or-level
        // step onto something, nothing but air around the body, and nothing
        // that lets the player hover (the server's allow-flight, mayfly,
        // levitation, gliding, riptide). A move whose ground claim the anticheat dropped never counts:
        // a false claim ends in the anticheat's own kick, logged and told to the operators, or not at all.
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
        if flags & 1 != 0 && !self.guests[i].rests_on_ground {
            self.force_send_player_support_blocks(i, ctx);
        }
        let accepted = Accepted {
            to,
            yaw,
            pitch,
            flags,
            stands,
            floating,
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
            let state = ctx.world.block(x, y, z);
            self.send_to(
                conn,
                &C::BlockUpdate(play::BlockUpdate {
                    pos,
                    state: state.0 as i32,
                }),
            );
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
            // A new player: a mod's collision exclusions were the dead one's (#3717).
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
            // ServerPlayer.restoreFrom: the game mode again (onUpdateAbilities).
            g.send(&g.abilities_packet());
            // The new player faces the level's respawn direction (snapTo), or
            // the one its TeleportTransition carries.
            [g.yaw, g.pitch] = rotation.unwrap_or(ctx.spawn_angle);
            g.position = at;
            g.previous_position = at;
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
        let spawn = self.default_spawn(ctx.spawn, ctx.spawn_angle);
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
        // ServerLevel.addRespawnedPlayer: the chunk tracking starts again at the player.
        self.guests.update(i, |g| {
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

    /// ServerPlayer.teleport(TeleportTransition) into the level the hub now
    /// names (`set_dimension`): ClientboundRespawnPacket keeping all data,
    /// then vanilla's packet order. Nothing tracked in the old level is
    /// removed one by one: the client drops its whole level.
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
        let left = self.guests[i].dimension.clone();
        self.guests.update(i, |g| {
            // Out of the old level with its chunks (ChunkMap.removeEntity).
            super::chunk_watch::unwatch_all(&mut self.chunk_watches, g);
            g.dimension = self.config.dimension.clone();
            g.level_entered = self.ticks;
            // A mod's collision exclusions were the old level's (#3717).
            g.collision_exclusions.clear();
            // The load timer is not restarted: vanilla resets the client's loaded state on death and
            // respawn only, and covers a level change with its separate `isChangingDimension` term.
            g.center = None;
            g.yaw = yaw;
            if let Some(pitch) = pitch {
                g.pitch = pitch;
            }
            g.position = at;
            g.previous_position = at;
            g.send(&C::Respawn(play::Respawn {
                spawn: info,
                // ClientboundRespawnPacket.KEEP_ALL_DATA.
                keep: 3,
            }));
            g.send(&C::ChangeDifficulty(play::ChangeDifficulty {
                difficulty: ctx.difficulty,
                locked: ctx.difficulty_locked,
            }));
        });
        // Its chunks, queues and entities left behind, or kept as a viewed level (#3716).
        self.leave_level(i, &left);
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
        gliding: g.fall_flying,
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

/// The profile properties the player list gives a player: those it logged
/// in with, or for the integrated server's owner (who logs in with none) the
/// signed-in account's, once fetched (`IntegratedServer.getSingleplayerProfile`).
fn listed_properties(
    logged_in: &[horizon_protocol::packets::login::Property],
    owner: bool,
    profile: Option<&super::OwnerProfile>,
) -> Vec<horizon_protocol::packets::login::Property> {
    match profile {
        Some(profile) if owner && logged_in.is_empty() => profile.properties().to_vec(),
        _ => logged_in.to_vec(),
    }
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

#[cfg(test)]
mod tests {
    mod cave_air_tests;
    mod collision_exclusion_tests;
    mod effect_meta_tests;
    #[path = "../far_tests.rs"]
    mod far_tests;
    mod ordered_inbox_tests;
    mod pending_move_bound_tests;
    mod pending_move_scope_tests;
    mod pending_move_tests;
    mod sound_tests;
    use super::*;
    use crate::fast_hash::FxHashSet;

    /// Queued one-shot fields wait out a change nobody sees, go in one built tag to
    /// every viewer, and are not sent again; a column sent whole doesn't lose them.
    #[test]
    fn client_update_fields_reach_every_viewer_once_and_wait_without_viewers() {
        use crate::block_entity_types::BlockEntityTypes;
        use crate::containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields};
        use horizon_core::nbt::{Compound, JavaString};
        use std::sync::atomic::AtomicUsize;
        static BUILT: AtomicUsize = AtomicUsize::new(0);
        let (mut hub, first) = hub_with_a_guest();
        let builtin = horizon_protocol::registry::Builtin::parse(
            r#"{"minecraft:block_entity_type":{"entries":{"test:machine":{"protocol_id":49}}}}"#,
        )
        .unwrap();
        let types = BlockEntityTypes::new(
            [("test:machine".into(), "test:machine".into())].into(),
            builtin
                .get("minecraft:block_entity_type")
                .unwrap()
                .names
                .clone(),
        );
        let states = ["minecraft:air", "test:machine"].map(|n| (Arc::<str>::from(n), false, None));
        hub.encoder = ChunkEncoder::with_types(states, &builtin, &[], &[], Some(&types));
        hub.encoder.mod_update_tag = Some(|_, entity| {
            BUILT.fetch_add(1, Ordering::Relaxed);
            Some(entity.nbt.clone())
        });
        let pos = (1, 64, 3);
        let progress = (JavaString::from("progress"), Tag::Int(7));
        let effect = (JavaString::from("effect"), Tag::Int(23));
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(pos.0, pos.1, pos.2, 1);
        world.chunks.insert((0, 0), Arc::new(chunk));
        let machine = ModBlockEntity::new(
            "test:machine".into(),
            ModBlockEntityFields {
                nbt: Compound::from([progress.clone()]),
                ..Default::default()
            },
        );
        let revision = machine.revision();
        machine.set_client_update_fields(Compound::from([effect.clone()]));
        world
            .block_entities
            .insert(pos, BlockEntity::Mod(Box::new(machine)));
        world.block_log = Some(vec![pos]);
        // Nobody has the chunk yet: no tag is built, and the fields wait.
        hub.broadcast_block_changes(&mut world);
        assert_eq!(BUILT.load(Ordering::Relaxed), 0);
        assert!(first.try_recv().is_err());
        hub.guests.update(0, |g| _ = g.sent_chunks.insert((0, 0)));
        let (tx, second) = std::sync::mpsc::channel();
        let mut other = Guest::new(
            crate::lan::Joined {
                conn: 2,
                uuid: 2,
                name: "Ada".into(),
                properties: Vec::new(),
                info: horizon_protocol::packets::configuration::ClientInformation::new(8),
                brand: "vanilla".into(),
                sender: crate::lan::PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(std::sync::atomic::AtomicI32::new(0)),
                address: "127.0.0.1:25566".parse().unwrap(),
                expected: false,
            },
            2,
            [0.5, 64.0, 0.5],
            8,
        );
        other.placed = true;
        other.dimension = "minecraft:overworld".into();
        other.sent_chunks.insert((0, 0));
        hub.guests.push(other);
        let tags = |rx: &std::sync::mpsc::Receiver<crate::lan::Out>| {
            let mut tags = Vec::new();
            while let Ok(out) = rx.try_recv() {
                let crate::lan::Out::Packet(body) = out else {
                    continue;
                };
                let mut r = horizon_protocol::buf::Reader::new(&body);
                let id = r.varint().unwrap();
                if let Ok(C::BlockEntityData(packet)) = C::decode(id, r.rest()) {
                    assert_eq!(packet.pos, [pos.0, pos.1, pos.2]);
                    tags.push(packet.tag);
                }
            }
            tags
        };
        // The column goes whole in the same broadcast: the change is kept for the fields.
        world.bulk_log.insert((0, 0));
        world.block_log.as_mut().unwrap().extend([pos, pos]);
        hub.broadcast_block_changes(&mut world);
        let once = Tag::Compound(Compound::from([progress.clone(), effect]));
        for viewer in [&first, &second] {
            assert_eq!(tags(viewer), std::slice::from_ref(&once));
        }
        let built = BUILT.load(Ordering::Relaxed);
        world.block_log.as_mut().unwrap().push(pos);
        hub.broadcast_block_changes(&mut world);
        let plain = Tag::Compound(Compound::from([progress.clone()]));
        for viewer in [&first, &second] {
            assert_eq!(tags(viewer), std::slice::from_ref(&plain));
        }
        assert_eq!(BUILT.load(Ordering::Relaxed), built + 1);
        let BlockEntity::Mod(machine) = &world.block_entities[&pos] else {
            unreachable!();
        };
        assert_eq!(machine.revision(), revision);
        assert_eq!(machine.nbt, Compound::from([progress]));
    }

    #[test]
    fn self_pose_wire_lifecycle_preserves_existing_publishers() {
        use crate::pose::Pose;
        fn drain(
            rx: &std::sync::mpsc::Receiver<crate::lan::Out>,
        ) -> (Vec<Vec<(u8, i32, Value)>>, Vec<i32>) {
            let mut frames = Vec::new();
            let mut keeps = Vec::new();
            while let Ok(out) = rx.try_recv() {
                let crate::lan::Out::Packet(body) = out else {
                    continue;
                };
                let mut r = horizon_protocol::buf::Reader::new(&body);
                let id = r.varint().unwrap();
                match C::decode(id, r.rest()).unwrap() {
                    C::SetEntityData(p) if p.values.iter().any(|(i, _, _)| *i == 0 || *i == 6) => {
                        assert_eq!(p.id, 1, "actual self recipient");
                        frames.push(p.values);
                    }
                    C::Respawn(p) => keeps.push(p.keep as i32),
                    _ => {}
                }
            }
            (frames, keeps)
        }
        let pose = |p: Pose| vec![(6, serializer::POSE, Value::Id(p.id()))];
        let flags = |f| vec![(0, serializer::BYTE, Value::Byte(f))];
        let (mut hub, rx) = hub_with_a_guest();
        hub.sync_player_state(0);
        assert!(drain(&rx).0.is_empty(), "default Standing is not dirty");
        hub.guests.update(0, |g| g.sneaking = true);
        hub.sync_player_state(0);
        assert_eq!(drain(&rx).0, [flags(2), pose(Pose::Crouching)]);
        hub.sync_player_state(0);
        assert!(drain(&rx).0.is_empty());
        // The existing refused-glide force-flags marker never forces Pose.
        hub.guests.update(0, |g| g.sent_shared ^= -128);
        hub.sync_player_state(0);
        assert_eq!(drain(&rx).0, [flags(2)]);
        hub.guests.update(0, |g| g.sneaking = false);
        hub.sync_player_state(0);
        assert_eq!(drain(&rx).0, [flags(0), pose(Pose::Standing)]);
        hub.sync_player_state(0);
        assert!(drain(&rx).0.is_empty());
        hub.guests.update(0, |g| g.bed = Some([1, 64, 2]));
        hub.guests.update(0, |g| g.sleeping = true);
        hub.sync_player_state(0);
        assert_eq!(
            drain(&rx).0,
            [vec![
                (6, serializer::POSE, Value::Id(2)),
                (
                    14,
                    serializer::OPTIONAL_BLOCK_POS,
                    Value::OptPos(Some([1, 64, 2]))
                ),
            ]]
        );
        assert_eq!(hub.guests[0].sent_pose, Pose::Sleeping);
        hub.wake(0, false);
        assert_eq!(
            drain(&rx).0,
            [vec![
                (6, serializer::POSE, Value::Id(0)),
                (14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(None)),
            ]]
        );
        hub.sync_player_state(0);
        assert!(drain(&rx).0.is_empty(), "wake already sent Standing");
        // The non-wake bed-delta publisher also owns its exact existing frame.
        hub.guests.update(0, |g| g.bed = Some([1, 64, 2]));
        hub.guests.update(0, |g| g.sleeping = true);
        hub.sync_player_state(0);
        drain(&rx);
        hub.guests.update(0, |g| g.bed = None);
        hub.guests.update(0, |g| g.sleeping = false);
        hub.sync_player_state(0);
        assert_eq!(
            drain(&rx).0,
            [vec![
                (6, serializer::POSE, Value::Id(0)),
                (14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(None)),
            ]]
        );
        hub.guests.update(0, |g| g.fall_flying = true);
        hub.sync_player_state(0);
        assert!(
            drain(&rx)
                .0
                .iter()
                .all(|v| !v.iter().any(|(i, _, _)| *i == 6)),
            "other normal poses stay outside this sender"
        );
        hub.guests.update(0, |g| g.fall_flying = false);
        hub.guests.update(0, |g| g.sneaking = true);
        hub.sync_player_state(0);
        drain(&rx);
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let mut next_entity = 2;
        let border = crate::worldborder::WorldBorder::default();
        let ctx = TickCtx {
            sea_level: 63,
            world: &mut world,
            entities: &[],
            next_entity: &mut next_entity,
            spawn: [0.5, 64.0, 0.5],
            spawn_angle: [0.0; 2],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &border,
            streaming: None,
        };
        hub.change_level(0, [100.5, 49.0, 0.5], 0.0, None, false, 0, &ctx);
        assert_eq!(drain(&rx).1, [3]);
        assert_eq!(hub.guests[0].sent_pose, Pose::Crouching);
        hub.sync_player_state(0);
        assert!(drain(&rx).0.is_empty(), "KEEP_ALL_DATA retains crouch");
        hub.guests.update(0, |g| g.sneaking = false);
        hub.sync_player_state(0);
        assert_eq!(drain(&rx).0, [flags(0), pose(Pose::Standing)]);
        hub.guests.update(0, |g| g.sneaking = true);
        hub.sync_player_state(0);
        drain(&rx);
        hub.respawn(0, [0.5, 64.0, 0.5], Some([0.0; 2]), &ctx);
        assert_eq!(drain(&rx).1, [0]);
        assert_eq!(hub.guests[0].sent_pose, Pose::Standing);
        hub.sync_player_state(0);
        assert_eq!(
            drain(&rx).0,
            [pose(Pose::Crouching)],
            "KEEP_NONE reset makes the actual crouch dirty again"
        );
        hub.sync_player_state(0);
        assert!(drain(&rx).0.is_empty());
    }

    /// A hub with one placed guest, the speed effect registered, and what the
    /// guest is sent.
    fn hub_with_a_guest() -> (Hub, std::sync::mpsc::Receiver<crate::lan::Out>) {
        let mut sync = SyncData::empty();
        sync.builtin.registries.insert(
            "minecraft:mob_effect".to_string(),
            horizon_protocol::registry::Ids::from_names(vec!["minecraft:speed".to_string()]),
        );
        hub_with_sync(sync)
    }

    #[test]
    fn modprofile_guest_tree_survives_refresh_with_permissions() {
        for level in 0..=4 {
            let mut sync = SyncData::empty();
            sync.commands = Some(crate::lan::commands::CommandTree {
                root: serde_json::json!({"type":"root","children":{}}),
            });
            let (mut hub, rx) = hub_with_sync(sync);
            hub.guests.update(0, |guest| guest.permission = level);
            for refresh in [false, true] {
                if refresh {
                    hub.set_mod_commands(7, serde_json::json!({"children":{"plugin":{"type":"literal","executable":true}}}));
                } else {
                    hub.send_commands(0);
                }
                let crate::lan::Out::Packet(body) = rx.try_recv().unwrap() else {
                    panic!("packet");
                };
                let mut reader = horizon_protocol::buf::Reader::new(&body);
                let id = reader.varint().unwrap();
                let play::Clientbound::Commands(packet) =
                    play::Clientbound::decode(id, &body[reader.position()..]).unwrap()
                else {
                    panic!("commands");
                };
                let root = &packet.nodes[packet.root as usize];
                let profile = root
                    .children
                    .iter()
                    .map(|&i| &packet.nodes[i as usize])
                    .find(|n| n.name == "modprofile");
                if level < 3 {
                    assert!(profile.is_none());
                } else {
                    let profile = profile.expect("admins receive modprofile on join and refresh");
                    assert!(profile.restricted && !profile.executable);
                    let actions: Vec<_> = profile
                        .children
                        .iter()
                        .map(|&i| {
                            let node = &packet.nodes[i as usize];
                            assert!(node.executable);
                            node.name.as_str()
                        })
                        .collect();
                    assert_eq!(actions, ["report", "start", "stop"]);
                }
                if refresh {
                    assert!(
                        root.children
                            .iter()
                            .any(|&i| packet.nodes[i as usize].name == "plugin")
                    );
                }
            }
        }
    }

    #[test]
    fn registered_bool_rule_reaches_guest_commands() {
        let mut sync = SyncData::empty();
        sync.commands = Some(crate::lan::commands::CommandTree {
            root: serde_json::json!({"type":"root","children":{"gamerule":{"type":"literal","permissions":{"permission":{"level":"gamemasters"}}}}}),
        });
        sync.builtin.registries.insert(
            "minecraft:command_argument_type".into(),
            horizon_protocol::registry::Ids::from_names(
                play::ARGUMENT_TYPES
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
            ),
        );
        let (mut hub, rx) = hub_with_sync(sync);
        let rule = crate::gamerules::Rule {
            name: "example:enabled".into(),
            category: "example:machines".into(),
            default: crate::gamerules::Value::Bool(true),
            min: 0,
            max: 0,
        };
        hub.rule_registry = Arc::new(
            crate::gamerules::RuleRegistry::with_bool_rules(&["example:machines".into()], &[rule])
                .unwrap(),
        );
        hub.guests.update(0, |guest| guest.permission = 2);
        hub.send_commands(0);
        let crate::lan::Out::Packet(body) = rx.try_recv().unwrap() else {
            panic!("packet")
        };
        let mut reader = horizon_protocol::buf::Reader::new(&body);
        let id = reader.varint().unwrap();
        let play::Clientbound::Commands(packet) =
            play::Clientbound::decode(id, &body[reader.position()..]).unwrap()
        else {
            panic!("commands")
        };
        let client =
            crate::command::network::from_packet(&packet, &crate::command::Dispatcher::new());
        let regs = crate::command::Registries {
            permission: 2,
            ..Default::default()
        };
        let line = "gamerule example:enabled ";
        let suggestions = client
            .client_suggestions(&client.parse(line, &regs), line.len(), &regs)
            .1
            .into_iter()
            .map(|s| s.text)
            .collect::<Vec<_>>();
        assert_eq!(suggestions, ["false", "true"]);
        assert!(
            client
                .parse("gamerule example:enabled false", &regs)
                .executable()
                .is_ok()
        );
    }

    #[test]
    fn guest_rule_tree_comes_from_session_registry() {
        let mut sync = SyncData::empty();
        sync.commands = Some(crate::lan::commands::CommandTree {
            root: serde_json::json!({"type":"root", "children": {
                "gamerule": {"type":"literal", "permissions":{"permission":{"level":"gamemasters"}},
                    "children":{"stale_rule":{"type":"literal"}}}
            }}),
        });
        sync.builtin.registries.insert(
            "minecraft:command_argument_type".into(),
            horizon_protocol::registry::Ids::from_names(
                play::ARGUMENT_TYPES
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect(),
            ),
        );
        let (mut hub, rx) = hub_with_sync(sync);
        hub.rule_registry = Arc::new(crate::gamerules::RuleRegistry::vanilla());
        hub.guests.update(0, |g| g.permission = 2);
        hub.send_commands(0);
        let crate::lan::Out::Packet(body) = rx.try_recv().unwrap() else {
            panic!("expected packet")
        };
        let mut reader = horizon_protocol::buf::Reader::new(&body);
        let id = reader.varint().unwrap();
        let play::Clientbound::Commands(packet) =
            play::Clientbound::decode(id, &body[reader.position()..]).unwrap()
        else {
            panic!("expected commands")
        };
        let root = packet.nodes.iter().find(|n| n.name == "gamerule").unwrap();
        let names: Vec<_> = root
            .children
            .iter()
            .map(|i| packet.nodes[*i as usize].name.clone())
            .collect();
        let mut expected: Vec<_> = hub
            .rule_registry
            .available_rules(false)
            .flat_map(|r| {
                [
                    r.name.clone(),
                    r.name.strip_prefix("minecraft:").unwrap().to_owned(),
                ]
            })
            .collect();
        expected.sort();
        assert_eq!(names, expected);
        let client =
            crate::command::network::from_packet(&packet, &crate::command::Dispatcher::new());
        let regs = crate::command::Registries {
            permission: 2,
            ..Default::default()
        };
        let suggestions = |line: &str| {
            client
                .client_suggestions(&client.parse(line, &regs), line.len(), &regs)
                .1
                .into_iter()
                .map(|s| s.text)
                .collect::<Vec<_>>()
        };
        for (name, report_bounds) in [
            ("random_tick_speed", serde_json::json!({"min": 0})),
            (
                "max_snow_accumulation_height",
                serde_json::json!({"min": 0, "max": 8}),
            ),
        ] {
            let rule = packet.nodes.iter().find(|n| n.name == name).unwrap();
            let value = &packet.nodes[rule.children[0] as usize];
            assert_eq!(
                value.parser.as_ref().unwrap().1,
                match report_bounds.get("max") {
                    None => vec![1, 0, 0, 0, 0],
                    Some(_) => vec![3, 0, 0, 0, 0, 0, 0, 0, 8],
                }
            );
        }
        assert!(suggestions("gamerule keep_").contains(&"keep_inventory".to_owned()));
        assert_eq!(suggestions("gamerule keep_inventory "), ["false", "true"]);
        assert!(suggestions("gamerule max_snow_accumulation_height ").is_empty());
        assert!(
            client
                .parse("gamerule max_snow_accumulation_height 8", &regs)
                .executable()
                .is_ok()
        );
        assert!(
            client
                .parse("gamerule max_snow_accumulation_height 9", &regs)
                .executable()
                .is_err()
        );
        assert!(!names.iter().any(|n| n.contains("max_minecart_speed")));
        let tree = hub.sync.commands.as_ref().unwrap();
        let enabled = tree.packet_with_features(
            2,
            false,
            &hub.sync.builtin,
            &serde_json::Value::Null,
            &[crate::gamerules::MINECART_IMPROVEMENTS.to_owned()],
            &hub.rule_registry,
        );
        let root = enabled.nodes.iter().find(|n| n.name == "gamerule").unwrap();
        let names: Vec<_> = root
            .children
            .iter()
            .map(|i| enabled.nodes[*i as usize].name.clone())
            .collect();
        let mut expected: Vec<_> = hub
            .rule_registry
            .available_rules(true)
            .flat_map(|r| {
                [
                    r.name.clone(),
                    r.name.strip_prefix("minecraft:").unwrap().to_owned(),
                ]
            })
            .collect();
        expected.sort();
        assert_eq!(names, expected);
        assert!(
            !hub.commands_packet(0)
                .nodes
                .iter()
                .any(|n| n.name == "gamerule")
        );
        let registry = hub.rule_registry.clone();
        hub.reload_resources(hub.sync.clone(), &mut Default::default());
        assert!(Arc::ptr_eq(&registry, &hub.rule_registry));
    }

    /// [`hub_with_a_guest`] over a sync the test built.
    fn hub_with_sync(sync: SyncData) -> (Hub, std::sync::mpsc::Receiver<crate::lan::Out>) {
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            ..crate::lan::NetConfig::loopback(0, "EffectSyncTest".into(), 2)
        };
        let sync = std::sync::Arc::new(sync);
        let lan = Lan::start_with(sync.clone(), config).unwrap();
        let encoder = ChunkEncoder::new(
            [] as [(std::sync::Arc<str>, bool); 0],
            &horizon_protocol::registry::Builtin::default(),
            &[],
            &[],
        );
        let hub_config = HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 10,
            simulation_distance: 10,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 20,
            integrated: false,
            levels: Default::default(),
        };
        let mut hub = Hub::new(
            lan,
            sync,
            encoder,
            hub_config,
            crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let mut bob = Guest::new(
            crate::lan::Joined {
                conn: 1,
                uuid: 1,
                name: "Bob".into(),
                properties: Vec::new(),
                info: horizon_protocol::packets::configuration::ClientInformation::new(8),
                brand: "vanilla".into(),
                sender: crate::lan::PlayerSender::new(tx, std::sync::Arc::default()),
                latency: std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            1,
            [0.5, 64.0, 0.5],
            8,
        );
        bob.placed = true;
        bob.dimension = "minecraft:overworld".into();
        hub.guests.push(bob);
        (hub, rx)
    }

    #[test]
    fn an_empty_advancement_tree_still_sends_a_first_reset_after_staging() {
        let (mut hub, rx) = hub_with_a_guest();
        hub.mods_loaded = true;
        hub.sync_player_state(0);
        let task = hub.native_advancement_grants.pop().unwrap();
        assert!(task.grants.is_empty());
        let before: Vec<_> = rx.try_iter().collect();
        assert!(!before.iter().any(|out| match out {
            crate::lan::Out::Packet(body) =>
                horizon_protocol::buf::Reader::new(body).varint().ok()
                    == Some(ids::UPDATE_ADVANCEMENTS),
            _ => false,
        }));
        hub.finish_native_first_tick(&task, &[], &mut AdvancementOwners::default());
        let found = rx.try_iter().any(|out| match out {
            crate::lan::Out::Packet(body) => {
                let mut reader = horizon_protocol::buf::Reader::new(&body);
                reader.varint().ok() == Some(ids::UPDATE_ADVANCEMENTS)
                    && reader.bool().ok() == Some(true)
            }
            _ => false,
        });
        assert!(found, "the first packet resets the client's empty tree");
    }

    /// `store_mod_commands` keeps a new mod tree without sending it (the
    /// game sends it after the mods filtered it: `native_refresh_commands`),
    /// and `set_mod_commands` still sends a changed tree, and only a changed one.
    #[test]
    fn stored_mod_commands_are_not_sent_until_asked() {
        let (mut hub, rx) = hub_with_a_guest();
        let trees = |rx: &std::sync::mpsc::Receiver<crate::lan::Out>| {
            let mut sent = 0;
            while let Ok(out) = rx.try_recv() {
                if let crate::lan::Out::Packet(body) = out {
                    let mut r = horizon_protocol::buf::Reader::new(&body);
                    let pid = r.varint().unwrap();
                    if let Ok(C::Commands(_)) = C::decode(pid, r.rest()) {
                        sent += 1;
                    }
                }
            }
            sent
        };
        let first = serde_json::json!({"hello": {}});
        assert!(hub.store_mod_commands(0, first.clone()));
        assert!(!hub.store_mod_commands(0, first.clone()));
        assert_eq!(trees(&rx), 0);
        assert_eq!(hub.mod_commands_key(), Some(0));
        hub.set_mod_commands(0, first);
        assert_eq!(trees(&rx), 0, "an unchanged tree is not sent again");
        hub.set_mod_commands(0, serde_json::json!({"hello": {}, "bye": {}}));
        assert_eq!(trees(&rx), 1);
        hub.resend_commands();
        assert_eq!(trees(&rx), 1, "every placed player gets their tree again");
    }

    /// `onGameRuleChanged`: a changed `reduced_debug_info` reaches each player
    /// as their own entity event 22 / 23, and an unchanged one sends nothing.
    #[test]
    fn reduced_debug_info_changes_reach_players_as_entity_events() {
        let (mut hub, rx) = hub_with_a_guest();
        let id = hub.guests[0].entity_id;
        hub.set_reduced_debug_info(false);
        assert!(sent_entity_events(&rx).is_empty());
        hub.set_reduced_debug_info(true);
        hub.set_reduced_debug_info(true);
        assert_eq!(sent_entity_events(&rx), vec![(id, 22)]);
        hub.set_reduced_debug_info(false);
        assert_eq!(sent_entity_events(&rx), vec![(id, 23)]);
    }

    /// `ServerLevel.broadcastEntityEvent`: an entity event reaches the guest
    /// tracking the entity and the guest whose player it is, but not a guest
    /// that does not track it, even one well within 128 blocks.
    #[test]
    fn entity_events_reach_tracking_players_and_self() {
        let mut sync = SyncData::empty();
        sync.builtin.registries.insert(
            "minecraft:entity_type".into(),
            horizon_protocol::registry::Ids::from_names(vec!["minecraft:mooshroom".into()]),
        );
        let (mut hub, rx) = hub_with_sync(sync);
        let own = hub.guests[0].entity_id;
        hub.guests.update(0, |g| g.sent_chunks.insert((0, 0)));
        let near = EntityState {
            id: 37,
            uuid: 37,
            kind: "minecraft:mooshroom".into(),
            position: [0.5, 64.0, 0.5],
            ..EntityState::default()
        };
        // In a chunk the guest was not sent, so it is not tracked.
        let untracked = EntityState {
            id: 38,
            uuid: 38,
            position: [100.5, 64.0, 0.5],
            ..near.clone()
        };
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let border = crate::worldborder::WorldBorder::default();
        let mut next_entity = 39;
        let entities = [near, untracked];
        let mut ctx = TickCtx {
            sea_level: 63,
            world: &mut world,
            entities: &entities,
            next_entity: &mut next_entity,
            spawn: [0.5, 64.0, 0.5],
            spawn_angle: [0.0; 2],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &border,
            streaming: None,
        };
        hub.track_entities(&mut ctx);
        assert!(hub.guests[0].tracker.is_tracking(37));
        assert!(!hub.guests[0].tracker.is_tracking(38));
        while rx.try_recv().is_ok() {}
        hub.entity_event(37, 2);
        hub.entity_event(38, 2);
        hub.entity_event(own, 35);
        assert_eq!(sent_entity_events(&rx), vec![(37, 2), (own, 35)]);
    }

    /// The entity events (id, event) the guest was sent, in order.
    fn sent_entity_events(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<(i32, i8)> {
        let mut sent = Vec::new();
        while let Ok(crate::lan::Out::Packet(body)) = rx.try_recv() {
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let pid = r.varint().unwrap();
            if let Ok(C::EntityEvent(p)) = C::decode(pid, r.rest()) {
                sent.push((p.id, p.event));
            }
        }
        sent
    }

    /// The blocks the guest was sent as `ClientboundBlockUpdatePacket`s, in order, with the
    /// state each one carries.
    fn sent_block_updates(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<([i32; 3], i32)> {
        let mut sent = Vec::new();
        while let Ok(out) = rx.try_recv() {
            let crate::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            if let Ok(C::BlockUpdate(p)) = C::decode(id, r.rest()) {
                sent.push((p.pos, p.state));
            }
        }
        sent
    }

    /// A world whose blocks at y = 63 are `state` under the four chunks around the origin.
    fn floor_at(state: u32) -> World {
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        for cz in -1..=0 {
            for cx in -1..=0 {
                let mut chunk = crate::chunk::Chunk::empty(cx, cz);
                for z in 0..16 {
                    for x in 0..16 {
                        chunk.set(x, 63, z, state);
                    }
                }
                chunk.lit = true;
                world.chunks.insert((cx, cz), std::sync::Arc::new(chunk));
            }
        }
        world
    }

    /// One tick of the hub that handles a single packet from the guest.
    fn tick_packet(hub: &mut Hub, world: &mut World, packet: S) {
        hub.guests
            .inbox_mut(0)
            .unwrap()
            .push_back(horizon_client::lan::guest::InboxItem::uncredited(packet));
        let mut next_entity = 2;
        let border = crate::worldborder::WorldBorder::default();
        let mut ctx = TickCtx {
            sea_level: 63,
            world,
            entities: &[],
            next_entity: &mut next_entity,
            spawn: [0.5, 64.0, 0.5],
            spawn_angle: [0.0; 2],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &border,
            streaming: None,
        };
        let _ = hub.drain_packets(&mut ctx);
    }

    /// One tick of the hub that handles a single position move from the guest.
    fn tick_guest_move(hub: &mut Hub, world: &mut World, to: [f64; 3], flags: u8) {
        // One position move per client tick (receivedPositionThisTick).
        {
            let value = false;
            hub.guests.update(0, |g| g.position_this_tick = value);
        }
        tick_packet(
            hub,
            world,
            S::MovePlayerPos(play::MovePlayerPos {
                x: to[0],
                y: to[1],
                z: to[2],
                flags,
            }),
        );
    }

    /// `ClientboundRespawnPacket.KEEP_ALL_DATA`: a level change keeps the client's copy of the
    /// player's shared flags, so one that clears afterwards still reaches it. (A creative player
    /// who leaves lava through a dimension change kept the fire on its screen.)
    #[test]
    fn a_shared_flag_cleared_after_a_level_change_reaches_the_player() {
        let (mut hub, rx) = hub_with_a_guest();
        let shared_flags_sent = |rx: &std::sync::mpsc::Receiver<crate::lan::Out>| {
            let mut sent = Vec::new();
            while let Ok(out) = rx.try_recv() {
                let crate::lan::Out::Packet(body) = out else {
                    continue;
                };
                let mut r = horizon_protocol::buf::Reader::new(&body);
                let id = r.varint().unwrap();
                if let Ok(C::SetEntityData(packet)) = C::decode(id, r.rest()) {
                    sent.extend(packet.values.iter().filter_map(|(index, _, value)| {
                        match (index, value) {
                            (0, Value::Byte(flags)) => Some(*flags),
                            _ => None,
                        }
                    }));
                }
            }
            sent
        };
        hub.guests.update(0, |g| g.sneaking = true);
        hub.sync_player_state(0);
        assert_eq!(shared_flags_sent(&rx), [0x02]);
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let mut next_entity = 2;
        let border = crate::worldborder::WorldBorder::default();
        let ctx = TickCtx {
            sea_level: 63,
            world: &mut world,
            entities: &[],
            next_entity: &mut next_entity,
            spawn: [0.5, 64.0, 0.5],
            spawn_angle: [0.0; 2],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &border,
            streaming: None,
        };
        hub.change_level(0, [100.5, 49.0, 0.5], 0.0, None, false, 0, &ctx);
        let _ = shared_flags_sent(&rx);
        hub.guests.update(0, |g| g.sneaking = false);
        hub.sync_player_state(0);
        assert_eq!(shared_flags_sent(&rx), [0]);
    }

    /// A mod menu whose part keeps it open (`menu_still_valid`) still closes
    /// when its player changes level or respawns: those close it without
    /// asking `stillValid` (`ServerPlayer.closeContainer`).
    #[test]
    fn a_menu_kept_open_by_its_part_closes_on_level_change_and_respawn() {
        let mod_type: &'static crate::containers::ModMenuType =
            Box::leak(Box::new(crate::containers::ModMenuType {
                id: "test:kept",
                slots: 27,
                data_slots: 0,
                results: vec![],
                links: None,
                crafting: None,
            }));
        let (mut hub, _rx) = hub_with_a_guest();
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let k = (0, 64, 40);
        world
            .block_entities
            .insert(k, crate::containers::BlockEntity::Chest(vec![None; 27]));
        let mut next_entity = 2;
        let border = crate::worldborder::WorldBorder::default();
        let ctx = TickCtx {
            sea_level: 63,
            world: &mut world,
            entities: &[],
            next_entity: &mut next_entity,
            spawn: [0.5, 64.0, 0.5],
            spawn_angle: [0.0; 2],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &border,
            streaming: None,
        };
        let open = |hub: &mut Hub| {
            hub.guests.update(0, |g| {
                g.menu = Some(super::super::menu::Menu {
                    window: 1,
                    kind: 2,
                    pos: Some(k),
                    slots: super::super::menu::Slots::Block(k),
                    size: 27,
                    state: 0,
                    sent: Vec::new(),
                    drag: None,
                    sent_data: Vec::new(),
                    hidden: Vec::new(),
                    mod_type: Some(mod_type),
                    registry_id: "test:kept".into(),
                    craft: None,
                    links: Vec::new(),
                    asks_still_valid: true,
                });
            });
        };
        open(&mut hub);
        hub.change_level(0, [100.5, 49.0, 0.5], 0.0, None, false, 0, &ctx);
        assert!(hub.guests[0].menu.is_none());
        open(&mut hub);
        hub.respawn(0, [0.5, 64.0, 0.5], None, &ctx);
        assert!(hub.guests[0].menu.is_none());
    }

    /// `ServerGamePacketListenerImpl.forceSendPlayerSupportBlocks`: a client that says it stands
    /// while the last move the server resolved came down onto nothing is sent the blocks under
    /// its box again (its four corners, each once), so a client that missed a block change cannot
    /// stay standing on air. At most once every 200 ticks of the connection.
    #[test]
    fn a_guest_standing_on_nothing_is_sent_the_blocks_below_it() {
        // Block state 1 is stone in the vanilla registry order.
        const STONE: u32 = 1;
        let (mut hub, rx) = hub_with_a_guest();
        {
            let value = 0;
            hub.guests.update(0, |g| g.load_timeout = value);
        }
        // Across a block corner: the box's four corners are four different blocks.
        {
            let value = [0.0, 64.0, 0.0];
            hub.guests.update(0, |g| g.position = value);
        }
        let mut world = floor_at(STONE);
        // The connection's 200th tick is the first that may send: vanilla starts the count at 0.
        hub.ticks = 199;
        tick_guest_move(&mut hub, &mut world, [0.0, 64.0, 0.0], 1);
        assert!(
            sent_block_updates(&rx).is_empty(),
            "the first 200 ticks of a connection send nothing"
        );
        hub.ticks = 200;
        tick_guest_move(&mut hub, &mut world, [0.0, 64.0, 0.0], 1);
        assert_eq!(
            sent_block_updates(&rx),
            vec![
                ([-1, 63, -1], STONE as i32),
                ([0, 63, -1], STONE as i32),
                ([-1, 63, 0], STONE as i32),
                ([0, 63, 0], STONE as i32),
            ],
            "the four blocks under the box, each once, with the server's states"
        );
        // A tick later: the next 200 have to pass first.
        hub.ticks = 201;
        tick_guest_move(&mut hub, &mut world, [0.0, 64.0, 0.0], 1);
        assert!(
            sent_block_updates(&rx).is_empty(),
            "no more than once every 200 ticks"
        );
        // The last move the server resolved came down onto something: it stands on its own.
        {
            let value = true;
            hub.guests.update(0, |g| g.rests_on_ground = value);
        }
        hub.ticks = 500;
        tick_guest_move(&mut hub, &mut world, [0.0, 64.0, 0.0], 1);
        assert!(
            sent_block_updates(&rx).is_empty(),
            "a player the server saw land is not resent the blocks"
        );
        // A client that does not claim the ground is not helped either.
        {
            let value = false;
            hub.guests.update(0, |g| g.rests_on_ground = value);
        }
        hub.ticks = 800;
        tick_guest_move(&mut hub, &mut world, [0.0, 64.0, 0.0], 0);
        assert!(
            sent_block_updates(&rx).is_empty(),
            "only a claim of standing on the ground asks for the blocks below"
        );
        // A box over one block gets that block once (the corners are the same cell).
        hub.ticks = 1100;
        {
            let value = [0.5, 64.0, 0.5];
            hub.guests.update(0, |g| g.position = value);
        }
        tick_guest_move(&mut hub, &mut world, [0.5, 64.0, 0.5], 1);
        assert_eq!(
            sent_block_updates(&rx),
            vec![([0, 63, 0], STONE as i32)],
            "one block under a box that sits across a block"
        );
    }

    #[test]
    fn held_network_guests_keep_level_updates_without_including_fake_actors() {
        let (mut hub, rx) = hub_with_a_guest();
        {
            let value = false;
            hub.guests.update(0, |g| g.placed = value);
        }
        {
            let value = true;
            hub.guests.update(0, |g| g.held = value);
        }
        let (mut other, fake_rx) = hub_with_a_guest();
        let mut fake = other.guests.remove(0);
        fake.kind = super::super::guest::GuestKind::Fake;
        fake.conn = 2;
        fake.uuid = 2;
        fake.entity_id = 2;
        fake.position = [64.5, 64.0, 0.5];
        fake.held = true;
        hub.guests.push(fake);
        assert!(!hub.here(&hub.guests[0]));
        assert!(hub.watches(&hub.guests[0]));
        assert!(!hub.watches(&hub.guests[1]));
        assert_eq!(hub.loading_chunks(|_| true).collect::<Vec<_>>(), [(0, 0)]);
        let packet = C::GameEvent(play::GameEvent {
            event: game_event::IMMEDIATE_RESPAWN,
            param: 1.0,
        });
        hub.broadcast_level(&packet);
        hub.broadcast_watching(&packet);
        hub.broadcast(&packet);
        assert_eq!(
            rx.try_iter().count(),
            2,
            "held guests watch but are not placed online players"
        );
        assert_eq!(fake_rx.try_iter().count(), 0);
    }

    #[test]
    fn fake_is_not_advertised_in_player_latency_updates() {
        let (mut hub, rx) = hub_with_a_guest();
        let (tx, _fake_rx) = std::sync::mpsc::channel();
        let mut fake = Guest::new(
            crate::lan::Joined {
                conn: 2,
                uuid: 2,
                name: "Fake".into(),
                properties: Vec::new(),
                info: horizon_protocol::packets::configuration::ClientInformation::new(8),
                brand: String::new(),
                sender: crate::lan::PlayerSender::new(tx, std::sync::Arc::default()),
                latency: std::sync::Arc::default(),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            2,
            [32.0, 64.0, 0.0],
            8,
        );
        fake.kind = super::super::guest::GuestKind::Fake;
        fake.placed = true;
        hub.guests.push(fake);
        assert_eq!(hub.online_count(), 1);
        let reply = hub.list_reply(false);
        let Some(Tag::List(args)) = reply.get("with") else {
            panic!("expected list reply arguments");
        };
        assert_eq!(args[0], Tag::Int(1));
        assert_eq!(args[2], display_name(&hub.loop_context, 1, "Bob"));
        hub.guests.update(0, |g| g.sent_chunks.insert((2, 0)));
        let mut world = crate::world::World::new(horizon_world::registry::BlockStateId(0), 0);
        let mut next_entity = 100;
        let border = crate::worldborder::WorldBorder::default();
        let mut ctx = TickCtx {
            sea_level: 63,
            world: &mut world,
            entities: &[],
            next_entity: &mut next_entity,
            spawn: [0.5, 64.0, 0.5],
            spawn_angle: [0.0; 2],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &border,
            streaming: None,
        };
        hub.track_entities(&mut ctx);
        assert!(!hub.guests[0].tracker.is_tracking(2));
        assert!(hub.guests[1].tracker.ids().is_empty());
        {
            let value = false;
            hub.guests.update(0, |g| g.placed = value);
        }
        hub.place_new_player(2, &mut ctx);
        assert!(rx.try_recv().is_err());
        hub.place_new_player(1, &mut ctx);
        let mut saw_list = false;
        while let Ok(crate::lan::Out::Packet(body)) = rx.try_recv() {
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let Ok(id) = reader.varint() else { continue };
            if let Ok(C::PlayerInfoUpdate(update)) = C::decode(id, reader.rest()) {
                saw_list = true;
                assert!(update.entries.iter().all(|entry| entry.uuid != 2));
            }
        }
        assert!(saw_list);
        hub.ticks = 601;
        hub.end_tick();
        let crate::lan::Out::Packet(body) =
            rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap()
        else {
            panic!("expected player info update");
        };
        let mut reader = horizon_protocol::buf::Reader::new(&body);
        let id = reader.varint().unwrap();
        let C::PlayerInfoUpdate(update) = C::decode(id, reader.rest()).unwrap() else {
            panic!("expected player info update");
        };
        assert_eq!(update.entries.len(), 1);
        assert_eq!(update.entries[0].uuid, 1);
    }

    /// Exercise the compiled conversion path, including full component equality,
    /// selection, removals and registry reload invalidation.
    #[test]
    fn equipment_is_converted_again_only_when_a_stack_changes() {
        use horizon_items::equipment::{BODY, SADDLE};
        let (mut hub, _rx) = hub_with_a_guest();
        hub.guests.update(0, |g| {
            g.inventory[0] = Some(ItemStack::new("minecraft:stone", 1));
            g.selected = 0;
            g.offhand = Some(ItemStack::new("minecraft:shield", 1));
            g.armor[0] = Some(ItemStack::new("minecraft:iron_helmet", 1));
            g.equipment[BODY] = Some(ItemStack::new("minecraft:white_carpet", 1));
            g.equipment[SADDLE] = Some(ItemStack::new("minecraft:saddle", 1));
        });
        let first = hub.guest_equipment(0);
        assert_eq!(
            first.iter().map(|(slot, _)| *slot).collect::<Vec<_>>(),
            [0, 1, 5, 6, 7]
        );
        for _ in 0..3 {
            assert_eq!(hub.guest_equipment(0), first);
            for slot in 0..equipment_cache::SLOTS {
                assert!(
                    !hub.guests[0]
                        .equipment_cache
                        .is_stale(slot, equipment_source(&hub.guests[0], slot))
                );
            }
        }
        hub.guests
            .update(0, |g| g.inventory[0].as_mut().unwrap().count = 3);
        let changed = hub.guest_equipment(0);
        assert_ne!(changed, first);
        let mut component_stack = ItemStack::new("minecraft:stone", 3);
        let mut data = crate::item_data::StackData::default();
        data.repair_cost = 7;
        component_stack.extra = Some(Box::new(data));
        hub.guests
            .update(0, |g| g.inventory[0] = Some(component_stack.clone()));
        assert!(
            hub.guests[0]
                .equipment_cache
                .is_stale(0, Some(&component_stack))
        );
        let expected = hub.stack(&Some(component_stack));
        assert_eq!(hub.guest_equipment(0)[0], (0, expected));
        hub.guests.update(0, |g| {
            g.selected = 1;
            g.offhand = None;
            g.equipment[BODY] = None;
            g.equipment[SADDLE] = None;
        });
        let cleared = hub.guest_equipment(0);
        assert_eq!(
            cleared.iter().map(|(slot, _)| *slot).collect::<Vec<_>>(),
            [5]
        );
        assert!(!hub.guests[0].equipment_cache.is_stale(1, None));
        hub.reload_resources(hub.sync.clone(), &mut Default::default());
        assert!(
            hub.guests[0]
                .equipment_cache
                .is_stale(5, hub.guests[0].armor[0].as_ref())
        );
        assert_eq!(hub.guest_equipment(0), cleared);
    }

    /// `LivingEntity.collectEquipmentChanges` walks every `EquipmentSlot`: a
    /// player's body (wire slot 6) and saddle (7) reach the players who see
    /// them, with the hands and armor.
    #[test]
    fn a_players_body_and_saddle_reach_the_players_who_see_them() {
        use horizon_items::equipment::{BODY, SADDLE};
        let (mut hub, rx) = hub_with_a_guest();
        let (tx, _ann_rx) = std::sync::mpsc::channel();
        let mut ann = Guest::new(
            crate::lan::Joined {
                conn: 2,
                uuid: 2,
                name: "Ann".into(),
                properties: Vec::new(),
                info: horizon_protocol::packets::configuration::ClientInformation::new(8),
                brand: "vanilla".into(),
                sender: crate::lan::PlayerSender::new(tx, std::sync::Arc::default()),
                latency: std::sync::Arc::default(),
                address: "127.0.0.1:25566".parse().unwrap(),
                expected: false,
            },
            2,
            [4.5, 64.0, 0.5],
            8,
        );
        ann.placed = true;
        ann.dimension = "minecraft:overworld".into();
        ann.offhand = Some(ItemStack::new("minecraft:shield", 1));
        ann.equipment[BODY] = Some(ItemStack::new("minecraft:white_carpet", 1));
        ann.equipment[SADDLE] = Some(ItemStack::new("minecraft:saddle", 1));
        hub.guests.push(ann);
        hub.guests.update(0, |g| g.sent_chunks.insert((0, 0)));
        // The test hub syncs no entity type registry.
        hub.entity_types.insert("minecraft:player".into(), 1);
        let mut world = crate::world::World::new(horizon_world::registry::BlockStateId(0), 0);
        let mut next_entity = 100;
        let border = crate::worldborder::WorldBorder::default();
        let mut ctx = TickCtx {
            sea_level: 63,
            world: &mut world,
            entities: &[],
            next_entity: &mut next_entity,
            spawn: [0.5, 64.0, 0.5],
            spawn_angle: [0.0; 2],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &border,
            streaming: None,
        };
        hub.track_entities(&mut ctx);
        assert!(hub.guests[0].tracker.is_tracking(2));
        let mut slots = None;
        while let Ok(crate::lan::Out::Packet(body)) = rx.try_recv() {
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let Ok(id) = reader.varint() else { continue };
            if let Ok(C::SetEquipment(p)) = C::decode(id, reader.rest())
                && p.id == 2
            {
                slots = Some(p.slots.iter().map(|(slot, _)| *slot).collect::<Vec<_>>());
            }
        }
        assert_eq!(slots, Some(vec![1, 6, 7]));
    }

    /// A mod's named effect particle goes out as id -1, which vanilla's
    /// `ParticleTypes.STREAM_CODEC` cannot read: the entity data sent to a
    /// Horizon client has it, a vanilla client's has the rest of the list. As
    /// the bytes on the wire, when a client starts tracking the entity, when
    /// the list changes, and when the entity is paired as it joins.
    #[test]
    fn a_named_effect_particle_is_sent_only_to_horizon_clients() {
        // id, name length, name.
        const NAMED: [u8; 19] = [
            0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 13, b'e', b'x', b'a', b'm', b'p', b'l', b'e', b':', b's',
            b'w', b'i', b'r', b'l',
        ];
        // ClientboundSetEntityDataPacket: the packet's id, then the entity's.
        let data = |entity: u8, entries: &[&[u8]]| {
            let mut body = vec![101, entity];
            body.extend(entries.concat());
            body.push(0xFF);
            body
        };
        // Entry 10, PARTICLES (17), then the count.
        let particles = |list: &[&[u8]]| {
            let mut entry = vec![10, 17, list.len() as u8];
            entry.extend(list.concat());
            entry
        };
        let ambient: &[u8] = &[11, 8, 1];
        let plain: &[u8] = &[0];

        let (mut hub, bob_rx) = hub_with_a_guest();
        let (tx, ann_rx) = std::sync::mpsc::channel();
        let mut ann = Guest::new(
            crate::lan::Joined {
                conn: 2,
                uuid: 2,
                name: "Ann".into(),
                properties: Vec::new(),
                info: horizon_protocol::packets::configuration::ClientInformation::new(8),
                brand: "horizon".into(),
                sender: crate::lan::PlayerSender::new(tx, std::sync::Arc::default()),
                latency: std::sync::Arc::default(),
                address: "127.0.0.1:25566".parse().unwrap(),
                expected: false,
            },
            2,
            [4.5, 64.0, 0.5],
            8,
        );
        ann.placed = true;
        ann.dimension = "minecraft:overworld".into();
        hub.guests.push(ann);
        hub.guests.update_all(|g| {
            g.sent_chunks.insert((0, 0));
        });
        // The test hub syncs no entity type registry.
        hub.entity_types.insert("minecraft:zombie".into(), 1);

        // The entity data a client was sent since the last look.
        let sent = |rx: &std::sync::mpsc::Receiver<crate::lan::Out>| {
            let mut bodies = Vec::new();
            while let Ok(out) = rx.try_recv() {
                match out {
                    crate::lan::Out::Packet(body) => bodies.push(body),
                    crate::lan::Out::Packets(list) => bodies.extend(list),
                    crate::lan::Out::Disconnect(_) => panic!("disconnected"),
                }
            }
            bodies.retain(|body| body.first() == Some(&101));
            bodies
        };
        let zombie = |id: i32, meta: Vec<(u8, i32, Value)>| EntityState {
            id,
            uuid: id as u128,
            kind: "minecraft:zombie".into(),
            position: [2.5, 64.0, 0.5],
            meta,
            ..EntityState::default()
        };
        let mut world = crate::world::World::new(horizon_world::registry::BlockStateId(0), 0);
        let mut next_entity = 200;
        let border = crate::worldborder::WorldBorder::default();
        let mut track = |hub: &mut Hub, state: EntityState| {
            let entities = [state];
            hub.track_entities(&mut TickCtx {
                sea_level: 63,
                world: &mut world,
                entities: &entities,
                next_entity: &mut next_entity,
                spawn: [0.5, 64.0, 0.5],
                spawn_angle: [0.0; 2],
                difficulty: 2,
                difficulty_locked: false,
                clock_rate: 1.0,
                tick_rate: 20.0,
                tick_frozen: false,
                tick_steps: Default::default(),
                collision: None,
                border: &border,
                streaming: None,
            });
        };

        // What a listener's replacement becomes.
        let named = crate::particles::ParticleOptions::named("example:swirl");
        let only_named = hub.particle_meta(std::slice::from_ref(&named), false);
        let Value::Particles(list) = &only_named[0].2 else {
            panic!("expected particles");
        };
        let mixed = vec![
            (
                10,
                serializer::PARTICLES,
                Value::Particles(vec![list[0].clone(), Particle { id: 0, data: None }]),
            ),
            (11, serializer::BOOLEAN, Value::Bool(true)),
        ];

        // Starting to track it.
        track(&mut hub, zombie(100, mixed.clone()));
        assert_eq!(
            sent(&ann_rx),
            [data(100, &[&particles(&[&NAMED, plain]), ambient])]
        );
        assert_eq!(sent(&bob_rx), [data(100, &[&particles(&[plain]), ambient])]);

        // The change leaves only the named particle: vanilla's list empties.
        let mut changed = only_named.clone();
        changed.push(mixed[1].clone());
        track(&mut hub, zombie(100, changed));
        assert_eq!(sent(&ann_rx), [data(100, &[&particles(&[&NAMED])])]);
        assert_eq!(sent(&bob_rx), [data(100, &[&particles(&[])])]);

        // No named particle: both clients get the same bytes.
        let ordinary = vec![(
            10,
            serializer::PARTICLES,
            Value::Particles(vec![Particle { id: 0, data: None }]),
        )];
        track(&mut hub, zombie(100, ordinary));
        let expected = [data(100, &[&particles(&[plain])])];
        assert_eq!(sent(&ann_rx), expected);
        assert_eq!(sent(&bob_rx), expected);

        // Paired as it joins, after the tracking pass.
        hub.note_joined(&[zombie(101, only_named)]);
        assert_eq!(sent(&ann_rx), [data(101, &[&particles(&[&NAMED])])]);
        assert_eq!(sent(&bob_rx), [data(101, &[&particles(&[])])]);
    }

    /// `handleChatSessionUpdate` shares `chatMessageChain` with chat: a new
    /// session is announced after a message that came before it. With mods
    /// loaded that message waits for the game (`ChatSubmitted`), so the
    /// announcement waits behind it; with nothing held it goes out at once.
    #[test]
    fn a_new_chat_session_waits_behind_chat_the_mods_hold() {
        let (mut hub, rx) = hub_with_a_guest();
        let update = || play::PlayerInfoUpdate {
            actions: 2,
            entries: vec![play::PlayerInfo {
                uuid: 1,
                ..Default::default()
            }],
        };
        let mut events = Vec::new();
        hub.announce_chat_session(update(), &mut events);
        assert!(events.is_empty());
        assert_eq!(rx.try_iter().count(), 1, "announced at once");

        let mut events = vec![HostEvent::ChatSubmitted {
            message: Box::new(chat::ChatMessage {
                sender: 1,
                index: 0,
                signature: None,
                content: "hi".into(),
                timestamp: 0,
                salt: 0,
                last_seen: Vec::new(),
                unsigned_content: None,
            }),
            bound: Box::new(play::ChatTypeBound {
                chat_type: 0,
                name: horizon_core::nbt::Tag::String("Bob".into()),
                target: None,
            }),
            name: "Bob".into(),
        }];
        hub.announce_chat_session(update(), &mut events);
        assert_eq!(rx.try_iter().count(), 0, "held behind the chat");
        assert!(matches!(
            events.as_slice(),
            [HostEvent::ChatSubmitted { .. }, HostEvent::ChatSessionStarted(u)] if **u == update()
        ));
    }
    /// The tabs the guest was sent in `ClientboundSelectAdvancementsTabPacket`s, in order.
    fn sent_tabs(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<Option<String>> {
        let mut sent = Vec::new();
        while let Ok(out) = rx.try_recv() {
            let crate::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            if let Ok(C::SelectAdvancementsTab(p)) = C::decode(id, r.rest()) {
                sent.push(p.tab);
            }
        }
        sent
    }

    /// A data folder holding two advancements: `minecraft:root`, a root with a display, and
    /// `minecraft:task`, which hangs off it.
    fn advancement_data() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "horizon-advancement-tab-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let folder = root.join("data").join("minecraft").join("advancement");
        std::fs::create_dir_all(&folder).unwrap();
        let display = |icon: &str, title: &str| {
            format!(r#"{{"icon":{{"id":"{icon}"}},"title":"{title}","description":"{title}"}}"#)
        };
        std::fs::write(
            folder.join("root.json"),
            format!(
                r#"{{"display":{},"criteria":{{}}}}"#,
                display("minecraft:stone", "Root")
            ),
        )
        .unwrap();
        std::fs::write(
            folder.join("task.json"),
            format!(
                r#"{{"parent":"minecraft:root","display":{},"criteria":{{}}}}"#,
                display("minecraft:dirt", "Task")
            ),
        )
        .unwrap();
        root
    }

    /// ServerGamePacketListenerImpl.handleSeenAdvancements with PlayerAdvancements.setSelectedTab:
    /// the tab a player opens is kept when it names a root with a display, and the player is told
    /// the tab the server keeps — none for anything else — whenever that changed. A closed screen
    /// says nothing about the tab.
    #[test]
    fn an_advancement_tab_a_guest_opens_is_kept_and_answered() {
        let data = advancement_data();
        let mut sync = SyncData::empty();
        sync.advancements = crate::lan::advancements::ServerAdvancements::load(
            &data,
            &horizon_world::loot::PackOverlay::NONE,
            Default::default(),
            Default::default(),
        );
        assert_eq!(
            sync.advancements.selectable("minecraft:root"),
            Some(true),
            "the fixture's root is selectable"
        );
        let (mut hub, rx) = hub_with_sync(sync);
        {
            let value = 0;
            hub.guests.update(0, |g| g.load_timeout = value);
        }
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let open = |tab: &str| {
            S::SeenAdvancements(play::SeenAdvancements {
                action: 0,
                tab: Some(tab.to_owned()),
            })
        };
        // The root the client opens is the tab the server keeps now.
        tick_packet(&mut hub, &mut world, open("minecraft:root"));
        assert_eq!(sent_tabs(&rx), vec![Some("minecraft:root".to_owned())]);
        assert_eq!(
            hub.guests[0].selected_tab.as_deref(),
            Some("minecraft:root")
        );
        // Opening it again changes nothing, so nothing is sent.
        tick_packet(&mut hub, &mut world, open("minecraft:root"));
        assert!(
            sent_tabs(&rx).is_empty(),
            "an unchanged tab is not sent again"
        );
        // A tab that is not a root takes the selection away, and the client is told so.
        tick_packet(&mut hub, &mut world, open("minecraft:task"));
        assert_eq!(sent_tabs(&rx), vec![None]);
        assert_eq!(hub.guests[0].selected_tab, None);
        // The root again, then an id the server has no advancement for: vanilla's
        // `ServerAdvancementManager.get` is null, `setSelectedTab` is not called, and the
        // tab stays.
        tick_packet(&mut hub, &mut world, open("minecraft:root"));
        assert_eq!(sent_tabs(&rx), vec![Some("minecraft:root".to_owned())]);
        tick_packet(&mut hub, &mut world, open("minecraft:nowhere"));
        assert!(sent_tabs(&rx).is_empty(), "an unknown id changes nothing");
        assert_eq!(
            hub.guests[0].selected_tab.as_deref(),
            Some("minecraft:root")
        );
        // The closed screen keeps the tab the server has.
        tick_packet(
            &mut hub,
            &mut world,
            S::SeenAdvancements(play::SeenAdvancements {
                action: 1,
                tab: None,
            }),
        );
        assert!(sent_tabs(&rx).is_empty(), "a closed screen keeps the tab");
        assert_eq!(
            hub.guests[0].selected_tab.as_deref(),
            Some("minecraft:root")
        );
        let _ = std::fs::remove_dir_all(&data);
    }

    /// `handlePlayerCommand` START_RIDING_JUMP (3) and STOP_RIDING_JUMP (4): the
    /// hub used to drop both, so a guest's released jump bar never reached the
    /// mount they ride, and neither did the horse's rear or its jump sound.
    #[test]
    fn a_guests_riding_jump_reaches_the_game() {
        let (mut hub, _rx) = hub_with_a_guest();
        // hasClientLoaded: the handlers ignore a player's actions until then.
        {
            let value = 0;
            hub.guests.update(0, |g| g.load_timeout = value);
        }
        let mut world = crate::world::World::new(horizon_world::registry::BlockStateId(0), 0);
        let mut next_entity = 100;
        let border = crate::worldborder::WorldBorder::default();
        let mut ctx = TickCtx {
            sea_level: 63,
            world: &mut world,
            entities: &[],
            next_entity: &mut next_entity,
            spawn: [0.5, 64.0, 0.5],
            spawn_angle: [0.0; 2],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &border,
            streaming: None,
        };
        for action in [3, 4] {
            hub.guests.inbox_mut(0).unwrap().push_back(
                horizon_client::lan::guest::InboxItem::uncredited(S::PlayerCommand(
                    play::PlayerCommand {
                        id: 1,
                        action,
                        data: 75,
                    },
                )),
            );
            let events = hub.tick(&mut ctx);
            let reached = events.iter().any(|event| {
                matches!(event, HostEvent::Packet(1, packet) if matches!(&**packet, S::PlayerCommand(c) if c.action == action && c.data == 75))
            });
            assert!(reached, "action {action} reached the game");
        }
    }

    /// The durations of the effect packets the guest was sent.
    fn sent_durations(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<i32> {
        let mut durations = Vec::new();
        while let Ok(out) = rx.try_recv() {
            let crate::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            if let Ok(C::UpdateMobEffect(p)) = C::decode(id, r.rest()) {
                durations.push(p.duration);
            }
        }
        durations
    }

    fn sent_frozen(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<i32> {
        let mut sent = Vec::new();
        while let Ok(out) = rx.try_recv() {
            let crate::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            if let Ok(C::SetEntityData(p)) = C::decode(id, r.rest()) {
                for (index, _, value) in p.values {
                    if let (7, Value::Int(ticks)) = (index, value) {
                        sent.push(ticks);
                    }
                }
            }
        }
        sent
    }

    #[test]
    fn a_guests_frozen_ticks_go_to_its_client() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut body =
            crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO)
                .unwrap();
        body.ticks_frozen = 140;
        let (mut hub, rx) = hub_with_a_guest();
        {
            let value = Some(Box::new(body));
            hub.guests.update(0, |g| g.motion = value);
        }
        hub.sync_player_state(0);
        assert_eq!(sent_frozen(&rx), vec![140]);
        hub.sync_player_state(0);
        assert_eq!(sent_frozen(&rx), Vec::<i32>::new());
        hub.guests
            .update(0, |g| g.motion.as_mut().unwrap().ticks_frozen = 0);
        hub.sync_player_state(0);
        assert_eq!(sent_frozen(&rx), vec![0]);
    }

    /// The host and port of the transfer packets the guest was sent.
    fn sent_transfers(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<(String, i32)> {
        let mut sent = Vec::new();
        while let Ok(out) = rx.try_recv() {
            let crate::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            if let Ok(C::Transfer(p)) = C::decode(id, r.rest()) {
                sent.push((p.host, p.port));
            }
        }
        sent
    }

    /// The relatives of the position packets the guest was sent.
    fn sent_position_relatives(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<i32> {
        let mut sent = Vec::new();
        while let Ok(out) = rx.try_recv() {
            let crate::lan::Out::Packet(body) = out else {
                continue;
            };
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            if let Ok(C::PlayerPosition(p)) = C::decode(id, r.rest()) {
                sent.push(p.relatives);
            }
        }
        sent
    }

    /// A command's teleport carries its relative axes as the position
    /// packet's DELTA flags (`TeleportCommand.performTeleport` builds them
    /// with `Relative.direction`, and `ServerPlayer.teleport` passes them to
    /// `ClientboundPlayerPositionPacket`): a relative axis keeps its motion
    /// on the client, an absolute axis is a zero change.
    #[test]
    fn a_command_teleport_carries_its_relative_axes() {
        use play::relative::{DELTA_X, DELTA_Z};
        let (mut hub, rx) = hub_with_a_guest();
        hub.teleport_command(0, [10.5, 70.0, 10.5], [true, false, true]);
        hub.teleport_command(0, [12.5, 70.0, 12.5], [false; 3]);
        assert_eq!(sent_position_relatives(&rx), vec![DELTA_X | DELTA_Z, 0]);
    }

    /// `TransferCommand.transfer`: the target's client is sent the play-state
    /// `ClientboundTransferPacket` with the hostname and the port, and nothing
    /// else disconnects it — the client closes and logs in there itself
    /// (`ClientPacketListener.handleTransfer`).
    #[test]
    fn a_transferred_player_is_sent_the_host_and_the_port() {
        let (hub, rx) = hub_with_a_guest();
        hub.transfer(1, "example.net", 25566);
        assert_eq!(sent_transfers(&rx), vec![("example.net".to_owned(), 25566)]);
        // A connection the hub does not know gets nothing.
        hub.transfer(9, "example.net", 25566);
        assert_eq!(
            sent_transfers(&rx),
            Vec::<(String, i32)>::new(),
            "an unknown connection is not a player of this server"
        );
    }

    /// `/effect give Bob minecraft:speed 30 1` from a dedicated server's
    /// console (#7115). Vanilla runs the console after the players' tick
    /// (`DedicatedServer.tickConnection`) and `ServerPlayer.onEffectAdded`
    /// sends the effect at once: 30 s is 600 ticks, and the tick after, which
    /// counts it down to 599, sends nothing (`LivingEntity.tickEffects` only
    /// refreshes it at a multiple of 600 left). The console syncs right after
    /// its command (`console_command`) for the same packet.
    #[test]
    fn an_effect_synced_after_its_command_is_sent_once_at_its_full_duration() {
        let (mut hub, rx) = hub_with_a_guest();
        hub.guests.update(0, |g| {
            g.effects.add(EffectInstance::new("speed", 30 * 20, 1));
        });
        // The console's command, then its sync.
        hub.sync_all_effects();
        // The next tick: the guests' effects count down (`tick_guests`), then
        // the hub syncs them (`tick_level`).
        hub.guests.update(0, |g| g.effects.tick());
        hub.sync_all_effects();
        hub.guests.update(0, |g| g.effects.tick());
        hub.sync_all_effects();
        assert_eq!(sent_durations(&rx), vec![600]);
        // Without the sync after the command, the first sync comes after the
        // tick has counted it down: what #7115's transcript shows.
        let (mut late, rx) = hub_with_a_guest();
        late.guests.update(0, |g| {
            g.effects.add(EffectInstance::new("speed", 30 * 20, 1));
        });
        late.guests.update(0, |g| g.effects.tick());
        late.sync_all_effects();
        assert_eq!(sent_durations(&rx), vec![599]);
    }

    /// Minecraft.startAttack sends the attack, then the arm swing, and the next client tick
    /// a turn. Vanilla handles both in the tick they came in; the swing starts the attack
    /// strength again, so a swing left for the next tick took a tick off every hit's
    /// charge: a guest's fists, 5 ticks apart, no longer broke a minecart (#7817).
    #[test]
    fn an_attacks_swing_goes_with_it_past_the_move_behind() {
        let (mut hub, _rx) = hub_with_a_guest();
        {
            let value = 0;
            hub.guests.update(0, |g| g.load_timeout = value);
        }
        let inbox = hub.guests.inbox_mut(0).unwrap();
        inbox.push_back(horizon_client::lan::guest::InboxItem::uncredited(
            S::Attack(play::Attack { id: 7 }),
        ));
        inbox.push_back(horizon_client::lan::guest::InboxItem::uncredited(S::Punch(
            horizon_protocol::packets::Empty,
        )));
        inbox.push_back(horizon_client::lan::guest::InboxItem::uncredited(
            S::MovePlayerRot(play::MovePlayerRot {
                yaw: 90.0,
                pitch: 0.0,
                flags: 1,
            }),
        ));
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let mut next_entity = 2;
        let border = crate::worldborder::WorldBorder::default();
        let mut ctx = TickCtx {
            sea_level: 63,
            world: &mut world,
            entities: &[],
            next_entity: &mut next_entity,
            spawn: [0.5, 64.0, 0.5],
            spawn_angle: [0.0; 2],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &border,
            streaming: None,
        };
        let events = hub.drain_packets(&mut ctx);
        let handled: Vec<&S> = events
            .iter()
            .filter_map(|e| match e {
                HostEvent::Packet(_, p) => Some(&**p),
                _ => None,
            })
            .collect();
        assert!(
            matches!(handled[..], [S::Attack(_), S::Punch(_)]),
            "the game was handed {handled:?}"
        );
        assert!(
            matches!(
                hub.guests[0].inbox.iter().collect::<Vec<_>>()[..],
                [InboxItem::Packet(S::MovePlayerRot(_), _)]
            ),
            "the turn did not wait for the next tick"
        );
    }

    /// Drains guest 0's inbox holding a player action (`action`) and the move
    /// its client sent after it in the same client tick: the packets handed to
    /// the game, and the guest's position and known movement before.
    fn drain_action_then_move(action: i32) -> (Hub, Vec<S>, ([f64; 3], [f64; 3])) {
        let action = S::PlayerAction(play::PlayerAction {
            action,
            pos: [0; 3],
            direction: 0,
            sequence: 0,
        });
        drain_then_move(action, |_| {})
    }

    /// [`drain_action_then_move`] for any packet, with `setup` applied to
    /// the guest first.
    fn drain_then_move(
        first: S,
        setup: impl FnOnce(&mut Guest),
    ) -> (Hub, Vec<S>, ([f64; 3], [f64; 3])) {
        let (mut hub, _rx) = hub_with_a_guest();
        let before = (hub.guests[0].position, hub.guests[0].known_movement);
        hub.guests.update(0, |guest| {
            guest.load_timeout = 0;
            setup(guest);
            let inbox = &mut guest.inbox;
            inbox.push_back(horizon_client::lan::guest::InboxItem::uncredited(first));
            inbox.push_back(horizon_client::lan::guest::InboxItem::uncredited(
                S::MovePlayerPos(play::MovePlayerPos {
                    x: before.0[0] + 0.3,
                    y: before.0[1],
                    z: before.0[2],
                    flags: 1,
                }),
            ));
        });
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let mut next_entity = 2;
        let border = crate::worldborder::WorldBorder::default();
        let mut ctx = TickCtx {
            sea_level: 63,
            world: &mut world,
            entities: &[],
            next_entity: &mut next_entity,
            spawn: [0.5, 64.0, 0.5],
            spawn_angle: [0.0; 2],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &border,
            streaming: None,
        };
        let handled = hub
            .drain_packets(&mut ctx)
            .into_iter()
            .filter_map(|e| match e {
                HostEvent::Packet(_, p) => Some(*p),
                _ => None,
            })
            .collect();
        (hub, handled, before)
    }

    /// #8423: a RELEASE_USE_ITEM shoots from the player's position and known
    /// movement of the last move the server handled; the move the client sent
    /// after it waits for the next tick, as vanilla handles the two in order.
    #[test]
    fn a_release_shoots_before_the_move_behind_it() {
        let (hub, handled, before) = drain_action_then_move(6);
        assert!(
            matches!(handled[..], [S::PlayerAction(_)]),
            "the game was handed {handled:?}"
        );
        assert!(
            matches!(
                hub.guests[0].inbox.iter().collect::<Vec<_>>()[..],
                [InboxItem::Packet(S::MovePlayerPos(_), _)]
            ),
            "the move did not wait for the next tick"
        );
        let g = &hub.guests[0];
        assert_eq!(
            (g.position, g.known_movement),
            before,
            "the shot's position"
        );
    }

    /// A STAB's move runs in the same tick: held for the next one, the jump it
    /// carries came a tick after the Lunge's impulse (lunge_motion).
    #[test]
    fn a_stab_keeps_the_move_behind_it_in_its_tick() {
        let (hub, handled, before) = drain_action_then_move(8);
        assert!(
            matches!(handled[..], [S::PlayerAction(_)]),
            "the game was handed {handled:?}"
        );
        let g = &hub.guests[0];
        assert!(g.inbox.is_empty(), "the move waited for the next tick");
        assert_eq!(g.position[0], before.0[0] + 0.3, "the move was not handled");
    }

    /// STOP_SLEEPING goes to the game, which wakes the guest after the mods
    /// hear it (`player_bed_leave`); the move the client sent after it waits
    /// for the next tick, so it never runs while the guest still sleeps.
    #[test]
    fn a_move_after_leave_bed_waits_for_the_wake() {
        let leave = S::PlayerCommand(play::PlayerCommand {
            id: 0,
            action: 0,
            data: 0,
        });
        let (hub, handled, before) = drain_then_move(leave, |guest| {
            guest.bed = Some([5, 64, 5]);
            guest.sleeping = true;
        });
        assert!(
            matches!(
                handled[..],
                [S::PlayerCommand(play::PlayerCommand { action: 0, .. })]
            ),
            "the game was handed {handled:?}"
        );
        let g = &hub.guests[0];
        assert!(
            matches!(
                g.inbox.iter().collect::<Vec<_>>()[..],
                [InboxItem::Packet(S::MovePlayerPos(_), _)]
            ),
            "the move did not wait for the wake"
        );
        assert_eq!(
            (g.bed, g.sleeping),
            (Some([5, 64, 5]), true),
            "the hub woke the guest"
        );
        assert_eq!(
            (g.position, g.known_movement),
            before,
            "the move ran in bed"
        );
    }

    /// The owner's own shoulder parrots are sent when a slot changes.
    #[test]
    fn the_owner_is_sent_the_shoulder_slots_that_changed() {
        assert!(shoulder_changes([None; 2], [None; 2]).is_empty());
        let landed = shoulder_changes([Some(1), None], [None; 2]);
        assert_eq!(landed.len(), 1);
        assert_eq!((landed[0].0, &landed[0].2), (19, &Value::Id(2)));
        let left = shoulder_changes([None; 2], [Some(1), Some(4)]);
        assert_eq!(left.len(), 2);
        assert_eq!((left[1].0, &left[1].2), (20, &Value::Id(0)));
    }

    /// The counts of what is stuck in a player or a mob are sent when set, and a
    /// count back at zero is sent as the default.
    #[test]
    fn stuck_counts_are_synched_as_entity_data() {
        let mut state = EntityState::default();
        stuck_meta(&mut state.meta, &mut state.defaults, (3, 0));
        assert_eq!(state.meta, vec![(12, serializer::INT, Value::Int(3))]);
        assert_eq!(
            state.defaults,
            vec![
                (12, serializer::INT, Value::Int(0)),
                (13, serializer::INT, Value::Int(0))
            ]
        );
    }

    /// Joined states are used in the server tick after the one that noted
    /// them, never later.
    #[test]
    fn joined_states_last_only_into_the_next_tick() {
        let joined = vec![EntityState {
            id: 7,
            ..EntityState::default()
        }];
        assert_eq!(noted_last_tick(Some((5, joined.clone())), 6), joined);
        assert!(noted_last_tick(Some((5, joined.clone())), 5).is_empty());
        assert!(noted_last_tick(Some((5, joined)), 9).is_empty());
        assert!(noted_last_tick(None, 6).is_empty());
    }

    /// `getMaximumFlyingTicks`: 80 ticks at normal gravity, ceil(80 * 0.08 /
    /// gravity) where it is weaker, and never a kick without gravity.
    #[test]
    fn maximum_flying_ticks_follow_gravity() {
        assert_eq!(maximum_flying_ticks(0.08), 80);
        assert_eq!(maximum_flying_ticks(0.16), 80);
        assert_eq!(maximum_flying_ticks(0.04), 160);
        assert_eq!(maximum_flying_ticks(0.03), 214);
        assert_eq!(maximum_flying_ticks(0.0), i32::MAX);
        assert_eq!(maximum_flying_ticks(1.0E-6), i32::MAX);
    }

    fn textures(value: &str) -> horizon_protocol::packets::login::Property {
        horizon_protocol::packets::login::Property {
            name: "textures".into(),
            value: value.into(),
            signature: Some("c2lnbmVk".into()),
        }
    }

    /// The owner is listed with the account's skin once the fetch answered,
    /// with none before (no wait) or when it failed; other players keep the
    /// properties they logged in with, and none of the owner's.
    #[test]
    fn the_owner_is_listed_with_the_accounts_skin_once_fetched() {
        let (answer, fetched) = std::sync::mpsc::channel();
        let profile = super::super::OwnerProfile::new(move || fetched.recv().unwrap_or_default());
        let start = std::time::Instant::now();
        assert!(listed_properties(&[], true, Some(&profile)).is_empty());
        assert!(start.elapsed() < std::time::Duration::from_millis(500));
        answer.send(vec![textures("owner")]).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while listed_properties(&[], true, Some(&profile)).is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "the fetch never landed"
            );
            std::thread::yield_now();
        }
        assert_eq!(
            listed_properties(&[], true, Some(&profile)),
            vec![textures("owner")]
        );
        // A guest: its own properties (online mode), or none (offline mode).
        assert_eq!(
            listed_properties(&[textures("guest")], false, Some(&profile)),
            vec![textures("guest")]
        );
        assert!(listed_properties(&[], false, Some(&profile)).is_empty());
        // No signed-in account (no fetch): as before.
        assert!(listed_properties(&[], true, None).is_empty());
        let failed = super::super::OwnerProfile::new(Vec::new);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while failed.properties.get().is_none() {
            let _ = failed.properties();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(listed_properties(&[], true, Some(&failed)).is_empty());
    }

    /// `requeue` as it was, scanning the queue for each chunk of the view.
    fn requeue_scan(
        queue: &mut Vec<(i32, i32)>,
        sent: &FxHashSet<(i32, i32)>,
        (cx, cz): (i32, i32),
        r: i32,
    ) {
        queue.retain(|&(x, z)| tracked(cx, cz, r, x, z));
        for dz in -r - 2..=r + 2 {
            for dx in -r - 2..=r + 2 {
                let key = (cx + dx, cz + dz);
                if tracked(cx, cz, r, key.0, key.1) && !sent.contains(&key) && !queue.contains(&key)
                {
                    queue.push(key);
                }
            }
        }
        queue.sort_by_key(|&(x, z)| i64::from(x - cx).pow(2) + i64::from(z - cz).pow(2));
    }

    /// A player walking a loop, sending a few chunks a tick: the queue
    /// after every move is the one the scan built.
    #[test]
    #[ignore = "daily"]
    fn requeue_matches_the_scan() {
        for r in [2, 5, 10, 12, 32] {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            let mut sent = FxHashSet::default();
            for step in 0..60i32 {
                let center = ((step % 17) - 8, (step * 3 % 11) - 5);
                requeue(&mut a, &sent, center, r);
                requeue_scan(&mut b, &sent, center, r);
                assert_eq!(a, b, "radius {r}, step {step}");
                for key in a.drain(..a.len().min(7)) {
                    sent.insert(key);
                }
                b.drain(..b.len().min(7));
                sent.retain(|&(x, z)| tracked(center.0, center.1, r, x, z));
            }
        }
    }

    /// The far ring's queue is what the view's leaves of the chunks tracked at the reach, closest
    /// first, with the sent ones out of both.
    #[test]
    fn the_far_ring_queues_what_the_view_leaves() {
        let mut sent = FxHashSet::default();
        let (mut near, mut far) = (Vec::new(), Vec::new());
        for step in 0..12 {
            let center = (step, -step / 2);
            requeue(&mut near, &sent, center, 10);
            requeue_ring(&mut far, &sent, center, 10, 24);
            let mut all = Vec::new();
            requeue(&mut all, &sent, center, 24);
            assert!(far.iter().all(|key| !near.contains(key)));
            let mut both = [near.clone(), far.clone()].concat();
            both.sort_unstable();
            all.sort_unstable();
            assert_eq!(both, all);
            let distance = |&(x, z): &(i32, i32)| {
                i64::from(x - center.0).pow(2) + i64::from(z - center.1).pow(2)
            };
            assert!(far.windows(2).all(|w| distance(&w[0]) <= distance(&w[1])));
            // A few of each sent.
            for key in near
                .drain(..near.len().min(40))
                .chain(far.drain(..far.len().min(90)))
            {
                sent.insert(key);
            }
        }
        // Without a ring, nothing.
        requeue_ring(&mut far, &sent, (0, 0), 10, 10);
        assert!(far.is_empty());
    }

    /// `MinecraftServer.onGameRuleChanged`: setting `limited_crafting` updates the hub's state
    /// and broadcasts `ClientboundGameEventPacket` (event 12) with 1.0 (on) or 0.0 (off) to guests.
    #[test]
    fn set_limited_crafting_broadcasts_game_event() {
        let (mut hub, rx) = hub_with_a_guest();
        assert!(!hub.limited_crafting);

        // Setting to true broadcasts game event 12 with param 1.0
        hub.set_limited_crafting(true);
        assert!(hub.limited_crafting);

        let out = rx.try_recv().expect("packet should be sent to guest");
        let body = match out {
            crate::lan::Out::Packet(b) => b,
            _ => panic!("expected Out::Packet"),
        };
        let mut reader = horizon_protocol::buf::Reader::new(&body);
        let id = reader.varint().unwrap();
        let packet = play::Clientbound::decode(id, &body[reader.position()..]).unwrap();
        match packet {
            play::Clientbound::GameEvent(p) => {
                assert_eq!(p.event, game_event::LIMITED_CRAFTING);
                assert_eq!(p.param, 1.0);
            }
            other => panic!("expected GameEvent, got {other:?}"),
        }

        // Setting to the same state should not broadcast another packet
        hub.set_limited_crafting(true);
        assert!(rx.try_recv().is_err());

        // Setting to false broadcasts game event 12 with param 0.0
        hub.set_limited_crafting(false);
        assert!(!hub.limited_crafting);

        let out = rx.try_recv().expect("packet should be sent to guest");
        let body = match out {
            crate::lan::Out::Packet(b) => b,
            _ => panic!("expected Out::Packet"),
        };
        let mut reader = horizon_protocol::buf::Reader::new(&body);
        let id = reader.varint().unwrap();
        let packet = play::Clientbound::decode(id, &body[reader.position()..]).unwrap();
        match packet {
            play::Clientbound::GameEvent(p) => {
                assert_eq!(p.event, game_event::LIMITED_CRAFTING);
                assert_eq!(p.param, 0.0);
            }
            other => panic!("expected GameEvent, got {other:?}"),
        }
    }

    /// cargo test --release -p horizon-client --lib lan::hub::tests::requeue_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn requeue_cost() {
        for r in [10, 16, 32] {
            for (name, f) in [
                ("scan", requeue_scan as fn(&mut _, &_, _, _)),
                ("set", requeue as fn(&mut _, &_, _, _)),
            ] {
                let sent = FxHashSet::default();
                let mut queue = Vec::new();
                let start = std::time::Instant::now();
                let moves = 200;
                for step in 0..moves {
                    // Nothing sent yet (an explorer waiting on generation).
                    f(&mut queue, &sent, (step, 0), r);
                }
                let per = start.elapsed().as_secs_f64() * 1e6 / f64::from(moves);
                println!(
                    "view {r}: {name} {per:.1} us per chunk crossed, {} queued",
                    queue.len()
                );
            }
        }
    }

    /// Listens to `chunk_watch`, so the hub records the watch trio.
    struct Watcher;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Watcher {
        fn chunk_watch(&self, _: &mut dyn mod_api::ServerHost, _: &mut mod_api::ChunkWatchEvent) {}
    }

    /// The hub's chunk step for its one guest, as many batches as the level holds.
    fn send_chunks(hub: &mut Hub, world: &mut World) {
        let mut next_entity = 100;
        let border = crate::worldborder::WorldBorder::default();
        let ctx = TickCtx {
            sea_level: 63,
            world,
            entities: &[],
            next_entity: &mut next_entity,
            spawn: [0.5, 64.0, 0.5],
            spawn_angle: [0.0; 2],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &border,
            streaming: None,
        };
        for _ in 0..3 {
            hub.ticks += 10;
            {
                let value = 0;
                hub.guests.update(0, |g| g.unacknowledged = value);
            }
            hub.update_chunks(&ctx);
        }
    }

    /// The recorded `(kind, dimension, chunk)`s since the last call, sorted
    /// within each kind's run.
    fn watches(
        hub: &mut Hub,
    ) -> Vec<(super::super::chunk_watch::ChunkWatchKind, String, [i32; 2])> {
        hub.chunk_watches
            .take()
            .into_iter()
            .flat_map(|run| {
                run.watches.into_iter().map(move |w| {
                    assert_eq!((w.uuid, w.entity_id), (1, 1));
                    (w.kind, run.dimension.clone(), w.chunk)
                })
            })
            .collect()
    }

    /// With no mod listening, a registered passage (`record_for_passages`) makes the hub
    /// record the chunks it sends from then on, for the passages the game sends with them.
    #[test]
    fn a_passage_makes_the_hub_record_the_chunks_it_sends() {
        use super::super::chunk_watch::ChunkWatchKind::{Sent, Watch};
        let (mut hub, _rx) = hub_with_a_guest();
        let mut world = floor_at(1);
        {
            let value = 2;
            hub.guests.update(0, |g| g.view_distance = value);
        }
        hub.chunk_watches.record_for_passages();
        send_chunks(&mut hub, &mut world);
        let mut got = watches(&mut hub);
        got.sort_by_key(|(k, _, c)| (*k as u8, *c));
        let overworld = "minecraft:overworld".to_string();
        let four = [[-1, -1], [-1, 0], [0, -1], [0, 0]];
        let expected: Vec<_> = [Watch, Sent]
            .into_iter()
            .flat_map(|kind| four.map(|c| (kind, overworld.clone(), c)))
            .collect();
        assert_eq!(got, expected);
    }

    /// `ChunkWatchEvent`: each near chunk sent is watched then sent, once;
    /// leaving the view, changing level and disconnecting unwatch it, in the
    /// level it was watched in. Nothing is recorded while no mod listens.
    #[test]
    fn the_watch_trio_follows_each_sent_chunk() {
        use super::super::chunk_watch::ChunkWatchKind::{Sent, Unwatch, Watch};
        let overworld = || "minecraft:overworld".to_string();
        let four = [[-1, -1], [-1, 0], [0, -1], [0, 0]];
        let run = |kind, dimension: &str| -> Vec<_> {
            four.iter()
                .map(|&c| (kind, dimension.to_string(), c))
                .collect()
        };
        let sorted = |mut v: Vec<_>| {
            v.sort_by_key(|(k, _, c): &(_, String, [i32; 2])| (*k as u8, *c));
            v
        };

        // No listener: chunks go out, nothing is recorded or kept.
        let (mut hub, _rx) = hub_with_a_guest();
        let mut world = floor_at(1);
        {
            let value = 2;
            hub.guests.update(0, |g| g.view_distance = value);
        }
        send_chunks(&mut hub, &mut world);
        assert_eq!(hub.guests[0].sent_chunks.len(), 4);
        assert!(hub.chunk_watches.is_empty() && hub.guests[0].watched.is_empty());

        let mods = vec![mod_api::ModEntry::new("watch", Box::new(Watcher))].into();
        mod_platform::testing::with_mods(mods, || {
            let (mut hub, _rx) = hub_with_a_guest();
            {
                let value = 2;
                hub.guests.update(0, |g| g.view_distance = value);
            }
            send_chunks(&mut hub, &mut world);
            let got = watches(&mut hub);
            // The batch's watches come before its sends.
            assert_eq!(sorted(got[..4].to_vec()), run(Watch, &overworld()));
            assert_eq!(sorted(got[4..].to_vec()), run(Sent, &overworld()));
            // A chunk crossing that leaves the four behind.
            {
                let value = [400.5, 64.0, 0.5];
                hub.guests.update(0, |g| g.position = value);
            }
            send_chunks(&mut hub, &mut world);
            assert_eq!(sorted(watches(&mut hub)), run(Unwatch, &overworld()));
            assert!(hub.guests[0].watched.is_empty());
            // Back, then through a portal: unwatched in the level left, in
            // vanilla's view order.
            {
                let value = [0.5, 64.0, 0.5];
                hub.guests.update(0, |g| g.position = value);
            }
            send_chunks(&mut hub, &mut world);
            assert_eq!(watches(&mut hub).len(), 8);
            hub.set_dimension("minecraft:the_nether");
            let mut next_entity = 100;
            let border = crate::worldborder::WorldBorder::default();
            let ctx = TickCtx {
                sea_level: 63,
                world: &mut world,
                entities: &[],
                next_entity: &mut next_entity,
                spawn: [0.5, 64.0, 0.5],
                spawn_angle: [0.0; 2],
                difficulty: 2,
                difficulty_locked: false,
                clock_rate: 1.0,
                tick_rate: 20.0,
                tick_frozen: false,
                tick_steps: Default::default(),
                collision: None,
                border: &border,
                streaming: None,
            };
            hub.change_level(0, [0.5, 64.0, 0.5], 0.0, None, true, 0, &ctx);
            // The nether's chunks: the level left and the new one are a run
            // each, which names its level once.
            send_chunks(&mut hub, &mut world);
            let runs = hub.chunk_watches.take();
            let levels: Vec<_> = runs
                .iter()
                .map(|r| (r.dimension.as_str(), r.watches.len()))
                .collect();
            assert_eq!(
                levels,
                [("minecraft:overworld", 4), ("minecraft:the_nether", 8)]
            );
            let left: Vec<_> = runs[0]
                .watches
                .iter()
                .map(|w| (w.kind, overworld(), w.chunk))
                .collect();
            assert_eq!(left, run(Unwatch, &overworld()));
            // Then a disconnect.
            let _ = hub.remove_guest(0, "bye");
            assert_eq!(watches(&mut hub), run(Unwatch, "minecraft:the_nether"));
        });
    }

    /// A single-player memory connection sends every ready view column before
    /// `PlayerLoaded`. After that, on a respawn, and for a socket guest, the
    /// batch is `chunks_per_tick` again (64). An unlit column stays out, and
    /// the gates that send nothing still do.
    #[test]
    fn memory_connection_first_view_sends_every_ready_column() {
        let (mut hub, _rx) = hub_with_a_guest();
        hub.ticks = 3;
        {
            hub.guests.update(0, |g| {
                g.chunks_per_tick = 64.0;
                g.batch_quota = 0.0;
                g.address = crate::lan::conn::MEMORY_ADDRESS;
                g.loaded = false;
            });
        }
        // The unlit column first, then 80 lit ones (`floor_at`'s empty column, lit).
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let mut dark = crate::chunk::Chunk::empty(100, 100);
        dark.lit = false;
        world.chunks.insert((100, 100), std::sync::Arc::new(dark));
        hub.guests.update(0, |g| {
            g.chunk_queue.push((100, 100));
        });
        for i in 0..80 {
            let (x, z) = (i % 16, i / 16);
            let mut chunk = crate::chunk::Chunk::empty(x, z);
            chunk.lit = true;
            world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            hub.guests.update(0, |g| {
                g.chunk_queue.push((x, z));
            });
        }

        let batch = hub
            .next_batch(0, &world, None)
            .expect("the first view sends");
        assert_eq!(
            batch.near.len(),
            80,
            "every ready column leaves in one batch"
        );
        assert!(
            batch.near.iter().all(|(_, c)| c.lit),
            "an unlit column is not ready"
        );
        assert!(batch.far.is_empty());
        assert!(hub.guests[0].batch_quota >= 80.0);

        hub.guests.update(0, |g| {
            g.loaded = true;
        });
        let loaded = hub
            .next_batch(0, &world, None)
            .expect("a loaded client still receives chunks");
        assert_eq!(loaded.near.len(), 64, "PlayerLoaded puts the cap back");

        hub.guests.update(0, |g| {
            g.address = "127.0.0.1:25565".parse().unwrap();
        });
        hub.guests.update(0, |g| {
            g.loaded = false;
        });
        hub.guests.update(0, |g| {
            g.batch_quota = 0.0;
        });
        let socket = hub
            .next_batch(0, &world, None)
            .expect("a socket guest still receives chunks");
        assert_eq!(socket.near.len(), 64, "a socket guest keeps the quota");

        hub.guests.update(0, |g| {
            g.address = crate::lan::conn::MEMORY_ADDRESS;
        });
        hub.guests.update(0, |g| {
            g.loaded = true;
        });
        hub.guests.update(0, |g| {
            g.loading = true;
        });
        hub.guests.update(0, |g| {
            g.batch_quota = 0.0;
        });
        let respawn = hub
            .next_batch(0, &world, None)
            .expect("a respawned player still receives chunks");
        assert_eq!(
            respawn.near.len(),
            64,
            "a respawn stays on the vanilla pace"
        );

        // Seventy lit columns, the unlit one still queued: all seventy, in one batch.
        hub.guests.update(0, |g| {
            g.loaded = false;
        });
        hub.guests.update(0, |g| {
            g.batch_quota = 0.0;
        });
        hub.guests.update(0, |g| {
            g.chunk_queue.truncate(71);
        });
        let seventy = hub
            .next_batch(0, &world, None)
            .expect("seventy ready columns send");
        assert_eq!(seventy.near.len(), 70);
        assert!(seventy.near.iter().all(|(_, c)| c.lit));

        hub.guests.update(0, |g| {
            g.unacknowledged = g.max_unacknowledged;
        });
        assert!(
            hub.next_batch(0, &world, None).is_none(),
            "too many batches still waiting sends nothing"
        );

        hub.guests.update(0, |g| {
            g.unacknowledged = 0;
        });
        hub.ticks = hub.guests[0].placed_at + 1;
        assert!(
            hub.next_batch(0, &world, None).is_none(),
            "the tick after placement still waits"
        );

        hub.ticks = 3;
        hub.guests.update(0, |g| {
            g.chunk_queue.clear();
        });
        hub.guests.update(0, |g| {
            g.far_queue.clear();
        });
        assert!(
            hub.next_batch(0, &world, None).is_none(),
            "an empty queue sends nothing"
        );
    }
    /// `ServerGamePacketListenerImpl.handleAcceptTeleportPacket`: an accept whose id is
    /// the last teleport's (`awaitingTeleport`) while none is awaited any more
    /// (`awaitingPositionFromClient` is null) disconnects with
    /// `multiplayer.disconnect.invalid_player_movement`; an accept of any other id is
    /// ignored, and the first accept of the awaited one moves the player as before.
    #[test]
    fn a_second_accept_of_the_same_teleport_disconnects() {
        let (mut hub, _rx) = hub_with_a_guest();
        let mut world = floor_at(0);
        let accept = |id: i32| {
            S::AcceptTeleportation(play::AcceptTeleportation {
                id,
                x: 5.0,
                y: 70.0,
                z: 5.0,
                yaw: 0.0,
                pitch: 0.0,
            })
        };
        hub.teleport(0, Some([5.0, 70.0, 5.0]));
        let id = hub.guests[0].teleport_counter;
        assert!(hub.guests[0].awaiting_teleport.is_some());
        // An id nobody sent is ignored while one is awaited.
        tick_packet(&mut hub, &mut world, accept(id + 1));
        assert!(hub.guests[0].awaiting_teleport.is_some());
        assert!(hub.guests[0].left_reason.is_empty());
        // The awaited one is accepted once.
        tick_packet(&mut hub, &mut world, accept(id));
        assert!(hub.guests[0].awaiting_teleport.is_none());
        assert!(hub.guests[0].left_reason.is_empty());
        // Another id with nothing awaited is ignored.
        tick_packet(&mut hub, &mut world, accept(id + 1));
        assert!(hub.guests[0].left_reason.is_empty());
        // The same id again, with nothing awaited: disconnected.
        tick_packet(&mut hub, &mut world, accept(id));
        assert!(!hub.guests[0].left_reason.is_empty());
    }
}
