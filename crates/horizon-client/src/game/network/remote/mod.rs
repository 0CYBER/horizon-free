//! Multiplayer: the game driven by a server connection instead of the local
//! simulation. Mirrors vanilla's ClientPacketListener (chunks, entities,
//! inventory, health, time, weather, chat, respawn and dimension changes),
//! LocalPlayer.sendPosition (movement packets) and MultiPlayerGameMode
//! (block breaking with sequence numbers, use-item, attacks, container
//! clicks). The player still moves with the local physics, as the vanilla
//! client predicts its own movement; the server stays authoritative for
//! everything else.
mod selector_metadata;
use selector_metadata::hurt_sound_for;
mod attack_target;
mod break_prediction;
mod fall_sound;
mod interaction_actions;
mod interaction_prediction;
mod level_format;
mod raw_ingress;
mod recipe_properties;
mod use_item;

mod block_entity_data;
#[cfg(test)]
mod block_entity_data_tests;
use super::raw_backlog::Received;
use super::remote_chunks::{ChunkDecoder, ChunkFormat, Next};
use super::render_tick::{mob_tracking_body, player_tracking_body};
use super::{Dim, Game, Mining, Screen, level_sounds::LevelSound};
use crate::game::api_values::TryIntoApi;
use horizon_client::actions::Action;
use horizon_client::first_person::{Hand, SwingAnimation};
use horizon_client::input_queue::InputQueue;
use horizon_client::{
    Result,
    blocks::Fluid,
    chunk::Chunk,
    containers::{BlockEntity, Furnace, FurnaceKind},
    entities::{Experience, ExperienceOrb, ItemEntity},
    hud::HudBatch,
    items::ItemStack,
    lan::data::{BIOME_REGISTRY, client_biomes},
    menu::Input,
    mobs::{Mob, MobType, Who},
    motion::{MotionWorld, PlayerMotion},
    net::{Event, Session},
    particles::TrackingTarget,
    random_source,
    save::SaveDir,
    screen_effects,
    sprint::Keys,
    streaming::{ChunkGenerator, Proto, Streaming},
    tab_list::SPECTATOR,
    text::Text,
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_protocol::{
    entity_data::{ITEM_ENTITY_ITEM, Value},
    item::{Hashed, Stack},
    packets::{
        Empty,
        play::{self, Clientbound as C, Serverbound as S},
    },
    registry::{Builtin, DimensionType, Dynamic, SyncedNames, ceil_log2},
    rsa::RsaPublicKey,
    secure_chat::{self, Signature},
    text::{Language, plain},
};
use mod_api::InteractionKey;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use winit::keyboard::KeyCode;
use {
    horizon_core::nbt::Tag,
    horizon_mod_runtime::attachments::{Holder, ResidentAttachments},
    horizon_world::{
        level::InteractionResult,
        registry::BlockStateId,
        stepped_interpolation::{Path, PosRot, Start, SteppedInterpolation},
    },
};

mod abilities;
mod chat_received;
use chat_received::QueuedChat;
pub(in crate::game) mod client_level_tick;
mod entity_section_steps;
mod item_use;
mod queued_keys;
mod remote_attributes;
mod tab_entries;
mod tables;

pub(super) use super::villager_client::builtin_registries;

mod chat_queue;

/// `ChatType`'s two decorations: how a message shows in the chat, and what the narrator says of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decoration {
    Chat,
    Narration,
}

/// `ChatTypeDecoration.decorate` of a chat type's `chat` or `narration`
/// entry: its translation, with the sender, the target or the message as its
/// parameters, in its style. A chat type the registry lacks decorates as
/// `minecraft:chat` (`chat.type.text`, narrated `chat.type.text.narrate`).
fn chat_type_decorate(
    chat_type: Option<&Tag>,
    decoration: Decoration,
    bound: &play::ChatTypeBound,
    content: &serde_json::Value,
) -> serde_json::Value {
    use horizon_client::scoreboard::tag_to_json;
    let (field, default_key) = match decoration {
        Decoration::Chat => ("chat", "chat.type.text"),
        Decoration::Narration => ("narration", "chat.type.text.narrate"),
    };
    let decoration = chat_type.and_then(|t| t.get(field));
    let key = match decoration.and_then(|d| d.get("translation_key")) {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => default_key.into(),
    };
    let parameter = |name: &str| match name {
        "sender" => tag_to_json(&bound.name),
        "target" => bound
            .target
            .as_ref()
            .map_or_else(|| serde_json::Value::from(""), tag_to_json),
        _ => content.clone(),
    };
    let with: Vec<serde_json::Value> = match decoration.and_then(|d| d.get("parameters")) {
        Some(Tag::List(list)) => list
            .iter()
            .map(|p| match p {
                Tag::String(s) => parameter(&s.to_string_lossy()),
                _ => serde_json::Value::from(""),
            })
            .collect(),
        _ => vec![parameter("sender"), parameter("content")],
    };
    let mut component = match decoration.and_then(|d| d.get("style")) {
        Some(style @ Tag::Compound(_)) => tag_to_json(style),
        _ => serde_json::json!({}),
    };
    component["translate"] = key.into();
    component["with"] = with.into();
    component
}

/// A text component the server sent, for the HUD.
pub(super) fn remote_text(tag: &Tag) -> horizon_client::text::Text {
    use horizon_client::scoreboard::{comp_text, tag_to_json};
    comp_text(&tag_to_json(tag))
}

/// `Player.getVoicePitch`: a constant 1.0
/// (`javap -p -c -constants net.minecraft.world.entity.player.Player`:
/// `0: fconst_1  1: freturn`), so a player's own hurt and death sounds are
/// neither raised for a baby nor jittered - `LivingEntity.getVoicePitch`, which
/// the mobs port, does both. It takes no random, so it draws nothing from the
/// level random either.
pub(super) fn player_voice_pitch() -> f32 {
    1.0
}

/// The live entity a bound sound names; absence means that object was removed.
fn entity_sound_state(
    id: i32,
    player: Option<(i32, [f64; 3], bool)>,
    mob: Option<horizon_client::sound_engine::SoundEntityState>,
    tracked: &HashMap<i32, Tracked>,
    dragon: Option<(i32, [f64; 3])>,
) -> Option<horizon_client::sound_engine::SoundEntityState> {
    use horizon_client::sound_engine::SoundEntityState;
    if let Some((player_id, position, silent)) = player
        && player_id == id
    {
        return Some(SoundEntityState {
            position,
            silent,
            removed: false,
        });
    }
    let mut state = mob.or_else(|| {
        tracked.get(&id).map(|t| SoundEntityState {
            position: t.position,
            silent: t.silent,
            removed: false,
        })
    })?;
    // The dragon's Mob is metadata storage; only Dragon follows its movement.
    if let Some((dragon_id, position)) = dragon
        && dragon_id == id
    {
        state.position = position;
    }
    Some(state)
}

/// `ClientboundOpenScreenPacket.title` as a container screen draws it: the
/// component's styles, and the `§` codes a literal holds (`StringDecomposer`).
fn menu_title(tag: &Tag) -> horizon_client::text::Text {
    remote_text(tag).resolve_legacy()
}

/// `ChatTrustLevel`; a modified message keeps the signed content it was changed from.
pub(super) enum ChatTrust {
    Secure,
    Modified(String),
    NotSecure,
    /// `GuiMessageTag.chatError`: the line `ChatListener.handleChatMessageError` adds.
    Error,
}
impl ChatTrust {
    /// `ChatTrustLevel.createTag`.
    fn tag(&self) -> Option<horizon_client::chat::Tag> {
        match self {
            Self::Secure => None,
            Self::Modified(original) => Some(horizon_client::chat::Tag::modified(original)),
            Self::NotSecure => Some(horizon_client::chat::Tag::not_secure()),
            Self::Error => Some(horizon_client::chat::Tag::chat_error()),
        }
    }
}

/// Streaming needs a generator; a server world never generates.
struct NoTerrain;
impl ChunkGenerator for NoTerrain {
    type Worker = ();
    fn worker(&self) {}
    fn proto(&self, _: &mut (), _: i32, _: i32) -> Proto {
        Box::new(())
    }
    fn finish(&self, _: Proto) -> Chunk {
        Chunk::empty(0, 0)
    }
}

/// A server entity the client tracks (position as last sent).
pub(super) struct Tracked {
    kind: String,
    position: [f64; 3],
    /// `Entity.getPositionCodec`'s base: the position the server last
    /// reported (AddEntity, MoveEntity, EntityPositionSync), which a move
    /// packet's delta counts from. A TeleportEntity moves the entity but not
    /// this base, so the server's next delta still lands where it means.
    codec_base: [f64; 3],
    silent: bool,
    /// `Entity.vehicle`: the entity it rides (`SetPassengers`), whatever either one's type.
    pub(in crate::game) vehicle: Option<u32>,
}

/// `Entity.startRiding` / `removePassenger` for a `SetPassengers` packet: the
/// listed entities ride `vehicle`, the ones that rode it and are no longer
/// listed got off. Every tracked entity counts, whatever its type, so a
/// server's item display or marker on a player hides that player's name tag
/// as vanilla's `isVehicle` does (#5666).
fn seat_passengers(entities: &mut HashMap<i32, Tracked>, vehicle: u32, passengers: &[i32]) {
    for (id, t) in entities.iter_mut() {
        if passengers.contains(id) {
            t.vehicle = Some(vehicle);
        } else if t.vehicle == Some(vehicle) {
            t.vehicle = None;
        }
    }
}
/// Smooth an entity toward a new position over three ticks (InterpolationHandler).
#[derive(Clone, Copy)]
/// `LinearInterpolationHandler`'s target: a position and rotation (yRot,
/// xRot) reached in `steps` ticks.
pub(super) struct Lerp {
    target: [f64; 3],
    rotation: Option<(f32, f32)>,
    steps: u32,
}

/// `NewMinecartBehavior.MinecartStep`, including the server's rail pitch.
#[derive(Clone, Copy)]
struct CartStep {
    position: Vec3,
    movement: Vec3,
    yaw: f32,
    pitch: f32,
    weight: f32,
}

/// The client's state of a minecart on `NewMinecartBehavior`: the steps the
/// server sent (`lerpSteps`), the ones being played over 3 ticks
/// (`currentLerpSteps`) and where the cart was when they began (`oldLerp`).
pub(super) struct CartLerp {
    pending: Vec<CartStep>,
    current: Vec<CartStep>,
    total_weight: f64,
    delay: i32,
    old: CartStep,
}

impl CartLerp {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
            current: Vec::new(),
            total_weight: 0.0,
            delay: 0,
            old: CartStep {
                position: Vec3::ZERO,
                movement: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.0,
                weight: 0.0,
            },
        }
    }

    /// `NewMinecartBehavior.getCartLerpPosition`, `getCartLerpMovements` and
    /// `getCartLerpYRot` / `getCartLerpXRot` at `partial`: the step the weights
    /// put this moment in, and how far along it from the one before.
    fn at(&self, partial: f32) -> (Vec3, Vec3, f32, f32) {
        let alpha = ((3 - self.delay) as f32 + partial) / 3.0;
        let mut count_up = 0.0f32;
        let mut in_step = 1.0f32;
        let mut found = None;
        for (i, s) in self.current.iter().enumerate() {
            if s.weight <= 0.0 {
                continue;
            }
            count_up += s.weight;
            if f64::from(count_up) >= self.total_weight * f64::from(alpha) {
                let before = count_up - s.weight;
                in_step = ((f64::from(alpha) * self.total_weight - f64::from(before))
                    / f64::from(s.weight)) as f32;
                found = Some(i);
                break;
            }
        }
        let index = found.unwrap_or(self.current.len() - 1);
        let current = self.current[index];
        let previous = if index > 0 {
            self.current[index - 1]
        } else {
            self.old
        };
        let t = f64::from(in_step);
        let lerp = |a: Vec3, b: Vec3| {
            Vec3::new(
                a.x + t * (b.x - a.x),
                a.y + t * (b.y - a.y),
                a.z + t * (b.z - a.z),
            )
        };
        (
            lerp(previous.position, current.position),
            lerp(previous.movement, current.movement),
            previous.yaw + in_step * wrap_degrees(current.yaw - previous.yaw),
            previous.pitch + in_step * wrap_degrees(current.pitch - previous.pitch),
        )
    }

    /// `NewMinecartBehavior.lerpClientPositionAndRotation`: once the steps
    /// being played are used up, the queued ones become the current ones;
    /// the cart then sits where the steps put it at the end of this tick.
    /// False when it has no steps to play.
    fn tick(&mut self, c: &mut super::minecarts::Minecart) -> bool {
        self.delay -= 1;
        if self.delay <= 0 {
            self.old = CartStep {
                position: c.position,
                movement: c.velocity,
                yaw: c.yaw,
                pitch: c.pitch,
                weight: 0.0,
            };
            self.current.clear();
            if !self.pending.is_empty() {
                self.current.append(&mut self.pending);
                self.total_weight = self.current.iter().map(|s| f64::from(s.weight)).sum();
                self.delay = if self.total_weight == 0.0 { 0 } else { 3 };
            }
        }
        if self.current.is_empty() {
            return false;
        }
        let (position, movement, yaw, pitch) = self.at(1.0);
        c.position = position;
        c.velocity = movement;
        c.yaw = yaw;
        c.pitch = pitch;
        true
    }
}

/// `SteppedInterpolationHandler.doInterpolate`'s `setPos` and `setRot` on a
/// mob: its yaw kept continuous (the same turn as vanilla's, without the
/// wrap that would spin it in the frame between ticks), its pitch clamped.
fn place_interpolated(m: &mut Mob, at: PosRot) {
    m.position = Vec3::new(at.position[0], at.position[1], at.position[2]);
    m.yaw += wrap_degrees(at.yaw - m.yaw);
    m.head_pitch = (at.pitch % 360.0).clamp(-90.0, 90.0);
}

/// The packet a living entity's move came in.
#[derive(Clone, Copy, PartialEq)]
enum MovePacket {
    /// `handleMoveEntity`: always interpolated.
    Move,
    /// `handleEntityPositionSync`.
    PositionSync,
    /// `handleTeleportEntity` (`setValuesFromPositionPacket`).
    Teleport,
}

impl MovePacket {
    /// `tooBigToInterpolate`: a move to `to` of more than 64 blocks snaps.
    /// A position sync measures it from where the entity is, a teleport from
    /// where it is headed (`PositionMoveRotation.of`'s
    /// `getClientPositionAndRotation`).
    fn snaps(self, handler: &SteppedInterpolation, entity: PosRot, to: [f64; 3]) -> bool {
        let from = match self {
            MovePacket::Move => return false,
            MovePacket::PositionSync => entity.position,
            MovePacket::Teleport => handler.heading(entity).position,
        };
        let d = [0, 1, 2].map(|i| to[i] - from[i]);
        d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 4096.0
    }
}

/// A move `packet` to `to` for a living entity at `entity`, through its
/// handler (`interpolateTo`): where it is put at once, if anywhere. A move
/// too big to interpolate is a `snapTo`; vanilla leaves the handler running
/// and its next step shifts what is left by the jump
/// (`adjustInterpolationTargetFromDeltas`), which this handler doesn't port,
/// so the steps are dropped instead. That is the same for an entity that
/// wasn't moving, as a teleported one usually isn't.
fn stepped_move(
    handler: &mut SteppedInterpolation,
    packet: MovePacket,
    entity: PosRot,
    to: [f64; 3],
    path: Option<Path>,
    rotation: Option<(f32, f32)>,
) -> Option<PosRot> {
    if packet.snaps(handler, entity, to) {
        handler.cancel();
        let (yaw, pitch) = rotation.unwrap_or((entity.yaw, entity.pitch));
        return Some(PosRot {
            position: to,
            yaw,
            pitch,
        });
    }
    match handler.interpolate_to(entity, path, rotation) {
        Start::Snap(at) => Some(at),
        Start::Interpolating | Start::Unchanged => None,
    }
}

/// `Minecraft.services().profileKeySignatureValidator()`: the services' profile-key keys, which a
/// player's chat key must be signed by. Asked for once per connection to a server, off the game
/// thread.
pub(super) enum ServicesKeys {
    Pending,
    Ready(Vec<RsaPublicKey>),
    /// `canValidateProfileKeys()` is false: none could be had.
    Missing,
}
/// The signed-in account's `/minecraft/profile` fetch after joining.
enum OwnProfile {
    Idle,
    Fetching(std::sync::mpsc::Receiver<Option<horizon_client::skins::Textures>>),
    Done(Option<horizon_client::skins::Textures>),
}

pub(super) struct Remote {
    pub(super) professions: Arc<horizon_client::mobs::villager::ResolvedProfessions>,
    pub session: Session,
    pub brand: String,
    pub(super) recipe_viewer: crate::game::recipe_book::ViewerChannels,
    /// This connection's configured feature flags, carried through reconfiguration.
    pub(super) features: std::sync::Arc<[String]>,
    /// The server announced it runs mods' quiet commands.
    pub quiet_commands: bool,
    /// The server advertised private custom recipe-book synchronization.
    pub custom_books: bool,
    /// The synced attachments the server sent, by holder.
    pub attachments: HashMap<Holder, ResidentAttachments>,
    /// The server samples far terrain for us, this far at most (`lod_net`).
    pub far_terrain: Option<horizon_client::lod_net::Offer>,
    /// Where an older release's server last broke a potion, whose sound this client played
    /// itself (`splash_sound`).
    pub(super) splash_sound: horizon_protocol::adapter::splash_sound::SplashSound,
    pub(super) builtin: Arc<Builtin>,
    lang: Arc<Language>,
    pub(in crate::game) registries: Arc<Dynamic>,
    /// The block, item and fluid tags of `registries` by name, as native mods read them
    /// (`install_remote_tags`).
    pub(super) tags: Option<horizon_client::items::Items>,
    /// The game's enchantments and the server's that it lacks (a data
    /// pack's), from their synced data; `None` when the server has none.
    pub(super) enchantments: Option<Arc<horizon_client::enchant::Enchantments>>,
    pub(super) joined: bool,
    /// `ServerReconfigScreen` is up since this tick.
    pub(super) reconfiguring: Option<u64>,
    /// `handleLogin` ran: vanilla's `Minecraft.player` exists from then on,
    /// while here the player is made at the first teleport.
    pub(super) logged_in: bool,
    status: String,
    pub(super) entity_id: i32,
    local_entity_silent: bool,
    /// `Minecraft.cameraEntity` when the server put the view on another
    /// entity (ClientboundSetCameraPacket); None while it is the player.
    pub(super) camera: Option<i32>,
    /// The item the player's use began with (`LivingEntity.useItem`).
    pub(super) use_item: Option<Arc<str>>,
    pub(super) dimension: DimensionType,
    /// The level's key (`minecraft:overworld`): where its far terrain is kept.
    pub(in crate::game) level: String,
    /// `level`'s key, as mods' handles name the level.
    pub(super) level_key: mod_api::DimensionKey,
    /// The level's facts from the dimension type this connection's server sent (`None` when it
    /// sent none the game takes): owned by the connection, so entering a level touches no other
    /// level's facts, the integrated server's included.
    pub(in crate::game) level_facts: Option<Arc<horizon_world::dimension_type::DimensionFacts>>,
    /// The sky, fog and cloud attributes (and timelines) of the level from the server's type, for a
    /// level of a mod or a pack (`None` for a built-in level, or when the type is refused).
    pub(in crate::game) level_environment: Option<horizon_client::environment::Environment>,
    /// Connection-owned facts, cached by level and the full named type identity.
    facts_cache: HashMap<
        String,
        (
            String,
            serde_json::Value,
            Arc<horizon_world::dimension_type::DimensionFacts>,
        ),
    >,
    /// The levels this connection added to the process's table of levels: the only ones it drops.
    added_levels: std::collections::HashSet<String>,
    /// `ClientPlayerNetworkEvent.LoggingOut` was fired for this connection: every way out
    /// of a world reports it once.
    logged_out: bool,
    /// The other levels' worlds the server opened on `horizon:levels` (`viewed_worlds`).
    pub(in crate::game) viewed: super::viewed_worlds::ViewedWorlds,
    /// The level's hashed seed (login, respawn): the folder of its kept chunks
    /// (`server_terrain::level_dir`).
    pub(super) hashed_seed: i64,
    /// What chunk packets decode with (the dimension's height and sky, the
    /// biome registry); replaced whenever one of them changes.
    pub(super) chunk_format: Arc<ChunkFormat>,
    /// Decodes chunk packets as they arrive, off the frame.
    decoder: ChunkDecoder,
    /// Options.serverRenderDistance: the server's view distance from the
    /// login or the last chunk-cache-radius packet.
    pub(super) view_distance: i32,
    /// `ClientLevel.serverSimulationDistance`: from the login or the last
    /// simulation-distance packet.
    pub(super) simulation_distance: i32,
    /// `BlockStatePredictionHandler.currentSequenceNr`: the level's, so a new level (a login,
    /// another dimension) counts from 0 again.
    sequence: i32,
    /// `BlockStatePredictionHandler.lastTeleportSequence`: the sequence of the last
    /// teleport, so a prediction made before it keeps its block but not its snap.
    last_teleport_sequence: i32,
    pub(super) position_sender: horizon_client::player_sync::PositionSender,
    last_input: u8,
    was_sprinting: bool,
    /// The slot the server last heard (`MultiPlayerGameMode.carriedIndex`): a cell, so `send` can
    /// send a new one first (`ensureHasSentCarriedItem`).
    carried: std::cell::Cell<Option<usize>>,
    was_shared_flags: u8,
    was_living_flags: u8,
    last_recorded_pose: Option<crate::pose::Pose>,
    /// The last ClientboundPlayerAbilitiesPacket. Vanilla's LocalPlayer
    /// holds its abilities from the login on; here the player is made at
    /// the first teleport, which takes them then.
    abilities: Option<play::PlayerAbilities>,
    loaded_sent: bool,
    /// The dimension of the client level the mods heard made
    /// (`native_client_level_load`), to be told dropped once.
    pub(super) heard_level: Option<String>,
    /// `ClientPacketListener.levelLoadTracker`: the loading screen after a login
    /// or respawn, until the player's section is shown.
    pub(super) level_load: horizon_client::level_loading::LevelLoadTracker,
    /// The ClientInformation last sent (`Options.broadcastOptions` sends it
    /// again when a view distance, Chat, skin or Main Hand option changes).
    sent_info: horizon_protocol::packets::configuration::ClientInformation,
    /// BlockStatePredictionHandler: blocks changed ahead of the server (a predicted break),
    /// with what it remembers of each of them. Until the server acknowledges the
    /// sequence its block updates only replace the remembered state; then the block
    /// takes it.
    predictions: HashMap<[i32; 3], Prediction>,
    /// The entities the level held when its `LocalPlayer` was added
    /// (`ClientLevel.addEntity` on login or respawn): `ClientLevel.tickEntities`
    /// ticks in the order entities were added, so these tick before the
    /// player and every later one after it.
    ticked_before_player: std::collections::HashSet<u32>,
    /// Open container: (window id, menu name).
    pub(super) menu: ClientMenu,
    /// The crafting result slot of the open menu as the server last set it
    /// (`CraftingMenu.slotChangedCraftingGrid`): the client never computes it.
    pub(super) craft_result: Option<ItemStack>,
    /// The open menu's title as the server named it, with its styles.
    menu_title: Option<horizon_client::text::Text>,
    menu_title_snapshot: Option<Arc<mod_api::Text>>,
    pub(in crate::game) state_id: i32,
    use_target: Option<[i32; 3]>,
    /// A mod menu's extra data, kept for the `OpenScreen` of its window that
    /// follows (`horizon:menu_extra`).
    pub(in crate::game) menu_extra: Option<horizon_protocol::menu_extra::Message>,
    pub(super) entities: HashMap<i32, Tracked>,
    /// Per-tick remote entity scratch retained between frames.
    tick_fluids: Vec<Option<horizon_client::mobs::living::Fluids>>,
    tick_rain: Vec<bool>,
    tick_standings: Vec<Option<String>>,
    tick_water_fx: Vec<horizon_client::mobs::MobEvent>,
    /// Complete network riding graph, including invisible plugin seat entities.
    pub(super) passenger_vehicles: HashMap<u32, u32>,
    pub(super) lerps: HashMap<i32, Lerp>,
    /// The minecarts the server moves with `ClientboundMoveMinecartPacket`
    /// (`NewMinecartBehavior`), which have no interpolation handler.
    pub(super) cart_lerps: HashMap<i32, CartLerp>,
    /// Living entities' `SteppedInterpolationHandler`s.
    pub(super) stepped: HashMap<i32, SteppedInterpolation>,
    /// `LivingEntity.lerpHeadTo`: the head yaw and the ticks left to reach it.
    pub(super) head_lerps: HashMap<i32, (f32, u32)>,
    /// The happy ghast the player just got off, until the server's teleport
    /// puts the player on top of it (`place_ghast_under`).
    pub(super) ghast_dismount: Option<u32>,
    /// Whether the player is the first passenger of its vehicle, the one
    /// `getControllingPassenger` names.
    pub(super) first_passenger: bool,
    /// The player's place in its vehicle's `getPassengers`.
    pub(super) passenger_index: usize,
    /// Item entities waiting for their item data: where they are and their motion.
    pub(super) pending_items: HashMap<i32, ([f64; 3], Vec3)>,
    health: (f32, i32, f32),
    /// `LocalPlayer.flashOnSetHealth`: set by the first health the server
    /// sends; from then on a drop starts the hurt animation.
    flash_on_set_health: bool,
    game_mode: i32,
    pub chat: Vec<(String, Instant)>,
    /// Lines received since joining, for the control socket's events.
    pub chat_received: u64,
    /// `ChatListener.delayedMessageQueue`: player chat held back by Chat
    /// Delay, with the signature a deletion names it by and what the
    /// narrator says once it shows.
    chat_queue: VecDeque<QueuedChat>,
    /// `ClientPacketListener.messageSignatureCache`: the signatures packed
    /// ones index, as the server's copy for this connection holds them.
    pub(super) signatures: horizon_protocol::secure_chat::SignatureCache,
    /// `ChatListener.previousMessageTime`: when a message last showed, if one has.
    chat_previous: Option<Instant>,
    /// `ClientPacketListener.nextChatIndex`: the global index the next chat packet must carry.
    next_chat_index: i32,
    /// `serverEnforcesSecureChat`, from the login packet.
    enforces_secure_chat: bool,
    services_keys: Arc<std::sync::Mutex<ServicesKeys>>,
    /// The `RemoteChatSession.Data` each player's list entry last carried (INITIALIZE_CHAT).
    chat_sessions: HashMap<u128, Option<(u128, i64, Vec<u8>, Vec<u8>)>>,
    /// `PlayerInfo.messageValidator` of the players whose chat has been read.
    chat_validators: HashMap<u128, secure_chat::MessageValidator>,
    /// `ConnectScreen.lastNarration`: when "Joining the world" was last said.
    joining_narrated: Option<Instant>,
    /// The tree the chat colours, completes and parses with: the server's
    /// (ClientboundCommandsPacket) once it has sent one, else the game's own,
    /// plus the commands native mods own (`chat::chat_tree_for`). Rebuilt where
    /// that base changes, not at each keystroke.
    pub chat_tree: horizon_client::command::Dispatcher,
    selector_metadata_supported: bool,
    /// Whether a `ClientboundCommandsPacket` has supplied the server's tree.
    /// The local fallback is for UI completion only; vanilla signs command
    /// arguments against an empty tree until the server sends one.
    pub(super) chat_tree_received: bool,
    /// The chat session, the last-seen window and the signature cache.
    pub(super) secure_chat: super::remote_chat::SecureChat,
    /// `ClientSuggestionProvider.customCompletionSuggestions`: the extra chat
    /// completions this server sent (ClientboundCustomChatCompletionsPacket).
    custom_completions: std::collections::BTreeSet<String>,
    players: HashMap<u128, String>,
    /// `ClientPacketListener.playerInfoMap` as player-list entries, and which of them are listed.
    tab_players: HashMap<u128, horizon_client::tab_list::TabPlayer>,
    /// Each listed profile's unpacked `textures` property.
    skin_textures: HashMap<u128, horizon_client::skins::Textures>,
    /// Active skin and cape from the services profile when the server sent none.
    own_profile: OwnProfile,
    /// The skin texture each profile's players show, once its download is ready.
    skins: HashMap<u128, String>,
    skins_revision: u64,
    published_skins: Option<(usize, u128, u64, u64)>,
    listed: std::collections::HashSet<u128>,
    /// `ClientboundLoginPacket.onlineMode`: the player list shows heads.
    pub(super) online_mode: bool,
    /// `ClientboundLoginPacket.levels`: the server's level keys, which
    /// `ClientPacketListener.levels()` hands to command suggestions.
    pub(super) levels: Vec<String>,
    /// Player entities in the level: entity id to profile UUID.
    pub(super) level_players: HashMap<i32, u128>,
    /// The profile name each player entity was created with (`RemotePlayer.getGameProfile`): it outlives the
    /// player-list entry, which servers remove to hide their NPCs from the list.
    level_player_names: HashMap<i32, String>,
    /// The `textures` each player entity found in the player list first
    /// (`AbstractClientPlayer.getPlayerInfo` keeps that `PlayerInfo`): its skin
    /// outlives the entry, which NPC plugins remove a moment after spawning.
    level_player_textures: HashMap<i32, horizon_client::skins::Textures>,
    /// `ClientPacketListener.seenPlayers`: the player-info entries of the
    /// players whose entity this connection has added. It outlives an entity's
    /// removal and a dimension change, as vanilla's does.
    seen_players: std::collections::HashSet<u128>,
    /// Entities with shared flag 6 (`Entity.isCurrentlyGlowing` on a client); mobs keep
    /// theirs in `Mob::glowing`.
    pub(super) glowing: std::collections::HashSet<i32>,
    /// The player's own shared flag 5 (`Entity.isInvisible`) as the server last set it.
    pub(super) own_invisible: bool,
    /// The player's own shared flag 0 (`Entity.isOnFire`) as the server last
    /// set it: its fire (blaze fireballs, fire aspect, lightning) never
    /// reaches the window's own `fire_ticks` otherwise.
    own_on_fire: bool,
    /// `LivingEntity.DATA_ARROW_COUNT_ID` and `DATA_STINGER_COUNT_ID` of the
    /// own player, as the server synced them.
    pub(super) own_arrows: i32,
    pub(super) own_stingers: i32,
    /// `Player.DATA_SHOULDER_PARROT_LEFT` and `_RIGHT` of the own player.
    pub(super) own_shoulders: [Option<i32>; 2],
    /// The local player's profile UUID (always in the level).
    pub(super) profile: u128,
    /// The local player's profile name: its score holder and team member.
    pub(super) profile_name: String,
    /// `MultiPlayerGameMode.destroyBlockPos`, `.destroyProgress` and the face last sent: the block
    /// the break targets, which outlives the break itself.
    destroying: Option<([i32; 3], u8, f32)>,
    /// `MultiPlayerGameMode.destroyingItem`: the main-hand stack the break began with.
    destroying_item: Option<ItemStack>,
    /// `MultiPlayerGameMode.isDestroying`: a break is running. `destroying` keeps the target after
    /// one ends (`destroyBlockPos` is written only when a break starts), so a dig resumed on the
    /// same block and item sends no new START and the server keeps its clock (arismoko/horizon#4124).
    is_destroying: bool,
    /// Per-hand swing policy, restored when the current input attempt ends.
    pub(in crate::game) interaction_swing: [bool; 2],
    destroy_delay: u32,
    pub(super) input: horizon_client::interaction_input::InteractionInput,
    /// Keys whose presses send a packet (swap hands, drop, a mount's
    /// inventory): like the click queue, they wait for the tick's
    /// `handleKeybinds` phase rather than going out between window events.
    pub(super) pending_keys: InputQueue<KeyCode>,
    /// Whether the main and off hand still wait for the server's answer to
    /// a SWAP_ITEM_WITH_OFFHAND (`remote_hand_slot_set`).
    swap_answer: [bool; 2],
    stop_mining: bool,
    placing: Option<([i32; 3], u64)>,
    /// `CommonPlayerSpawnInfo.isFlat`: the level is a superflat one.
    pub(super) flat: bool,
    death_message: Option<horizon_client::text::Text>,
    /// `LocalPlayer.showDeathScreen`: off with `immediate_respawn`.
    show_death_screen: bool,
    /// `Player.isReducedDebugInfo`: the server's `reduced_debug_info` rule.
    pub(super) reduced_debug_info: bool,
    /// `LocalPlayer.doLimitedCrafting`: whether limited crafting is enabled.
    pub(super) do_limited_crafting: bool,
    player_type: Option<Arc<MobType>>,
    /// Inventory, off hand and armour as last synchronised, for creative-mode
    /// slot updates (`creative_sync_stacks`).
    synced: Vec<Option<ItemStack>>,
    decode_errors: u32,
    /// Connection-owned `ClientClockManager` state, retained across dimensions.
    clocks: HashMap<i32, play::ClockState>,
    default_clock: Option<i32>,
    overworld_clock: Option<i32>,
    clock_game_time: i64,
    /// `ClientChunkCache.Storage`'s view centre (ClientboundSetChunkCacheCenterPacket).
    chunk_center: (i32, i32),
    /// Received events a frame left for the next one, in order, with each
    /// item's arrival time so `BACKLOG_LIMIT` tracks the front item's age
    /// instead of staying overdue for an entire continuous chunk stream.
    backlog: VecDeque<(Received, Instant)>,
    /// Arrival time of the front item in `backlog`, kept updated for
    /// mod-API internals compatibility while per-item timestamps drive
    /// `BACKLOG_LIMIT`.
    #[allow(dead_code)]
    backlog_since: Option<Instant>,
    /// `ClientboundTransferPacket`'s target: when the connection ends the
    /// game connects here, the stored cookies carried over (`TransferState`).
    transfer: Option<String>,
    /// `ClientCommonPacketListenerImpl.serverLinks`: the links "Server
    /// Links..." in the pause menu lists (only `http`/`https` URIs kept).
    pub(super) server_links: Vec<horizon_protocol::packets::configuration::ServerLink>,
    /// `ClientPacketListener.getDebugQueryHandler`: the F3+I query waiting for the server's data.
    pub(super) debug_query: super::debug_keys::DebugQuery,
}

/// The time a frame spends on chunk packets before it leaves the rest of
/// the received events for the next frame. A batch of 25 to 60 columns
/// (0.1 ms each on an M4 Pro) all in one frame was a 3 to 8 ms frame every
/// time the player crossed a chunk border at 32 chunks; they decode on the
/// chunk decoder's thread now, and the frame only inserts them.
const CHUNK_PACKETS_PER_FRAME: Duration = Duration::from_millis(1);
/// Events that have waited this long are all handled in the next frame,
/// whatever inserting their columns costs, up to a column still decoding:
/// the packets after a slow machine's chunks don't wait on its frames.
const BACKLOG_LIMIT: Duration = Duration::from_millis(50);
/// The open menu name of a server's horse inventory, then its chest's
/// columns (the packet names no `MenuType`).
const MOUNT_MENU: &str = "horizon:mount";
/// ServerboundPlayerInputPacket's shift bit.
const SHIFT_INPUT: u8 = 1 << 5;

fn face_direction(face: [i32; 3]) -> u8 {
    match face {
        [0, -1, 0] => 0,
        [0, 1, 0] => 1,
        [0, 0, -1] => 2,
        [0, 0, 1] => 3,
        [-1, 0, 0] => 4,
        [1, 0, 0] => 5,
        _ => 1,
    }
}

/// A protocol number format as the scoreboard's.
fn number_format(f: &play::NumberFormat) -> horizon_client::scoreboard::NumberFormat {
    use horizon_client::scoreboard::{NumberFormat, tag_to_json};
    match f {
        play::NumberFormat::Blank => NumberFormat::Blank,
        play::NumberFormat::Styled(t) => NumberFormat::Styled(tag_to_json(t)),
        play::NumberFormat::Fixed(t) => NumberFormat::Fixed(tag_to_json(t)),
    }
}

mod apply_packet;
mod blocks;
mod chat;
mod connection;
mod entities;
mod entity_updates;
mod input;
mod level_state;
mod menu_slots;
mod menus;
mod mining;
mod player_state;
mod polling;
mod scoreboard;
mod screens;
mod skins;
mod tick;
mod tick_entities;
mod use_prediction;
mod wire;

// The `Game` methods moved into the files above stay here as the mod points they were, by name, module path and
// signature; each forwards to its body in its file.
impl Game<'_> {
    // connection.rs
    pub(super) fn remote_sound_name(
        &self,
        sound: &(i32, Option<(String, Option<f32>)>),
    ) -> Option<String> {
        self.remote_sound_name_body(sound)
    }
    pub(super) fn remote_sound_entity_state(
        &mut self,
        id: i32,
    ) -> Option<horizon_client::sound_engine::SoundEntityState> {
        self.remote_sound_entity_state_body(id)
    }
    pub(super) fn remote_hurt_sound(&self, source_type: i32) -> &'static str {
        self.remote_hurt_sound_body(source_type)
    }
    pub(super) fn connect_server(&mut self, address: &str, name: &str) -> Result<()> {
        self.connect_server_body(address, name)
    }
    pub(super) fn connect_server_with_cookies(
        &mut self,
        address: &str,
        name: &str,
        cookies: Option<horizon_client::net::Cookies>,
    ) -> Result<()> {
        self.connect_server_with_cookies_body(address, name, cookies)
    }
    pub(super) fn connect_server_with_options(
        &mut self,
        address: &str,
        name: &str,
        cookies: Option<horizon_client::net::Cookies>,
        lan: bool,
        realm: bool,
    ) -> Result<()> {
        self.connect_server_with_options_body(address, name, cookies, lan, realm)
    }
    pub(super) fn connect_remote_session(
        &mut self,
        address: &str,
        name: &str,
        local: Option<horizon_protocol::connection::Pipe>,
        cookies: Option<horizon_client::net::Cookies>,
        lan: bool,
        realm: bool,
    ) -> Result<()> {
        self.connect_remote_session_body(address, name, local, cookies, lan, realm)
    }
    pub(super) fn init_remote_for_replay(&mut self) -> Result<()> {
        self.init_remote_for_replay_body()
    }
    pub(super) fn leave_server(&mut self, reason: Option<Text>) -> Result<()> {
        self.leave_server_body(reason)
    }

    // wire.rs
    fn hud_disconnected(&mut self) {
        self.hud_disconnected_body()
    }
    pub(super) fn send(&self, packet: S) {
        self.send_body(packet)
    }
    pub(super) fn send_play(&self, packet: S) -> bool {
        self.send_play_body(packet)
    }
    fn send_action(&self, action: i32) {
        self.send_action_body(action)
    }
    fn remote_shift_down(&self) -> bool {
        self.remote_shift_down_body()
    }
    fn next_sequence(&mut self) -> i32 {
        self.next_sequence_body()
    }
    fn item_name(&self, id: i32) -> Arc<str> {
        self.item_name_body(id)
    }
    pub(super) fn remote_effect_network_id(&self, effect: &str) -> Option<i32> {
        self.remote_effect_network_id_body(effect)
    }
    pub(super) fn remote_effect_id(&self, effect: i32) -> Option<&'static str> {
        self.remote_effect_id_body(effect)
    }
    fn remote_effect_def(
        &self,
        effect: i32,
    ) -> Option<&'static horizon_client::brewing::EffectDef> {
        self.remote_effect_def_body(effect)
    }
    fn remote_effect(&mut self, p: &play::UpdateMobEffect) {
        self.remote_effect_body(p)
    }
    fn item_id(&self, name: &str) -> i32 {
        self.item_id_body(name)
    }
    pub(super) fn to_stack(&self, s: &Option<Stack>) -> Option<ItemStack> {
        self.to_stack_body(s)
    }
    fn to_network(&self, s: &Option<ItemStack>) -> Option<Stack> {
        self.to_network_body(s)
    }
    fn hashed_stack(&self, s: &Option<ItemStack>) -> Option<Hashed> {
        self.hashed_stack_body(s)
    }

    // chat.rs
    pub(super) fn add_chat_line(
        &mut self,
        content: horizon_client::text::Text,
        source: horizon_client::chat::ChatSource,
    ) -> bool {
        self.add_chat_line_body(content, source)
    }
    fn signed_chat_line(
        &mut self,
        content: horizon_client::text::Text,
        source: horizon_client::chat::ChatSource,
        signature: Option<Signature>,
        trust: ChatTrust,
    ) -> bool {
        self.signed_chat_line_body(content, source, signature, trust)
    }
    fn handle_player_chat(&mut self, chat: QueuedChat) {
        self.handle_player_chat_body(chat)
    }
    fn handle_chat_message_error(
        &mut self,
        chat_type: &play::ChatTypeBound,
        signature: Option<&Signature>,
    ) {
        self.handle_chat_message_error_body(chat_type, signature)
    }
    fn chat_trust(
        &self,
        sender: u128,
        signed: bool,
        timestamp: i64,
        content: &str,
        unsigned: Option<&Tag>,
        decorated: &horizon_client::text::Text,
    ) -> ChatTrust {
        self.chat_trust_body(sender, signed, timestamp, content, unsigned, decorated)
    }
    fn handle_delete_chat(&mut self, packed: &(i32, Option<Vec<u8>>)) -> Result<()> {
        self.handle_delete_chat_body(packed)
    }
    fn accept_chat(&mut self, chat: QueuedChat) -> bool {
        self.accept_chat_body(chat)
    }
    fn player_chat_line(&mut self, chat: QueuedChat) -> bool {
        self.player_chat_line_body(chat)
    }

    // polling.rs
    pub(super) fn remote_carts(&self) -> Option<Vec<serde_json::Value>> {
        self.remote_carts_body()
    }
    pub(super) fn auth(&self) -> Option<std::sync::Arc<horizon_client::auth::AuthSession>> {
        self.auth_body()
    }
    pub(super) fn online_server(&self) -> bool {
        self.online_server_body()
    }
    pub(super) fn remote_custom_completions(&self) -> std::collections::BTreeSet<String> {
        self.remote_custom_completions_body()
    }
    pub(super) fn remote_online_names(&self) -> Option<Vec<String>> {
        self.remote_online_names_body()
    }
    pub(super) fn remote_menu_id(&self) -> i32 {
        self.remote_menu_id_body()
    }
    pub(super) fn remote_chat(&self, n: usize) -> Option<Vec<String>> {
        self.remote_chat_body(n)
    }
    pub(super) fn remote_backlog(&self) -> usize {
        self.remote_backlog_body()
    }
    pub(super) fn poll_stationary_client(&mut self) -> Result<()> {
        self.poll_stationary_client_body()
    }
    pub(super) fn poll_remote(&mut self) -> Result<()> {
        self.poll_remote_body()
    }
    fn send_suggestion_request(&mut self) {
        self.send_suggestion_request_body()
    }
    fn handle_command_suggestions(&mut self, data: &[u8]) {
        self.handle_command_suggestions_body(data)
    }

    // level_state.rs
    fn set_registries(&mut self, registries: Arc<Dynamic>) {
        self.set_registries_body(registries)
    }
    pub(in crate::game) fn enter_remote_dimension(
        &mut self,
        spawn: &play::SpawnInfo,
        keep_world: bool,
        reason: horizon_client::level_loading::Reason,
    ) {
        self.enter_remote_dimension_body(spawn, keep_world, reason)
    }
    pub(super) fn remote_chunk_in_range(&self, x: i32, z: i32) -> bool {
        self.remote_chunk_in_range_body(x, z)
    }
    fn chunk_inside_remote_range(center: (i32, i32), distance: i32, x: i32, z: i32) -> bool {
        Self::chunk_inside_remote_range_body(center, distance, x, z)
    }
    pub(in crate::game) fn drop_remote_chunks_out_of_range(
        &mut self,
        center: (i32, i32),
        distance: i32,
    ) -> Vec<(i32, i32)> {
        self.drop_remote_chunks_out_of_range_body(center, distance)
    }
    pub(in crate::game) fn forget_all_remote_columns(&mut self) -> Vec<(i32, i32)> {
        self.forget_all_remote_columns_body()
    }
    pub(in crate::game) fn forget_remote_columns(
        &mut self,
        gone: &mut horizon_client::fast_hash::FxHashSet<(i32, i32)>,
        keep_terrain: bool,
    ) -> Vec<(i32, i32)> {
        self.forget_remote_columns_body(gone, keep_terrain)
    }
    fn remote_animate(&mut self, id: i32, action: u8) {
        self.remote_animate_body(id, action)
    }
    fn remote_server_data(&mut self, icon: Option<&[u8]>) {
        self.remote_server_data_body(icon)
    }

    // blocks.rs
    pub(super) fn sync_block_entity(
        &mut self,
        pos: (i32, i32, i32),
        previous: Option<BlockStateId>,
        state: BlockStateId,
    ) {
        self.sync_block_entity_body(pos, previous, state)
    }
    fn forget_block_entity_attachments(&mut self, (x, y, z): (i32, i32, i32)) {
        self.forget_block_entity_attachments_body((x, y, z))
    }
    fn server_block(&mut self, pos: [i32; 3], state: i32) {
        self.server_block_body(pos, state)
    }
    fn end_predictions(&mut self, sequence: i32) {
        self.end_predictions_body(sequence)
    }
    fn player_position(&self) -> [f64; 3] {
        self.player_position_body()
    }
    fn block_meets_player(&self, pos: [i32; 3], state: i32) -> bool {
        self.block_meets_player_body(pos, state)
    }
    fn snap_player_to(&mut self, at: [f64; 3]) {
        self.snap_player_to_body(at)
    }
    fn set_block(&mut self, pos: [i32; 3], state: i32, predicted: bool) {
        self.set_block_body(pos, state, predicted)
    }

    // entities.rs
    pub(super) fn remote_vehicle_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.remote_vehicle_ids_body()
    }
    pub(super) fn remote_seat_passengers(&mut self, vehicle: u32, passengers: &[i32]) {
        self.remote_seat_passengers_body(vehicle, passengers)
    }
    pub(super) fn client_attachment_entity_exists(&self, holder: Holder) -> bool {
        self.client_attachment_entity_exists_body(holder)
    }
    pub(super) fn spawn_entity(
        &mut self,
        id: i32,
        kind: &str,
        position: [f64; 3],
        rotation: [f32; 3],
    ) {
        self.spawn_entity_body(id, kind, position, rotation)
    }
    fn move_entity(
        &mut self,
        id: i32,
        path: Option<Path>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
        packet: MovePacket,
    ) {
        self.move_entity_body(id, path, rotation, on_ground, packet)
    }
    pub(super) fn detach_remote_tracking_entity(&mut self, id: i32) {
        self.detach_remote_tracking_entity_body(id)
    }

    // menu_slots.rs
    pub(super) fn inventory_screen_slot(menu: &str, slot: i32) -> Option<usize> {
        Self::inventory_screen_slot_body(menu, slot)
    }
    pub(super) fn menu_slot_index(menu: &str, internal: usize) -> Option<usize> {
        Self::menu_slot_index_body(menu, internal)
    }
    pub(super) fn remote_station_kind(menu: &str) -> Option<super::stations::Kind> {
        Self::remote_station_kind_body(menu)
    }
    fn open_menu_name(&self, container: i32) -> String {
        self.open_menu_name_body(container)
    }
    fn set_menu_slot(&mut self, container: i32, slot: i32, item: Option<ItemStack>) {
        self.set_menu_slot_body(container, slot, item)
    }

    // apply_packet.rs
    pub(in crate::game) fn apply_packet(&mut self, packet: C) -> Result<()> {
        self.apply_packet_body(packet)
    }

    // scoreboard.rs
    fn remote_objective(&mut self, p: play::SetObjective) {
        self.remote_objective_body(p)
    }
    fn remote_team(&mut self, p: play::SetPlayerTeam) {
        self.remote_team_body(p)
    }

    // skins.rs
    pub(super) fn publish_tab_object_skins(&mut self) {
        self.publish_tab_object_skins_body()
    }
    pub(super) fn remote_tab_players(
        &mut self,
    ) -> Option<(Vec<horizon_client::tab_list::TabPlayer>, bool, Vec<u128>)> {
        self.remote_tab_players_body()
    }
    pub(super) fn resolve_inline_profiles(
        &mut self,
        profiles: &[std::sync::Arc<serde_json::Value>],
    ) {
        self.resolve_inline_profiles_body(profiles)
    }
    fn poll_own_profile(&mut self) {
        self.poll_own_profile_body()
    }
    fn skin_3d_model(
        &self,
        kind: &std::sync::Arc<horizon_client::mobs::MobType>,
        skin: &str,
        slim: bool,
    ) -> Option<std::sync::Arc<horizon_client::mobs::Part>> {
        self.skin_3d_model_body(kind, skin, slim)
    }
    pub(super) fn update_remote_skins(&mut self) {
        self.update_remote_skins_body()
    }
    pub(super) fn store_skin(&mut self, location: String, rgba: Vec<u8>) {
        self.store_skin_body(location, rgba)
    }
    fn again_skin(&mut self, location: &str) -> bool {
        self.again_skin_body(location)
    }
    pub(super) fn drawn_skin(
        &mut self,
        uuid: u128,
        (skin, slim): (String, bool),
    ) -> (String, bool) {
        self.drawn_skin_body(uuid, (skin, slim))
    }
    pub(super) fn face_skin(&mut self, uuid: u128, skin: &str) -> String {
        self.face_skin_body(uuid, skin)
    }
    pub(super) fn decorate_chat(
        &self,
        bound: &play::ChatTypeBound,
        content: &serde_json::Value,
        decoration: Decoration,
    ) -> horizon_client::text::Text {
        self.decorate_chat_body(bound, content, decoration)
    }

    // entity_updates.rs
    fn remote_item_data(&mut self, id: i32, stack: &Option<Stack>) {
        self.remote_item_data_body(id, stack)
    }
    fn remote_motion(&mut self, id: i32, velocity: [f64; 3]) {
        self.remote_motion_body(id, velocity)
    }
    fn apply_remote_health(&mut self) {
        self.apply_remote_health_body()
    }
    fn place_ghast_under(&mut self, id: u32, spot: Vec3) {
        self.place_ghast_under_body(id, spot)
    }
    fn remote_look_at(&mut self, p: &play::PlayerLookAt) {
        self.remote_look_at_body(p)
    }
    fn remote_teleport(&mut self, p: &play::PlayerPosition) -> Result<()> {
        self.remote_teleport_body(p)
    }

    // tick.rs
    fn tick_client_load(&mut self) {
        self.tick_client_load_body()
    }
    pub(super) fn tick_remote(&mut self) -> Result<()> {
        self.tick_remote_body()
    }
    fn tick_remote_player(&mut self, running: bool, hold: bool) -> Result<()> {
        self.tick_remote_player_body(running, hold)
    }
    fn tick_local_player(
        &mut self,
        entity_collisions: Vec<horizon_core::geometry::Aabb>,
        pushed: Vec3,
        sprint_window: i32,
    ) -> Result<()> {
        self.tick_local_player_body(entity_collisions, pushed, sprint_window)
    }

    // tick_entities.rs
    pub(super) fn remote_abilities(&self) -> Option<&play::PlayerAbilities> {
        self.remote_abilities_body()
    }
    fn remote_pushes(&self, before_player: bool) -> Vec3 {
        self.remote_pushes_body(before_player)
    }
    fn tick_remote_guardian_eyes(&mut self, running: bool) {
        self.tick_remote_guardian_eyes_body(running)
    }
    pub(in crate::game) fn tick_remote_arm_poses(&mut self) {
        self.tick_remote_arm_poses_body()
    }
    fn tick_remote_entities(&mut self, running: bool) {
        self.tick_remote_entities_body(running)
    }

    // player_state.rs
    pub(super) fn remote_minecart_lerp(
        &self,
        id: u32,
        partial: f32,
    ) -> Option<([f64; 3], f32, f32)> {
        self.remote_minecart_lerp_body(id, partial)
    }
    fn remote_move_minecart(&mut self, p: play::MoveMinecartAlongTrack) {
        self.remote_move_minecart_body(p)
    }
    fn remote_player_eating(
        &mut self,
        item: &str,
        ticks: u32,
        at: [f64; 3],
        eye: f64,
        pitch: f32,
        yaw: f32,
    ) {
        self.remote_player_eating_body(item, ticks, at, eye, pitch, yaw)
    }
    fn remote_send_state(&mut self, input: u8) {
        self.remote_send_state_body(input)
    }
    pub(in crate::game) fn creative_stacks(&self) -> Vec<Option<ItemStack>> {
        self.creative_stacks_body()
    }
    pub(super) fn send_creative_slots(&mut self) {
        self.send_creative_slots_body()
    }
    fn remote_synced(&mut self) {
        self.remote_synced_body()
    }
    pub(in crate::game) fn remote_synced_since(&mut self, before: &[Option<ItemStack>]) {
        self.remote_synced_since_body(before)
    }

    // mining.rs
    pub(super) fn stop_remote_mining(&mut self) -> bool {
        self.stop_remote_mining_body()
    }
    fn remote_mining(&mut self, start: bool) -> bool {
        self.remote_mining_body(start)
    }
    fn veto_mining_prediction(
        &self,
        pos: [i32; 3],
        face: [i32; 3],
        starting: bool,
        progress: f32,
    ) -> bool {
        self.veto_mining_prediction_body(pos, face, starting, progress)
    }
    fn remote_break_restricted(&self, pos: [i32; 3]) -> bool {
        self.remote_break_restricted_body(pos)
    }
    fn remote_use_block(&mut self, hand: Hand) -> bool {
        self.remote_use_block_body(hand)
    }
    pub(in crate::game) fn predict_item_on_block(
        &mut self,
        stack: &ItemStack,
        hit: &super::Hit,
    ) -> bool {
        self.predict_item_on_block_body(stack, hit)
    }
    fn predict_map_on_banner(&self, stack: &ItemStack, hit: &super::Hit) -> bool {
        self.predict_map_on_banner_body(stack, hit)
    }
    fn remote_bone_meal_succeeds(&mut self, hit: &super::Hit, stack: &ItemStack) -> bool {
        self.remote_bone_meal_succeeds_body(hit, stack)
    }
    fn predict_use(&mut self, (changes, sounds): &DryUse, seq: i32) {
        self.predict_use_body((changes, sounds), seq)
    }

    // use_prediction.rs
    pub(super) fn is_level_player(&self, id: i32) -> bool {
        self.is_level_player_body(id)
    }
    pub(super) fn block_takes_click(&mut self, hit: &super::Hit, held: Option<&ItemStack>) -> bool {
        self.block_takes_click_body(hit, held)
    }
    pub(super) fn block_swallows_click(
        &mut self,
        hit: &super::Hit,
        held: Option<&ItemStack>,
    ) -> bool {
        self.block_swallows_click_body(hit, held)
    }
    fn remote_block_reacts(
        &mut self,
        hit: &super::Hit,
        held: Option<&ItemStack>,
    ) -> (InteractionResult, DryUse) {
        self.remote_block_reacts_body(hit, held)
    }
    pub(super) fn remote_block_template(
        &mut self,
        hit: &super::Hit,
        held: Option<&ItemStack>,
        without_item: bool,
    ) -> (InteractionResult, DryUse) {
        self.remote_block_template_body(hit, held, without_item)
    }
    fn predict_place(&mut self, (pos, changes): &DryPlace, seq: i32) {
        self.predict_place_body((pos, changes), seq)
    }
    pub(super) fn predict_blocks(&mut self, changes: &DryChanges, seq: i32) {
        self.predict_blocks_body(changes, seq)
    }
    pub(super) fn remote_can_place(
        &mut self,
        hit: &super::Hit,
        stack: &ItemStack,
    ) -> Option<DryPlace> {
        self.remote_can_place_body(hit, stack)
    }
    fn remote_use_bucket(
        &mut self,
        stack: &ItemStack,
    ) -> (bool, Option<DryChanges>, Vec<DrySound>) {
        self.remote_use_bucket_body(stack)
    }
    fn use_finds_no_target(&self, held: &ItemStack) -> bool {
        self.use_finds_no_target_body(held)
    }
    fn remote_start_held_use(&mut self, held: &ItemStack) -> bool {
        self.remote_start_held_use_body(held)
    }
    fn remote_use_instrument(&mut self, held: &ItemStack) {
        self.remote_use_instrument_body(held)
    }
    fn held_until_released(&self, stack: &ItemStack) -> bool {
        self.held_until_released_body(stack)
    }
    fn patched_held_use(&self, stack: &ItemStack) -> bool {
        self.patched_held_use_body(stack)
    }
    fn remote_use_hands(&mut self, attempt: mod_api::InteractionAttempt) {
        self.remote_use_hands_body(attempt)
    }
    fn remote_tick_using(&mut self) {
        self.remote_tick_using_body()
    }
    fn remote_complete_using(&mut self) {
        self.remote_complete_using_body()
    }

    // input.rs
    pub(super) fn request_remote_mining_stop(&mut self) {
        self.request_remote_mining_stop_body()
    }
    fn remote_handle_input(&mut self) {
        self.remote_handle_input_body()
    }
    fn remote_send_carried(&self) {
        self.remote_send_carried_body()
    }
    fn remote_sweep_event(&mut self, target: &str, living: bool) {
        self.remote_sweep_event_body(target, living)
    }
    fn remote_attacked_object(&self, id: i32) -> Option<(String, bool)> {
        self.remote_attacked_object_body(id)
    }
    fn remote_player_attack(&mut self, mob: usize) {
        self.remote_player_attack_body(mob)
    }
    pub(in crate::game) fn own_latency(&self) -> Option<i32> {
        self.own_latency_body()
    }
    fn own_attribute(&mut self, id: &str) -> Option<f64> {
        self.own_attribute_body(id)
    }
    pub(super) fn remote_key(&mut self, key: KeyCode) -> Result<bool> {
        self.remote_key_body(key)
    }
    pub(super) fn release_use_for_keybind_screen(&mut self) {
        self.release_use_for_keybind_screen_body()
    }
    fn release_remote_use(&mut self) {
        self.release_remote_use_body()
    }
    pub(super) fn send_signed_chat_input(&mut self, message: &str) {
        self.send_signed_chat_input_body(message)
    }
    pub(super) fn send_chat_from_screen(&mut self, message: &str) {
        self.send_chat_from_screen_body(message)
    }
    pub(super) fn remote_drop(&mut self, all: bool) {
        self.remote_drop_body(all)
    }
    pub(super) fn remote_swap_hands(&mut self) {
        self.remote_swap_hands_body()
    }
    fn remote_hand_slot_set(&mut self, slot: usize, changed: bool) {
        self.remote_hand_slot_set_body(slot, changed)
    }
    pub(super) fn remote_click(&mut self, right: bool) {
        self.remote_click_body(right)
    }
    fn remote_menu_state(&mut self, container: i32, state: i32) {
        self.remote_menu_state_body(container, state)
    }

    // menus.rs
    pub(super) fn remote_menu(&self) -> (i32, String) {
        self.remote_menu_body()
    }
    pub(super) fn remote_menu_ops(&mut self, ops: &[(Option<usize>, i32, Input)]) -> bool {
        self.remote_menu_ops_body(ops)
    }
    pub(super) fn remote_quick_move_unpredicted(&mut self, internal: usize) -> bool {
        self.remote_quick_move_unpredicted_body(internal)
    }
    pub(super) fn remote_quick_move_all(&mut self, slot: usize, moved: &ItemStack) {
        self.remote_quick_move_all_body(slot, moved)
    }
    fn send_clicks(&mut self, container: i32, clicks: Vec<Predicted>) {
        self.send_clicks_body(container, clicks)
    }
    pub(super) fn remote_storage_open(&self) -> bool {
        self.remote_storage_open_body()
    }
    pub(super) fn remote_throw(&mut self, right: bool, input: horizon_client::menu::Input) {
        self.remote_throw_body(right, input)
    }
    pub(super) fn remote_creative_drop(&mut self, right: bool) {
        self.remote_creative_drop_body(right)
    }
    pub(super) fn remote_creative_throw(&mut self, thrown: ItemStack) {
        self.remote_creative_throw_body(thrown)
    }
    pub(super) fn remote_close_container(&mut self) {
        self.remote_close_container_body()
    }
    pub(super) fn remote_button_click(&self, button: i32) -> bool {
        self.remote_button_click_body(button)
    }
    fn remote_reject_container(&mut self, container: i32) {
        self.remote_reject_container_body(container)
    }
    fn remote_close_local(&mut self) {
        self.remote_close_local_body()
    }
    fn remote_open_merchant(
        &mut self,
        container: i32,
        title: String,
        text: horizon_client::text::Text,
    ) {
        self.remote_open_merchant_body(container, title, text)
    }
    fn remote_merchant_offers(&mut self, p: play::MerchantOffers) {
        self.remote_merchant_offers_body(p)
    }
    fn remote_open_screen(&mut self, p: play::OpenScreen) {
        self.remote_open_screen_body(p)
    }
    fn remote_open_mount(&mut self, p: play::MountScreenOpen) {
        self.remote_open_mount_body(p)
    }

    // screens.rs
    pub(super) fn remote_menu_type(&self) -> Option<&str> {
        self.remote_menu_type_body()
    }
    pub(super) fn remote_menu_title(&self) -> Option<horizon_client::text::Text> {
        self.remote_menu_title_body()
    }
    pub(super) fn refresh_remote_language(&mut self) {
        self.refresh_remote_language_body()
    }
    pub(super) fn remote_chest_menu(&self) -> Option<(usize, horizon_client::text::Text)> {
        self.remote_chest_menu_body()
    }
    pub(super) fn remote_stop_sleeping(&mut self) {
        self.remote_stop_sleeping_body()
    }
    pub(super) fn remote_respawn(&mut self) {
        self.remote_respawn_body()
    }
    pub(super) fn remote_screen_up(&self) -> bool {
        self.remote_screen_up_body()
    }
    pub(super) fn remote_overlay(&mut self, batch: &mut HudBatch) -> bool {
        self.remote_overlay_body(batch)
    }
    pub(super) fn remote_loading(&self) -> bool {
        self.remote_loading_body()
    }
    fn reconfigure_layout(&self, (w, h): (f32, f32)) -> ((f32, f32), (f32, f32)) {
        self.reconfigure_layout_body((w, h))
    }
    fn draw_reconfiguring(&self, batch: &mut HudBatch, since: u64) {
        self.draw_reconfiguring_body(batch, since)
    }
    fn connect_cancel(&self, (w, h): (f32, f32)) -> Option<(f32, f32)> {
        self.connect_cancel_body((w, h))
    }
    pub(super) fn connect_screen_click(&mut self) -> Result<()> {
        self.connect_screen_click_body()
    }
    pub(super) fn remote_hud(&self, batch: &mut HudBatch) {
        self.remote_hud_body(batch)
    }
    pub(super) fn remote_glowing(&self, id: u32) -> Option<bool> {
        self.remote_glowing_body(id)
    }
    pub(super) fn remote_player_name(&self, id: u32) -> Option<&str> {
        self.remote_player_name_body(id)
    }
    pub(super) fn remote_player_uuid(&self, id: u32) -> Option<u128> {
        self.remote_player_uuid_body(id)
    }
    pub(super) fn remote_player_names(&self) -> Vec<(u32, String)> {
        self.remote_player_names_body()
    }
    fn remote_clock_tick(&mut self) {
        self.remote_clock_tick_body()
    }
    pub(super) fn remote_status(&self) -> Option<serde_json::Value> {
        self.remote_status_body()
    }
}

/// `Entity.lookAt`'s rotation from `from` towards `to`, as (yaw, pitch).
fn look_at_rotation(from: [f64; 3], to: [f64; 3]) -> (f32, f32) {
    use horizon_core::geometry::trig::atan2;
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let horizontal = (dx * dx + dz * dz).sqrt();
    // `* 57.2957763671875` (`Mth.RAD_TO_DEG` widened) on the double.
    let degrees = |r: f64| r * f64::from(horizon_core::geometry::trig::RAD_TO_DEG);
    let pitch = wrap_degrees((-degrees(atan2(dy, horizontal))) as f32);
    let yaw = wrap_degrees(degrees(atan2(dz, dx)) as f32 - 90.0);
    (yaw, pitch)
}

/// `PositionMoveRotation.calculateAbsolute`'s pitch: the packet's pitch,
/// added to `source` when it is relative, clamped to +-90.
fn position_packet_pitch(source: f32, change: f32, relative: bool) -> f32 {
    let offset = if relative { source } else { 0.0 };
    (offset + change).clamp(-90.0, 90.0)
}

/// `Mth.wrapDegrees`.
fn wrap_degrees(a: f32) -> f32 {
    let a = a % 360.0;
    if a >= 180.0 {
        a - 360.0
    } else if a < -180.0 {
        a + 360.0
    } else {
        a
    }
}

/// `LivingEntity.tick`'s body target and `tickHeadTurn` for another player:
/// the body turns toward where it walks (backwards when walking back), or to
/// the look while the arm swings, and stays within 50 degrees of the look.
fn remote_player_head_turn(m: &mut Mob) {
    let (dx, dz) = (m.position.x - m.previous.x, m.position.z - m.previous.z);
    let mut target = m.body_yaw;
    if (dx * dx + dz * dz) as f32 > 0.002_500_000_2 {
        let walk = (dz.atan2(dx) as f32).to_degrees() - 90.0;
        let facing = (wrap_degrees(m.yaw) - walk).abs();
        target = if 95.0 < facing && facing < 265.0 {
            walk - 180.0
        } else {
            walk
        };
    }
    if m.anim.attack_anim > 0.0 {
        target = m.yaw;
    }
    m.body_yaw += wrap_degrees(target - m.body_yaw) * 0.3;
    let head = wrap_degrees(m.yaw - m.body_yaw);
    if head.abs() > 50.0 {
        m.body_yaw += head - head.signum() * 50.0;
    }
}

/// A click sent to a server: its slot, button and input, the slots it
/// changed and the carried stack after it.
type Predicted = (
    i16,
    i32,
    Input,
    Vec<(i16, Option<ItemStack>)>,
    Option<ItemStack>,
);

/// How many stacks the creative screen keeps in step with the server (`Remote::synced`).
const CREATIVE_SYNCED: usize = 41;

/// The player's stacks in `Remote::synced` order: the 36 inventory slots, the off hand, then
/// the armour from the head down. `CreativeInventoryListener.slotChanged` sends every
/// InventoryMenu slot the creative screen changes, the armour slots too.
fn creative_sync_stacks(
    inventory: &[Option<ItemStack>],
    offhand: &Option<ItemStack>,
    armor: &[Option<ItemStack>; 4],
) -> Vec<Option<ItemStack>> {
    let mut stacks: Vec<Option<ItemStack>> = inventory.iter().take(36).cloned().collect();
    stacks.resize(36, None);
    stacks.push(offhand.clone());
    stacks.extend(armor.iter().cloned());
    stacks
}

/// Entries of `creative_sync_stacks` that went from `before` to `now` are synced as `now`.
fn mark_synced(
    synced: &mut [Option<ItemStack>],
    before: &[Option<ItemStack>],
    now: Vec<Option<ItemStack>>,
) {
    for (i, stack) in now.into_iter().enumerate() {
        if before.get(i) != Some(&stack)
            && let Some(s) = synced.get_mut(i)
        {
            *s = stack;
        }
    }
}

/// InventoryMenu's slot for entry `i` of `creative_sync_stacks`: the hotbar is 36..=44, the
/// main inventory 9..=35, the off hand 45 and the armour 5..=8 (head first).
fn creative_sync_slot(i: usize) -> i16 {
    (match i {
        0..9 => 36 + i,
        36 => 45,
        37.. => 5 + (i - 37),
        _ => i,
    }) as i16
}

/// `MultiPlayerGameMode.handleContainerInput` for each click on the engine's
/// copy of the menu: the click with the slots it changed and the carried
/// stack after it, and whether a click dropped a stack. Every drop of
/// `AbstractContainerMenu.doClick` (a THROW, a click outside the window, a
/// swap or a result move with no room left) is `Prediction.PREDICTED`.
/// None when the engine refuses a click.
fn predict_clicks(
    m: &mut horizon_client::menu::Menu,
    env: &horizon_client::menu::Env,
    clicks: &[(i16, i32, Input)],
) -> Option<(Vec<Predicted>, bool)> {
    let mut predicted = Vec::with_capacity(clicks.len());
    let mut dropped = false;
    for &(slot, button, input) in clicks {
        let before = menu_items(m);
        let thrown = m.dropped.len();
        m.click(env, i32::from(slot), button, input).ok()?;
        dropped |= m.dropped.len() > thrown;
        let changed = changed_slots(&before, m);
        predicted.push((slot, button, input, changed, m.carried.clone()));
    }
    Some((predicted, dropped))
}

fn menu_items(m: &horizon_client::menu::Menu) -> Vec<Option<ItemStack>> {
    (0..m.slot_count()).map(|i| m.slot(i).cloned()).collect()
}

/// The slots whose stack is no longer `before`'s (`ItemStack.matches`).
/// `MultiPlayerGameMode.handleContainerInput`'s changed slots, in the order the packet writes them.
fn changed_slots(
    before: &[Option<ItemStack>],
    m: &horizon_client::menu::Menu,
) -> Vec<(i16, Option<ItemStack>)> {
    let changed: Vec<i16> = (0..m.slot_count())
        .filter(|&i| m.slot(i) != before.get(i).and_then(Option::as_ref))
        .map(|i| i as i16)
        .collect();
    open_hash_order(&changed)
        .into_iter()
        .map(|i| (i, m.slot(i as usize).cloned()))
        .collect()
}

/// The order a new fastutil `Int2ObjectOpenHashMap` iterates `keys` put in this order: vanilla
/// collects a click's changed slots in one and writes them as it iterates. The key 0 comes first,
/// then the table from its last position down; a key sits at `HashCommon.mix(key) & mask` or the
/// next free position. The table starts at 32 (16 expected, load factor 0.75) and doubles when an
/// insert finds it holding 24 (`size++ >= maxFill`), re-inserting from its last position down.
fn open_hash_order(keys: &[i16]) -> Vec<i16> {
    fn mix(key: i16) -> usize {
        let h = i32::from(key).wrapping_mul(0x9E37_79B9_u32 as i32);
        (h ^ ((h as u32) >> 16) as i32) as u32 as usize
    }
    fn put(table: &mut [Option<i16>], key: i16) {
        let mask = table.len() - 1;
        let mut pos = mix(key) & mask;
        while table[pos].is_some() {
            pos = (pos + 1) & mask;
        }
        table[pos] = Some(key);
    }
    // HashCommon.maxFill(n, 0.75): min(ceil(n * 0.75), n - 1).
    let max_fill = |n: usize| (n * 3).div_ceil(4).min(n - 1);
    let mut table = vec![None; 32];
    let (mut zero, mut size) = (false, 0);
    for &key in keys {
        if key == 0 {
            zero = true;
        } else {
            put(&mut table, key);
        }
        size += 1;
        // `size++ >= maxFill`: the size before this key.
        if size > max_fill(table.len()) {
            // HashCommon.arraySize(size + 1, 0.75): the power of two holding size + 1 at that load.
            let n = ((size + 1) * 4).div_ceil(3).next_power_of_two();
            let mut grown = vec![None; n];
            for &key in table.iter().rev().flatten() {
                put(&mut grown, key);
            }
            table = grown;
        }
    }
    let rest = table.iter().rev().flatten().copied();
    zero.then_some(0).into_iter().chain(rest).collect()
}

/// ServerboundContainerClickPacket's `ContainerInput` ordinal.
pub(super) fn input_ordinal(input: Input) -> i32 {
    match input {
        Input::Pickup => 0,
        Input::QuickMove => 1,
        Input::Swap => 2,
        Input::Clone => 3,
        Input::Throw => 4,
        Input::QuickCraft => 5,
        Input::PickupAll => 6,
    }
}

mod dry_level;
use dry_level::DryLevel;

type DrySound = (
    horizon_core::position::BlockPos,
    std::sync::Arc<str>,
    f32,
    f32,
);
/// The blocks a use on the dry level set.
type DryChanges = Vec<(horizon_core::position::BlockPos, BlockStateId)>;
/// What a use on the dry level changed, and the sounds it played.
pub(super) type DryUse = (DryChanges, Vec<DrySound>);
/// Where `BlockItem.place` on the dry level placed, and the blocks it set.
type DryPlace = (horizon_core::position::BlockPos, DryChanges);

/// The blocks a guest's hand flips that its use predicts (and hears at once): a wooden or
/// copper door, trapdoor or fence gate, a lever, a button or a cauldron, whose fill level
/// changes as the bucket, bottle or dye lands. An iron door or trapdoor has no hand use.
pub(super) fn hand_operated(name: &str) -> bool {
    let name = name.trim_start_matches("minecraft:");
    ((name.ends_with("_door") || name.ends_with("_trapdoor")) && !name.starts_with("iron_"))
        || name.ends_with("_fence_gate")
        || name.ends_with("_button")
        || name == "lever"
        || name.ends_with("cauldron")
}

/// `InteractionHand` on the wire.
fn hand_id(hand: Hand) -> i32 {
    match hand {
        Hand::Main => 0,
        Hand::Off => 1,
    }
}

/// Whether `Item.use` of `info` ends `Minecraft.startUseItem`'s click, so the
/// off hand gets no turn. `FireworkRocketItem.use` answers SUCCESS while the
/// player glides (the rocket boosts them) and PASS otherwise.
fn use_ends_click(info: &horizon_client::items::ItemInfo, fall_flying: bool) -> bool {
    item_has_use(info) || (fall_flying && &*info.id == "minecraft:firework_rocket")
}

/// Whether `Item.use` of this item answers SUCCESS on this client, which
/// swings (`MultiPlayerGameMode.useItem`'s `result.shouldSwing()`), rather
/// than CONSUME, which only ends the click: a bow, crossbow, trident or
/// spear starting its charge, `ItemUtils.startUsingInstantly`'s food, drink,
/// spyglass, shield and goat horn, and the ender eye's SUCCESS_SERVER (the
/// server's swing source, not this client's).
fn remote_use_swings(held: &ItemStack, consumable: bool) -> bool {
    let id = &*held.item;
    !matches!(
        id,
        "minecraft:bow"
            | "minecraft:crossbow"
            | "minecraft:trident"
            | "minecraft:spyglass"
            | "minecraft:goat_horn"
            | "minecraft:ender_eye"
            | "minecraft:brush"
    ) && !consumable
        && horizon_client::spear::Kinetic::of(id).is_none()
}

/// Whether an object interaction on the client predicts a hand swing (`InteractionResult.SUCCESS`).
/// Boarding a minecart answers CONSUME and does not swing.
fn remote_object_swings(response: Option<bool>, is_boarding: bool) -> bool {
    response.unwrap_or(!is_boarding)
}

/// Whether right-clicking an item frame passes (`InteractionResult.PASS`).
/// A fixed frame or an empty frame when clicking with an empty hand passes.
fn item_frame_interact_passes(fixed: bool, frame_has_item: bool, hand_has_item: bool) -> bool {
    fixed || (!frame_has_item && !hand_has_item)
}

/// Whether right-clicking a boat passes (`InteractionResult.PASS`) on a client.
/// `AbstractBoat.interact` passes when sneaking or out of control (60 ticks), and
/// answers SUCCESS otherwise without checking for room (only the server's
/// `startRiding` does). `AbstractChestBoat.interact` opens the chest instead of
/// passing, unless it is out of control, not sneaked at and has room.
fn boat_interact_passes(chest: bool, sneaking: bool, out_of_control: bool, has_room: bool) -> bool {
    if chest {
        !sneaking && out_of_control && has_room
    } else {
        sneaking || out_of_control
    }
}

/// Whether `Item.use` usually acts on this item (eats, drinks, equips, raises,
/// throws, shoots, fills or places), which ends `Minecraft.startUseItem` at
/// its hand. Tools and materials pass, and the off hand gets its turn. The
/// server decides; this is the client's guess of whether to go on. A carrot or warped fungus
/// on a stick is not here: `FoodOnAStickItem.use` passes on a client, and only the server's
/// answer to a boost that can run swings the hand.
pub(super) fn item_has_use(info: &horizon_client::items::ItemInfo) -> bool {
    let id = info.id.trim_start_matches("minecraft:");
    id.ends_with("_spear")
        || info.is("minecraft:fishing_rod") // a content mod's rod too
        || info.consumable.is_some()
        || (info.equip_slot.is_some() && info.swappable)
        || info.blocks_attacks.is_some()
        || horizon_client::items::bucket_contents(&info.id).is_some()
        || id.ends_with("_boat")
        || id.ends_with("_raft")
        || id.ends_with("_bundle")
        || matches!(
            id,
            "bow"
                | "crossbow"
                | "trident"
                | "spyglass"
                | "snowball"
                | "egg"
                | "blue_egg"
                | "brown_egg"
                | "ender_pearl"
                | "ender_eye"
                | "wind_charge"
                | "splash_potion"
                | "lingering_potion"
                | "experience_bottle"
                | "bucket"
                | "glass_bottle"
                | "map"
                | "writable_book"
                | "written_book"
                | "goat_horn"
                | "bundle"
                | "brush"
                | "knowledge_book"
        )
}

/// The view distance a client asks for: its render distance, which Horizon
/// lets go past vanilla's 32 (a vanilla server still caps it at its own).
fn client_view_distance(render_distance: i32) -> i8 {
    render_distance.clamp(2, i32::from(i8::MAX)) as i8
}

/// `ClientPacketListener.handleBlockEntityData` loads the tag into the existing
/// block entity, so the state the client builds itself stays: copied from `old`
/// into `new`, the block entity decoded from the tag.
fn keep_client_state(old: Option<&BlockEntity>, new: &mut BlockEntity) {
    match (old, new) {
        // The turn of a vault's item and a trial spawner's mob (VaultClientData).
        (Some(BlockEntity::Vault(old)), BlockEntity::Vault(new)) => {
            new.spin = old.spin;
            new.o_spin = old.o_spin;
        }
        (Some(BlockEntity::TrialSpawner(old)), BlockEntity::TrialSpawner(new)) => {
            new.spin = old.spin;
            new.o_spin = old.o_spin;
        }
        // A beacon's beam (`BeaconBlockEntity.loadAdditional` reads only the
        // powers): a fresh one draws nothing until the next tick walks it again.
        (Some(BlockEntity::Beacon(old)), BlockEntity::Beacon(new)) => {
            new.sections.clone_from(&old.sections);
            new.checking.clone_from(&old.checking);
            (new.last_check_y, new.levels) = (old.last_check_y, old.levels);
        }
        _ => {}
    }
}

/// `LocalPlayer.onSyncedDataUpdated(DATA_LIVING_ENTITY_FLAGS)`: the server's
/// flags (1 using, 2 the off hand) start a use the client is not running
/// (`startUsingItem`, which needs an item in that hand), and stop one it is
/// (`stopUsingItem`). `using` is the slot in use and its ticks.
fn synced_local_use(
    using: Option<(usize, u32)>,
    flags: i8,
    selected: usize,
    has_item: impl Fn(usize) -> bool,
) -> Option<(usize, u32)> {
    let server_using = flags & 1 != 0;
    match using {
        None if server_using => {
            let slot = if flags & 2 != 0 {
                super::shield::OFFHAND_SLOT
            } else {
                selected
            };
            has_item(slot).then_some((slot, 0))
        }
        Some(_) if !server_using => None,
        using => using,
    }
}

/// The game as the tick's input sees it.
struct RemoteInputWorld<'g, 'a>(&'g mut Game<'a>);

struct RemoteUseWorld<'g, 'a>(&'g mut Game<'a>, mod_api::InteractionAttempt);

impl horizon_client::interaction_input::TargetUseWorld for RemoteUseWorld<'_, '_> {
    fn use_entity(&mut self) -> bool {
        self.0.remote_use_entity(Hand::Main)
    }
    fn use_block(&mut self) -> bool {
        self.0.remote_use_block(Hand::Main)
    }
    fn use_item(&mut self) {
        self.0.remote_use_hands(self.1);
    }
}

impl horizon_client::interaction_input::InteractionWorld for RemoteInputWorld<'_, '_> {
    fn pick_hit_result(&mut self) {
        self.0.pick_tick_targets();
    }
    fn release_hit_result(&mut self) {
        self.0.release_tick_targets();
    }
    fn using_item(&self) -> bool {
        self.0.using.is_some()
    }
    fn release_using(&mut self) {
        if let Some((slot, ticks)) = self.0.using
            && self.0.mod_use_tick(slot, ticks, true)
        {
            return;
        }
        self.0.release_remote_use();
    }
    fn start_attack(&mut self) -> bool {
        self.0.remote_start_attack()
    }
    fn start_use(&mut self, held: bool) {
        let attempt = if held {
            mod_api::InteractionAttempt::Held
        } else {
            mod_api::InteractionAttempt::Press
        };
        self.0.remote_use(attempt);
    }
    fn pick(&mut self) {
        // Minecraft.pickBlockOrEntity: an entity before the block is asked
        // for (`handlePickItemFromEntity`), else the block.
        if let Some(entity) = self.0.pick_entity_hit() {
            if let Some(id) = entity {
                if self
                    .0
                    .native_interaction(
                        InteractionKey::PickBlock,
                        Hand::Main,
                        mod_api::InteractionAttempt::Press,
                    )
                    .canceled
                {
                    return;
                }
                self.0.send(S::PickItemFromEntity(play::PickItemFromEntity {
                    id,
                    include_data: self.0.has_control_down(),
                }));
            }
        } else if let Some(hit) = self.0.pick() {
            if self
                .0
                .native_interaction(
                    InteractionKey::PickBlock,
                    Hand::Main,
                    mod_api::InteractionAttempt::Press,
                )
                .canceled
            {
                return;
            }
            self.0.send(S::PickItemFromBlock(play::PickItemFromBlock {
                pos: hit.pos,
                include_data: self.0.has_control_down(),
            }));
        }
    }
    fn use_delay(&self) -> u32 {
        self.0.use_delay
    }
    fn continue_attack(&mut self, down: bool) {
        self.0.remote_continue_attack(down);
    }
}

#[derive(Default)]
pub(super) struct ClientMenu {
    current: Option<(i32, String)>,
    pub(super) generation: u64,
}

impl ClientMenu {
    pub(super) fn set(&mut self, current: Option<(i32, String)>) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("menu generation exhausted");
        self.current = current;
    }

    pub(super) fn as_ref(&self) -> Option<&(i32, String)> {
        self.current.as_ref()
    }
}

impl Remote {
    pub(super) fn set_menu_title(&mut self, title: Option<horizon_client::text::Text>) {
        self.menu_title_snapshot = title.try_into_api().ok().flatten().map(Arc::new);
        self.menu_title = title;
    }

    pub(super) fn menu_title_snapshot(&self) -> Option<&mod_api::Text> {
        self.menu_title_snapshot.as_deref()
    }

    pub(super) fn shared_menu_title_snapshot(&self) -> Option<Arc<mod_api::Text>> {
        self.menu_title_snapshot.clone()
    }

    /// Resolve registry keys only when registries or the dimension change.
    fn resolve_clocks(&mut self) {
        self.overworld_clock = self
            .registries
            .id("minecraft:world_clock", "minecraft:overworld");
        self.default_clock = self
            .dimension
            .default_clock
            .as_deref()
            .and_then(|name| self.registries.id("minecraft:world_clock", name));
        for id in [self.overworld_clock, self.default_clock]
            .into_iter()
            .flatten()
        {
            self.clocks.entry(id).or_insert(play::ClockState {
                clock: id,
                total_ticks: 0,
                partial_tick: 0.0,
                rate: 1.0,
            });
        }
    }

    fn clock_time(&self, id: Option<i32>) -> i64 {
        id.and_then(|id| self.clocks.get(&id))
            .map_or(0, |c| c.total_ticks)
    }

    pub(super) fn overworld_clock_time(&self) -> i64 {
        self.clock_time(self.overworld_clock)
    }

    /// `Minecraft.level != null`: from `handleLogin` until the next
    /// `handleConfigurationStart` (`clearClientLevel`). Minecraft.tick sends
    /// CLIENT_TICK_END only then, so a client waiting for the server's Login
    /// (a busy server takes seconds to place a new player) sends nothing.
    pub(in crate::game) fn has_level(&self) -> bool {
        client_has_level(self.logged_in, self.reconfiguring)
    }
    /// An entity the server added to this client level: its type
    /// (`minecraft:text_display`) and its position as last sent.
    pub(super) fn tracked_entity(&self, id: i32) -> Option<(&str, [f64; 3])> {
        self.entities
            .get(&id)
            .map(|t| (t.kind.as_str(), t.position))
    }
    /// The profile of the player entity `id`: this window's own player, or
    /// another one by the UUID it was added with and its profile name.
    pub(super) fn level_player(&self, id: i32) -> Option<(u128, Option<&str>)> {
        if id == self.entity_id {
            return Some((self.profile, Some(self.profile_name.as_str())));
        }
        let uuid = *self.level_players.get(&id)?;
        Some((uuid, self.level_player_names.get(&id).map(String::as_str)))
    }
    /// `ClientPacketListener.getSeenPlayers`: the player-info entries of the
    /// players this connection has seen, without this client's own profile.
    pub(super) fn seen_players_other_than_self(&self) -> usize {
        self.seen_players
            .iter()
            .filter(|id| **id != self.profile)
            .count()
    }
    /// `PlayerInfo.getMessageValidator().updateAndValidate(message)` for `message.sender`.
    fn validate_chat(
        &mut self,
        message: &secure_chat::Incoming,
        now: i64,
    ) -> Option<secure_chat::Validated> {
        let sender = message.sender;
        if let Some(validator) = self.chat_validators.get_mut(&sender) {
            return validator.update_and_validate(message, now);
        }
        let (mut validator, settled) = self.new_chat_validator(sender);
        let validated = validator.update_and_validate(message, now);
        // While the services' keys are still on their way the validator is a stopgap, not kept.
        if settled {
            self.chat_validators.insert(sender, validator);
        }
        validated
    }
    /// `ClientPacketListener.initializeChatSession` for `sender`'s entry: a session the services'
    /// keys vouch for makes a keyed validator, anything else the fallback of
    /// `enforcesSecureChat()` (`canValidateProfileKeys() && serverEnforcesSecureChat`). `false`
    /// while the keys are not known yet.
    fn new_chat_validator(&self, sender: u128) -> (secure_chat::MessageValidator, bool) {
        let keys = self.services_keys.lock().unwrap_or_else(|e| e.into_inner());
        let (can_validate, settled) = match &*keys {
            ServicesKeys::Ready(_) => (true, true),
            ServicesKeys::Missing => (false, true),
            ServicesKeys::Pending => (false, false),
        };
        let fallback =
            secure_chat::MessageValidator::fallback(can_validate && self.enforces_secure_chat);
        let (ServicesKeys::Ready(keys), Some(Some((id, expires_at, key, signature)))) =
            (&*keys, self.chat_sessions.get(&sender))
        else {
            if matches!(&*keys, ServicesKeys::Missing)
                && self.chat_sessions.get(&sender).is_some_and(Option::is_some)
            {
                log::warn!(
                    "Ignoring chat session from {sender:032x} due to missing Services public key"
                );
            }
            return (fallback, settled);
        };
        let profile_key = secure_chat::ProfileKey {
            expires_at: *expires_at,
            key: key.clone(),
            signature: signature.clone(),
        };
        match secure_chat::ChatSession::validate(*id, profile_key, keys, sender) {
            Ok(session) => (secure_chat::MessageValidator::keyed(session), true),
            Err(why) => {
                log::error!("Failed to validate profile key for player {sender:032x}: {why}");
                (fallback, true)
            }
        }
    }
    /// How far the server sends chunks: its view distance and the ring
    /// around (`ChunkTrackingView`), at least two.
    pub(super) fn sent_radius(&self) -> i32 {
        self.view_distance.max(2).saturating_add(1)
    }
    /// A player entity's profile name: the one it was created with, else its
    /// player-list entry's (an entity the list named after it spawned).
    fn level_player_name(&self, id: i32, uuid: &u128) -> Option<&str> {
        self.level_player_names
            .get(&id)
            .or_else(|| self.players.get(uuid))
            .map(String::as_str)
    }
    /// Listed UUIDs always have a tab entry; removal clears both together.
    pub(super) fn listed_player_count(&self) -> usize {
        self.listed.len()
    }
    /// The local player's profile UUID.
    pub(super) fn profile(&self) -> u128 {
        self.profile
    }
    /// Another player's profile UUID, by their entity id (None for the local player).
    pub(super) fn player_uuid(&self, id: u32) -> Option<u128> {
        let uuid = *self.level_players.get(&(id as i32))?;
        (uuid != self.profile).then_some(uuid)
    }
    /// The entity id of a player in the level, by profile UUID.
    pub(super) fn player_entity(&self, uuid: u128) -> Option<u32> {
        self.level_players
            .iter()
            .find(|(_, u)| **u == uuid)
            .map(|(id, _)| *id as u32)
    }
    /// The kind and last sent position of a tracked server entity.
    pub(super) fn tracked(&self, id: i32) -> Option<(&str, [f64; 3])> {
        self.entities.get(&id).map(|t| (&*t.kind, t.position))
    }
}

fn object_skin_changed(published: &HashMap<String, String>, key: &str, skin: &str) -> bool {
    published.get(key).is_none_or(|previous| previous != skin)
}

/// The textures to resolve skins from: every list entry's, and for a player
/// entity whose entry is gone the profile it kept. `kept` first takes, for each
/// player entity without one, its entry's textures (`AbstractClientPlayer.getPlayerInfo`
/// looks the PlayerInfo up once and holds it, removed from the list or not).
fn skin_entries(
    level_players: &HashMap<i32, u128>,
    listed: &HashMap<u128, horizon_client::skins::Textures>,
    kept: &mut HashMap<i32, horizon_client::skins::Textures>,
) -> Vec<(u128, horizon_client::skins::Textures)> {
    for (id, uuid) in level_players {
        if !kept.contains_key(id)
            && let Some(textures) = listed.get(uuid)
        {
            kept.insert(*id, textures.clone());
        }
    }
    let mut entries: Vec<_> = listed.iter().map(|(u, t)| (*u, t.clone())).collect();
    for (id, textures) in kept.iter() {
        if let Some(uuid) = level_players.get(id)
            && !listed.contains_key(uuid)
            && !entries.iter().any(|(u, _)| u == uuid)
        {
            entries.push((*uuid, textures.clone()));
        }
    }
    entries
}

/// `AttributeInstance.calculateValue` before the attribute's range: the base,
/// `ADD_VALUE`s, then `ADD_MULTIPLIED_BASE`s of that, then each
/// `ADD_MULTIPLIED_TOTAL`.
fn snapshot_value(s: &play::AttributeSnapshot) -> f64 {
    let amounts = |op: i32| {
        s.modifiers
            .iter()
            .filter(move |m| m.operation == op)
            .map(|m| m.amount)
    };
    let base = s.base + amounts(0).sum::<f64>();
    let mut value = base + amounts(1).map(|a| base * a).sum::<f64>();
    for amount in amounts(2) {
        value *= 1.0 + amount;
    }
    value
}

/// `Util.getMillis` for the level load tracker's timeouts.
fn client_millis() -> u64 {
    static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    ORIGIN
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// What `ChatScreen.handleChatInput` sends of typed chat once `ClientChatEvent` (`ask`, with
/// the outgoing message and the cancel flag) had its say: nothing when canceled or emptied.
fn chat_after_event(message: &str, ask: impl FnOnce(&mut String, &mut bool)) -> Option<String> {
    let mut outgoing = message.to_owned();
    let mut cancel = false;
    ask(&mut outgoing, &mut cancel);
    (!cancel && !outgoing.is_empty()).then_some(outgoing)
}
/// `Entity.isOnFire` for the window's player: while the server's flag is set
/// the predicted tick's countdown never takes `fire_ticks` below 1.
fn hold_server_fire(fire_ticks: i32, server_on_fire: bool) -> i32 {
    if server_on_fire {
        fire_ticks.max(1)
    } else {
        fire_ticks
    }
}

/// `BlockStatePredictionHandler.ServerVerifiedState`: the sequence of the action
/// that last changed the block, the state the server last said it has, and where
/// the player stood when this client first predicted a change there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Prediction {
    pub sequence: i32,
    pub state: i32,
    pub player_pos: [f64; 3],
}

/// `Entity.isColliding`: whether the collision `shape` of a block at `pos`
/// meets `entity`'s box. Shapes are block-local, so they move onto the block
/// first (`VoxelShape.move`), and any box that overlaps is a collision
/// (`Shapes.joinIsNotEmpty` with `BooleanOp.AND`).
pub(super) fn shape_meets(
    shape: &horizon_core::geometry::VoxelShape,
    pos: [i32; 3],
    entity: horizon_core::geometry::Aabb,
) -> bool {
    let at = Vec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
    shape
        .boxes()
        .iter()
        .any(|b| b.move_by(at).intersects(entity))
}

/// `ClientLevel.syncBlockState`'s snap: a restored block that meets the player
/// sends it back to the position the prediction remembered, unless the player
/// was teleported after that prediction was made
/// (`lastTeleportSequence < sequence`: the remembered position is stale).
pub(super) fn snap_target(
    recorded: [f64; 3],
    last_teleport_sequence: i32,
    sequence: i32,
    meets: bool,
) -> Option<[f64; 3]> {
    (meets && last_teleport_sequence < sequence).then_some(recorded)
}

/// [`Remote::has_level`]: logged in (a Login arrived) and no reconfiguration
/// since (`reconfiguring` is set by StartConfiguration, cleared by the next
/// Login).
fn client_has_level(logged_in: bool, reconfiguring: Option<u64>) -> bool {
    logged_in && reconfiguring.is_none()
}

/// `ClientChunkCache.Storage.inRange`: whether column `x`, `z` is within `calculateStorageRange`
/// (the server's radius, at least two, plus three) of the view centre. The radius and the centre
/// are the server's to send, any `i32`: the distances are taken without overflow, which panicked
/// a debug build and, wrapping in release, dropped every loaded chunk.
fn in_storage_range(view_distance: i32, center: (i32, i32), x: i32, z: i32) -> bool {
    let range = view_distance.max(2).saturating_add(3).unsigned_abs();
    x.abs_diff(center.0) <= range && z.abs_diff(center.1) <= range
}

// A packet's removals (`RemoveEntities`), apart: this file may not grow.
#[cfg(test)]
mod interaction_event_tests;

#[cfg(test)]
mod native_entity_tests;

#[cfg(test)]
mod effect_blend_tests;
#[cfg(test)]
mod level_sound_tests;

#[cfg(test)]
mod attachment_query_tests;
#[cfg(test)]
mod passage_cross_tests;
#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod movement_input_tests;
#[cfg(test)]
#[path = "player_tick_tests.rs"]
mod player_tick_tests;
