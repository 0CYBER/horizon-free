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
mod remote_attributes;
mod tables;

pub(super) use super::villager_client::builtin_registries;

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
    /// same block and item sends no new START and the server keeps its clock (#4124).
    is_destroying: bool,
    /// Per-hand swing policy, restored when the current input attempt ends.
    pub(in crate::game) interaction_swing: [bool; 2],
    destroy_delay: u32,
    pub(super) input: horizon_client::interaction_input::InteractionInput,
    /// Keys whose presses send a packet (swap hands, drop, a mount's
    /// inventory): like the click queue, they wait for the tick's
    /// `handleKeybinds` phase rather than going out between window events.
    pub(super) pending_keys: Vec<KeyCode>,
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
    /// Received events a frame left for the next one, in order, and when
    /// the oldest of them first waited (`poll_remote`).
    backlog: VecDeque<Received>,
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

impl Game<'_> {
    /// A `Holder<SoundEvent>` off the wire: an inline event's location, or
    /// the name of a registered one.
    pub(super) fn remote_sound_name(
        &self,
        sound: &(i32, Option<(String, Option<f32>)>),
    ) -> Option<String> {
        match &sound.1 {
            Some((location, _)) => Some(location.clone()),
            None => self.remote.as_ref().and_then(|r| {
                r.builtin
                    .get("minecraft:sound_event")?
                    .name(sound.0)
                    .map(str::to_owned)
            }),
        }
    }
    pub(super) fn remote_sound_entity_state(
        &mut self,
        id: i32,
    ) -> Option<horizon_client::sound_engine::SoundEntityState> {
        let r = self.remote.as_ref()?;
        let player = self.player.as_ref().map(|p| {
            (
                r.entity_id,
                [p.position.x, p.position.y, p.position.z],
                r.local_entity_silent,
            )
        });
        // The local player is not in mobs, so avoid a guaranteed index miss.
        if player.is_some_and(|p| p.0 == id) {
            return entity_sound_state(id, player, None, &r.entities, None);
        }
        let mob = self.mob_index.get(&self.mobs, id as u32).map(|m| {
            horizon_client::sound_engine::SoundEntityState {
                position: [m.position.x, m.position.y, m.position.z],
                silent: m.silent,
                removed: m.removed,
            }
        });
        let dragon = self
            .dragon
            .as_ref()
            .map(|d| (d.id as i32, [d.position.x, d.position.y, d.position.z]));
        let mut state = entity_sound_state(id, player, mob, &r.entities, dragon)?;
        // Mob states already use the live position. In particular,
        // level_entity would find the dragon's stale Mob before its Dragon.
        if mob.is_some() || dragon.is_some_and(|d| d.0 == id) {
            return Some(state);
        }
        // Vehicles, projectiles and other simulated objects may have moved
        // since the last tracking packet.
        if let Some(position) = self
            .level_entity(id)
            .and_then(|target| self.target_position(target))
        {
            state.position = position;
        }
        Some(state)
    }
    /// The hurt sound the hurt player's own client plays from
    /// `ClientboundDamageEventPacket` (`LivingEntity.handleDamageEvent`):
    /// `Player.getHurtSound` is the damage type's effects, with the codec's
    /// `hurt` default when the type is unknown.
    pub(super) fn remote_hurt_sound(&self, source_type: i32) -> &'static str {
        match self.remote.as_ref() {
            Some(r) => hurt_sound_for(&r.registries, &self.health_resources.damage, source_type),
            None => horizon_client::health::hurt_sound("hurt"),
        }
    }
    /// Leave the title for a server (ConnectScreen): an empty world fed by the connection.
    pub(super) fn connect_server(&mut self, address: &str, name: &str) -> Result<()> {
        self.connect_server_with_cookies(address, name, None)
    }
    /// `connect_server` carrying a transfer's stored server cookies
    /// (`TransferState`); None on an ordinary join.
    pub(super) fn connect_server_with_cookies(
        &mut self,
        address: &str,
        name: &str,
        cookies: Option<horizon_client::net::Cookies>,
    ) -> Result<()> {
        self.connect_server_with_options(address, name, cookies, false, false)
    }
    /// `ConnectScreen.startConnecting`: keep LAN discovery's server kind through loading.
    pub(super) fn connect_server_with_options(
        &mut self,
        address: &str,
        name: &str,
        cookies: Option<horizon_client::net::Cookies>,
        lan: bool,
        realm: bool,
    ) -> Result<()> {
        super::world_loading::preload_server_loot(
            &self.options.resources,
            &self.res.core.item_data,
        );
        if self.defer_world_entry(super::PendingWorldEntry::Server {
            address: address.to_owned(),
            name: name.to_owned(),
            cookies: cookies.clone(),
            lan,
            realm,
        })? {
            return Ok(());
        }
        self.join_world_rest()?;
        self.client_logging_out();
        if let Err(error) = self.join_server(address, name, None, cookies, lan, realm) {
            log::warn!("Horizon: could not join server: {error}");
            let skip = self.settings.file.bool("skipMultiplayerWarning");
            match &mut self.title {
                Some(title) => {
                    title.skip_multiplayer_warning = skip;
                    title.failed(error);
                }
                None => {
                    let mut title = self
                        .new_title(std::env::current_dir()?.join("saves"))
                        .with_user(|| self.options.player_name());
                    title.skip_multiplayer_warning = skip;
                    title.failed(error);
                    self.show_title(title);
                }
            }
            self.capture(false);
        }
        Ok(())
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
        self.realm_session = realm;
        self.reset_tutorial();
        self.perf.world_opening();
        // A server's own biomes come over the connection; the last local
        // world's packed table must not name its ids. The window's own
        // server thread keeps the one it just set.
        if local.is_none() {
            self.world_biomes = None;
            self.world_biomes_base = None;
        }
        let root = self.options.resources.clone();
        let item_data = Arc::clone(&self.res.core.item_data);
        let builtin = builtin_registries(&root).ok_or("Resources are incomplete: registries.json is missing or invalid. Click \"Prepare again\" in the launcher (or run horizon-prepare), then try again.")?;
        let lang = Arc::new(horizon_client::net::load_language(&root));
        // Options.createInformation: what the server learns about this player,
        // sent again when one of those options changes.
        let info = self
            .settings
            .client_information(client_view_distance(self.settings.render_distance));
        if local.is_none() && !self.options.split_guest {
            mod_api::content::crash::set_detail(
                "Server",
                horizon_client::net::shown_address(address),
            );
        }
        // A signed-in account always plays under its own name; a splitscreen player signs in
        // with its own account when it picked one.
        let auth = self.auth();
        let name = auth.as_ref().map_or(name, |a| a.name.as_str());
        let user = auth.as_ref().map_or_else(
            || {
                (
                    name.to_owned(),
                    horizon_protocol::crypto::offline_uuid(name),
                )
            },
            |a| (a.name.clone(), a.uuid),
        );
        self.server_packs_connect(local.is_none().then_some(address), user);
        let session = match local {
            Some(pipe) => Session::connect_local(
                pipe,
                name,
                auth.clone(),
                info.clone(),
                lang.clone(),
                item_data.clone(),
            ),
            None => {
                self.extras_connecting(address, name, lan);
                match cookies {
                    Some(cookies) => Session::connect_transferred(
                        address,
                        name,
                        auth.clone(),
                        info.clone(),
                        lang.clone(),
                        cookies,
                        item_data.clone(),
                    ),
                    None => {
                        let connect = if lan {
                            Session::connect_lan
                        } else {
                            Session::connect
                        };
                        connect(
                            address,
                            name,
                            auth.clone(),
                            info.clone(),
                            lang.clone(),
                            item_data.clone(),
                        )
                    }
                }
            }
        };
        let player_type = super::perspective::player_type(&root);
        let mut streaming = Streaming::new(
            Arc::new(NoTerrain),
            self.worker_context(),
            self.res.threads(),
        );
        streaming.remote = true;
        streaming.radius = self.settings.render_distance;
        self.streaming = streaming;
        self.world = Game::world_for_mods(self.res.facts().air, 0);
        self.save = Arc::new(SaveDir::new(
            std::env::temp_dir().join("horizon-remote-unused"),
            horizon_client::save::LevelShape::OVERWORLD,
        ));
        self.inventory = vec![None; 36];
        self.offhand = None;
        self.armor = [None, None, None, None];
        self.craft_grid = vec![None; 9];
        self.craft_size = 2;
        self.remove_local_player_body();
        self.spawn = None;
        self.mobs.clear();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.creatures.projectiles.clear();
        self.dropped.clear();
        self.clear_client_particles();
        self.falling.clear();
        self.mining = None;
        self.container = None;
        self.carried = None;
        self.using = None;
        self.end_mod_use();
        self.clicks.miss_time = 0;
        // A new LocalPlayer has no effects: the server sends what it has
        // (ClientboundUpdateMobEffectPacket on placement).
        self.potions = Default::default();
        self.set_mode_id(0);
        self.experience = Experience {
            level: 0,
            progress: 0.0,
            total: 0,
        };
        // The connected client never opens a server world (single player also
        // joins here), so load the registries its argument parsers and local
        // suggestions need before receiving the server's command tree.
        self.ui.registries = Default::default();
        self.build_registries();
        self.ui.registries.permission = 0;
        self.ui.chat.clear();
        self.set_screen(Screen::Playing);
        self.hide_title();
        self.dim = Dim::Overworld;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        self.apply_settings();
        self.clear_waypoints();
        self.boss_overlay.reset();
        // The chat's tree until the server sends its own, or forever when it
        // sends none: the game's own, plus the commands native mods own.
        let chat_tree = self.chat_tree_for(None);
        // Minecraft.services() has the keys wherever the game is played, single player's own
        // server included: its guests' chat keys are checked against them.
        let services_keys = Arc::new(std::sync::Mutex::new(ServicesKeys::Pending));
        let slot = services_keys.clone();
        let _ = std::thread::Builder::new()
            .name("Services keys".into())
            .spawn(move || {
                use horizon_server_files::profiles;
                let keys = profiles::fetch_profile_key_keys(&profiles::discovery_url());
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = match keys {
                    Ok(keys) if !keys.is_empty() => ServicesKeys::Ready(keys),
                    Ok(_) => ServicesKeys::Missing,
                    Err(why) => {
                        log::error!("Horizon: {why}");
                        ServicesKeys::Missing
                    }
                };
            });
        self.ui.registries.selector_tokens.clear();
        self.remote = Some(Remote {
            selector_metadata_supported: true,
            professions: self.client_professions()?,
            session,
            brand: "unknown".into(),
            recipe_viewer: Default::default(),
            features: ["minecraft:vanilla".to_string()].into(),
            quiet_commands: false,
            custom_books: false,
            logged_out: false,
            attachments: HashMap::new(),
            far_terrain: None,
            builtin,
            status: lang
                .get("connect.connecting")
                .cloned()
                .unwrap_or_else(|| "Connecting to the server...".into()),
            lang,
            registries: Arc::new(Dynamic::default()),
            tags: None,
            enchantments: None,
            joined: false,
            reconfiguring: None,
            logged_in: false,
            entity_id: 0,
            local_entity_silent: false,
            camera: None,
            use_item: None,
            dimension: DimensionType::vanilla("minecraft:overworld"),
            level: "minecraft:overworld".into(),
            level_key: mod_api::DimensionKey::OVERWORLD,
            level_facts: None,
            level_environment: None,
            facts_cache: Default::default(),
            added_levels: Default::default(),
            viewed: Default::default(),
            hashed_seed: 0,
            chunk_format: Arc::new(ChunkFormat::new(
                &DimensionType::vanilla("minecraft:overworld"),
                Vec::new(),
                7,
            )),
            decoder: ChunkDecoder::new(),
            view_distance: self.settings.render_distance,
            simulation_distance: self.settings.simulation_distance,
            sequence: 0,
            last_teleport_sequence: -1,
            position_sender: Default::default(),
            last_input: 0,
            was_sprinting: false,
            carried: std::cell::Cell::new(None),
            was_shared_flags: 0,
            was_living_flags: 0,
            last_recorded_pose: None,
            abilities: None,
            loaded_sent: false,
            heard_level: None,
            level_load: horizon_client::level_loading::LevelLoadTracker::new(0),
            sent_info: info,
            predictions: HashMap::new(),
            menu: ClientMenu::default(),
            ticked_before_player: std::collections::HashSet::new(),
            craft_result: None,
            menu_title: None,
            menu_title_snapshot: None,
            state_id: 0,
            use_target: None,
            menu_extra: None,
            entities: HashMap::new(),
            tick_fluids: Vec::new(),
            tick_rain: Vec::new(),
            tick_standings: Vec::new(),
            tick_water_fx: Vec::new(),
            passenger_vehicles: HashMap::new(),
            lerps: HashMap::new(),
            cart_lerps: HashMap::new(),
            stepped: HashMap::new(),
            head_lerps: HashMap::new(),
            ghast_dismount: None,
            first_passenger: false,
            passenger_index: 0,
            pending_items: HashMap::new(),
            health: (20.0, 20, 5.0),
            flash_on_set_health: false,
            game_mode: 0,
            chat: Vec::new(),
            chat_received: 0,
            chat_queue: VecDeque::new(),
            signatures: Default::default(),
            chat_previous: None,
            next_chat_index: 0,
            enforces_secure_chat: false,
            services_keys,
            chat_sessions: HashMap::new(),
            chat_validators: HashMap::new(),
            joining_narrated: None,
            chat_tree,
            chat_tree_received: false,
            secure_chat: Default::default(),
            custom_completions: Default::default(),
            players: HashMap::new(),
            tab_players: HashMap::new(),
            skin_textures: HashMap::new(),
            own_profile: OwnProfile::Idle,
            skins: HashMap::new(),
            skins_revision: 0,
            published_skins: None,
            listed: Default::default(),
            online_mode: false,
            levels: Vec::new(),
            level_players: HashMap::new(),
            level_player_names: HashMap::new(),
            level_player_textures: HashMap::new(),
            glowing: Default::default(),
            own_invisible: false,
            own_on_fire: false,
            own_arrows: 0,
            own_stingers: 0,
            own_shoulders: [None; 2],
            profile: 0,
            profile_name: String::new(),
            destroying: None,
            destroying_item: None,
            is_destroying: false,
            interaction_swing: [true; 2],
            destroy_delay: 0,
            input: Default::default(),
            pending_keys: Vec::new(),
            swap_answer: [false; 2],
            stop_mining: false,
            placing: None,
            flat: false,
            death_message: None,
            show_death_screen: true,
            reduced_debug_info: false,
            do_limited_crafting: false,
            player_type,
            synced: vec![None; CREATIVE_SYNCED],
            decode_errors: 0,
            clocks: HashMap::new(),
            default_clock: None,
            overworld_clock: None,
            clock_game_time: 0,
            chunk_center: (0, 0),
            backlog: VecDeque::new(),
            backlog_since: None,
            server_links: Vec::new(),
            debug_query: Default::default(),
            transfer: None,
        });
        // The level's TickRateManager follows the server's ticking packets.
        self.clock = horizon_client::clock::FixedStep::client();
        // The player's advancements and statistics are the server's.
        self.adv = Default::default();
        self.stats = Default::default();
        Ok(())
    }

    /// Prepare a client session and remote state to receive packets from a studio replay.
    pub(super) fn init_remote_for_replay(&mut self) -> Result<()> {
        self.realm_session = false;
        self.reset_tutorial();
        self.perf.world_opening();
        self.world_biomes = None;
        self.world_biomes_base = None;
        let root = self.options.resources.clone();
        let builtin = builtin_registries(&root).ok_or("Resources are incomplete")?;
        let lang = Arc::new(horizon_client::net::load_language(&root));
        let info = self
            .settings
            .client_information(client_view_distance(self.settings.render_distance));
        let auth = self.auth();
        let name = auth.as_ref().map_or("ReplayPlayer", |a| a.name.as_str());
        let user = auth.as_ref().map_or_else(
            || {
                (
                    name.to_owned(),
                    horizon_protocol::crypto::offline_uuid(name),
                )
            },
            |a| (a.name.clone(), a.uuid),
        );
        self.server_packs_connect(None, user);
        let session = Session::replay(name);
        let player_type = super::perspective::player_type(&root);
        let mut streaming = Streaming::new(
            Arc::new(NoTerrain),
            self.worker_context(),
            self.res.threads(),
        );
        streaming.remote = true;
        streaming.radius = self.settings.render_distance;
        self.streaming = streaming;
        self.world = Game::world_for_mods(self.res.facts().air, 0);
        self.save = Arc::new(SaveDir::new(
            std::env::temp_dir().join("horizon-replay-unused"),
            horizon_client::save::LevelShape::OVERWORLD,
        ));
        self.inventory = vec![None; 36];
        self.offhand = None;
        self.armor = [None, None, None, None];
        self.craft_grid = vec![None; 9];
        self.craft_size = 2;
        self.remove_local_player_body();
        self.spawn = None;
        self.mobs.clear();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.creatures.projectiles.clear();
        self.dropped.clear();
        self.particles.clear();
        self.pickups.clear();
        self.orbs.clear();
        self.falling.clear();
        self.mining = None;
        self.container = None;
        self.carried = None;
        self.using = None;
        self.end_mod_use();

        self.set_mode_id(0);
        self.experience = Experience {
            level: 0,
            progress: 0.0,
            total: 0,
        };
        self.ui.registries = Default::default();
        self.build_registries();
        self.ui.registries.permission = 0;
        self.ui.chat.clear();
        self.set_screen(Screen::Playing);
        self.title = None;
        self.dim = Dim::Overworld;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        self.apply_settings();
        self.clear_waypoints();
        self.boss_overlay.reset();

        let chat_tree = self.chat_tree_for(None);
        let services_keys = Arc::new(std::sync::Mutex::new(ServicesKeys::Ready(Vec::new())));

        self.ui.registries.selector_tokens.clear();
        self.remote = Some(Remote {
            selector_metadata_supported: true,
            professions: self.client_professions()?,
            session,
            brand: "horizon-studio".into(),
            recipe_viewer: Default::default(),
            features: ["minecraft:vanilla".to_string()].into(),
            quiet_commands: false,
            custom_books: false,
            attachments: HashMap::new(),
            far_terrain: None,
            builtin,
            status: "Playing Studio Replay".into(),
            lang,
            registries: Arc::new(Dynamic::default()),
            tags: None,
            enchantments: None,
            joined: false,
            reconfiguring: None,
            logged_in: true,
            logged_out: false,
            local_entity_silent: false,
            entity_id: 0,
            camera: None,
            use_item: None,
            dimension: DimensionType::vanilla("minecraft:overworld"),
            level: "minecraft:overworld".into(),
            level_key: mod_api::DimensionKey::OVERWORLD,
            level_facts: None,
            level_environment: None,
            facts_cache: Default::default(),
            added_levels: Default::default(),
            viewed: Default::default(),
            hashed_seed: 0,
            chunk_format: Arc::new(ChunkFormat::new(
                &DimensionType::vanilla("minecraft:overworld"),
                Vec::new(),
                7,
            )),
            decoder: ChunkDecoder::new(),
            view_distance: self.settings.render_distance,
            simulation_distance: self.settings.simulation_distance,
            sequence: 0,
            last_teleport_sequence: -1,
            position_sender: Default::default(),
            last_input: 0,
            was_sprinting: false,
            was_shared_flags: 0,
            was_living_flags: 0,
            last_recorded_pose: None,
            carried: std::cell::Cell::new(None),
            abilities: None,
            loaded_sent: false,
            heard_level: None,
            level_load: horizon_client::level_loading::LevelLoadTracker::new(0),
            sent_info: info,
            predictions: HashMap::new(),
            menu: ClientMenu::default(),
            ticked_before_player: std::collections::HashSet::new(),
            craft_result: None,
            menu_title: None,
            menu_title_snapshot: None,
            state_id: 0,
            use_target: None,
            menu_extra: None,
            entities: HashMap::new(),
            tick_fluids: Vec::new(),
            tick_rain: Vec::new(),
            tick_standings: Vec::new(),
            tick_water_fx: Vec::new(),
            passenger_vehicles: HashMap::new(),
            lerps: HashMap::new(),
            cart_lerps: HashMap::new(),
            stepped: HashMap::new(),
            head_lerps: HashMap::new(),
            ghast_dismount: None,
            first_passenger: false,
            passenger_index: 0,
            pending_items: HashMap::new(),
            health: (20.0, 20, 5.0),
            flash_on_set_health: false,
            game_mode: 0,
            chat: Vec::new(),
            chat_received: 0,
            chat_queue: VecDeque::new(),
            signatures: Default::default(),
            chat_previous: None,
            next_chat_index: 0,
            enforces_secure_chat: false,
            services_keys,
            chat_sessions: HashMap::new(),
            chat_validators: HashMap::new(),
            joining_narrated: None,
            chat_tree,
            chat_tree_received: false,
            secure_chat: Default::default(),
            custom_completions: Default::default(),
            players: HashMap::new(),
            tab_players: HashMap::new(),
            skin_textures: HashMap::new(),
            own_profile: OwnProfile::Idle,
            skins: HashMap::new(),
            skins_revision: 0,
            published_skins: None,
            listed: Default::default(),
            online_mode: false,
            levels: Vec::new(),
            level_players: HashMap::new(),
            level_player_names: HashMap::new(),
            level_player_textures: HashMap::new(),
            glowing: Default::default(),
            own_invisible: false,
            own_on_fire: false,
            own_arrows: 0,
            own_stingers: 0,
            own_shoulders: [None; 2],
            profile: 0,
            profile_name: String::new(),
            destroying: None,
            destroying_item: None,
            is_destroying: false,
            interaction_swing: [true; 2],
            destroy_delay: 0,
            input: Default::default(),
            pending_keys: Vec::new(),
            swap_answer: [false; 2],
            stop_mining: false,
            placing: None,
            flat: false,
            death_message: None,
            show_death_screen: true,
            reduced_debug_info: false,
            do_limited_crafting: false,
            player_type,
            synced: vec![None; CREATIVE_SYNCED],
            decode_errors: 0,
            clocks: HashMap::new(),
            default_clock: None,
            overworld_clock: None,
            clock_game_time: 0,
            chunk_center: (0, 0),
            backlog: VecDeque::new(),
            backlog_since: None,
            server_links: Vec::new(),
            debug_query: Default::default(),
            transfer: None,
        });
        self.clock = horizon_client::clock::FixedStep::client();
        self.adv = Default::default();
        self.stats = Default::default();
        Ok(())
    }

    /// Disconnect and return to the multiplayer screen (DisconnectedScreen).
    pub(super) fn leave_server(&mut self, reason: Option<String>) -> Result<()> {
        if reason.is_none() && self.integrated.as_ref().is_some_and(|s| !s.stopped()) {
            return self.begin_closing_save(false);
        }
        self.client_logging_out();
        self.finish_perf();
        if reason.is_none() {
            // Leaving on purpose: nothing for auto-reconnect to retry.
            self.extras_left();
        }
        self.clear_waypoints();
        self.hud_disconnected();
        // Vanilla's Hud also ticks on the title and connect screens, so the
        // action bar has run out by the next world; Horizon's ticks in one.
        self.titles.overlay_time = 0;
        // GameRenderer.resetData and narrator.clear (Minecraft.disconnect).
        self.item_activation.reset();
        // Kept mod block entity geometry belongs to the world leaving.
        self.kept_block_entities.get_mut().clear();
        self.mod_view_distances.take();
        self.clear_narrator();
        // updateLevelInEngines(null): every sound stops and the music manager
        // lets go of its track. Leaving a server comes straight here, not
        // through finish_close_world, so the biome's ambient loop would
        // otherwise play on over the title screen until the next world.
        self.ambience.clear(&mut self.audio);
        self.music.stop_playing(None, &mut self.audio);
        self.audio.stop_all();
        self.mod_sounds.clear();
        // A world is leaving either way: its icon attempt goes with it.
        self.world_icon = None;
        // Minecraft.disconnect: the client level goes.
        if self.remote.is_some() {
            self.forget_all_remote_columns();
        }
        if let Some(old) = self.remote.as_mut().and_then(|r| r.heard_level.take()) {
            self.native_client_level_unload(&old);
        }
        if self.remote.is_some() {
            self.native_client_disconnecting();
        }
        if let Some(mut r) = self.remote.take() {
            r.session.disconnect();
        }
        super::lan::sync::forget_sync_data();
        self.gui.atlas.clear_object_skins();
        self.server_packs_disconnect();
        self.code_of_conduct = None;
        self.reset_tutorial();
        self.close_dialogs();
        // Gui.setScreen -> KeyMapping.releaseAll: the title or disconnect
        // screen releases every held key. Its release goes to that screen,
        // so a key kept here would still be down in the next world (#3246).
        self.keys.clear();
        self.left = false;
        self.right = false;
        // The shared world is closing: its invite code ends with it.
        self.friends_lan_closed();
        // Single player's server thread saves the world before the title shows.
        let single_player = match self.integrated.take() {
            Some(server) => {
                server.stop()?;
                true
            }
            None => false,
        };
        // A splitscreen player leaving says nothing of where P1 plays.
        if !self.options.split_guest {
            mod_api::content::crash::clear_detail("Server");
        }
        self.clock = Default::default();
        self.adv = Default::default();
        self.stats = Default::default();
        self.score.board = Default::default();
        self.streaming = Streaming::new(Arc::new(NoTerrain), self.worker_context(), 1);
        self.streaming.paused = true;
        self.reset_client_world(0);
        self.remove_local_player_body();
        // The vehicle went with the level: an id kept here would seat the
        // next login on an entity that is not there.
        self.riding = None;
        // Out of the server's bed: InBedChatScreen goes with the level.
        self.sleeping = None;
        self.sleep_timer = 0;
        self.clear_entities();
        self.score.board = Default::default();
        self.tab_list = Default::default();
        self.border = Default::default();
        self.set_screen(Screen::Playing);
        self.container = None;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        let saves = std::env::current_dir()?.join("saves");
        let mut title = self
            .new_title(saves.clone())
            .with_user(|| self.options.player_name());
        title.last_server = self.settings.file.string("lastServer");
        title.skip_multiplayer_warning = self.settings.file.bool("skipMultiplayerWarning");
        if single_player && reason.is_none() {
            self.show_title(title);
            self.capture(false);
            return Ok(());
        }
        title.mode = horizon_client::title::Mode::Multiplayer(match reason {
            Some(reason) => horizon_client::multiplayer::Screen::Disconnected {
                reason,
                retry: None,
                lines: std::cell::Cell::new(1),
            },
            None => horizon_client::multiplayer::Screen::list(&saves),
        });
        self.show_title(title);
        self.capture(false);
        Ok(())
    }
    /// `Hud.onDisconnected`, from `Minecraft.disconnect` and `clearClientLevel`
    /// (a proxy's reconfiguration too), but the chat: joining clears it and a
    /// reconfiguration keeps it (`ChatComponent.storeState`).
    fn hud_disconnected(&mut self) {
        self.tab_list.reset();
        self.boss_overlay.reset();
        self.adv.toasts.clear();
        self.debug_charts.reset();
        self.titles.clear();
        self.titles.reset_times();
    }

    pub(super) fn send(&self, packet: S) {
        if self.remote.is_some() {
            // MultiPlayerGameMode's attack, interact, useItem, useItemOn, releaseUsingItem, dropItem and
            // piercingAttack run ensureHasSentCarriedItem before their packet: a slot the number keys
            // chose this tick goes out first. A miss, a dig's start or a swap waits for the next tick.
            if let S::Attack(_) | S::Interact(_) | S::UseItem(_) | S::UseItemOn(_) = &packet {
                self.remote_send_carried();
            } else if let S::PlayerAction(a) = &packet
                && matches!(a.action, 4 | 5 | 6 | 8)
            {
                self.remote_send_carried();
            }
            self.send_play(packet);
        }
    }
    /// `Connection.send` for the game's play packets: a mod may cancel one
    /// (`client_packet_send`), and the server never hears of it. True when queued.
    pub(super) fn send_play(&self, packet: S) -> bool {
        let queued = self.remote.is_some() && !self.client_packet_send_canceled(&packet);
        if let (true, Some(r)) = (queued, &self.remote) {
            r.session.send(packet);
        }
        queued
    }
    fn send_action(&self, action: i32) {
        self.send(S::PlayerAction(play::PlayerAction {
            action,
            pos: [0; 3],
            direction: 0,
            sequence: 0,
        }));
    }
    /// `LocalPlayer.isShiftKeyDown` (and so `isSecondaryUseActive`): the input as of the player's
    /// last tick (`input.keyPresses`, what the last ServerboundPlayerInputPacket said), not the key
    /// now: a sneak pressed in the frame of a click acts from the next tick, as on the server.
    fn remote_shift_down(&self) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|r| r.last_input & SHIFT_INPUT != 0)
    }
    fn next_sequence(&mut self) -> i32 {
        let r = self.remote.as_mut().expect("remote");
        r.sequence += 1;
        r.sequence
    }
    fn item_name(&self, id: i32) -> Arc<str> {
        let r = self.remote.as_ref().expect("remote");
        Arc::from(
            r.builtin
                .get("minecraft:item")
                .and_then(|i| i.name(id))
                .unwrap_or("minecraft:air"),
        )
    }
    /// The server's network id of a mob effect (`brewing::EFFECTS` id).
    pub(super) fn remote_effect_network_id(&self, effect: &str) -> Option<i32> {
        let r = self.remote.as_ref()?;
        r.builtin
            .get("minecraft:mob_effect")?
            .id(&format!("minecraft:{effect}"))
    }
    /// A mob effect's network id as the effect (`brewing::EFFECTS` id).
    pub(super) fn remote_effect_id(&self, effect: i32) -> Option<&'static str> {
        self.remote_effect_def(effect).map(|def| def.id)
    }
    fn remote_effect_def(
        &self,
        effect: i32,
    ) -> Option<&'static horizon_client::brewing::EffectDef> {
        let r = self.remote.as_ref()?;
        let name = r.builtin.get("minecraft:mob_effect")?.name(effect)?;
        horizon_client::brewing::effect(name)
    }
    /// ClientPacketListener.handleUpdateMobEffect for the player itself
    /// (`LivingEntity.forceAddEffect`): the effect replaces any of its kind.
    /// Other entities' effects are not shown.
    fn remote_effect(&mut self, p: &play::UpdateMobEffect) {
        if self.remote.as_ref().is_none_or(|r| r.entity_id != p.id) {
            return;
        }
        let Some(def) = self.remote_effect_def(p.effect) else {
            return;
        };
        let id = def.id;
        let mut e = horizon_client::effects::EffectInstance::from_def(def, p.duration, p.amplifier);
        e.ambient = p.flags & 1 != 0;
        e.visible = p.flags & 2 != 0;
        e.show_icon = p.flags & 4 != 0;
        let list = &mut self.potions.effects.list;
        match list.iter_mut().find(|x| x.id == id) {
            // forceAddEffect's copyBlendState: an updated effect keeps its fade.
            Some(existing) => {
                e.client_blend = existing.client_blend;
                *existing = e;
            }
            None => {
                // ClientboundUpdateMobEffectPacket.shouldBlend (flag 8).
                let blend = p.flags & 8 != 0;
                if id == "darkness" {
                    self.sculk.darkness = super::sculk::darkness_blend_added(&e, blend);
                } else if id == "nausea" {
                    let (o_nausea, nausea) = screen_effects::nausea_blend_added(&e, blend);
                    self.screen_fx.o_nausea = o_nausea;
                    self.screen_fx.nausea = nausea;
                } else if !blend {
                    let has_effect = !e.ends_within(0);
                    e.client_blend = (has_effect, has_effect);
                }
                list.push(e);
            }
        }
    }
    fn item_id(&self, name: &str) -> i32 {
        let r = self.remote.as_ref().expect("remote");
        r.builtin
            .get("minecraft:item")
            .and_then(|i| i.id(name))
            .unwrap_or(0)
    }
    /// A stack from the server, decoded as the saved and command item NBT
    /// are, so tooltips and models see every field.
    pub(super) fn to_stack(&self, s: &Option<Stack>) -> Option<ItemStack> {
        let r = self.remote.as_ref()?;
        let stack = s.as_ref()?;
        if !horizon_protocol::item::custom::permitted(stack, r.registries.custom_components)
            .unwrap_or(false)
        {
            log::warn!("Dropping a stack with unnegotiated custom components");
            return None;
        }
        Some(horizon_client::item_sync::from_network(
            stack,
            &r.builtin,
            &r.registries,
        ))
    }
    /// A stack to the server (`SetCreativeModeSlot`): the same component
    /// patch a Horizon server sends its clients, with the registries' ids, so
    /// the server's echo doesn't hand back a bare item.
    fn to_network(&self, s: &Option<ItemStack>) -> Option<Stack> {
        let r = self.remote.as_ref()?;
        // A stack as the server sent it, only its count changed: its own
        // components go back, as a vanilla client keeps them.
        if let Some((stack, patch)) = s.as_ref().and_then(|s| Some((s, s.wire_patch()?))) {
            let mut out = Stack::new(self.item_id(&stack.item), stack.count as i32);
            let safe = r.registries.custom_components
                || horizon_protocol::item::custom::entries().next().is_none()
                || horizon_protocol::item::WirePatch::decode(&patch)
                    .and_then(|(added, removed)| {
                        let decoded = Stack {
                            added,
                            removed,
                            ..Stack::new(out.item, out.count)
                        };
                        horizon_protocol::item::custom::permitted(&decoded, false)
                    })
                    .unwrap_or(false);
            if safe {
                out.wire = horizon_protocol::item::WirePatch(Some(patch));
                return Some(out);
            }
            log::warn!("Cannot send cached custom components to this peer");
        }
        horizon_client::lan::guest::stack_to_network(
            |name| self.item_id(name),
            // Built-in first, as a hub's `SyncData::id` does: potions are one.
            |registry, name| {
                r.builtin
                    .get(registry)
                    .and_then(|ids| ids.id(name))
                    .or_else(|| r.registries.id(registry, name))
                    .or_else(|| {
                        (r.registries.custom_components
                            && registry == "minecraft:data_component_type")
                            .then(|| horizon_protocol::item::custom::id(name))
                            .flatten()
                    })
            },
            s,
        )
    }
    /// `HashedStack.create` with the connection's registries (the client's
    /// `decoratedHashOpsGenerator`).
    fn hashed_stack(&self, s: &Option<ItemStack>) -> Option<Hashed> {
        let r = self.remote.as_ref()?;
        let names = SyncedNames {
            builtin: &r.builtin,
            dynamic: &r.registries,
        };
        self.to_network(s).map(|s| Hashed::create(&s, &names))
    }
    /// A line in the connection's chat, with its styles, click and hover
    /// events (ChatComponent.addMessage). The lines the Chat option hides
    /// are dropped there (its `visibleMessageFilter`).
    pub(super) fn add_chat_line(
        &mut self,
        content: horizon_client::text::Text,
        source: horizon_client::chat::ChatSource,
    ) -> bool {
        self.signed_chat_line(content, source, None, ChatTrust::Secure)
    }
    /// `chat_line` of a player message, keeping its signature for a
    /// deletion (`GuiMessage.signature`).
    fn signed_chat_line(
        &mut self,
        content: horizon_client::text::Text,
        source: horizon_client::chat::ChatSource,
        signature: Option<Signature>,
        trust: ChatTrust,
    ) -> bool {
        if !self.ui.chat.shows(source) {
            return false;
        }
        let text = content.plain();
        let focused = self.screen == Screen::Chat;
        if source == horizon_client::chat::ChatSource::Player {
            self.ui.chat.add_signed(
                &self.gui.atlas,
                content,
                signature,
                trust.tag(),
                self.visual_ticks(),
                focused,
            );
        } else {
            // addServerSystemMessage and addClientSystemMessage both carry
            // GuiMessageTag.systemSinglePlayer: the grey bar at the line's left.
            let indicator = Some(horizon_client::chat::SYSTEM_INDICATOR);
            self.ui.chat.add(
                &self.gui.atlas,
                content,
                indicator,
                source,
                self.visual_ticks(),
                focused,
            );
        }
        if let Some(r) = self.remote.as_mut() {
            r.chat.push((text, Instant::now()));
            r.chat_received += 1;
            if r.chat.len() > 100 {
                r.chat.remove(0);
            }
        }
        true
    }

    /// `ChatListener.handleMessage` for player and disguised chat: held in the
    /// queue while Chat Delay is set, shown at once otherwise. `narration` is
    /// the chat type's narration of it (`decorateNarration`).
    fn handle_player_chat(&mut self, chat: QueuedChat) {
        if self.chat_delay_ms() > 0 {
            if let Some(r) = self.remote.as_mut() {
                r.chat_queue.push_back(chat);
            }
        } else {
            self.accept_chat(chat);
        }
    }

    /// `ChatListener.handleChatMessageError`: a message that fails validation shows as the red,
    /// italic `chat.validation_error` in its chat type (queued under Chat Delay as any
    /// message). Its signature counts as processed but not shown, for the acknowledgement the
    /// client sends the server. Vanilla also
    /// skips a blocked sender and one restricted to friends (not kept here either) and
    /// narrates the line.
    fn handle_chat_message_error(
        &mut self,
        chat_type: &play::ChatTypeBound,
        signature: Option<&Signature>,
    ) {
        if let Some(signature) = signature {
            self.mark_message_processed(signature, false);
        }
        let error = serde_json::json!({
            "translate": "chat.validation_error",
            "color": "red",
            "italic": true,
        });
        let line = self.decorate_chat(chat_type, &error, Decoration::Chat);
        // saySystemChatQueued(boundChatType.decorateNarration(CHAT_VALIDATION_ERROR)).
        let narration = self
            .decorate_chat(chat_type, &error, Decoration::Narration)
            .plain();
        self.handle_player_chat(QueuedChat {
            line,
            signature: None,
            trust: ChatTrust::Error,
            narration,
            sender: 0,
            chat_type: chat_type.chat_type,
            player: None,
        });
    }

    /// `ChatTrustLevel.evaluate` of a player message, with
    /// `ChatListener.evaluateTrustLevel`: the player's own message on their
    /// own server is secure. `decorated` is the line shown, `unsigned` the
    /// server's replacement for the signed content.
    fn chat_trust(
        &self,
        sender: u128,
        signed: bool,
        timestamp: i64,
        content: &str,
        unsigned: Option<&Tag>,
        decorated: &horizon_client::text::Text,
    ) -> ChatTrust {
        use horizon_client::scoreboard::{comp_text, tag_to_json};
        if self.integrated.is_some() && self.remote.as_ref().is_some_and(|r| r.profile == sender) {
            return ChatTrust::Secure;
        }
        // PlayerChatMessage.hasExpiredClient: MESSAGE_EXPIRES_AFTER_CLIENT is
        // the server's five minutes and two more.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let expired = now
            > timestamp.saturating_add(
                horizon_protocol::secure_chat::EXPIRES_AFTER_SERVER + 2 * 60 * 1000,
            );
        if !signed || expired {
            return ChatTrust::NotSecure;
        }
        // ChatTrustLevel.isModified: the signed content is not in the line, or
        // the replacement uses a font other than the default.
        let font_changed = unsigned.is_some_and(|tag| {
            comp_text(&tag_to_json(tag)).spans.iter().any(|span| {
                span.style
                    .font
                    .as_deref()
                    .is_some_and(|font| font != "minecraft:default")
            })
        });
        if !decorated.plain().contains(content) || font_changed {
            ChatTrust::Modified(content.to_owned())
        } else {
            ChatTrust::Secure
        }
    }

    /// `ClientPacketListener.handleDeleteChat`: the packed signature names a
    /// message; one still held back by Chat Delay never shows
    /// (`ChatListener.removeFromDelayedMessageQueue`), a shown one is
    /// replaced (`ChatComponent.deleteMessage`). A signature the cache
    /// doesn't hold is an invalid packet.
    fn handle_delete_chat(&mut self, packed: &(i32, Option<Vec<u8>>)) -> Result<()> {
        let Some(r) = self.remote.as_mut() else {
            return Ok(());
        };
        let Some(signature) = r.signatures.unpack(packed) else {
            let reason =
                horizon_client::text::Text::translate("multiplayer.disconnect.invalid_packet", &[]);
            return self.leave_server(Some(reason.plain()));
        };
        r.secure_chat.ignore_pending(&signature);
        let queued = r.chat_queue.len();
        r.chat_queue
            .retain(|chat| chat.signature.as_ref() != Some(&signature));
        if r.chat_queue.len() == queued {
            let ticks = self.visual_ticks();
            self.ui
                .chat
                .delete_message(&self.gui.atlas, &signature, ticks);
        }
        Ok(())
    }

    /// A message's `accept`: shown, it is narrated (`narrateChatMessage`) and
    /// starts the Chat Delay wait again.
    fn accept_chat(&mut self, chat: QueuedChat) -> bool {
        let signature = chat.signature;
        let shown = self.player_chat_line(chat);
        if let Some(signature) = &signature {
            self.mark_message_processed(signature, shown);
        }
        if shown && let Some(r) = self.remote.as_mut() {
            r.chat_previous = Some(Instant::now());
        }
        shown
    }

    /// `showMessageToPlayer`: the line joins the chat if Only Show Secure
    /// Chat, mods and the Chat option let it through, and the narrator says
    /// it (`sayChatQueued`).
    fn player_chat_line(&mut self, chat: QueuedChat) -> bool {
        let QueuedChat {
            mut line,
            signature,
            trust,
            narration,
            sender,
            chat_type,
            player,
        } = chat;
        // ChatListener.showMessageToPlayer: Only Show Secure Chat drops what is not secure.
        if matches!(trust, ChatTrust::NotSecure) && self.settings.file.bool("onlyShowSecureChat") {
            return false;
        }
        // handleChatMessageError says its line as system chat, and mods are not told of it.
        let error = matches!(trust, ChatTrust::Error);
        let (system, kind, player) = (false, Some(chat_type), player.as_ref());
        if !error && !self.chat_received(system, false, sender, kind, player, &mut line) {
            return false;
        }
        let source = horizon_client::chat::ChatSource::Player;
        let shown = self.signed_chat_line(line, source, signature, trust);
        if shown && error {
            self.say_system_chat_queued(&horizon_client::text::Text::untrusted(&narration));
        } else if shown {
            self.say_chat_queued(&narration);
        }
        shown
    }

    /// `ChatListener.messageDelay`: Chat Delay in milliseconds, as Java truncates it.
    fn chat_delay_ms(&self) -> i64 {
        (self.settings.chat_delay * 1000.0) as i64
    }

    /// `ChatListener.tick`: while paused the wait does not run down; with no
    /// Chat Delay the queue flushes; otherwise the next message the chat takes
    /// shows once the delay has passed since the last shown one.
    pub(super) fn tick_chat_queue(&mut self) {
        let delay = self.chat_delay_ms();
        // Minecraft.isPaused: a single-player pausing screen, not every pause menu.
        if self.pause {
            if delay > 0
                && let Some(r) = self.remote.as_mut()
            {
                r.chat_previous = r.chat_previous.map(|t| t + Duration::from_millis(50));
            }
            return;
        }
        if delay == 0 {
            if self
                .remote
                .as_ref()
                .is_some_and(|r| !r.chat_queue.is_empty())
            {
                self.flush_chat_queue();
            }
            return;
        }
        let ready = self.remote.as_ref().is_some_and(|r| {
            r.chat_previous
                .is_none_or(|t| t.elapsed() >= Duration::from_millis(delay as u64))
        });
        if !ready {
            return;
        }
        // A message the Chat option hides is dropped and the next one is tried,
        // until one shows.
        while let Some(chat) = self.remote.as_mut().and_then(|r| r.chat_queue.pop_front()) {
            if self.accept_chat(chat) {
                break;
            }
        }
    }

    /// `ChatListener.flushQueue`: every held-back message shows now, and the
    /// delay starts over.
    fn flush_chat_queue(&mut self) {
        let Some(queue) = self
            .remote
            .as_mut()
            .map(|r| std::mem::take(&mut r.chat_queue))
        else {
            return;
        };
        for chat in queue {
            self.accept_chat(chat);
        }
        if let Some(r) = self.remote.as_mut() {
            r.chat_previous = None;
        }
    }

    /// The minecarts the server sent, for the control socket's `status`
    /// (their velocity is not tracked here).
    pub(super) fn remote_carts(&self) -> Option<Vec<serde_json::Value>> {
        let r = self.remote.as_ref()?;
        let mut carts: Vec<&Tracked> = r
            .entities
            .values()
            .filter(|t| t.kind.ends_with("minecart"))
            .collect();
        carts.sort_by(|a, b| {
            a.position
                .partial_cmp(&b.position)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Some(
            carts
                .into_iter()
                .map(|t| serde_json::json!({"kind": t.kind, "position": t.position, "velocity": [0.0, 0.0, 0.0]}))
                .collect(),
        )
    }

    /// This player's account: a splitscreen player's own, else the game's (P1's).
    pub(super) fn auth(&self) -> Option<std::sync::Arc<horizon_client::auth::AuthSession>> {
        match self.options.split_guest {
            true => self.split_auth.clone(),
            false => horizon_client::auth::current(),
        }
    }
    /// Connected to a server in online mode (its Login): each player needs its own account.
    pub(super) fn online_server(&self) -> bool {
        self.remote.as_ref().is_some_and(|r| r.online_mode)
    }

    /// `ClientSuggestionProvider.customCompletionSuggestions` of the server
    /// connection (none without one).
    pub(super) fn remote_custom_completions(&self) -> std::collections::BTreeSet<String> {
        self.remote
            .as_ref()
            .map(|r| r.custom_completions.clone())
            .unwrap_or_default()
    }
    /// The names in the server's player list, sorted.
    pub(super) fn remote_online_names(&self) -> Option<Vec<String>> {
        let r = self.remote.as_ref()?;
        let mut names: Vec<String> = r.players.values().cloned().collect();
        names.sort();
        Some(names)
    }
    /// The open menu's window id (0: the inventory).
    pub(super) fn remote_menu_id(&self) -> i32 {
        self.remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map_or(0, |m| m.0)
    }
    /// The server's last `n` chat lines, newest first.
    pub(super) fn remote_chat(&self, n: usize) -> Option<Vec<String>> {
        let r = self.remote.as_ref()?;
        Some(r.chat.iter().rev().take(n).map(|l| l.0.clone()).collect())
    }

    /// Received events left for the next frame (`poll_remote`): until they
    /// are handled, the terrain the server sent is not all there.
    pub(super) fn remote_backlog(&self) -> usize {
        self.remote.as_ref().map_or(0, |r| r.backlog.len())
    }
    /// A headless command oracle has a real connection but no movement tick.
    /// Acknowledge loaded terrain without predicting or sending movement.
    pub(super) fn poll_stationary_client(&mut self) -> Result<()> {
        self.poll_remote()?;
        let loaded = self.player.as_ref().is_some_and(|p| {
            self.streaming.is_lit(
                (p.position.x.floor() as i32) >> 4,
                (p.position.z.floor() as i32) >> 4,
            )
        });
        if loaded && self.remote.as_ref().is_some_and(|r| !r.loaded_sent) {
            self.send(S::PlayerLoaded(Empty));
            let r = self.remote.as_mut().unwrap();
            r.loaded_sent = true;
            r.level_load.finish();
        }
        Ok(())
    }

    /// Process what the network thread received (once per frame, like the
    /// client's packet processor), in order. Chunk packets get
    /// `CHUNK_PACKETS_PER_FRAME`: once a frame has spent it, the events from
    /// the next chunk packet on wait for the next frame, unless they have
    /// already waited `BACKLOG_LIMIT`. A chunk packet still decoding holds
    /// back what follows it.
    pub(super) fn poll_remote(&mut self) -> Result<()> {
        let polled = self.poll_remote_events();
        // The block events and block changes of a piston clock start and end
        // thousands of moves at once: the collision view and the renderer
        // take them all in one sync, before the tick or frame reads them.
        if std::mem::take(&mut self.moving_pistons_stale) {
            self.sync_moving_pistons();
        }
        polled
    }
    fn poll_remote_events(&mut self) -> Result<()> {
        let Some(r) = self.remote.as_mut() else {
            return Ok(());
        };
        let mut events = std::mem::take(&mut r.backlog);
        r.receive_credited_backlog(&mut events);
        let overdue = r
            .backlog_since
            .is_some_and(|since| since.elapsed() >= BACKLOG_LIMIT);
        let mut chunks = Duration::ZERO;
        // A run of ForgetLevelChunk (thousands when the server's view moves far: a
        // teleport at 32 chunks) is forgotten at once, before whatever follows it.
        let mut forgotten = horizon_client::fast_hash::FxHashSet::default();
        let mut forgotten_credits = Vec::new();
        while let Some(item) = events.pop_front() {
            if let Received::Event(Event::Packet(packet), _) = &item
                && let C::ForgetLevelChunk(p) = &**packet
            {
                self.observe_client_packet(packet);
                forgotten.insert((p.pos[0], p.pos[1]));
                forgotten_credits.push(item.into_event_credit());
                continue;
            }
            horizon_client::spans::time("pr_forget", || {
                self.forget_remote_columns(&mut forgotten, true)
            });
            forgotten_credits.clear();
            let chunk = matches!(item, Received::Chunk(_, _));
            if chunk && !overdue && chunks >= CHUNK_PACKETS_PER_FRAME {
                events.push_front(item);
                break;
            }
            let started = chunk.then(Instant::now);
            let (event, _credit) = match item {
                Received::Event(event, credit) => (event, credit),
                Received::Chunk(format, credit) => {
                    // A column still decoding waits for the next frame, with
                    // what came after it, overdue or not.
                    let Some(next) = self.remote.as_ref().map(|r| r.decoder.next()) else {
                        break;
                    };
                    let (packet, column) = match next {
                        Next::Ready(decoded) => decoded,
                        Next::Decoding => {
                            events.push_front(Received::Chunk(format, credit));
                            break;
                        }
                        Next::Lost => {
                            log::warn!("Horizon: the chunk decoder stopped; a column is missing");
                            continue;
                        }
                    };
                    self.observe_client_packet(&packet);
                    if let C::LevelChunkWithLight(p) = *packet {
                        horizon_client::spans::time("pr_chunk", || {
                            self.remote_chunk(&p, Some((&format, column)))
                        });
                    }
                    chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
                    continue;
                }
            };
            match event {
                Event::Status(s) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.status = s;
                    }
                }
                Event::Joined {
                    registries,
                    name,
                    uuid,
                } => {
                    self.set_registries(registries);
                    self.extras_joined();
                    self.close_dialogs();
                    if let Some(r) = self.remote.as_mut() {
                        r.joined = true;
                        r.profile = uuid;
                        r.profile_name = name.clone();
                        r.status = horizon_client::lang::get_or(
                            "multiplayer.downloadingTerrain",
                            "Loading terrain...",
                        );
                    }
                    self.begin_book_negotiation();
                }
                Event::Features(features) => {
                    if let Some(remote) = &mut self.remote {
                        remote.features = features.into();
                    }
                }
                Event::Brand(brand) => {
                    if let Some(remote) = &mut self.remote {
                        remote.brand = brand;
                    }
                }
                Event::QuietCommands => {
                    if let Some(remote) = &mut self.remote {
                        remote.quiet_commands = true;
                    }
                }
                Event::CustomBooks => {
                    if let Some(remote) = &mut self.remote {
                        remote.custom_books = true;
                    }
                }
                Event::FarTerrain(offer) => {
                    if let Some(remote) = &mut self.remote {
                        remote.far_terrain = Some(offer);
                    }
                }
                Event::Reconfiguring => {
                    self.reset_book_view();
                    // handleConfigurationStart: the held-back chat shows, then
                    // clearClientLevel opens ServerReconfigScreen over no level.
                    self.flush_chat_queue();
                    self.send_chat_acknowledgement();
                    self.close_dialogs();
                    // The server configuring us now (a proxy's next one)
                    // offers its own far terrain, or none, and announces its
                    // own features again.
                    if let Some(remote) = &mut self.remote {
                        remote.viewed.clear();
                        remote.far_terrain = None;
                        remote.quiet_commands = false;
                        remote.custom_books = false;
                    }
                    self.clear_lod();
                    if let Some(remote) = &self.remote {
                        remote.session.reset_far_terrain();
                    }
                    self.remove_local_player_body();
                    self.set_screen(Screen::Playing);
                    self.capture(false);
                    // clearClientLevel drops the client level now, so its
                    // unload comes before the configuration screen, not
                    // after the next login's load.
                    self.forget_all_remote_columns();
                    if let Some(old) = self.remote.as_mut().and_then(|r| r.heard_level.take()) {
                        self.native_client_level_unload(&old);
                    }
                    self.native_lifecycle.begin_close();
                    self.native_lifecycle_world_closed();
                    let ticks = self.ticks;
                    if let Some(r) = self.remote.as_mut() {
                        r.reconfiguring = Some(ticks);
                        r.chat_tree_received = false;
                    }
                }
                Event::Reconfigured(registries) => {
                    // handleConfigurationStart → clearClientLevel, and the next
                    // ClientPacketListener starts with no players or scoreboard,
                    // no waypoints (its own ClientWaypointManager: a proxy's next
                    // server never untracks the last one's), and its new
                    // ClientSuggestionProvider with no custom chat completions.
                    // clearClientLevel clears the narrator.
                    self.clear_narrator();
                    self.hud_disconnected();
                    self.score.board = Default::default();
                    self.clear_waypoints();
                    if let Some(r) = self.remote.as_mut() {
                        r.attachments.clear();
                        r.players.clear();
                        r.tab_players.clear();
                        r.listed.clear();
                        r.signatures = Default::default();
                        r.custom_completions.clear();
                    }
                    self.ui.registries.custom_completions.clear();
                    self.set_registries(registries);
                }
                // handleTransfer: "Cannot transfer to server from
                // singleplayer" (no ServerData), so the world just closes.
                Event::Transfer { host, port } => {
                    if let Some(r) = self.remote.as_mut()
                        && r.session.address != "local"
                    {
                        r.transfer = Some(format!("{host}:{port}"));
                    }
                }
                Event::PackPush(p) => self.handle_pack_push(p),
                Event::PackPop(id) => self.handle_pack_pop(id),
                Event::ServerLinks(links) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.server_links = super::server_links::trusted(links);
                    }
                }
                Event::ShowDialog(dialog) => self.show_dialog(&dialog),
                Event::ClearDialog => self.clear_dialog(),
                Event::CodeOfConduct(text) => self.handle_code_of_conduct(text),
                Event::DecodeError(e) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.decode_errors += 1;
                        if r.decode_errors <= 20 {
                            log::warn!("Horizon: skipped undecodable packet {e}");
                        }
                    }
                }
                Event::Disconnected {
                    reason,
                    key,
                    from_server,
                } => {
                    self.reset_book_view();
                    log::info!("Horizon: disconnected: {reason}");
                    // ClientboundTransferPacket left a target: connect there
                    // instead of showing the disconnect, the stored cookies
                    // carried over (ClientCommonPacketListenerImpl.
                    // handleTransfer → ConnectScreen.startConnecting).
                    let transfer = self.remote.as_mut().and_then(|r| {
                        r.transfer.take().map(|address| {
                            (address, r.session.name.clone(), r.session.cookies.clone())
                        })
                    });
                    if let Some((address, name, cookies)) = transfer {
                        self.leave_server(None)?;
                        self.connect_server_with_cookies(&address, &name, Some(cookies))?;
                        return Ok(());
                    }
                    let retry = self.reconnect_after(&reason, key.as_deref(), from_server);
                    self.leave_server(Some(reason))?;
                    self.offer_reconnect(retry);
                    return Ok(());
                }
                Event::RawPayload(incoming) => self.receive_client_payload(*incoming)?,
                Event::Packet(packet) => match *packet {
                    C::CustomPayload(p) if p.channel == horizon_client::perf_net::CHANNEL => {
                        self.remote_perf_reply(&p.data)
                    }
                    packet => self.handle_packet(packet)?,
                },
            }
            chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
            if self.remote.is_none() {
                break;
            }
        }
        if self.remote.is_some() {
            horizon_client::spans::time("pr_forget", || {
                self.forget_remote_columns(&mut forgotten, true)
            });
        }
        if let Some(r) = self.remote.as_mut() {
            r.backlog_since = if events.is_empty() {
                None
            } else {
                Some(r.backlog_since.unwrap_or_else(Instant::now))
            };
            r.backlog = events;
        }
        self.send_suggestion_request();
        Ok(())
    }
    /// ClientSuggestionProvider.customSuggestion: the chat's completion
    /// request for an `ask_server` node goes to the server.
    fn send_suggestion_request(&mut self) {
        if self.remote.is_none() {
            return;
        }
        let Some((id, command)) = self
            .ui
            .chat_screen
            .as_mut()
            .and_then(|s| s.take_request())
            .or_else(|| self.command_block_suggestion_request())
        else {
            return;
        };
        let mut w = horizon_protocol::buf::Writer::new();
        w.varint(id);
        w.string(&command);
        self.send(S::Unknown(
            horizon_protocol::ids::play::serverbound::COMMAND_SUGGESTION,
            w.data,
        ));
    }
    /// ClientboundCommandSuggestionsPacket: id, range start and length, then
    /// each suggestion's text and optional tooltip.
    fn handle_command_suggestions(&mut self, data: &[u8]) {
        let lang = self
            .remote
            .as_ref()
            .map(|r| r.lang.clone())
            .unwrap_or_default();
        let mut r = horizon_protocol::buf::Reader::new(data);
        let decoded = (|| {
            let id = r.varint()?;
            let start = r.varint()?;
            let length = r.varint()?;
            let n = r.count(usize::MAX)?;
            let mut list = Vec::with_capacity(n.min(1000));
            for _ in 0..n {
                let text = r.string(32767)?;
                let tooltip = if r.bool()? {
                    Some(plain(&r.nbt()?, &lang))
                } else {
                    None
                };
                list.push((text, tooltip));
            }
            horizon_protocol::Result::Ok((id, start, length, list))
        })();
        if let Ok((id, start, length, list)) = decoded {
            let range = (start.max(0) as usize, length.max(0) as usize);
            self.chat_server_suggestions(id, range, list.clone());
            self.command_block_server_suggestions(id, range, list);
        }
    }
    fn set_registries(&mut self, registries: Arc<Dynamic>) {
        let entries: Vec<(&str, serde_json::Value)> = registries
            .registries
            .get("minecraft:enchantment")
            .into_iter()
            .flatten()
            .filter_map(|(name, data)| {
                Some((
                    name.as_str(),
                    horizon_client::scoreboard::tag_to_json(data.as_ref()?),
                ))
            })
            .collect();
        let enchantments = match self
            .stations()
            .enchantments
            .with_entries(entries.iter().map(|(n, v)| (*n, v)))
        {
            Ok(table) => table.map(Arc::new),
            Err(error) => {
                log::warn!("Horizon: refused enchantment registry: {error}");
                return;
            }
        };
        // The server's biomes the game lacks (a world data pack's) join the
        // game's table, from the data it sent: colours, meshing and the
        // chunk format all go through that table. The window's own server
        // thread set the world's table already.
        if self.integrated.is_none() {
            let table = registries
                .registries
                .get(BIOME_REGISTRY)
                .and_then(|list| client_biomes(self.res.biomes(), list))
                .map(Arc::new);
            if table.is_some() || self.world_biomes.is_some() {
                self.world_biomes_base = table.clone();
                self.world_biomes = table;
                let context = self.worker_context();
                self.streaming.reload_resources(&mut self.world, context);
            }
        }
        let biomes: Vec<u16> = registries
            .registries
            .get(BIOME_REGISTRY)
            .map(|list| {
                list.iter()
                    .map(|(name, _)| world_biomes!(self).id(name))
                    .collect()
            })
            .unwrap_or_default();
        self.server_items = self.server_items(&registries);
        if let Some(r) = self.remote.as_mut() {
            r.enchantments = enchantments;
            let bits = ceil_log2(biomes.len());
            let column = r.chunk_format.layout();
            r.chunk_format =
                Arc::new(ChunkFormat::new(&r.dimension, biomes, bits).in_column(column));
            r.registries = registries;
            r.resolve_clocks();
        }
    }
    /// Switch to a (possibly new) dimension: a fresh client level.
    pub(in crate::game) fn enter_remote_dimension(
        &mut self,
        spawn: &play::SpawnInfo,
        keep_world: bool,
        reason: horizon_client::level_loading::Reason,
    ) {
        // Every level has the sea level its server says (`CommonPlayerSpawnInfo.seaLevel`): the
        // table may still hold a local world's (a superflat Overworld's).
        if let Ok(key) = horizon_core::LevelKey::parse(&spawn.dimension) {
            horizon_world::level::register_sea_level(key, spawn.sea_level);
        }
        self.track_remote_levels(&spawn.dimension);
        // The replacement player starts with its inventory menu, even if the
        // server switched levels without closing the previous container.
        self.remote_close_local();
        self.set_screen(Screen::Playing);
        let Some(r) = self.remote.as_ref() else {
            return;
        };
        // handleRespawn keeps the ClientLevel only for the same level key:
        // two levels sharing a dimension type are still two levels.
        let same = keep_world && r.heard_level.as_deref() == Some(spawn.dimension.as_str());
        let transition_screen = horizon_client_render::dimension_transition::select(
            mod_api::__private::dimension_key(&spawn.dimension),
            r.heard_level
                .as_deref()
                .map(mod_api::__private::dimension_key),
        );
        if !same {
            self.forget_all_remote_columns();
        }
        let r = self.remote.as_mut().unwrap();
        // A Horizon server's bulk edit sends whole columns with the light
        // from before its relight; any other server's packet light is final.
        let relight_resent = r.quiet_commands;
        let format = r.level_format(spawn, self.res.environment());
        let (dimension, shape) = (format.dimension, format.shape);
        r.chunk_format = format.chunk_format;
        r.level_facts = format.facts;
        r.level_environment = format.environment;
        r.dimension = dimension;
        r.resolve_clocks();
        self.world.day_time = r.clock_time(r.default_clock);
        r.level = spawn.dimension.clone();
        r.level_key = mod_api::__private::dimension_key(&spawn.dimension);
        r.hashed_seed = spawn.seed;
        r.game_mode = spawn.game_type;
        r.flat = spawn.flat;
        r.loaded_sent = false;
        // A replacement LevelLoadingScreen starts its bar from nothing (the
        // first one carries on from the server's start).
        if self.player.is_some() {
            self.loading_progress = 0.0;
        }
        // startWaitingForNewLevel: `LevelLoadTracker.startClientLoad` and the
        // `LevelLoadingScreen`.
        r.level_load.start_client_load(client_millis(), reason);
        r.level_load.set_transition_screen(transition_screen);
        // A replacement LocalPlayer has a fresh ambient mood and sendPosition baseline.
        self.ambience.moodiness = 0.0;
        r.position_sender = Default::default();
        r.predictions.clear();
        self.set_mode_id(spawn.game_type);
        self.dim = Dim::from_id(&spawn.dimension);
        self.audio.set_world(&spawn.dimension);
        // ClientPacketListener.handleLogin / handleRespawn: the server's
        // Player.getLastDeathLocation, for the recovery compass.
        self.level.last_death = spawn
            .last_death
            .as_ref()
            .map(|d| (d.dimension.clone(), d.pos));
        if !same {
            // A new ClientLevel: Minecraft.setLevel stops every sound.
            self.stop_sounds_for_new_level();
            let mut streaming = Streaming::new(
                Arc::new(NoTerrain),
                self.worker_context(),
                self.res.threads(),
            );
            streaming.remote = true;
            streaming.radius = self.streaming.radius;
            self.streaming = streaming;
            let (time, day, weather) = (
                self.world.game_time,
                self.world.day_time,
                self.world.weather,
            );
            self.reset_client_world(spawn.seed);
            // Entity IDs belong to this ClientLevel, not the connection. A
            // same-level respawn keeps it; a replacement invalidates handles.
            self.native_lifecycle_open_started();
            self.native_lifecycle_world_ready();
            self.world.shape = shape;
            self.world.game_time = time;
            self.world.day_time = day;
            self.world.weather = weather;
            self.mobs.clear();
            self.clear_remote_objects();
            self.clear_remote_projectiles();
            self.dropped.clear();
            self.clear_client_particles();
            if let Some(r) = self.remote.as_mut() {
                r.entities.clear();
                r.passenger_vehicles.clear();
                r.lerps.clear();
                r.cart_lerps.clear();
                r.stepped.clear();
                r.head_lerps.clear();
                r.pending_items.clear();
                // The entity ids are the old world's; the new dimension's
                // player entities re-map to their profiles on AddEntity.
                r.level_players.clear();
                r.level_player_names.clear();
                r.level_player_textures.clear();
                // A new ClientLevel: a new chunk storage, centred on 0, 0.
                r.chunk_center = (0, 0);
            }
            self.clear_lod();
            self.pending_uploads.clear();
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.clear_sections(self.world.layout());
            }
            // The new ClientLevel is told before setLevel drops the old one.
            let old = self
                .remote
                .as_mut()
                .and_then(|r| r.heard_level.replace(spawn.dimension.clone()));
            self.native_client_level_load(&spawn.dimension);
            if let Some(old) = old {
                self.native_client_level_unload(&old);
            }
        }
        self.streaming.relight_resent = relight_resent;
        let has_sky = self
            .remote
            .as_ref()
            .is_none_or(|r| r.dimension.has_skylight);
        self.streaming.sky_light = has_sky;
        self.streaming.nether_shading = self.nether_lighting();
        // The seed in SpawnInfo is already the hashed one the client's BiomeManager holds.
        self.streaming.biome_zoom = horizon_core::biome::BiomeZoom::new(spawn.seed);
        let player = self.player.as_ref();
        self.particles
            .detach_tracking_target(TrackingTarget::LocalPlayer, || {
                player.map(player_tracking_body)
            });
        self.drop_remote_player();
        // handleLogin / handleRespawn: LocalPlayer.setPortalCooldown, so a
        // player who arrives standing in a portal gets no overlay from it.
        self.portals.player.cooldown = spawn.portal_cooldown;
        if let Some(profile) = self.remote.as_ref().map(|remote| remote.profile) {
            self.initialize_entity(Holder::Player(profile), "minecraft:player");
        }
        self.finish_remote_level();
    }

    /// `ClientChunkCache.Storage.inRange`: within the storage around the
    /// view centre, `calculateStorageRange` (the server's radius, at least
    /// two, plus three) chunks each way.
    pub(super) fn remote_chunk_in_range(&self, x: i32, z: i32) -> bool {
        if self.replay.playing() {
            return true;
        }
        self.remote
            .as_ref()
            .is_none_or(|r| in_storage_range(r.view_distance, r.chunk_center, x, z))
    }
    fn chunk_inside_remote_range(center: (i32, i32), distance: i32, x: i32, z: i32) -> bool {
        in_storage_range(distance, center, x, z)
    }
    /// The columns a moved or resized storage no longer holds. Returns the
    /// positions it freed (`forget_remote_columns`).
    pub(in crate::game) fn drop_remote_chunks_out_of_range(
        &mut self,
        center: (i32, i32),
        distance: i32,
    ) -> Vec<(i32, i32)> {
        let mut gone: horizon_client::fast_hash::FxHashSet<(i32, i32)> = self
            .world
            .chunks
            .keys()
            .copied()
            .filter(|&(x, z)| !Self::chunk_inside_remote_range(center, distance, x, z))
            .collect();
        self.forget_remote_columns(&mut gone, true)
    }
    /// Every column goes with the client level. Returns the positions it
    /// freed (`forget_remote_columns`).
    pub(in crate::game) fn forget_all_remote_columns(&mut self) -> Vec<(i32, i32)> {
        let mut gone = self.world.chunks.keys().copied().collect();
        self.forget_remote_columns(&mut gone, false)
    }
    /// Forgets the `gone` columns (and empties the set): their chunks (freed on a thread
    /// of their own when there are many), their sections in the renderer and their block
    /// entities, each store once for all of them. When the level stays open,
    /// far terrain keeps their last state (`server_terrain`). Returns the
    /// positions it freed, in `gone`'s order: only those still resident,
    /// so a later forget of the same positions frees nothing.
    pub(in crate::game) fn forget_remote_columns(
        &mut self,
        gone: &mut horizon_client::fast_hash::FxHashSet<(i32, i32)>,
        keep_terrain: bool,
    ) -> Vec<(i32, i32)> {
        if gone.is_empty() {
            return Vec::new();
        }
        // The client chunk seam: one unload per column still resident, with
        // its data still there, before `take_remote_columns` frees it.
        let freed_at: Vec<(i32, i32)> = gone
            .iter()
            .filter(|&&(x, z)| self.world.chunks.contains_key(&(x, z)))
            .copied()
            .collect();
        for &(x, z) in &freed_at {
            self.client_chunk_unload(x, z, keep_terrain);
        }
        let freed =
            horizon_client::streaming::Streaming::take_remote_columns(&mut self.world, gone);
        self.streaming.forget_columns(&mut self.world, gone);
        if keep_terrain && let Some(lod) = &mut self.lod {
            for chunk in &freed {
                lod.observe(chunk.clone(), self.res.facts());
            }
        }
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.remove_columns(gone.iter().copied());
        }
        horizon_client::chunk::free_columns(freed, gone.len());
        self.world
            .block_entities
            .unload(|k| gone.contains(&(k.0 >> 4, k.2 >> 4)));
        if let Some(r) = self.remote.as_mut() {
            r.attachments.retain(|holder, _| match *holder {
                Holder::Chunk([x, z]) => !gone.contains(&(x, z)),
                Holder::BlockEntity([x, _, z]) => !gone.contains(&(x >> 4, z >> 4)),
                _ => true,
            });
        }
        gone.clear();
        freed_at
    }
    /// `handleAnimate`: a player wakes up, or an entity gets the critical
    /// hit (`CRIT`) or magic critical hit (`ENCHANTED_HIT`) emitter.
    fn remote_animate(&mut self, id: i32, action: u8) {
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let Some((target, body)) = self.remote_tracking_body(id) else {
            return;
        };
        let particle = match action {
            // Player.stopSleepInBed.
            0 => {
                if target == TrackingTarget::LocalPlayer {
                    self.sleeping = None;
                    self.sleep_timer = 100;
                }
                return;
            }
            1 => ParticleType::Crit,
            2 => ParticleType::EnchantedHit,
            _ => return,
        };
        self.particles
            .tracking_emitter(ParticleOptions::simple(particle), target, body);
    }
    /// `handleServerData`: the server list entry of the server joined keeps
    /// the icon the server sends (`ServerList.saveSingleServer`). Single
    /// player has no entry.
    fn remote_server_data(&mut self, icon: Option<&[u8]>) {
        let (Some(r), Some(png)) = (self.remote.as_ref(), icon) else {
            return;
        };
        if self.integrated.is_some() {
            return;
        }
        if let Ok(dir) = std::env::current_dir() {
            horizon_client::multiplayer::save_server_icon(
                &dir.join("saves"),
                &r.session.address,
                png,
            );
        }
    }
    /// Keep the renderable block entities (chests, furnaces) in step with blocks.
    /// `previous`: the block's state before, when it changed. A column sent again has
    /// none, and keeps its block entities' synced attachments: the server does not send
    /// them again (`remote_chunk` drops those of block entities the column lost).
    pub(super) fn sync_block_entity(
        &mut self,
        pos: (i32, i32, i32),
        previous: Option<BlockStateId>,
        state: BlockStateId,
    ) {
        let name = self.res.facts().get(state).name.clone();
        // LevelChunk.setBlockState: another block's block entity goes, even
        // one of the same type (a mod's blocks may share one).
        let other_block = previous.is_some_and(|p| self.res.facts().get(p).name != name);
        self.ensure_stateful_entity_table();
        // State-aware factories decide creation only. A valid same-block
        // instance survives even when the new state would create none.
        if !other_block
            && self.world.block_entities.stateful_entity(state.0)
            && matches!(
                self.world.block_entities.get(&pos),
                Some(BlockEntity::Mod(_))
            )
        {
            return;
        }
        // A mod block's is the mod's own, as on the server.
        match self.new_block_entity_for_state(state) {
            Some(entity) => {
                let same = self.world.block_entities.get(&pos).is_some_and(|e| {
                    e.id() == entity.id() && !(other_block && matches!(e, BlockEntity::Mod(_)))
                });
                if !same {
                    if previous.is_some() {
                        self.forget_block_entity_attachments(pos);
                    }
                    self.world.block_entities.replace(pos, entity);
                }
            }
            None => {
                if previous.is_some() {
                    self.forget_block_entity_attachments(pos);
                }
                self.world.block_entities.remove(&pos);
            }
        }
    }
    /// The server's synced attachments of a block entity that goes with it.
    fn forget_block_entity_attachments(&mut self, (x, y, z): (i32, i32, i32)) {
        if let Some(r) = self.remote.as_mut() {
            r.attachments.remove(&Holder::BlockEntity([x, y, z]));
        }
    }
    /// ClientLevel.setServerVerifiedBlockState: a block the client predicted keeps its
    /// predicted state until the server acknowledges the action; the server's state is
    /// remembered for then.
    fn server_block(&mut self, pos: [i32; 3], state: i32) {
        if let Some(p) = self
            .remote
            .as_mut()
            .and_then(|r| r.predictions.get_mut(&pos))
        {
            p.state = state;
            return;
        }
        self.set_block(pos, state, false);
    }
    /// BlockStatePredictionHandler.endPredictionsUpTo (ClientboundBlockChangedAckPacket): the
    /// actions up to `sequence` are settled, so the blocks they predicted take the state the
    /// server gave them (a break the server refused comes back).
    fn end_predictions(&mut self, sequence: i32) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let last_teleport = r.last_teleport_sequence;
        let settled: Vec<_> = r
            .predictions
            .iter()
            .filter(|(_, p)| p.sequence <= sequence)
            .map(|(&pos, &p)| (pos, p))
            .collect();
        for (pos, prediction) in settled {
            if let Some(r) = self.remote.as_mut() {
                r.predictions.remove(&pos);
            }
            if self.world.block(pos[0], pos[1], pos[2]).0 as i32 == prediction.state {
                continue;
            }
            self.set_block(pos, prediction.state, false);
            // ClientLevel.syncBlockState: the block that comes back may now hold
            // the player, and then it goes to the position the prediction
            // remembered.
            let meets = self.block_meets_player(pos, prediction.state);
            if let Some(at) = snap_target(prediction.player_pos, last_teleport, sequence, meets) {
                self.snap_player_to(at);
            }
        }
    }
    /// The player's position, for `retainKnownServerState`'s `player.position()`.
    fn player_position(&self) -> [f64; 3] {
        let at = self
            .player
            .as_ref()
            .map(|p| p.position)
            .unwrap_or(Vec3::ZERO);
        [at.x, at.y, at.z]
    }
    /// `Entity.isColliding(pos, state)` for the local player: the state's own
    /// collision shape, at the player's pose, against the player's box.
    fn block_meets_player(&self, pos: [i32; 3], state: i32) -> bool {
        let Some(player) = self.player.as_ref() else {
            return false;
        };
        let entity = player.bounds();
        let context = horizon_world::collision_context::CollisionContext::for_entity(
            horizon_world::collision_context::CollisionEntity {
                identity: horizon_world::entity_obstruction::EntityIdentity(0),
                kind: horizon_world::collision_context::EntityContextKind::Player,
                y: player.position.y,
                descending: player.shift_key_down,
                main_hand: None,
            },
            false,
        );
        let mut scene = horizon_client::collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        match self.collision.collision_shape_with_context(
            &mut scene,
            BlockStateId(state.max(0) as u32),
            at,
            &context,
        ) {
            Ok(shape) => shape_meets(&shape, pos, entity),
            Err(_) => false,
        }
    }
    /// `Entity.absSnapTo`: the remembered point, clamped to the level's edge.
    /// The velocity is left alone, and the player has no separate previous
    /// position to reset (its box is drawn from `position`).
    fn snap_player_to(&mut self, at: [f64; 3]) {
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let x = at[0].clamp(-3.0e7, 3.0e7);
        let z = at[2].clamp(-3.0e7, 3.0e7);
        player.position = Vec3::new(x, at[1], z);
    }
    fn set_block(&mut self, pos: [i32; 3], state: i32, predicted: bool) {
        // A state we don't have (a server with mods we lack) reads as air, as in its chunks.
        let state = state.max(0) as u32;
        let known = horizon_client::chunk::known_block_state(state);
        let state = BlockStateId(if known { state } else { 0 });
        let previous = self.world.block(pos[0], pos[1], pos[2]);
        if self.world.loaded(pos[0], pos[2])
            && self.world.set(pos[0], pos[1], pos[2], state).is_ok()
        {
            self.sync_block_entity((pos[0], pos[1], pos[2]), Some(previous), state);
            self.client_block_written(pos, previous, state, predicted);
        }
        // A block we just placed: play its place sound (the server skips the placer).
        if let Some((target, _)) = self.remote.as_ref().and_then(|r| r.placing)
            && target == pos
            && !self.res.facts().is_air(state)
        {
            let s = self.own_sound_type(pos, state, mod_api::BlockSoundCause::Place);
            let at = [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            ];
            self.play_client_local_sound(
                LevelSound::new(&s.place, at, (s.volume + 1.0) / 2.0, s.pitch * 0.8)
                    .in_source(mod_api::SoundSource::Blocks),
            );
            if let Some(r) = self.remote.as_mut() {
                r.placing = None;
            }
        }
    }
    /// `Entity.isVehicle` on a client: the entities a tracked entity rides.
    pub(super) fn remote_vehicle_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.remote
            .iter()
            .flat_map(|r| r.entities.values().filter_map(|t| t.vehicle))
    }
    /// The `SetPassengers` bookkeeping of the tracked entities.
    pub(super) fn remote_seat_passengers(&mut self, vehicle: u32, passengers: &[i32]) {
        if let Some(r) = self.remote.as_mut() {
            seat_passengers(&mut r.entities, vehicle, passengers);
        }
    }
    /// `recreateFromPacket`: yRot and xRot from the packet, the body and head
    /// both at its head yaw (LivingEntity).
    pub(super) fn client_attachment_entity_exists(&self, holder: Holder) -> bool {
        self.remote.as_ref().is_some_and(|remote| match holder {
            Holder::Entity(id) => remote.entities.contains_key(&(id as i32)),
            Holder::Player(uuid) => {
                remote.profile == uuid || remote.level_players.values().any(|p| *p == uuid)
            }
            _ => false,
        })
    }

    pub(super) fn spawn_entity(
        &mut self,
        id: i32,
        kind: &str,
        position: [f64; 3],
        rotation: [f32; 3],
    ) {
        self.detach_remote_tracking_entity(id);
        let [yaw, pitch, head] = rotation;
        let at = Vec3::new(position[0], position[1], position[2]);
        let mob_type = if kind == "minecraft:player" {
            self.remote.as_ref().and_then(|r| r.player_type.clone())
        } else {
            self.res.mob_types().get(kind).cloned()
        };
        if kind == "minecraft:ender_dragon" {
            self.spawn_remote_dragon(id, position, yaw);
        }
        if kind == "minecraft:experience_orb" {
            // Its value (ExperienceOrb.DATA_VALUE) follows in SetEntityData.
            self.orbs.retain(|o| o.id != id as u32);
            self.orbs
                .push(ExperienceOrb::new(id as u32, at, Vec3::ZERO, 1));
        }
        if let Some(t) = mob_type {
            // The window's copy of the server's mob: no mod part goals —
            // their `register_goals` ran on the authoritative side (#1077).
            let mut mob = Mob::new_remote(id as u32, t, at, head);
            super::remote_mob_data::client_defaults(&mut mob);
            (mob.yaw, mob.previous_yaw) = (yaw, yaw);
            (mob.head_pitch, mob.previous_head_pitch) = (pitch, pitch);
            mob.persistent = true;
            self.mobs.retain(|m| m.id != id as u32);
            self.mobs.push(mob);
        }
        if let Some(r) = self.remote.as_mut() {
            // A new entity has a new interpolation handler (a mob's or an
            // armor stand's), even under an id the server used before.
            r.stepped.remove(&id);
            if kind == "minecraft:item" {
                r.pending_items.insert(id, (position, Vec3::ZERO));
            }
            r.entities.insert(
                id,
                Tracked {
                    kind: kind.to_owned(),
                    position,
                    codec_base: position,
                    silent: false,
                    vehicle: None,
                },
            );
        }
    }
    /// A move packet for entity `id`: to the end of `path` (None: a
    /// rotation alone), turned to `rotation` if it has one. `packet` says
    /// whether a living entity more than 64 blocks away is put there at once
    /// (its `snaps`).
    fn move_entity(
        &mut self,
        id: i32,
        path: Option<Path>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
        packet: MovePacket,
    ) {
        // The dragon is drawn from its own state (game/mobs/dragon.rs), which
        // tick_remote_dragon eases to the server's moves through `lerps`. Its
        // copy in the mob list draws nothing: given the move, it kept the
        // dragon where it spawned (#2083).
        let dragon = self.dragon.as_ref().is_some_and(|d| d.id as i32 == id);
        let driven = self.driven_mount().is_some_and(|m| m.id as i32 == id);
        let mob = self
            .mob_index
            .get_mut(&mut self.mobs, id as u32)
            .filter(|_| !dragon);
        let tracked = self
            .remote
            .as_ref()
            .and_then(|r| r.entities.get(&id))
            .map(|t| t.position);
        let position = path
            .as_ref()
            .map(Path::end)
            .or(tracked)
            .or_else(|| {
                mob.as_ref()
                    .map(|m| [m.position.x, m.position.y, m.position.z])
            })
            .unwrap_or([0.0; 3]);
        if let Some(r) = self.remote.as_mut() {
            if let Some(t) = r.entities.get_mut(&id) {
                t.position = position;
            }
            if driven {
                // handleMoveEntity, handleEntityPositionSync: the mount this
                // client moves (isLocalInstanceAuthoritative) keeps its own
                // position, and no steps pile up for after the ride.
                r.stepped.remove(&id);
                return;
            }
            if let Some(m) = mob {
                // Shulker.createInterpolationHandler is NO_OP, so a move packet
                // is setPos at once. The 6-tick slide starts from the block
                // the shulker occupied before this packet.
                if m.species() == horizon_client::mobs::Species::Shulker {
                    r.stepped.remove(&id);
                    if path.is_some() {
                        horizon_client::mobs::shulker::client_set_pos(
                            m,
                            position[0],
                            position[1],
                            position[2],
                        );
                    }
                    if let Some((yaw, pitch)) = rotation {
                        m.yaw += wrap_degrees(yaw - m.yaw);
                        m.head_pitch = (pitch % 360.0).clamp(-90.0, 90.0);
                    }
                    m.on_ground = on_ground;
                    return;
                }
                // A living entity: LivingEntity.createInterpolationHandler's
                // SteppedInterpolationHandler, over its type's update interval.
                let entity = PosRot {
                    position: [m.position.x, m.position.y, m.position.z],
                    yaw: m.yaw,
                    pitch: m.head_pitch,
                };
                let handler = r.stepped.entry(id).or_insert_with(|| {
                    let interval = horizon_client::lan::tracking::settings(&m.kind.id).1;
                    SteppedInterpolation::new(interval as i32)
                });
                if let Some(at) = stepped_move(handler, packet, entity, position, path, rotation) {
                    // snapTo: setOldPosAndRot too.
                    place_interpolated(m, at);
                    m.previous = m.position;
                    m.previous_yaw = m.yaw;
                    m.previous_head_pitch = m.head_pitch;
                }
                m.on_ground = on_ground;
                return;
            }
            // An armor stand is a living entity too: the same handler, over
            // its type's update interval, stepped in `tick_remote_entities`.
            if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id as u32) {
                let entity = PosRot {
                    position: [a.position.x, a.position.y, a.position.z],
                    yaw: a.yaw,
                    pitch: 0.0,
                };
                let handler = r.stepped.entry(id).or_insert_with(|| {
                    let interval =
                        horizon_client::lan::tracking::settings("minecraft:armor_stand").1;
                    SteppedInterpolation::new(interval as i32)
                });
                if let Some(at) = stepped_move(handler, packet, entity, position, path, rotation) {
                    a.snap(
                        Vec3::new(at.position[0], at.position[1], at.position[2]),
                        at.yaw,
                    );
                }
                a.on_ground = on_ground;
                return;
            }
            // Interaction extends Entity and keeps its NO_OP interpolation
            // handler: the server's position replaces its own at once.
            if let Some(i) = self.interactions.get_mut(&id) {
                if path.is_some() {
                    i.position = Vec3::new(position[0], position[1], position[2]);
                }
                return;
            }
            // Projectiles keep Entity's NO_OP InterpolationHandler too: the
            // server's position replaces theirs at once and their own tick
            // flies them on (`tick_remote_projectiles`). Eased towards it
            // instead, they only moved when an update came, every 10 to 20
            // ticks.
            if let Some(i) = self
                .creatures
                .projectiles
                .iter()
                .position(|p| p.id == id as u32)
            {
                self.creatures.projectiles.update(i, |p| {
                    if path.is_some() {
                        p.position = Vec3::new(position[0], position[1], position[2]);
                    }
                    if let Some((yaw, pitch)) = rotation {
                        p.yaw = yaw;
                        p.pitch = pitch;
                    }
                });
                return;
            }
            if path.is_some()
                && (self.potions.move_remote_thrown(
                    id as u32,
                    Vec3::new(position[0], position[1], position[2]),
                ) || self.end.move_remote_eye(id as u32, position))
            {
                return;
            }
            // A fishing hook keeps a LinearInterpolationHandler: eased towards the
            // update over three ticks, on top of its own movement (`tick_remote_hooks`).
            if path.is_some() {
                if let Some(h) = self.hook.as_mut().filter(|h| h.id() == id as u32) {
                    h.move_to(position, on_ground);
                    return;
                }
                if let Some(h) = self.remote_hooks.iter_mut().find(|h| h.id() == id as u32) {
                    h.move_to(position, on_ground);
                    return;
                }
            }
            // A thrown trident is an arrow: placed at once, flown on by its tick.
            if let Some(t) = self.tridents.iter_mut().find(|t| t.id == id as u32) {
                if path.is_some() {
                    t.position = Vec3::new(position[0], position[1], position[2]);
                }
                if let Some((yaw, pitch)) = rotation {
                    t.yaw = yaw;
                    t.pitch = pitch;
                }
                return;
            }
            // So does a firework rocket: its own tick flies it on.
            if path.is_some()
                && let Some(f) = self.rockets.iter_mut().find(|f| f.id == id as u32)
            {
                f.position = Vec3::new(position[0], position[1], position[2]);
                return;
            }
            if let Some(p) = r.pending_items.get_mut(&id) {
                p.0 = position;
            }
            // ItemEntity, FallingBlockEntity and PrimedTnt keep Entity's
            // NO_OP InterpolationHandler: moveOrInterpolateTo puts them there
            // at once, and their own tick moves them on between the server's
            // updates (`tick_remote_entities`, `tick_remote_projectiles`).
            let at = Vec3::new(position[0], position[1], position[2]);
            if let Some(d) = self.dropped.iter_mut().find(|d| d.id == id as u32) {
                d.position = at;
                d.on_ground = on_ground;
                return;
            }
            if let Some(i) = self.falling.iter().rposition(|f| f.id == id as u32) {
                self.falling.update(i, |f| f.position = at);
                return;
            }
            if let Some(t) = self.tnts.iter_mut().rev().find(|t| t.id == id as u32) {
                t.position = at;
                t.on_ground = on_ground;
                return;
            }
            // ExperienceOrb.createInterpolationHandler is a
            // LinearInterpolationHandler: the correction is spread over 3
            // ticks (`tick_remote_entities`), while `ExperienceOrb.tick` keeps
            // moving it.
            if let Some(o) = self.orbs.iter_mut().find(|o| o.id == id as u32) {
                o.on_ground = on_ground;
                r.lerps.insert(
                    id,
                    Lerp {
                        target: position,
                        rotation: None,
                        steps: 3,
                    },
                );
                return;
            }
            if r.cart_lerps.contains_key(&id) {
                // A minecart on NewMinecartBehavior keeps Entity's NO_OP
                // InterpolationHandler: a teleport puts it there at once.
                if let Some(i) = self.carts.iter().position(|c| c.id == id as u32) {
                    self.carts.update(i, |c| {
                        if path.is_some() {
                            c.position = at;
                        }
                        if let Some((yaw, pitch)) = rotation {
                            c.yaw = yaw;
                            c.pitch = pitch;
                        }
                    });
                }
            } else {
                // AbstractInterpolationHandler.interpolateTo: without a
                // rotation, the one it was heading for.
                let rotation = rotation.or_else(|| r.lerps.get(&id).and_then(|l| l.rotation));
                r.lerps.insert(
                    id,
                    Lerp {
                        target: position,
                        rotation,
                        steps: 3,
                    },
                );
            }
        }
        self.turn_remote_object(id, rotation.map(|(yaw, _)| yaw), on_ground);
        self.move_remote_text_display(id, position, rotation);
        self.turn_remote_projectile(id, rotation);
    }
    pub(super) fn detach_remote_tracking_entity(&mut self, id: i32) {
        let mobs = &self.mobs;
        let index = &mut self.mob_index;
        self.particles
            .detach_tracking_target(TrackingTarget::Entity(id), || {
                index.get(mobs, id as u32).map(mob_tracking_body)
            });
    }

    /// Map a menu slot to Horizon's inventory screen slot ids.
    pub(super) fn inventory_screen_slot(menu: &str, slot: i32) -> Option<usize> {
        let s = usize::try_from(slot).ok()?;
        // A container of `n` slots, then the inventory and the hotbar.
        let slots = |n: usize| -> Option<usize> {
            match s {
                _ if s < n => Some(300 + s),
                _ if s < n + 27 => Some(9 + s - n),
                _ if s < n + 36 => Some(s - n - 27),
                _ => None,
            }
        };
        let rows = |n: usize| slots(n * 9);
        match menu {
            "inventory" => match s {
                0 => Some(200),
                1..=4 => Some(100 + s - 1),
                5..=8 => Some(500 + s - 5),
                9..=35 => Some(s),
                36..=44 => Some(s - 36),
                // InventoryMenu.SHIELD_SLOT: the off hand.
                45 => Some(504),
                _ => None,
            },
            "minecraft:crafting" => match s {
                0 => Some(200),
                1..=9 => Some(100 + s - 1),
                10..=36 => Some(9 + s - 10),
                37..=45 => Some(s - 37),
                _ => None,
            },
            "minecraft:merchant" => match s {
                0 | 1 => Some(super::stations::INPUT + s),
                2 => Some(super::stations::RESULT),
                3..=29 => Some(9 + s - 3),
                30..=38 => Some(s - 30),
                _ => None,
            },
            "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => match s {
                0..=2 => Some(300 + s),
                3..=29 => Some(9 + s - 3),
                30..=38 => Some(s - 30),
                _ => None,
            },
            "minecraft:generic_9x1" => rows(1),
            "minecraft:generic_9x2" => rows(2),
            "minecraft:generic_9x3" | "minecraft:shulker_box" => rows(3),
            "minecraft:generic_9x4" => rows(4),
            "minecraft:generic_9x5" => rows(5),
            "minecraft:generic_9x6" => rows(6),
            // HopperMenu; DispenserMenu (dispensers and droppers).
            "minecraft:hopper" => slots(5),
            "minecraft:generic_3x3" => slots(9),
            "minecraft:brewing_stand" => slots(5),
            // CrafterMenu: the grid, the inventory, then the result (the
            // client's own).
            "minecraft:crafter_3x3" => slots(9),
            // HorseInventoryMenu: the saddle, the body armour, the chest,
            // then the inventory.
            m if m.starts_with(MOUNT_MENU) => {
                let n = m[MOUNT_MENU.len()..].parse::<usize>().ok()? * 3;
                match s {
                    0 => Some(super::mount_menu::SADDLE),
                    1 => Some(super::mount_menu::ARMOR),
                    _ if s < 2 + n => Some(super::mount_menu::CHEST + s - 2),
                    _ if s < 2 + n + 27 => Some(9 + s - 2 - n),
                    _ if s < 2 + n + 36 => Some(s - 2 - n - 27),
                    _ => None,
                }
            }
            // A content mod's menu type: its container, then the inventory.
            // A workstation: its inputs, its result, then the inventory.
            m => {
                if let Some(t) = horizon_client::containers::mod_menu_type(m) {
                    return slots(t.slots);
                }
                let kind = Self::remote_station_kind(m)?;
                let inputs = kind.inputs();
                let own = inputs + usize::from(kind.has_result());
                match s {
                    _ if s < inputs => Some(super::stations::INPUT + s),
                    _ if s < own => Some(super::stations::RESULT),
                    _ if s < own + 27 => Some(9 + s - own),
                    _ if s < own + 36 => Some(s - own - 27),
                    _ => None,
                }
            }
        }
    }
    /// Find a game slot in the menu's complete, contiguous slot mapping.
    pub(super) fn menu_slot_index(menu: &str, internal: usize) -> Option<usize> {
        (0..)
            .map_while(|slot| Self::inventory_screen_slot(menu, slot))
            .position(|slot| slot == internal)
    }
    /// The workstation screen of a menu type a server opens.
    pub(super) fn remote_station_kind(menu: &str) -> Option<super::stations::Kind> {
        use super::stations::Kind;
        Some(match menu {
            "minecraft:loom" => Kind::Loom,
            "minecraft:stonecutter" => Kind::Stonecutter,
            "minecraft:grindstone" => Kind::Grindstone,
            "minecraft:anvil" => Kind::Anvil,
            "minecraft:smithing" => Kind::Smithing,
            "minecraft:enchantment" => Kind::Enchanting,
            "minecraft:beacon" => Kind::Beacon,
            "minecraft:cartography_table" => Kind::Cartography,
            _ => return None,
        })
    }
    fn open_menu_name(&self, container: i32) -> String {
        match self.remote.as_ref().and_then(|r| r.menu.as_ref()) {
            Some((id, name)) if *id == container && container != 0 => name.clone(),
            _ => "inventory".into(),
        }
    }
    fn set_menu_slot(&mut self, container: i32, slot: i32, item: Option<ItemStack>) {
        let menu = self.open_menu_name(container);
        if menu == "minecraft:lectern" {
            if slot == 0 {
                self.remote_lectern_book(item);
            }
            return;
        }
        if container != 0 && menu == "inventory" {
            return;
        }
        let Some(internal) = Self::inventory_screen_slot(&menu, slot) else {
            return;
        };
        if internal == 200 {
            // ClientboundContainerSetSlotPacket for the open menu's slot 0: the
            // server's result of its own recipes (a world's data packs' too).
            if container == self.remote_menu().0
                && let Some(r) = self.remote.as_mut()
            {
                r.craft_result = item;
            }
            return;
        }
        let slot = self.slot_mut(internal);
        let changed = !super::hand::matches_for_swap(slot.as_ref(), item.as_ref());
        *slot = item;
        if internal < 36 {
            self.remote_hand_slot_set(internal, changed);
        } else if internal == 504 {
            self.remote_hand_slot_set(super::shield::OFFHAND_SLOT, changed);
        }
        if internal >= super::stations::INPUT && self.station.is_some() {
            self.remote_station_slot_changed(internal);
        }
    }

    pub(in crate::game) fn apply_packet(&mut self, packet: C) -> Result<()> {
        self.observe_client_packet(&packet);
        match packet {
            // ClientCommonPacketListenerImpl.handlePing runs on the game
            // thread, after all preceding packets (especially teleports).
            C::Ping(p) => self.send(S::Pong(p)),
            // The server's next generation of mod registries: all of it or the connection.
            C::CustomPayload(p) if p.channel == horizon_client::datapack_net::CHANNEL => {
                horizon_client::datapack_net::accept_chunk(&p.data)?;
            }
            C::CustomPayload(p)
                if p.channel == horizon_client::lan::commands::selector_sync::CHANNEL =>
            {
                self.receive_selector_tokens(&p.data)?;
            }
            // Another level's world: its open, its packets and its close (`viewed_worlds`).
            C::CustomPayload(p) if p.channel == horizon_protocol::viewed_levels::CHANNEL => {
                self.viewed_levels_payload(&p.data)?;
            }
            // Private custom recipe-book settings and placement protocol.
            C::CustomPayload(p)
                if p.channel == horizon_client::recipe_book::settings_sync::CHANNEL =>
            {
                self.remote_book_settings(&p.data);
            }
            // The synced attachments of a holder the server shows the window.
            C::CustomPayload(p) if p.channel == horizon_protocol::attachment_sync::CHANNEL => {
                self.remote_attachment_sync(&p.data);
            }
            // A mod menu type's block and its mod's bytes, just before the
            // menu's `OpenScreen`.
            C::CustomPayload(p) if p.channel == horizon_protocol::menu_extra::CHANNEL => {
                self.receive_menu_extra(&p.data);
            }
            C::CustomPayload(p) if horizon_client::lan::recipes::viewer_channel(&p.channel) => {
                self.remote_recipe_viewer(&p.channel, &p.data);
            }
            // An anvil cost the data-slot short cannot hold, after that short.
            C::CustomPayload(p) if p.channel == horizon_protocol::anvil_cost::CHANNEL => {
                self.remote_anvil_wide_cost(&p.data);
            }
            // The server's recipes its native mods asked to send.
            C::CustomPayload(p) if p.channel == horizon_client::lan::recipes::CONTENT_CHANNEL => {
                self.remote_recipe_content(&p.data);
            }
            // The passages of the chunks the server shows the window.
            C::CustomPayload(p) if p.channel == horizon_protocol::passage_sync::CHANNEL => {
                self.remote_passage_sync(&p.data);
            }
            // PingDebugMonitor.onPongReceived.
            C::PongResponse(p) => {
                let delay = self.debug_millis() - p.time;
                self.debug_charts.ping.log(delay);
            }
            C::Login(p) => {
                // A client's parts, from the tags it plays with.
                if let Err(e) = self.resolve_parts() {
                    log::warn!("Horizon: {e}");
                }
                // Configuration's tags, over the block ids the parts just bound.
                self.install_remote_tags();
                self.refresh_render_visibility();
                self.test_highlights.clear();
                self.forget_synced_recipes();
                self.level.hardcore = p.hardcore;
                // handleLogin: startWaitingForNewLevel with Reason.OTHER, which
                // replaces ServerReconfigScreen.
                if let Some(r) = self.remote.as_mut() {
                    r.reconfiguring = None;
                    r.attachments.clear();
                    r.viewed.clear();
                    // A new ClientPacketListener owns a fresh clock manager.
                    r.clocks.clear();
                    r.clock_game_time = 0;
                    self.world.game_time = 0;
                }
                // handleLogin makes a new MultiPlayerGameMode (carried index 0) and a
                // new LocalPlayer (slot 0, no effects, riding nothing: the server
                // pairs a vehicle again with ClientboundSetPassengersPacket).
                self.selected = 0;
                self.potions.effects.clear();
                self.riding = None;
                if let Some(player) = self.player.as_mut() {
                    player.entity_id = p.player_id;
                }
                if let Some(r) = self.remote.as_mut() {
                    r.carried.set(Some(0));
                    // handleLogin makes a new ClientLevel: its predictions count from 0.
                    r.sequence = 0;
                    r.own_invisible = false;
                    r.own_on_fire = false;
                    r.online_mode = p.online_mode;
                    r.levels.clone_from(&p.levels);
                    r.enforces_secure_chat = p.enforces_secure_chat;
                    r.entity_id = p.player_id;
                    r.local_entity_silent = false;
                    // handleLogin: the new LocalPlayer is the camera.
                    r.camera = None;
                    r.logged_in = true;
                    r.view_distance = p.chunk_radius;
                    r.simulation_distance = p.simulation_distance;
                    r.show_death_screen = p.show_death_screen;
                    r.reduced_debug_info = p.reduced_debug_info;
                    r.do_limited_crafting = p.limited_crafting;
                }
                self.bind_local_player_residency();
                self.refresh_debug_renderers();
                // LocalPlayer.setReducedDebugInfo always rebuilds, also on a second login.
                self.debug_entries.invalidate();
                self.streaming.radius = self
                    .settings
                    .render_distance
                    .min(p.chunk_radius.saturating_add(1))
                    .max(2);

                // handleLogin: startWaitingForNewLevel(.., Reason.OTHER).
                self.enter_remote_dimension(
                    &p.spawn,
                    false,
                    horizon_client::level_loading::Reason::Other,
                );
                // No ClientInformation here: the configuration phase sent it,
                // and ClientPacketListener.broadcastClientInformation sends it
                // again only when an option changes it.
                self.remote_chat_login();
                if let Some(remote) = &self.remote {
                    self.native_client_player_logging_in(p.player_id, remote.profile);
                }
            }
            C::Respawn(p) => {
                // The new LocalPlayer copies reduced debug info through its setter.
                self.debug_entries.invalidate();
                let old_player = self.remote.as_ref().map(|r| (r.entity_id, r.profile));
                if let Some((_, uuid)) = old_player {
                    self.forget_entity_initialization(Holder::Player(uuid));
                }
                if let Some(r) = self.remote.as_ref() {
                    self.audio.remove_sound_entity(r.entity_id as u32);
                }
                // A new player, without effects (the server sends them
                // again) and with no item in use.
                self.potions.effects.clear();
                // Every synced attachment but the player's is sent again; the viewed
                // levels go or stay with the level entered (`enter_respawn_level`).
                if let Some(r) = self.remote.as_mut() {
                    r.attachments
                        .retain(|holder, _| matches!(holder, Holder::Player(_)));
                }
                self.using = None;
                self.end_mod_use();
                // After a death (no KEEP_ENTITY_DATA) the new LocalPlayer
                // rides nothing: the server pairs a vehicle again with its
                // passengers. A dimension change keeps the vehicle it travels in.
                if p.keep & 2 == 0 {
                    self.riding = None;
                    // KEEP_ENTITY_DATA copies the old player's synched data
                    // (its shared flags) to the new one; without it they reset.
                    if let Some(r) = self.remote.as_mut() {
                        r.own_invisible = false;
                        r.own_on_fire = false;
                        r.local_entity_silent = false;
                    }
                }
                // handleRespawn compares the levels' keys: a mod's level is
                // its own dimension even where `Dim` has no name for it.
                let dimension_changed = self
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.level != p.spawn.dimension);
                let died = self.screen == Screen::Dead
                    || self
                        .player
                        .as_ref()
                        .is_some_and(|m| m.health.health() <= 0.0);
                let reason = horizon_client::level_loading::reason_for_respawn(
                    died,
                    &p.spawn.dimension,
                    self.dim.id(),
                );
                if dimension_changed {
                    horizon_client::spans::startup::arm("dimension.change");
                }
                // handleRespawn makes a new ClientLevel for another dimension
                // key (a proxy's server switch too): its predictions count from 0.
                if let Some(r) = self.remote.as_mut()
                    && r.level != p.spawn.dimension
                {
                    r.sequence = 0;
                }
                self.portals.respawn_attributes.respawn(p.keep & 1 != 0);
                self.enter_respawn_level(&p, reason);
                // handleRespawn adds the new LocalPlayer after what the kept
                // level holds; a new level holds nothing yet.
                let before: std::collections::HashSet<u32> =
                    self.mobs.iter().map(|m| m.id).collect();
                if let Some(r) = self.remote.as_mut() {
                    r.ticked_before_player = before;
                }
                if dimension_changed {
                    // LevelExtractor.setLevel: a new level drops the highlights.
                    self.test_highlights.clear();
                    let (music, _) = self.situational_music();
                    self.music.stop_playing(music.as_ref(), &mut self.audio);
                }
                self.set_screen(Screen::Playing);
                if let Some(r) = self.remote.as_mut() {
                    r.death_message = None;
                    r.health = (20.0, 20, 5.0);
                    // handleRespawn: the new LocalPlayer is the camera.
                    r.camera = None;
                }
                self.capture(true);
                if let Some((old_id, uuid)) = old_player {
                    self.native_client_player_clone(
                        old_id,
                        self.own_entity_id(),
                        uuid,
                        died,
                        dimension_changed,
                    );
                }
            }
            C::PlayerPosition(p) => self.remote_teleport(&p)?,
            C::PlayerRotation(p) => self.remote_rotation(&p),
            C::PlayerLookAt(p) => self.remote_look_at(&p),
            // ClientCommonPacketListenerImpl.handlePostEffects.
            C::PostEffects(p) => self.post_effects = p.effects,
            C::SetCamera(p) => self.remote_set_camera(p.camera_id),
            C::LevelChunkWithLight(p) => self.remote_chunk(&p, None),
            C::LightUpdate(p) => self.remote_light_update(&p),
            C::ChunksBiomes(p) => self.remote_chunks_biomes(&p),
            // `poll_remote` forgets a run of them at once.
            C::ForgetLevelChunk(p) => {
                self.forget_remote_columns(&mut [(p.pos[0], p.pos[1])].into_iter().collect(), true);
            }
            C::ChunkBatchFinished(_) => {
                // ChunkBatchSizeCalculator's ceiling; meshing runs on workers.
                self.send(S::ChunkBatchReceived(play::ChunkBatchReceived {
                    chunks_per_tick: 64.0,
                }));
            }
            C::BlockUpdate(p) => self.server_block(p.pos, p.state),
            C::AddTransientBlock(p) => self.add_transient_block(p.pos, p.state),
            // ClientPacketListener.handleBlockDestruction: another player's crack.
            C::BlockDestruction(p) => {
                self.destroy_block_progress(p.id, p.pos, i32::from(p.progress));
            }
            C::SectionBlocksUpdate(p) => {
                for (pos, state) in super::remote_chunks::section_blocks(&p) {
                    self.server_block(pos, state);
                }
            }
            C::BlockChangedAck(p) => self.end_predictions(p.sequence),
            // Level.blockEvent on the client level: the block's triggerEvent
            // runs on the client's copy of whatever is at pos (a piston's
            // moveBlocks builds the moving block entities it animates, a note
            // block plays); what it doesn't handle reaches the client copy of
            // the block entity (chest lids, shulker boxes).
            C::BlockEvent(p) => {
                let pos = horizon_core::position::BlockPos::new(p.pos[0], p.pos[1], p.pos[2]);
                let triggered = self.with_level(|b, level, ticks| {
                    b.client_block_event(level, ticks, pos, i32::from(p.b0), i32::from(p.b1))
                });
                self.moving_pistons_stale = true;
                self.process_level_events()?;
                if !triggered {
                    let state = self.world.block(p.pos[0], p.pos[1], p.pos[2]);
                    let block = &self.res.facts().get(state).name;
                    let named = self.remote.as_ref().and_then(|r| {
                        r.builtin
                            .get("minecraft:block")?
                            .name(p.block)
                            .map(str::to_owned)
                    });
                    if named.as_deref() == Some(&**block) {
                        self.block_entity_event(p.pos, i32::from(p.b0), i32::from(p.b1));
                    }
                }
            }
            // ClientLevel.levelEvent: LevelEventHandler, as this client's own
            // level plays them (particles and sounds: a spawner's burst, bone
            // meal, waxing, a block broken by someone else).
            // ClientPacketListener.handleLevelEvent: a global one is heard
            // from anywhere (ClientLevel.globalLevelEvent).
            C::LevelEvent(p) if p.global => self.global_level_event_sound(p.kind, p.pos),
            C::LevelEvent(p) => self.level_event_at(p.kind, p.pos, p.data),
            C::LevelParticles(p) => self.remote_level_particles(&p),
            C::Explode(p) => self.remote_explode(&p),
            C::Sound(p) => {
                if let Some(name) = self.remote_sound_name(&p.sound)
                    && !self.pvp_mutes_sound(&name)
                {
                    let at = [
                        f64::from(p.x) / 8.0,
                        f64::from(p.y) / 8.0,
                        f64::from(p.z) / 8.0,
                    ];
                    let mut sound = LevelSound::new(&name, at, p.volume, p.pitch);
                    if let Ok(source) = usize::try_from(p.source)
                        && let Some(source) = mod_api::SoundSource::ALL.get(source)
                    {
                        sound.source = *source;
                    }
                    self.play_client_level_sound(sound, p.seed);
                }
            }
            C::SoundEntity(p) => {
                if let Some(name) = self.remote_sound_name(&p.sound)
                    && !self.pvp_mutes_sound(&name)
                {
                    let mut sound = LevelSound::new(&name, [0.0; 3], p.volume, p.pitch);
                    sound.emitter = Some(p.id as u32);
                    if let Ok(source) = usize::try_from(p.source)
                        && let Some(source) = mod_api::SoundSource::ALL.get(source)
                    {
                        sound.source = *source;
                    }
                    self.play_client_level_sound(sound, p.seed);
                }
            }
            C::StopSound(p) => self
                .audio
                .stop_sounds(p.sound.as_deref(), p.source.map(|s| s as usize)),
            C::AddEntity(p) => {
                self.audio.remove_sound_entity(p.id as u32);
                let kind = self
                    .remote
                    .as_ref()
                    .and_then(|r| {
                        r.builtin
                            .get("minecraft:entity_type")?
                            .name(p.kind)
                            .map(str::to_owned)
                    })
                    .unwrap_or_default();
                let rotation =
                    [p.yaw, p.pitch, p.head_yaw].map(horizon_protocol::buf::unpack_degrees);
                if let Some(r) = self.remote.as_mut() {
                    // ClientLevel.addEntity: an id it holds already is removed
                    // and added again, after the player.
                    r.ticked_before_player.remove(&(p.id as u32));
                    // A new entity looks its PlayerInfo up afresh.
                    r.level_player_textures.remove(&p.id);
                    if kind == "minecraft:player" {
                        r.level_players.insert(p.id, p.uuid);
                        // createEntityFromPacket: the player takes the profile of its list entry.
                        match r.players.get(&p.uuid) {
                            Some(name) => r.level_player_names.insert(p.id, name.clone()),
                            None => r.level_player_names.remove(&p.id),
                        };
                    } else {
                        r.level_players.remove(&p.id);
                        r.level_player_names.remove(&p.id);
                    }
                }
                // Entity.getTeam: a mob is on a team by its UUID, a player by
                // its profile name (`Game::scoreboard_name`).
                self.persist.uuids.insert(
                    p.id as u32,
                    horizon_client::lan::playerdata::uuid_ints(p.uuid),
                );
                let holder = if kind == "minecraft:player" {
                    Holder::Player(p.uuid)
                } else {
                    Holder::Entity(p.id as u32)
                };
                self.initialize_entity(holder, &kind);
                let replace = self.tracks_remote_entity(p.id);
                self.spawn_entity(p.id, &kind, p.position, rotation);
                if let Some(r) = self.remote.as_mut() {
                    r.cart_lerps.remove(&p.id);
                    r.lerps.remove(&p.id);
                }
                if !self.spawn_remote_object(
                    p.id,
                    &kind,
                    p.position,
                    (rotation[0], rotation[1]),
                    p.data,
                ) {
                    self.spawn_added_projectile(&p, &kind, (rotation[0], rotation[1]), replace);
                }
                // Entity.recreateFromPacket: objects must exist before applying motion.
                self.remote_motion(p.id, p.velocity);
                if self.minecart_improvements()
                    && self.carts.iter().any(|c| c.id == p.id as u32)
                    && let Some(r) = self.remote.as_mut()
                {
                    r.cart_lerps.insert(p.id, CartLerp::new());
                }
                self.move_remote_text_display(p.id, p.position, Some((rotation[0], rotation[1])));
                self.move_remote_interaction(p.id, p.position);
            }
            C::RemoveEntities(p) => {
                for &id in &p.ids {
                    self.forget_remote_entity_construction(id);
                    if let Some(r) = self.remote.as_mut() {
                        if let Some(uuid) = r.level_players.remove(&id) {
                            r.attachments.remove(&Holder::Player(uuid));
                        }
                        r.level_player_names.remove(&id);
                        r.level_player_textures.remove(&id);
                        r.attachments.remove(&Holder::Entity(id as u32));
                    }
                }
                self.remove_entities(&p.ids);
            }
            C::MoveEntityPos(play::MoveEntityPos(m))
            | C::MoveEntityPosRot(play::MoveEntityPosRot(m))
            | C::MoveEntityRot(play::MoveEntityRot(m)) => {
                // handleMoveEntity: the delta counts from the position codec's
                // base, which then moves to the delta's end.
                let base = self
                    .remote
                    .as_ref()
                    .and_then(|r| r.entities.get(&m.id))
                    .map(|t| t.codec_base);
                let Some(base) = base else { return Ok(()) };
                // VecDeltaCodec.decode: each step's position from the last.
                let path = m.delta.as_ref().map(|d| match d {
                    play::VecDelta::Linear(_) => Path::Linear(d.end(base)),
                    play::VecDelta::Stepped(steps) => {
                        let mut at = base;
                        Path::Stepped(
                            steps
                                .iter()
                                .map(|(delta, ticks)| {
                                    at = play::VecDelta::apply(at, *delta);
                                    (at, *ticks)
                                })
                                .collect(),
                        )
                    }
                });
                let rotation = m.rotation.map(|(y, x)| {
                    (
                        horizon_protocol::buf::unpack_degrees(y),
                        horizon_protocol::buf::unpack_degrees(x),
                    )
                });
                if let (Some(path), Some(t)) = (
                    &path,
                    self.remote.as_mut().and_then(|r| r.entities.get_mut(&m.id)),
                ) {
                    t.codec_base = path.end();
                }
                self.remote_move_entity_and_file(m.id, path, rotation, m.on_ground);
            }
            C::EntityPositionSync(p) => {
                let path = match p.path {
                    play::PositionPath::Linear(at) => Path::Linear(at),
                    play::PositionPath::Stepped(steps) => Path::Stepped(steps),
                };
                // handleEntityPositionSync: the position codec's base is set.
                if let Some(t) = self.remote.as_mut().and_then(|r| r.entities.get_mut(&p.id)) {
                    t.codec_base = path.end();
                }
                self.remote_position_sync_and_file(p.id, path, p.yaw, p.pitch, p.on_ground);
            }
            C::TeleportEntity(p) => {
                let base = self
                    .remote
                    .as_ref()
                    .and_then(|r| r.entities.get(&p.id))
                    .map_or([0.0; 3], |t| t.position);
                let rel = |bit: i32, i: usize| {
                    if p.relatives & bit != 0 {
                        base[i] + p.position[i]
                    } else {
                        p.position[i]
                    }
                };
                let position = [
                    rel(play::relative::X, 0),
                    rel(play::relative::Y, 1),
                    rel(play::relative::Z, 2),
                ];
                // setValuesFromPositionPacket: interpolated, unless too far.
                self.move_entity(
                    p.id,
                    Some(Path::Linear(position)),
                    Some((p.yaw, p.pitch)),
                    p.on_ground,
                    MovePacket::Teleport,
                );
                // handleTeleportEntity: the mount this client moves is put
                // there at once.
                if self.driven_mount().is_some_and(|m| m.id as i32 == p.id) {
                    self.remote_move_vehicle(position, p.yaw);
                }
                self.entity_section_moved_id(p.id as u32);
            }
            C::SetPassengers(p) => self.remote_set_passengers(p.vehicle, &p.passengers),
            // ClientPacketListener.handleEntityLinkPacket: who holds a mob's
            // lead (this player, another player, a mob or a fence knot), for
            // the rope.
            C::SetEntityLink(p) => {
                let me = self.remote.as_ref().map(|r| r.entity_id);
                use horizon_client::mobs::leash::Holder;
                let holder = match p.dest {
                    0 => None,
                    d if Some(d) == me => Some(Holder::Player),
                    d => match self.remote_knots.get(&(d as u32)) {
                        Some(&block) => Some(Holder::Knot(block)),
                        // Not a known knot yet: kept by id until the knot
                        // arrives (game/network/remote_objects.rs).
                        None if self.boats.iter().any(|b| b.id == d as u32) => {
                            Some(Holder::Boat(d as u32))
                        }
                        None => Some(Holder::Mob(d as u32)),
                    },
                };
                if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.source as u32) {
                    m.extra.leash = holder;
                } else if let Some(i) = self.boats.iter().position(|b| b.id == p.source as u32) {
                    self.boats.update(i, |b| {
                        // A leashed boat or raft (AbstractBoat is Leashable).
                        b.leash = holder;
                    });
                }
            }
            C::MoveVehicle(p) => self.remote_move_vehicle_and_file(p.position, p.yaw),
            C::MoveMinecartAlongTrack(p) => self.remote_move_minecart(p),
            // ClientPacketListener.handleRotateMob: lerpHeadTo(yHeadRot, 3).
            C::RotateHead(p) => {
                if let Some(r) = self.remote.as_mut() {
                    let yaw = horizon_protocol::buf::unpack_degrees(p.head_yaw);
                    r.head_lerps.insert(p.id, (yaw, 3));
                }
            }
            C::SetEntityData(p) => {
                if let Some(r) = self.remote.as_mut() {
                    for (index, _, value) in &p.values {
                        if let (4, Value::Bool(silent)) = (index, value) {
                            if p.id == r.entity_id {
                                r.local_entity_silent = *silent;
                            }
                            if let Some(t) = r.entities.get_mut(&p.id) {
                                t.silent = *silent;
                            }
                        }
                    }
                }
                // Entity.DATA_SHARED_FLAGS_ID, DATA_CUSTOM_NAME and
                // DATA_CUSTOM_NAME_VISIBLE: server NPCs and holograms rely on
                // these instead of saved NBT.
                if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                    for (index, _, value) in &p.values {
                        match (index, value) {
                            (0, Value::Byte(flags)) => {
                                m.invisible = flags & 0x20 != 0;
                                m.glowing = flags & 0x40 != 0;
                            }
                            (2, Value::OptText(name)) => {
                                let text = name.as_ref().map(horizon_client::entity_nbt::name_text);
                                m.custom_name =
                                    text.as_ref().map(horizon_client::text::Text::plain);
                                m.custom_name_text = text;
                            }
                            (2, Value::Text(name)) => {
                                let text = horizon_client::entity_nbt::name_text(name);
                                m.custom_name = Some(text.plain());
                                m.custom_name_text = Some(text);
                            }
                            (3, Value::Bool(visible)) => m.name_visible = *visible,
                            _ => {}
                        }
                    }
                }
                if let Some((_, _, Value::Byte(flags))) = p.values.iter().find(|v| v.0 == 0)
                    && let Some(r) = self.remote.as_mut()
                {
                    if flags & 0x40 != 0 {
                        r.glowing.insert(p.id);
                    } else {
                        r.glowing.remove(&p.id);
                    }
                }
                self.remote_mob_data(p.id, &p.values);
                if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32)
                    && m.species() == horizon_client::mobs::Species::Creeper
                {
                    for (index, _, value) in &p.values {
                        match (index, value) {
                            // Creeper.DATA_SWELL_DIR, DATA_IS_POWERED, DATA_IS_IGNITED.
                            (16, Value::Int(dir)) => m.extra.counter = *dir,
                            (17, Value::Bool(powered)) => m.extra.charged = *powered,
                            (18, Value::Bool(ignited)) => m.extra.flag = *ignited,
                            _ => {}
                        }
                    }
                }
                if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                    for (index, _, value) in &p.values {
                        match (index, value) {
                            // Entity.DATA_TICKS_FROZEN: isFullyFrozen shakes it.
                            (7, Value::Int(ticks)) => m.ticks_frozen = *ticks,
                            // Strider.DATA_SUFFOCATING (after AgeableMob's two
                            // and DATA_BOOST_TIME): a cold strider shakes.
                            (19, Value::Bool(cold))
                                if m.species() == horizon_client::mobs::Species::Strider =>
                            {
                                m.extra.flag = *cold;
                            }
                            _ => {}
                        }
                    }
                }
                // LivingEntity.DATA_LIVING_ENTITY_FLAGS: for another player
                // too, remote_mob_data's apply reads index 8 above.
                let mannequin = self
                    .mobs
                    .iter()
                    .any(|m| m.id == p.id as u32 && &*m.kind.id == "minecraft:mannequin");
                let other = mannequin
                    || self.remote.as_ref().is_some_and(|r| {
                        r.level_players.contains_key(&p.id) && r.entity_id != p.id
                    });
                // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): another
                // player lies down in their bed (drawn lying in it), or gets up.
                if other {
                    for (index, _, value) in &p.values {
                        if let (14, Value::OptPos(bed)) = (index, value) {
                            let code = bed.map_or(0, |b| self.bed_facing(b));
                            let at = bed.and_then(|head| self.sleeping_position(head));
                            if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                                m.extra.ints[1] = code;
                                if bed.is_some() {
                                    // setPosToBed leaves the server's position alone
                                    // when the client's bed has no sleep shape.
                                    if let Some(at) = at {
                                        m.position = at;
                                        m.previous = at;
                                    }
                                    m.head_pitch = 0.0;
                                    m.previous_head_pitch = 0.0;
                                }
                            }
                        }
                        // Avatar.DATA_PLAYER_MAIN_HAND (HumanoidArm: 0 left, 1
                        // right): the arm a swing of the main hand moves.
                        if let (15, Value::Id(arm)) = (index, value)
                            && let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32)
                        {
                            m.extra.main_hand_left = *arm == 0;
                        }
                        // Avatar.DATA_PLAYER_MODE_CUSTOMISATION: the skin layers
                        // the player shows (PlayerModelPart bits).
                        if let (16, Value::Byte(bits)) = (index, value)
                            && let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32)
                        {
                            m.extra.hidden_model_parts = !(*bits as u8) & 0x7F;
                        }
                    }
                }
                // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): this
                // player lies down in the bed, or gets up.
                if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                    for (index, _, value) in &p.values {
                        match (index, value) {
                            (14, Value::OptPos(bed)) => {
                                if bed.is_some() && self.sleeping.is_none() {
                                    self.sleep_timer = 0;
                                }
                                self.sleeping = *bed;
                                if let Some(head) = *bed {
                                    // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): setPosToBed.
                                    let at = self.sleeping_position(head);
                                    if let Some(pl) = self.player.as_mut() {
                                        if let Some(at) = at {
                                            pl.position = at;
                                        }
                                        pl.velocity = Vec3::ZERO;
                                        pl.pitch = 0.0;
                                    }
                                    if let Some(at) = at {
                                        self.previous_position = at;
                                    }
                                    self.pitch = 0.0;
                                    self.snap_player_model_pitch();
                                }
                            }
                            // Entity.DATA_SHARED_FLAGS_ID's sprinting bit: the
                            // server stops the sprint after a hit's extra
                            // knockback (the flag only, as assignValues sets
                            // it: the speed modifier and wasSprinting stay).
                            // Its invisible bit (Entity.isInvisible): the
                            // Invisibility effect's, or a plugin's that hides
                            // the real body under a pose (#3788).
                            (0, Value::Byte(flags)) => {
                                if let Some(pl) = self.player.as_mut() {
                                    pl.sprinting = flags & 0x08 != 0;
                                    // Entity.DATA_SHARED_FLAGS_ID's fall-flying
                                    // bit (0x80, `Entity.setSharedFlag(7, ...)`):
                                    // `LivingEntity.updateFallFlying` clears the
                                    // glide on the server only and `LocalPlayer`
                                    // never ends one itself, so this packet is
                                    // the glide's own end. Applied when it is
                                    // handled — the frame's poll, before its
                                    // ticks — as ClientPacketListener does, so a
                                    // clear that arrives mid-tick lets that tick
                                    // keep its glide travel (#7616).
                                    pl.fall_flying = flags & -128 != 0;
                                }
                                let on_fire = flags & 0x01 != 0;
                                if let Some(r) = self.remote.as_mut() {
                                    r.own_invisible = flags & 0x20 != 0;
                                    let was_on_fire =
                                        std::mem::replace(&mut r.own_on_fire, on_fire);
                                    // The server put the fire out: the
                                    // window's countdown ends with it.
                                    if was_on_fire
                                        && !on_fire
                                        && let Some(pl) = self.player.as_mut()
                                    {
                                        pl.fire_ticks = pl.fire_ticks.min(0);
                                    }
                                }
                            }
                            // Entity.DATA_AIR_SUPPLY_ID and
                            // Player.DATA_PLAYER_ABSORPTION_ID: the bubbles
                            // and golden hearts the server's player has.
                            (1, Value::Int(air)) => {
                                if let Some(pl) = self.player.as_mut() {
                                    pl.air = *air;
                                }
                            }
                            // Entity.DATA_TICKS_FROZEN: the server's count of the
                            // powder snow freeze, which the client's own copy reads
                            // for the frost overlay, the frozen hearts and the
                            // shiver of its third-person model (#11217).
                            (7, Value::Int(ticks)) => {
                                if let Some(pl) = self.player.as_mut() {
                                    pl.ticks_frozen = *ticks;
                                }
                            }
                            // LivingEntity.DATA_ARROW_COUNT_ID and DATA_STINGER_COUNT_ID.
                            (12, Value::Int(count)) => {
                                if let Some(r) = self.remote.as_mut() {
                                    r.own_arrows = *count;
                                }
                            }
                            (13, Value::Int(count)) => {
                                if let Some(r) = self.remote.as_mut() {
                                    r.own_stingers = *count;
                                }
                            }
                            (19 | 20, Value::Id(v)) => {
                                if let Some(r) = self.remote.as_mut() {
                                    r.own_shoulders[usize::from(*index == 20)] =
                                        (*v > 0).then(|| (*v - 1).clamp(0, 4));
                                }
                            }
                            (17, Value::Float(amount)) => {
                                if let Some(pl) = self.player.as_mut() {
                                    let _ = pl.internal_set_absorption(*amount);
                                }
                            }
                            // LivingEntity.DATA_LIVING_ENTITY_FLAGS: the server
                            // starts or stops this player's use, as an axe that
                            // disables the raised shield does (Player.disableShield's
                            // stopUsingItem). LocalPlayer.onSyncedDataUpdated.
                            (8, Value::Byte(flags)) => {
                                let selected = self.selected;
                                let using = self.using;
                                self.using = synced_local_use(using, *flags, selected, |slot| {
                                    self.stack_in(slot).is_some()
                                });
                                // onSyncedDataUpdated's useItemRemaining: getUseDuration,
                                // without startUsingItem's event.
                                if using.is_none()
                                    && let Some((slot, _)) = self.using
                                    && let Some(stack) = self.stack_in(slot).cloned()
                                {
                                    self.use_duration = self.use_duration(&stack);
                                    self.use_start = super::using_item::UseStart::of(stack);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                // Entity.DATA_SHARED_FLAGS_ID's crouching and sprinting bits:
                // another player's or a mannequin's crouch (their model and
                // Entity.isDiscrete's name tag), and the dust a runner raises.
                if (mannequin
                    || self
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.level_players.contains_key(&p.id)))
                    && let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32)
                {
                    for (index, _, value) in &p.values {
                        if let (0, Value::Byte(flags)) = (index, value) {
                            m.extra.crouching = flags & 2 != 0;
                            m.extra.sprinting = flags & 8 != 0;
                        }
                    }
                }
                for (index, _, value) in &p.values {
                    if *index == ITEM_ENTITY_ITEM
                        && let Value::Item(stack) = value
                    {
                        self.remote_item_data(p.id, stack);
                    }
                    // ExperienceOrb.DATA_VALUE (the icon's size).
                    if *index == 8
                        && let Value::Int(v) = value
                        && let Some(o) = self.orbs.iter_mut().find(|o| o.id == p.id as u32)
                    {
                        o.value = *v;
                    }
                }
                self.remote_object_data(p.id, &p.values);
                self.remote_projectile_data(p.id, &p.values);
                self.remote_dragon_data(p.id, &p.values);
            }
            // ClientPacketListener.handleSetEquipment: what a mob or another
            // player holds and wears (EquipmentSlot ordinals).
            C::SetEquipment(p) => {
                self.remote_object_equipment(p.id, &p.slots);
                let items: Vec<(u8, Option<Arc<str>>, Option<ItemStack>)> = p
                    .slots
                    .iter()
                    .map(|(slot, stack)| {
                        (
                            *slot,
                            stack.as_ref().map(|s| self.item_name(s.item)),
                            self.to_stack(stack),
                        )
                    })
                    .collect();
                // CrossbowItem.isCharged: the main hand's `charged_projectiles`
                // (AvatarRenderer.getArmPose holds a loaded crossbow up).
                let charged = p.slots.iter().find(|(slot, _)| *slot == 0).map(|(_, s)| {
                    s.as_ref().is_some_and(|s| {
                        matches!(
                            s.component(horizon_protocol::item::component::CHARGED_PROJECTILES),
                            Some(horizon_protocol::item::Component::Stacks(list))
                                if !list.is_empty()
                        )
                    })
                });
                if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                    if let Some(charged) = charged
                        && &*m.kind.id == "minecraft:player"
                    {
                        m.extra.charged = charged;
                    }
                    for (slot, item, stack) in items {
                        let stack = stack.or_else(|| item.map(|item| ItemStack::new(&item, 1)));
                        m.set_equipment_stack(slot as usize, stack);
                    }
                }
            }
            C::TakeItemEntity(p) => {
                // ClientPacketListener.handleTakeItemEntity: the pickup sound at
                // the orb, item or arrow, whoever collects it (the server sends
                // none); an entity this client doesn't track sounds at this player.
                let orb_at = self
                    .orbs
                    .iter()
                    .find(|o| o.id == p.item as u32)
                    .map(|o| o.position);
                let orb = orb_at.is_some();
                let item = self.dropped.iter().find(|d| d.id == p.item as u32);
                let shot = self
                    .creatures
                    .projectiles
                    .iter()
                    .find(|s| s.id == p.item as u32);
                let shot = shot.map(|s| s.position).or_else(|| {
                    self.tridents
                        .iter()
                        .find(|t| t.id == p.item as u32)
                        .map(|t| t.position)
                });
                let at = match (item, shot) {
                    (Some(d), _) => Some([d.position.x, d.position.y, d.position.z]),
                    (None, Some(s)) => Some([s.x, s.y, s.z]),
                    (None, None)
                        if self
                            .remote
                            .as_ref()
                            .is_some_and(|r| r.entity_id == p.player) =>
                    {
                        self.eye_position()
                    }
                    (None, None) => None,
                };
                if let Some(o) = orb_at {
                    let pitch = (self.world.random.next_float() - self.world.random.next_float())
                        * 0.35
                        + 0.9;
                    self.audio.play(
                        "minecraft:entity.experience_orb.pickup",
                        Some([o.x, o.y, o.z]),
                        0.1,
                        pitch,
                    );
                } else if let Some(at) = at {
                    let pitch = ((self.world.random.next_float() - self.world.random.next_float())
                        * 0.7
                        + 1.0)
                        * 2.0;
                    self.audio
                        .play("minecraft:entity.item.pickup", Some(at), 0.2, pitch);
                }
                // ItemPickupParticle toward the collector (this player, another one or a mob).
                let collector = if self
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.entity_id == p.player)
                {
                    super::pickups::Collector::Player
                } else {
                    super::pickups::Collector::Mob(p.player as u32)
                };
                if let Some(item) = self.dropped.iter().find(|d| d.id == p.item as u32).cloned() {
                    self.add_pickup(super::pickups::Taken::Item(Box::new(item)), collector);
                } else if let Some(orb) = self.orbs.iter().find(|o| o.id == p.item as u32).cloned()
                {
                    self.add_pickup(super::pickups::Taken::Orb(Box::new(orb)), collector);
                } else if let Some(shot) = self
                    .creatures
                    .projectiles
                    .iter()
                    .find(|s| s.id == p.item as u32)
                    .cloned()
                {
                    // An arrow flies to its collector too, then goes.
                    self.add_pickup(super::pickups::Taken::Arrow(Box::new(shot)), collector);
                } else if let Some(i) = self.tridents.iter().position(|t| t.id == p.item as u32) {
                    // So does a trident: a server's client keeps them apart.
                    let trident = self.tridents.remove(i);
                    self.add_pickup(super::pickups::Taken::Trident(Box::new(trident)), collector);
                }
                // An orb stays: the server may still hold merged ones in it.
                if !orb {
                    self.remove_entity(p.item);
                }
            }
            // ClientPacketListener.handleSetEntityMotion: knockback on this player.
            C::SetEntityMotion(p) if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) => {
                if let Some(pl) = self.player.as_mut() {
                    pl.velocity = Vec3::new(p.velocity[0], p.velocity[1], p.velocity[2]);
                }
            }
            // The same packet for another entity: Entity.lerpMotion.
            C::SetEntityMotion(p) => self.remote_motion(p.id, p.velocity),
            // ClientPacketListener.handleProjectilePowerPacket: the power a
            // hurting projectile accelerates by, sent bundled with its motion.
            C::ProjectilePower(p) => {
                super::remote_projectiles::remote_projectile_power(
                    &mut self.creatures.projectiles,
                    p.id,
                    p.power,
                );
            }
            // ClientPacketListener.handleDamageEvent: LivingEntity.handleDamageEvent
            // jolts the walk animation and restarts the hurt flash, and the
            // hurt player's own client plays its hurt sound (`Player.getHurtSound`:
            // the damage type's effects) at `getSoundVolume` and
            // `Player.getVoicePitch`, which is 1.0 for every player. The
            // server's sound packet reaches every other player around, never
            // this one (#4149), and that copy is played at 1.0.
            // This player's own hurt tilt and flash come from the server too
            // (sent to the player itself): its own moves never hurt it.
            C::DamageEvent(p) => {
                let me = self.remote.as_ref().map(|r| r.entity_id);
                let own = me == Some(p.id);
                self.pvp_hurt(p.id, own, p.direct.map(|d| Some(d) == me));
                if own {
                    // For `events`: this client has no combat tracker of its own here.
                    let cause = self.remote.as_ref().and_then(|r| {
                        r.registries
                            .name("minecraft:damage_type", p.source_type)
                            .map(str::to_owned)
                    });
                    let by = p
                        .cause
                        .and_then(|id| u32::try_from(id).ok())
                        .and_then(|id| {
                            self.mob_index
                                .get(&self.mobs, id)
                                .map(|m| m.kind.id.to_string())
                        });
                    self.events
                        .note_hit(cause.unwrap_or_else(|| "unknown".into()), by);
                    if let Some(pl) = self.player.as_mut() {
                        pl.health.animate_hurt();
                        let sound = self.remote_hurt_sound(p.source_type);
                        let pitch = player_voice_pitch();
                        self.play_client_player_sound(sound, 1.0, pitch);
                    }
                } else if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                    m.limb_speed = 1.5;
                    m.hurt_time = 10;
                } else {
                    self.remote_dragon_hurt(p.id);
                }
            }
            C::HurtAnimation(p) => {
                if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                    // Player.animateHurt: the tilt turns around the yaw.
                    if let Some(pl) = self.player.as_mut() {
                        pl.health.animate_hurt_dir(p.yaw);
                    }
                } else if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                    m.hurt_time = 10;
                } else {
                    self.remote_dragon_hurt(p.id);
                }
            }
            C::EntityEvent(p) => {
                if p.event == super::fireworks::FIREWORK_EXPLODE_EVENT {
                    self.remote_firework_explosion(p.id);
                }
                self.remote_fangs_event(p.id as u32, p.event);
                self.remote_boat_event(p.id, p.event);
                // MinecartTNT.handleEntityEvent 70: prime the fuse the window draws.
                self.remote_tnt_cart_event(p.id, p.event);
                // Arrow.handleEntityEvent 0: a tipped arrow's burst of particles.
                if p.event == 0 {
                    self.remote_arrow_puff(p.id as u32);
                }
                // LivingEntity.handleEntityEvent 2: a spear's hit felt.
                if p.event == 2 {
                    self.remote_kinetic_hit(p.id);
                }
                // LocalPlayer.setPermissions also refreshes an open World Options screen.
                if (24..=28).contains(&p.event)
                    && self.remote.as_ref().is_some_and(|r| r.entity_id == p.id)
                {
                    self.ui.registries.permission = (p.event - 24) as u8;
                    self.remote_world_options_permission(p.event >= 26);
                }
                // Player.handleEntityEvent 22 / 23: reduced debug info on / off.
                if (22..=23).contains(&p.event)
                    && let Some(r) = self.remote.as_mut().filter(|r| r.entity_id == p.id)
                {
                    r.reduced_debug_info = p.event == 22;
                    self.debug_entries.invalidate();
                }
                // Ravager.handleEntityEvent 69: the roar's puff and push.
                if p.event == 69 {
                    self.remote_ravager_roar(p.id);
                }
                // ClientPacketListener.handleEntityEvent 63: a sniffer's digging sound.
                if p.event == 63 {
                    self.start_sniffer_digging_sound(p.id as u32);
                }
                // ClientPacketListener.handleEntityEvent 21: a guardian's hum.
                if p.event == 21 {
                    self.start_guardian_sound(p.id as u32);
                }
                // ZombieVillager.handleEntityEvent 16: the cure starts.
                if p.event == 16 {
                    self.zombie_villager_cure_sound(p.id as u32);
                }
                // Witch.handleEntityEvent 15: the magic puffs over its hat.
                if p.event == 15 {
                    self.witch_magic_particles(p.id as u32);
                }
                // MinecartTNT.handleEntityEvent 70: primeFuse on the client.
                if p.event == 70 {
                    self.remote_cart_primed(p.id as u32);
                }
                // Fox.handleEntityEvent 45: the crumbs of the food in its mouth.
                if p.event == 45 {
                    self.fox_eat_particles(p.id as u32);
                }
                // Rabbit.handleEntityEvent(1): the hop clocks, and
                // Entity.spawnSprintParticle (one block-dust crumb).
                if p.event == 1 {
                    let hop = {
                        let m = self.mob_index.get_mut(&mut self.mobs, p.id as u32);
                        if let Some(m) = m
                            && m.species() == horizon_client::mobs::Species::Rabbit
                        {
                            let sample =
                                (m.position, m.velocity, f64::from(m.width()), m.supporting);
                            horizon_client::mobs::rabbit::hop_event(m);
                            Some(sample)
                        } else {
                            None
                        }
                    };
                    if let Some((position, movement, width, support)) = hop {
                        let id = p.id as u32;
                        self.sprint_particle(position, movement, support, width, |game| {
                            // this.random: the rabbit's own, x then z.
                            game.mob_index
                                .get_mut(&mut game.mobs, id)
                                .map(|m| (m.random.next_double(), m.random.next_double()))
                        });
                    }
                }
                if p.event == 19
                    && let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32)
                {
                    horizon_client::mobs::aquatic::squid_cycle_event(m);
                }
                // LivingEntity.handleEntityEvent 55: another player swapped
                // its main and off hand (`swapHandItems`).
                if p.event == 55
                    && let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32)
                {
                    m.extra.swap_hands();
                }
                // LivingEntity.handleEntityEvent 47 to 52, 65 and 68: an
                // equipped item broke (onEquippedItemBroken).
                if let Some(slot) = horizon_client::mobs::item_break::equipment_break_slot(p.event)
                {
                    self.remote_equipment_break(p.id, slot);
                }
                // EntityEvent.DEATH: start the death animation.
                if p.event == 3
                    && let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32)
                {
                    m.health = 0.0;
                }
                // LivingEntity.handleEntityEvent 3 on this player: its death
                // sound, which Player.playSound plays for this client alone
                // (the server's goes to everyone else).
                if p.event == 3
                    && self.remote.as_ref().is_some_and(|r| r.entity_id == p.id)
                    && self.player.is_some()
                {
                    let sound = "minecraft:entity.player.death";
                    let pitch = player_voice_pitch();
                    self.play_client_player_sound(sound, 1.0, pitch);
                }
                // The events that start a mob's animation (the warden's).
                if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                    horizon_client::mobs::sync::entity_event(m, p.event as u8);
                }
                // LivingEntity.handleEntityEvent 46: portal particles from the
                // old position to the new one (chorus fruit, an enderman).
                if p.event == 46 {
                    self.entity_teleport_particles(p.id);
                }
                // LivingEntity.handleEntityEvent 60: the poof as the dead body goes.
                if p.event == 60
                    && let Some(m) = self.mob_index.get(&self.mobs, p.id as u32)
                {
                    let (at, w, h) = (m.position, f64::from(m.width()), f64::from(m.height()));
                    self.death_poof([at.x, at.y, at.z], w, h);
                }
                if p.event == super::totem::TOTEM_EVENT {
                    self.remote_totem(p.id);
                }
                // Mob.handleEntityEvent 20: the spawn poof.
                if p.event == 20 && self.mobs.iter().any(|m| m.id == p.id as u32) {
                    self.poof_particles(p.id as u32);
                }
                // The particle bursts of Villager 12 to 14 and 42, TamableAnimal
                // and AbstractHorse 6 and 7, Animal and Allay 18, Ocelot 40 and
                // 41, Dolphin 38: the table in `entity_event_burst` is the one
                // list of them; any other event draws nothing here.
                self.entity_event_particles(p.id as u32, p.event);
                // Snowball and ThrownEgg.handleEntityEvent 3: the crumbs where
                // the projectile is (the server removes it right after).
                if p.event == 3 {
                    let crumbs = self
                        .creatures
                        .projectiles
                        .iter()
                        .find(|pr| pr.id == p.id as u32)
                        .and_then(|pr| {
                            let item = match pr.kind {
                                horizon_client::projectiles::ProjectileKind::Snowball => {
                                    "minecraft:snowball"
                                }
                                horizon_client::projectiles::ProjectileKind::Egg => "minecraft:egg",
                                horizon_client::projectiles::ProjectileKind::Thrown(t) => t.item,
                                _ => return None,
                            };
                            Some(([pr.position.x, pr.position.y, pr.position.z], item))
                        });
                    if let Some((at, item)) = crumbs {
                        self.item_burst(at, item);
                    }
                }
                if p.event == super::combat::DROWN_EVENT {
                    self.remote_drown(p.id);
                }
                // Entity events 53 and 54: honey crumbs while sliding, and the
                // ten from a landing (`HoneyBlock.showSlideParticles` /
                // `showJumpParticles`).
                if horizon_client::landing::honey_particle_count(p.event).is_some() {
                    let at = if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                        self.player
                            .as_ref()
                            .map(|q| [q.position.x, q.position.y, q.position.z])
                    } else {
                        self.mobs
                            .iter()
                            .find(|m| m.id == p.id as u32)
                            .map(|m| [m.position.x, m.position.y, m.position.z])
                    };
                    if let Some(at) = at {
                        self.honey_entity_particles(at, p.event);
                    }
                }
                if p.event == super::consume::USE_ITEM_COMPLETE_EVENT
                    && self.remote.as_ref().is_some_and(|r| r.entity_id == p.id)
                {
                    self.remote_complete_using();
                }
            }
            C::Animate(p) => self.remote_animate(p.id, p.action),
            // ClientPacketListener.handleSwingAnimation: the entity swings
            // (LivingEntity.swing, which a swing in its first half ignores).
            C::SwingAnimation(p) => {
                let hand = if p.hand == 1 { Hand::Off } else { Hand::Main };
                let animation = horizon_client::first_person::SwingAnimation {
                    kind: p.kind.clamp(0, 255) as u8,
                    duration: p.duration,
                };
                if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                    self.swing_hand(hand, animation);
                } else if let Some(m) = self.mob_index.get_mut(&mut self.mobs, p.id as u32) {
                    m.swing_hand_modified(hand == Hand::Off, animation);
                }
            }
            // The player's own instances: its maximum health (the hearts the
            // HUD draws and the health it may hold), reach, speed and so on.
            // Then the received health again, which the old maximum clamped.
            C::UpdateAttributes(p) if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) => {
                let nameplate = self.remote_nameplate_values(&p.values);
                self.apply_own_attributes(p.values);
                if let Some(m) = self.player_model.as_mut() {
                    if let Some(value) = nameplate.name_tag {
                        m.set_name_tag_distance(value);
                    }
                    if let Some(value) = nameplate.below_name {
                        m.set_below_name_distance(value);
                    }
                }
                self.apply_remote_health();
            }
            C::UpdateAttributes(p) => self.remote_mob_attributes(p.id, &p.values),
            C::UpdateMobEffect(p) => self.remote_effect(&p),
            C::RemoveMobEffect(p) => {
                if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id)
                    && let Some(id) = self.remote_effect_id(p.effect)
                {
                    self.potions.effects.list.retain(|e| e.id != id);
                }
            }
            C::ChangeDifficulty(p) => {
                self.ui.difficulty = p.difficulty.clamp(0, 3) as u8;
                self.level.difficulty_locked = p.locked;
                self.remote_world_options_difficulty(self.ui.difficulty, p.locked);
            }
            C::GameRuleValues(p) => self.remote_world_options_rules(p.entries),
            // ClientPacketListener.handleSetHealth: LocalPlayer.hurtTo.
            C::SetHealth(p) => {
                let flash = self.remote.as_mut().is_some_and(|r| {
                    r.health = (p.health, p.food, p.saturation);
                    std::mem::replace(&mut r.flash_on_set_health, true)
                });
                if flash && let Some(pl) = self.player.as_mut() {
                    pl.health.flash_health_change(p.health);
                }
                self.apply_remote_health();
                // The death screen opens (or the respawn is asked for) with the
                // death packet when the rule hides it.
                let shown = self.remote.as_ref().is_none_or(|r| r.show_death_screen);
                if p.health <= 0.0 && shown && self.screen != Screen::Dead && self.player.is_some()
                {
                    self.close_dialogs();
                    self.set_screen(Screen::Dead);
                    self.ui.death_time = 0;
                    self.ui.death_confirm = None;
                    self.capture(false);
                }
            }
            C::PlayerCombatKill(p) => {
                // The component resolves in the client's current language, as
                // the chat line does, not in the language the session joined
                // with (`Remote.lang` is the vanilla server-language table).
                let message = remote_text(&p.message);
                let me = self.remote.as_ref().is_some_and(|r| r.entity_id == p.id);
                if let Some(r) = self.remote.as_mut() {
                    r.death_message = Some(message);
                }
                // ClientPacketListener.handlePlayerCombatKill: the death
                // screen, whatever health was last sent, or `respawn()` at
                // once when it is hidden.
                if me && self.screen != Screen::Dead && self.player.is_some() {
                    if self.remote.as_ref().is_none_or(|r| r.show_death_screen) {
                        self.close_dialogs();
                        self.set_screen(Screen::Dead);
                        self.ui.death_time = 0;
                        self.ui.death_confirm = None;
                        self.capture(false);
                    } else {
                        self.respawn_player()?;
                    }
                }
            }
            C::TrackedWaypoint(p) => self.handle_waypoint(&p),
            C::SetExperience(p) => {
                self.experience = Experience {
                    level: p.level,
                    progress: p.progress,
                    total: p.total,
                }
            }
            // ClientPacketListener.handleSetTime / ClientClockManager.handleUpdates:
            // advance all clocks to the packet's game time before applying its
            // (possibly partial) updates. The sky still uses only the default clock.
            C::SetTime(p) => {
                self.world.game_time = p.game_time;
                self.remote_clock_tick();
                if let Some(r) = self.remote.as_mut() {
                    for clock in p.clocks {
                        if r.registries
                            .name("minecraft:world_clock", clock.clock)
                            .is_some()
                        {
                            r.clocks.insert(clock.clock, clock);
                        }
                    }
                    self.world.day_time = r.clock_time(r.default_clock);
                }
            }
            C::GameEvent(p) => match p.event {
                // NO_RESPAWN_BLOCK_AVAILABLE: ClientPacketListener.handleGameEvent
                // sends the player this system message.
                0 => self.system_message(horizon_client::text::Text::translate(
                    "block.minecraft.spawn.not_valid",
                    &[],
                )),
                // LEVEL_CHUNKS_LOAD_START: the server has begun sending the level.
                13 => {
                    if let Some(r) = self.remote.as_mut() {
                        r.level_load.loading_packets_received();
                    }
                }
                // GUARDIAN_ELDER_EFFECT: the apparition, and the curse sound
                // unless the elder guardian is silent.
                10 => {
                    self.particles.elder_guardian();
                    if p.param == 1.0
                        && let Some(at) = self.player.as_ref().map(|m| m.position)
                    {
                        self.play_client_local_sound(
                            LevelSound::new(
                                "minecraft:entity.elder_guardian.curse",
                                [at.x, at.y, at.z],
                                1.0,
                                1.0,
                            )
                            .in_source(mod_api::SoundSource::Hostile),
                        );
                    }
                }
                // WIN_GAME: the poem and credits, then PERFORM_RESPAWN.
                4 if self.screen != Screen::Win => {
                    self.open_win_screen(true, super::win::Finish::RemoteRespawn);
                }
                // START_RAINING, STOP_RAINING, RAIN_LEVEL_CHANGE and THUNDER_LEVEL_CHANGE.
                event @ (1 | 2 | 7 | 8) => self.world.weather.game_event(event, p.param),
                3 => {
                    let mode = p.param as i32;
                    self.set_mode_id(mode);
                    if let Some(r) = self.remote.as_mut() {
                        r.game_mode = mode;
                    }
                }
                // IMMEDIATE_RESPAWN: LocalPlayer.setShowDeathScreen.
                11 => {
                    if let Some(r) = self.remote.as_mut() {
                        r.show_death_screen = p.param == 0.0;
                    }
                }
                // PLAY_ARROW_HIT_SOUND: ClientPacketListener.handleGameEvent
                // plays ARROW_HIT_PLAYER at player eye height.
                6 => {
                    if let Some(at) = self.eye_position() {
                        self.play_client_local_sound(
                            LevelSound::new("minecraft:entity.arrow.hit_player", at, 0.18, 0.45)
                                .in_source(mod_api::SoundSource::Players),
                        );
                    }
                }
                // PUFFER_FISH_STING: ClientPacketListener.handleGameEvent
                // plays PUFFER_FISH_STING at player position.
                9 => {
                    if let Some(at) = self.player.as_ref().map(|m| m.position) {
                        self.play_client_local_sound(
                            LevelSound::new(
                                "minecraft:entity.puffer_fish.sting",
                                [at.x, at.y, at.z],
                                1.0,
                                1.0,
                            )
                            .in_source(mod_api::SoundSource::Neutral),
                        );
                    }
                }
                // LIMITED_CRAFTING: LocalPlayer.setDoLimitedCrafting.
                12 => {
                    if let Some(r) = self.remote.as_mut() {
                        r.do_limited_crafting = p.param == 1.0;
                    }
                }
                _ => {}
            },
            C::PlayerAbilities(p) => {
                self.creative = p.flags & 8 != 0;
                if let Some(r) = self.remote.as_mut() {
                    r.abilities = Some(p);
                }
                self.apply_remote_abilities();
            }
            // ClientPacketListener.handleSetHeldSlot: a hotbar slot only, and the
            // carried index stays, so the next tick answers with SetCarriedItem.
            C::SetHeldSlot(p) => {
                if (0..9).contains(&p.slot) {
                    self.selected = p.slot as usize;
                }
            }
            C::ContainerSetContent(p) => {
                self.remote_menu_state(p.container, p.state);
                for (slot, item) in p.items.iter().enumerate() {
                    let stack = self.to_stack(item);
                    self.set_menu_slot(p.container, slot as i32, stack);
                }
                self.carried = self.to_stack(&p.carried);
                self.remote_synced();
            }
            C::ContainerSetSlot(p) => {
                self.remote_menu_state(p.container, p.state);
                let stack = self.to_stack(&p.item);
                self.tutorial_get_item(stack.as_ref());
                // ClientPacketListener.handleContainerSetSlot: a hotbar slot that
                // gains items squeezes for 5 ticks.
                if p.container == 0
                    && (36..=45).contains(&p.slot)
                    && let Some(new) = &stack
                {
                    let (pop, old) = match p.slot {
                        45 => (9, self.offhand.as_ref()),
                        s => (s as usize - 36, self.inventory[s as usize - 36].as_ref()),
                    };
                    if old.is_none_or(|old| old.count < new.count) {
                        self.hotbar_pop[pop] = 5;
                    }
                }
                let before = self.creative_stacks();
                self.set_menu_slot(p.container, i32::from(p.slot), stack);
                self.remote_synced_since(&before);
            }
            C::SetCursorItem(p) => {
                let stack = self.to_stack(&p.item);
                self.tutorial_get_item(stack.as_ref());
                self.carried = stack;
            }
            C::SetPlayerInventory(p) => {
                let stack = self.to_stack(&p.item);
                self.tutorial_get_item(stack.as_ref());
                let before = self.creative_stacks();
                let held = match p.slot {
                    0..=35 => self.inventory[p.slot as usize].as_ref(),
                    40 => self.offhand.as_ref(),
                    _ => None,
                };
                let changed = !super::hand::matches_for_swap(held, stack.as_ref());
                if let Ok(slot) = usize::try_from(p.slot) {
                    self.remote_hand_slot_set(slot, changed);
                }
                match p.slot {
                    0..=35 => self.inventory[p.slot as usize] = stack,
                    36..=39 => self.armor[3 - (p.slot - 36) as usize] = stack,
                    // Inventory.SLOT_OFFHAND.
                    40 => self.offhand = stack,
                    _ => {}
                }
                self.remote_synced_since(&before);
            }
            C::ContainerSetData(p) if self.open_menu_name(p.container) == "minecraft:lectern" => {
                self.remote_lectern_page(i32::from(p.value));
            }
            C::ContainerSetData(p) if self.remote_station_open(p.container) => {
                self.remote_station_data(usize::try_from(p.id).unwrap_or(usize::MAX), p.value);
            }
            C::ContainerSetData(p) if self.native_screen_data(p.container, p.id, p.value) => {}
            C::ContainerSetData(p) => {
                let v = i32::from(p.value);
                match self
                    .container
                    .and_then(|key| self.world.block_entities.get_mut(&key))
                {
                    Some(BlockEntity::Furnace(f)) => match p.id {
                        0 => f.lit_remaining = v,
                        1 => f.lit_total = v,
                        2 => f.cooking_timer = v,
                        3 => f.cooking_total = v,
                        _ => {}
                    },
                    // CrafterBlockEntity.containerData.
                    Some(BlockEntity::Crafter(c)) => match usize::try_from(p.id) {
                        Ok(k @ 0..9) => c.disabled[k] = v == 1,
                        Ok(9) => c.triggered = v,
                        _ => {}
                    },
                    // BrewingStandBlockEntity.dataAccess.
                    Some(BlockEntity::Brewing(b)) => match p.id {
                        0 => b.brew_time = v,
                        1 => b.fuel = v,
                        2 => b.total_brew_time = v,
                        3 => b.total_fuel = v,
                        _ => {}
                    },
                    _ => {}
                }
            }
            C::OpenScreen(p) => self.remote_open_screen(p),
            C::MountScreenOpen(p) => self.remote_open_mount(p),
            C::UpdateRecipes(p) => self.remote_recipe_properties(p),

            C::RecipeBookAdd(p) => self.remote_recipe_book_add(p),
            C::RecipeBookRemove(p) => self.remote_recipe_book_remove(p),
            C::RecipeBookSettings(p) => self.remote_recipe_book_settings(p),
            C::PlaceGhostRecipe(p) => self.remote_ghost_recipe(p),
            C::MerchantOffers(p) => self.remote_merchant_offers(p),
            C::ContainerClose(_) => {
                // `LocalPlayer.clientSideCloseContainer` sets the screen to
                // null, including dialogs and WaitingForResponseScreen. This
                // differs from clearDialog, which restores the parent screen.
                let dialog = self.dialog_open();
                if dialog {
                    self.close_dialogs();
                    self.ui.chat_screen = None;
                    self.book = None;
                    self.sign_edit = None;
                }
                if !self.remote_lectern_closed() && (self.screen == Screen::Inventory || dialog) {
                    self.remote_close_local();
                    self.set_screen(Screen::Playing);
                    self.capture(true);
                }
            }
            C::TagQuery(p) => self.answer_tag_query(p.transaction, p.tag),
            C::Unknown(id, data)
                if id == horizon_protocol::ids::play::clientbound::COMMAND_SUGGESTIONS =>
            {
                self.handle_command_suggestions(&data);
            }
            C::CustomChatCompletions(p) => {
                // ClientPacketListener.handleCustomChatCompletions.
                if let Some(r) = self.remote.as_mut() {
                    horizon_client::command::modify_custom_completions(
                        &mut r.custom_completions,
                        horizon_client::command::CustomCompletionAction::from_id(p.action),
                        &p.entries,
                    );
                    self.ui.registries.custom_completions = r.custom_completions.clone();
                }
            }
            C::UpdateTags(p) => self.remote_update_tags(&p),
            C::Commands(p) => {
                let tree = horizon_client::command::network::from_packet(
                    &p,
                    horizon_client::command::vanilla::dispatcher(),
                );
                // The chat's tree follows the server's now; a mod's commands
                // go into it again (`chat_tree_for`).
                let chat_tree = self.chat_tree_for(Some(&tree));
                if let Some(r) = self.remote.as_mut() {
                    r.chat_tree = chat_tree;
                    r.chat_tree_received = true;
                }
            }
            C::Cooldown(p) => {
                // ClientPacketListener.handleItemCooldown: the server starts
                // (or confirms) a group's cooldown; 0 ends it.
                let group: Arc<str> = Arc::from(p.group.as_str());
                if p.duration == 0 {
                    self.cooldowns.remove(&group);
                } else {
                    self.cooldowns.add(group, p.duration);
                }
            }
            // ClientPacketListener.handleGameTestHighlightPos.
            C::GameTestHighlightPos(p) => self.highlight_test_pos(p.absolute, p.relative),
            // ClientPacketListener.handleTestInstanceBlockStatus.
            C::TestInstanceBlockStatus(p) => {
                self.block_editor_status(remote_text(&p.status), p.size);
            }
            C::SystemChat(p) => {
                // The component as sent, translated in the client's own
                // language; mods may change or cancel it. ChatListener.handleOverlay
                // for the action bar.
                let mut text = remote_text(&p.content);
                if !self.chat_received(true, p.overlay, 0, None, None, &mut text) {
                    return Ok(());
                }
                if p.overlay {
                    self.say_system_queued(&text);
                    self.titles.set_overlay(text, false);
                } else if self.add_chat_line(text.clone(), horizon_client::chat::ChatSource::Server)
                {
                    // handleSystemMessage: said once the chat takes it.
                    self.say_system_chat_queued(&text);
                }
            }
            C::PlayerChat(p) => {
                use horizon_client::scoreboard::tag_to_json;
                // SignedMessageBody.Packed.unpack: the last seen it names must
                // be in the cache; then the message joins it
                // (MessageSignatureCache.push).
                let Some(r) = self.remote.as_mut() else {
                    return Ok(());
                };
                // handlePlayerChat: the global index counts every chat packet; one out of
                // order disconnects.
                let expected = r.next_chat_index;
                r.next_chat_index = r.next_chat_index.wrapping_add(1);
                if p.global_index != expected {
                    log::error!(
                        "Missing or out-of-order chat message from server, expected index {expected} but got {}",
                        p.global_index
                    );
                    let reason = horizon_client::text::Text::translate(
                        "multiplayer.disconnect.bad_chat_index",
                        &[],
                    );
                    return self.leave_server(Some(reason.plain()));
                }
                let last_seen: Option<Vec<Signature>> =
                    p.last_seen.iter().map(|s| r.signatures.unpack(s)).collect();
                let Some(last_seen) = last_seen else {
                    log::error!(
                        "Message from player with ID {:032x} referenced unrecognized signature id",
                        p.sender
                    );
                    let reason = horizon_client::text::Text::translate(
                        "multiplayer.disconnect.invalid_packet",
                        &[],
                    );
                    return self.leave_server(Some(reason.plain()));
                };
                let signature = p
                    .signature
                    .as_deref()
                    .and_then(horizon_protocol::secure_chat::signature);
                r.signatures.push(&last_seen, signature.as_ref());
                // The sender's PlayerInfo: none is an error line; else their
                // `SignedMessageValidator` decides (`updateAndValidate`), and a message it
                // takes without a signature of its own shows as unsigned.
                if !r.tab_players.contains_key(&p.sender) {
                    log::error!(
                        "Received player chat packet for unknown player with ID: {:032x}",
                        p.sender
                    );
                    self.handle_chat_message_error(&p.chat_type, signature.as_ref());
                    return Ok(());
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64);
                let incoming = secure_chat::Incoming {
                    sender: p.sender,
                    index: p.index,
                    signature: signature.as_ref(),
                    body: secure_chat::Body {
                        content: &p.content,
                        timestamp: p.timestamp,
                        salt: p.salt,
                        last_seen: &last_seen,
                    },
                    unsigned_content: p.unsigned_content.as_ref(),
                    filter: &p.filter,
                };
                let Some(validated) = r.validate_chat(&incoming, now) else {
                    self.handle_chat_message_error(&p.chat_type, signature.as_ref());
                    return Ok(());
                };
                let signature = signature.filter(|_| validated == secure_chat::Validated::Signed);
                // ChatListener.handlePlayerChatMessage: Only Show Secure Chat
                // shows the signed content, never the server's replacement.
                // The narrator says the message's own decorated content
                // (`narrateChatMessage(boundChatType, message.decoratedContent())`).
                let decorated = p
                    .unsigned_content
                    .as_ref()
                    .map_or_else(|| serde_json::Value::from(p.content.as_str()), tag_to_json);
                let content = if self.settings.file.bool("onlyShowSecureChat") {
                    serde_json::Value::from(p.content.as_str())
                } else {
                    decorated.clone()
                };
                let narration = self
                    .decorate_chat(&p.chat_type, &decorated, Decoration::Narration)
                    .plain();
                let line = self.decorate_chat(&p.chat_type, &content, Decoration::Chat);
                let trust = self.chat_trust(
                    p.sender,
                    signature.is_some(),
                    p.timestamp,
                    &p.content,
                    p.unsigned_content.as_ref(),
                    &line,
                );
                // A hidden or fully filtered message counts as processed, not shown.
                let Some((line, narration)) = self.shown_player_chat(&p, line, narration) else {
                    if let Some(signature) = &signature {
                        self.mark_message_processed(signature, false);
                    }
                    return Ok(());
                };
                let listened = mod_api::__private::listened(mod_api::HookId::ClientChatReceived);
                let player =
                    listened.then(|| chat_received::player_chat_info(&p, signature.is_some()));
                self.handle_player_chat(QueuedChat {
                    line,
                    signature,
                    trust,
                    narration,
                    sender: p.sender,
                    chat_type: p.chat_type.chat_type,
                    player,
                });
            }
            C::DisguisedChat(p) => {
                let content = horizon_client::scoreboard::tag_to_json(&p.message);
                let narration = self
                    .decorate_chat(&p.chat_type, &content, Decoration::Narration)
                    .plain();
                let line = self.decorate_chat(&p.chat_type, &content, Decoration::Chat);
                self.handle_player_chat(QueuedChat {
                    line,
                    signature: None,
                    trust: ChatTrust::Secure,
                    narration,
                    sender: 0,
                    chat_type: p.chat_type.chat_type,
                    player: None,
                });
            }
            C::DeleteChat(p) => self.handle_delete_chat(&p.signature)?,
            C::PlayerInfoUpdate(p) => {
                if let Some(r) = self.remote.as_mut() {
                    // handlePlayerInfoUpdate: new entries first, then every
                    // action on every known entry.
                    // INITIALIZE_CHAT: the session (or none) each entry carries; the
                    // player's validator is made again when their chat is next read.
                    if p.actions & 2 != 0 {
                        for e in &p.entries {
                            r.chat_sessions.insert(e.uuid, e.chat_session.clone());
                            r.chat_validators.remove(&e.uuid);
                        }
                    }
                    if p.actions & 1 != 0 {
                        for e in &p.entries {
                            if let Some(name) = &e.name {
                                r.tab_players.entry(e.uuid).or_insert_with(|| {
                                    horizon_client::tab_list::TabPlayer::new(e.uuid, name.clone())
                                });
                                if let Some(t) = e.properties.iter().find(|p| p.name == "textures")
                                {
                                    // Without the services' profile property keys a
                                    // signature can't be checked; a signed property
                                    // counts as genuine.
                                    use horizon_client::skins::{self, Signature};
                                    let signature = if t.signature.is_some() {
                                        Signature::Signed
                                    } else {
                                        Signature::Unsigned
                                    };
                                    let textures =
                                        skins::unpack(&t.value, signature, skins::texture_uris());
                                    r.skin_textures.entry(e.uuid).or_insert(textures);
                                }
                            }
                        }
                    }
                }
                for e in p.entries {
                    if mod_api::__private::listened(mod_api::HookId::ClientPlayerChangeGameType)
                        && self.player.is_some()
                        && let Some(mode) = e.game_mode
                        && let Some(r) = self.remote.as_ref()
                        && r.profile == e.uuid
                        && let Some(info) = r.tab_players.get(&e.uuid)
                        && info.game_mode != mode
                    {
                        let current = info.game_mode;
                        self.native_client_player_change_game_type(
                            self.own_entity_id(),
                            e.uuid,
                            current,
                            mode,
                        );
                    }
                    if let Some(r) = self.remote.as_mut() {
                        if let Some(t) = r.tab_players.get_mut(&e.uuid) {
                            if let Some(mode) = e.game_mode {
                                t.game_mode = mode;
                            }
                            if let Some(latency) = e.latency {
                                t.latency = latency;
                            }
                            if let Some(name) = &e.display_name {
                                t.display_name =
                                    name.as_ref().map(horizon_client::scoreboard::tag_to_json);
                            }
                            if let Some(order) = e.list_order {
                                t.list_order = order;
                            }
                            if let Some(hat) = e.show_hat {
                                t.show_hat = hat;
                            }
                            match e.listed {
                                Some(true) => {
                                    r.listed.insert(e.uuid);
                                }
                                Some(false) => {
                                    r.listed.remove(&e.uuid);
                                }
                                None => {}
                            }
                        }
                        if let Some(name) = e.name {
                            r.players.insert(e.uuid, name);
                        }
                    }
                }
            }
            C::PlayerInfoRemove(p) => {
                if let Some(r) = self.remote.as_mut() {
                    for u in p.uuids {
                        r.players.remove(&u);
                        r.tab_players.remove(&u);
                        r.chat_sessions.remove(&u);
                        r.chat_validators.remove(&u);
                        r.skin_textures.remove(&u);
                        r.listed.remove(&u);
                        r.attachments.remove(&Holder::Player(u));
                    }
                }
            }
            C::TabList(p) => {
                // handleTabListCustomisation: empty text clears it.
                let text = |t: &Tag| {
                    let json = horizon_client::scoreboard::tag_to_json(t);
                    let text = horizon_client::scoreboard::comp_text(&json);
                    text.spans
                        .iter()
                        .any(|span| !span.text.is_empty())
                        .then_some(text)
                };
                self.tab_list.header = text(&p.header);
                self.tab_list.footer = text(&p.footer);
            }
            C::InitializeBorder(p) => {
                let b = &mut self.border;
                b.set_center([p.center_x, p.center_z]);
                if p.lerp_time > 0 {
                    b.lerp_between(p.old_size, p.new_size, p.lerp_time);
                } else {
                    b.set_size(p.new_size);
                }
                b.set_absolute_max_size(p.absolute_max_size);
                b.warning_distance = p.warning_blocks;
                b.warning_time = p.warning_time;
            }
            C::SetBorderCenter(p) => self.border.set_center([p.x, p.z]),
            C::SetBorderLerpSize(p) => {
                self.border
                    .lerp_between(p.old_size, p.new_size, p.lerp_time)
            }
            C::SetBorderSize(p) => self.border.set_size(p.size),
            C::SetBorderWarningDelay(p) => self.border.warning_time = p.delay,
            C::SetBorderWarningDistance(p) => self.border.warning_distance = p.blocks,
            C::SetObjective(p) => self.remote_objective(p),
            C::SetDisplayObjective(p) => {
                // DisplaySlot ids: list, sidebar, below_name, then the team sidebars.
                let slot = match p.slot {
                    0 => "list".to_owned(),
                    2 => "below_name".to_owned(),
                    n @ 3..=18 => format!(
                        "sidebar.team.{}",
                        horizon_client::text::COLORS[(n - 3) as usize].1
                    ),
                    _ => "sidebar".to_owned(),
                };
                let board = &mut self.score.board;
                let objective = board.objective(&p.objective).map(|o| o.name.clone());
                board.set_display_objective(&slot, objective.as_deref());
            }
            C::SetScore(p) => {
                let board = &mut self.score.board;
                if board.objective(&p.objective).is_some() {
                    let s = board.score_mut(&p.owner, &p.objective);
                    s.value = p.score;
                    s.display = p
                        .display
                        .as_ref()
                        .map(horizon_client::scoreboard::tag_to_json);
                    s.number_format = p.number_format.as_ref().map(number_format);
                }
            }
            C::ResetScore(p) => match &p.objective {
                Some(o) => self.score.board.reset_one(&p.owner, o),
                None => self.score.board.reset_all(&p.owner),
            },
            C::SetPlayerTeam(p) => self.remote_team(p),
            C::SetSimulationDistance(p) => {
                if let Some(r) = self.remote.as_mut() {
                    r.simulation_distance = p.distance;
                }
            }
            C::SetChunkCacheRadius(p) => {
                self.streaming.radius = self
                    .settings
                    .render_distance
                    .min(p.distance.saturating_add(1))
                    .max(2);
                if let Some(r) = self.remote.as_mut() {
                    r.view_distance = p.distance;
                    let center = r.chunk_center;
                    // ClientChunkCache.updateViewRadius: a new storage of that size.
                    self.drop_remote_chunks_out_of_range(center, p.distance);
                }
            }
            C::SetChunkCacheCenter(p) => {
                if let Some(r) = self.remote.as_mut() {
                    r.chunk_center = (p.x, p.z);
                    let distance = r.view_distance;
                    self.drop_remote_chunks_out_of_range((p.x, p.z), distance);
                }
            }
            C::BlockEntityData(p) => self.remote_block_entity_packet(p),
            C::OpenSignEditor(p) => self.remote_open_sign_editor(p),
            C::OpenBook(p) => self.remote_open_book(p.hand),
            // handleSetSpawn: ClientLevel.setRespawnData, which spawn compasses point at.
            C::SetDefaultSpawnPosition(p) => {
                let [x, y, z] = p.pos.pos;
                self.spawn = (p.pos.dimension == "minecraft:overworld")
                    .then(|| Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5));
                // 1.20.2 handleSetSpawn notified ReceivingLevelScreen here; the later
                // LEVEL_CHUNKS_LOAD_START game event did not exist yet.
                if let Some(r) = self.remote.as_mut()
                    && r.session
                        .server_version()
                        .at_most(horizon_protocol::adapter::ServerVersion::V1_20_2)
                {
                    r.level_load.loading_packets_received();
                }
            }
            C::ServerData(p) => {
                if let Some(r) = self.remote.as_mut()
                    && let Some(enforced) = r.session.legacy_secure_chat_enforcement()
                    && r.enforces_secure_chat != enforced
                {
                    r.enforces_secure_chat = enforced;
                    r.chat_validators.clear();
                }
                self.remote_server_data(p.icon.as_deref());
            }
            C::MapItemData(p) => self.remote_map_item_data(&p),
            // handleTickingState / handleTickingStep: the level's TickRateManager.
            C::TickingState(p) => {
                self.clock.set_rate(p.rate);
                self.clock.set_frozen(p.frozen);
            }
            C::TickingStep(p) => self.clock.set_frozen_ticks_to_run(p.steps),
            C::UpdateAdvancements(p) => self.remote_advancements(p),
            // handleAwardStats: StatsCounter.setValue for each.
            C::AwardStats(p) => {
                let Some(r) = self.remote.as_ref() else {
                    return Ok(());
                };
                let mut known = Vec::new();
                for (ty, value, count) in p.stats {
                    let Some(ty) = r
                        .builtin
                        .get("minecraft:stat_type")
                        .and_then(|t| t.name(ty))
                    else {
                        continue;
                    };
                    let values = match ty {
                        "minecraft:mined" => "minecraft:block",
                        "minecraft:killed" | "minecraft:killed_by" => "minecraft:entity_type",
                        "minecraft:custom" => "minecraft:custom_stat",
                        _ => "minecraft:item",
                    };
                    if let Some(key) = r.builtin.get(values).and_then(|v| v.name(value)) {
                        known.push((ty.to_owned(), key.to_owned(), count));
                    }
                }
                for (ty, key, count) in known {
                    self.stats.values.entry(ty).or_default().insert(key, count);
                }
            }
            // ClientPacketListener.setActionBarText, setTitleText,
            // setSubtitleText, setTitlesAnimation and handleTitlesClear.
            C::BossEvent(p) => self.apply_boss_event(&p),
            C::SetActionBarText(p) => self.titles.set_overlay(remote_text(&p.text), false),
            C::SetTitleText(p) => self.titles.set_title(remote_text(&p.text)),
            C::SetSubtitleText(p) => self.titles.set_subtitle(remote_text(&p.text)),
            C::SetTitlesAnimation(p) => self.titles.set_times(p.fade_in, p.stay, p.fade_out),
            // ClientCommonPacketListenerImpl.handleShowDialog / handleClearDialog.
            C::ShowDialog(p) => self.show_play_dialog(&p.dialog),
            C::ClearDialog(_) => self.clear_dialog(),
            // ClientPacketListener.handleLowDiskSpaceWarning.
            C::LowDiskSpaceWarning(_) => horizon_client::chunk_io::report_low_disk_space(),
            C::ClearTitles(p) => {
                self.titles.clear();
                if p.value {
                    self.titles.reset_times();
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// `handleAddObjective`: add, remove or change an objective.
    fn remote_objective(&mut self, p: play::SetObjective) {
        use horizon_client::scoreboard::{RenderType, tag_to_json};
        let board = &mut self.score.board;
        match (p.method, p.display) {
            (1, _) => board.remove_objective(&p.name),
            (method, Some((name, render, format))) => {
                let render = if render == 1 {
                    RenderType::Hearts
                } else {
                    RenderType::Integer
                };
                let format = format.as_ref().map(number_format);
                if method == 0 {
                    board.add_objective(
                        &p.name,
                        "dummy",
                        tag_to_json(&name),
                        render,
                        false,
                        format,
                    );
                } else if let Some(o) = board.objective_mut(&p.name) {
                    o.display_name = tag_to_json(&name);
                    o.render_type = render;
                    o.number_format = format;
                }
            }
            _ => {}
        }
    }
    /// `handleSetPlayerTeamPacket`.
    fn remote_team(&mut self, p: play::SetPlayerTeam) {
        use horizon_client::scoreboard::{CollisionRule, Visibility, tag_to_json};
        let board = &mut self.score.board;
        if p.method == 1 {
            board.remove_team(&p.name);
            return;
        }
        if p.method == 0 {
            board.add_team(&p.name);
        }
        if let Some(params) = &p.parameters
            && let Some(team) = board.team_mut(&p.name)
        {
            team.display_name = tag_to_json(&params.display_name);
            team.prefix = tag_to_json(&params.prefix);
            team.suffix = tag_to_json(&params.suffix);
            team.color = params
                .color
                .and_then(|c| horizon_client::text::COLORS.get(c as usize))
                .map(|c| c.1.to_owned());
            team.allow_friendly_fire = params.options & 1 != 0;
            team.see_friendly_invisibles = params.options & 2 != 0;
            team.nametag_visibility = Visibility::ALL
                .get(params.name_tag_visibility as usize)
                .copied()
                .unwrap_or(Visibility::Always);
            team.collision_rule = CollisionRule::ALL
                .get(params.collision_rule as usize)
                .copied()
                .unwrap_or(CollisionRule::Always);
        }
        match p.method {
            0 | 3 => {
                for player in &p.players {
                    board.add_to_team(player, &p.name);
                }
            }
            // Scoreboard.removePlayerFromTeam: only out of this team (vanilla
            // throws for a player in another one).
            4 => {
                for player in &p.players {
                    if board.players_team(player).is_some_and(|t| t.name == p.name) {
                        board.remove_from_team(player);
                    }
                }
            }
            _ => {}
        }
    }
    /// The player list's view of the connection: the listed players (with
    /// their downloaded skins once ready), online mode, and which players
    /// are in the level with an upside-down name.
    pub(super) fn publish_tab_object_skins(&mut self) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let fingerprint = (
            r.listed.len(),
            r.listed.iter().fold(0, |xor, uuid| xor ^ uuid),
            r.skins_revision,
            self.gui.atlas.object_skins_revision,
        );
        if r.published_skins == Some(fingerprint) {
            return;
        }
        for uuid in &r.listed {
            let Some(p) = r.tab_players.get(uuid) else {
                continue;
            };
            let skin = r.skins.get(uuid).map_or_else(
                || horizon_client::tab_list::default_skin(*uuid),
                String::as_str,
            );
            for key in [
                format!("id:{uuid:032x}"),
                format!("name:{}", p.name.to_ascii_lowercase()),
            ] {
                if object_skin_changed(&self.gui.atlas.object_skins, &key, skin) {
                    self.gui.atlas.set_object_skin(key, skin.to_owned());
                }
            }
        }
        // Publication itself mutates the atlas; cache its final revision.
        r.published_skins = Some((
            fingerprint.0,
            fingerprint.1,
            fingerprint.2,
            self.gui.atlas.object_skins_revision,
        ));
    }

    pub(super) fn remote_tab_players(
        &mut self,
    ) -> Option<(Vec<horizon_client::tab_list::TabPlayer>, bool, Vec<u128>)> {
        self.update_remote_skins();
        self.publish_tab_object_skins();
        let mut players: Vec<_> = {
            let r = self.remote.as_ref()?;
            r.listed
                .iter()
                .filter_map(|u| r.tab_players.get(u).cloned())
                .collect()
        };
        for p in &mut players {
            p.skin = self
                .remote
                .as_ref()
                .and_then(|r| r.skins.get(&p.uuid).cloned())
                .unwrap_or_else(|| horizon_client::tab_list::default_skin(p.uuid).to_owned());
        }
        let r = self.remote.as_ref()?;
        let upside_down = players
            .iter()
            .filter(|p| {
                (p.name == "Dinnerbone" || p.name == "Grumm")
                    && (p.uuid == r.profile || r.level_players.values().any(|u| *u == p.uuid))
            })
            .map(|p| p.uuid)
            .collect();
        Some((players, r.online_mode, upside_down))
    }

    /// Resolve inline `object:player` profiles through the same skin cache as
    /// player heads on items. The next frame uses the uploaded atlas texture.
    pub(super) fn resolve_inline_profiles(
        &mut self,
        profiles: &[std::sync::Arc<serde_json::Value>],
    ) {
        use horizon_client::skins;
        for profile in profiles {
            let skin = skins::head_skin(
                profile,
                &mut self.skin_profiles.borrow_mut(),
                &mut self.skin_downloads.borrow_mut(),
                |location| self.gui.atlas.sprites.contains_key(location),
            );
            let id = skins::parse_profile(profile).id;
            let name = profile
                .as_str()
                .or_else(|| profile.get("name").and_then(serde_json::Value::as_str));
            // The player list's skin for the same profile is drawn too.
            let keys = [
                Some(format!("id:{id:032x}")),
                name.map(|n| format!("name:{}", n.to_ascii_lowercase())),
            ];
            for key in keys.iter().flatten() {
                if let Some(listed) = self.gui.atlas.object_skins.get(key)
                    && listed.starts_with("skins/")
                {
                    let listed = listed.clone();
                    self.again_skin(&listed);
                }
            }
            self.gui
                .atlas
                .set_object_skin(format!("id:{id:032x}"), skin.clone());
            if let Some(name) = name {
                self.gui
                    .atlas
                    .set_object_skin(format!("name:{}", name.to_ascii_lowercase()), skin);
            }
        }
    }
    fn poll_own_profile(&mut self) {
        let auth = self.auth();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        match &mut r.own_profile {
            OwnProfile::Idle => {
                let Some(auth) = auth else {
                    r.own_profile = OwnProfile::Done(None);
                    return;
                };
                let (send, recv) = std::sync::mpsc::channel();
                std::thread::Builder::new()
                    .name("Own profile".into())
                    .spawn(move || {
                        let _ = send.send(horizon_client::auth::own_textures(auth.as_ref()));
                    })
                    .ok();
                r.own_profile = OwnProfile::Fetching(recv);
            }
            OwnProfile::Fetching(recv) => {
                if let Ok(own) = recv.try_recv() {
                    r.own_profile = OwnProfile::Done(own);
                }
            }
            OwnProfile::Done(_) => {}
        }
    }
    /// `PlayerInfo.getSkin` for every player entity, including NPCs that are
    /// never in the tab list: resolve each profile's textures and give the
    /// level's player models their skin.
    /// The 3D skin layers model for a player (Options → Skin Customization,
    /// `threeDSkinLayers`): `kind`'s outer layer with one slab per painted
    /// texel, built once per (skin, slim) from the decoded image the skin
    /// downloader still holds (`horizon_client::mobs::skin_layers`). `None`
    /// while the option is off or the image is not in memory yet.
    fn skin_3d_model(
        &self,
        kind: &std::sync::Arc<horizon_client::mobs::MobType>,
        skin: &str,
        slim: bool,
    ) -> Option<std::sync::Arc<horizon_client::mobs::Part>> {
        use horizon_client::mobs::skin_layers;
        if !self.settings.extras.three_d_skin_layers {
            return None;
        }
        let key = (skin.to_owned(), slim);
        if let Some(model) = self.skin_3d.borrow_mut().get(&key) {
            return Some(model);
        }
        let model_key = if slim { "player_slim#main" } else { "main" };
        let base = kind
            .layer(model_key)
            .cloned()
            .unwrap_or_else(|| kind.model.clone());
        let image = self.skin_downloads.borrow().image(skin)?.to_vec();
        let model = std::sync::Arc::new(skin_layers::extrude(
            &base,
            &image,
            skin_layers::SIZE,
            skin_layers::DEPTH,
        )?);
        self.skin_3d.borrow_mut().insert(key, model.clone());
        Some(model)
    }

    pub(super) fn update_remote_skins(&mut self) {
        let local = self.remote.as_ref().map(|r| r.profile);
        self.poll_own_profile();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let local = local.unwrap_or(r.profile);
        let mut entries = skin_entries(
            &r.level_players,
            &r.skin_textures,
            &mut r.level_player_textures,
        );
        let own = match &r.own_profile {
            OwnProfile::Done(Some(t)) => Some(t.clone()),
            _ => None,
        };
        // Minecraft.profileFuture: the signed-in account's active skin and cape
        // from `/minecraft/profile` when the server sent none (single player) or
        // only a skin without the equipped cape.
        if let Some(own) = own {
            if let Some((_, t)) = entries.iter_mut().find(|(u, _)| *u == local) {
                if t.skin.is_none() {
                    t.skin = own.skin.clone();
                    if own.skin.is_some() {
                        t.slim = own.slim;
                    }
                }
                if t.cape.is_none() {
                    t.cape = own.cape.clone();
                }
            } else if own.skin.is_some() || own.cape.is_some() {
                entries.retain(|(u, _)| *u != local);
                entries.push((local, own));
            }
        }
        let mut resolved: Vec<(u128, String)> = Vec::new();
        // PlayerSkin.model of each resolved skin: the textures' `slim`.
        let mut slim = std::collections::HashSet::new();
        // PlayerSkin.cape of each resolved skin.
        let mut capes: HashMap<u128, String> = HashMap::new();
        for (uuid, textures) in entries {
            // PlayerInfo.createSkinLookup: only the local player's own skin
            // may be unsigned.
            let skin = {
                let mut downloads = self.skin_downloads.borrow_mut();
                horizon_client::skins::lookup(&textures, uuid != local, |kind, url| {
                    downloads.state(kind, url)
                })
            };
            if let Some(skin) = skin {
                if textures.slim {
                    slim.insert(uuid);
                }
                if let Some(cape) = horizon_client::skins::cape(&textures) {
                    capes.insert(uuid, cape);
                }
                resolved.push((uuid, skin));
            }
        }
        // An offline name's skin from the launcher, where neither the server
        // nor an account gives the local player one.
        if !resolved.iter().any(|(u, _)| *u == local)
            && let Some((rgba, is_slim)) = horizon_client::skins::offline_skin()
        {
            const OFFLINE: &str = "horizon/offline_skin";
            if !self.skin_slots.contains(OFFLINE) || !self.gui.atlas.sprites.contains_key(OFFLINE) {
                self.store_skin(OFFLINE.into(), rgba.clone());
            }
            if *is_slim {
                slim.insert(local);
            }
            resolved.push((local, OFFLINE.into()));
        }
        // An offline name's cape from the launcher, where the profile has none.
        if !capes.contains_key(&local)
            && let Some(rgba) = horizon_client::skins::offline_cape()
        {
            const OFFLINE: &str = "capes/offline";
            if !self.skin_slots.contains(OFFLINE) || !self.gui.atlas.sprites.contains_key(OFFLINE) {
                self.store_skin(OFFLINE.into(), rgba.to_vec());
            }
            capes.insert(local, OFFLINE.into());
        }
        // PlayerInfo.getSkin: the downloaded skin and its model, else the
        // profile's default skin (DefaultPlayerSkin.get), slim or wide.
        let skin_of = |skins: &HashMap<u128, String>, uuid: u128| match skins.get(&uuid) {
            Some(skin) => (skin.clone(), slim.contains(&uuid)),
            None => {
                let skin = horizon_client::tab_list::default_skin(uuid);
                (skin.to_owned(), skin.contains("/slim/"))
            }
        };
        let skins: HashMap<_, _> = resolved.into_iter().collect();
        let Some((level, own)) = self.remote.as_ref().map(|r| {
            (
                r.level_players
                    .iter()
                    .map(|(id, uuid)| (*id as u32, *uuid))
                    .collect::<Vec<_>>(),
                r.profile,
            )
        }) else {
            return;
        };
        // Only the skins drawn hold atlas slots: the level's players, not
        // every entry of a busy server's player list.
        // Build once: resolving each player must not scan every level entity.
        let players: HashMap<_, _> = self
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| &*m.kind.id == "minecraft:player")
            .map(|(i, m)| (m.id, i))
            .collect();
        for (id, uuid) in level {
            let Some(&i) = players.get(&id) else {
                continue;
            };
            let (skin, is_slim) = self.drawn_skin(uuid, skin_of(&skins, uuid));
            let kind = self.mobs[i].kind.clone();
            let skin_3d = self.skin_3d_model(&kind, &skin, is_slim);
            let cape = capes.get(&uuid).filter(|c| self.again_skin(c)).cloned();
            let mob = &mut self.mobs[i];
            mob.skin = Some(skin);
            mob.slim = is_slim;
            mob.skin_3d = skin_3d;
            mob.cape = cape;
        }
        if self.player_model.is_some() {
            let (skin, is_slim) = self.drawn_skin(own, skin_of(&skins, own));
            // Avatar.isModelPartShown(CAPE): the Skin Customization option.
            let cape = capes
                .get(&own)
                .filter(|_| self.settings.file.bool("modelPart_cape"))
                .filter(|c| self.again_skin(c))
                .cloned();
            let kind = self.player_model.as_ref().map(|m| m.kind.clone());
            let skin_3d = kind
                .as_ref()
                .and_then(|kind| self.skin_3d_model(kind, &skin, is_slim));
            if let Some(mob) = self.player_model.as_mut() {
                mob.skin = Some(skin);
                mob.slim = is_slim;
                mob.skin_3d = skin_3d;
                mob.cape = cape;
            }
        }
        if let Some(r) = self.remote.as_mut()
            && r.skins != skins
        {
            r.skins_revision = r.skins_revision.wrapping_add(1);
            r.skins = skins;
        }
        // ClientMannequin.updateSkin (PlayerSkinRenderCache.lookup): a
        // mannequin's DATA_PROFILE `textures` property resolves like a
        // player's, never required to be signed.
        let mannequins: Vec<(usize, horizon_client::skins::Textures)> = self
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| &*m.kind.id == "minecraft:mannequin" && m.profile.is_some())
            .map(|(i, m)| {
                let profile = m.profile.as_ref();
                let t = profile.and_then(|p| p.properties.iter().find(|p| p.name == "textures"));
                let textures = if let Some(t) = t {
                    let signature = if t.signature.is_some() {
                        horizon_client::skins::Signature::Signed
                    } else {
                        horizon_client::skins::Signature::Unsigned
                    };
                    horizon_client::skins::unpack(
                        &t.value,
                        signature,
                        horizon_client::skins::texture_uris(),
                    )
                } else {
                    // `ResolvableProfile.resolveProfile`: no `textures` and
                    // only a name or an id resolves through the services.
                    let key = profile.and_then(|p| match (&p.name, p.uuid) {
                        (Some(name), None) => {
                            Some(horizon_client::skins::NameOrId::Name(name.clone()))
                        }
                        (None, Some(id)) => Some(horizon_client::skins::NameOrId::Id(id)),
                        _ => None,
                    });
                    key.and_then(|key| match self.skin_profiles.borrow_mut().state(&key) {
                        horizon_client::skins::Resolve::Done(Some(profile)) => {
                            profile.textures().map(|(value, signature)| {
                                horizon_client::skins::unpack(
                                    &value,
                                    signature,
                                    horizon_client::skins::texture_uris(),
                                )
                            })
                        }
                        _ => None,
                    })
                    .unwrap_or_default()
                };
                (i, textures)
            })
            .collect();
        let mut resolved: Vec<(usize, Option<String>, bool)> = Vec::new();
        for (id, textures) in mannequins {
            let skin = {
                let mut downloads = self.skin_downloads.borrow_mut();
                horizon_client::skins::lookup(&textures, false, |kind, url| {
                    downloads.state(kind, url)
                })
            };
            let slim = skin.is_some() && textures.slim;
            resolved.push((id, skin, slim));
        }
        for (i, skin, slim) in resolved {
            let skin = skin.filter(|s| !s.starts_with("skins/") || self.again_skin(s));
            // Resolution and atlas uploads do not insert or remove mobs.
            let mob = &mut self.mobs[i];
            mob.slim = slim && skin.is_some();
            mob.skin = skin;
        }
    }
    /// Puts a downloaded skin in an atlas skin slot (`SkinSlots::place`),
    /// unless every slot holds a skin drawn in the last frame.
    pub(super) fn store_skin(&mut self, location: String, rgba: Vec<u8>) {
        // 3D skin layers reads the pixels back when it builds the slabs.
        self.skin_downloads
            .borrow_mut()
            .remember_image(&location, &rgba);
        let Some((slot, evicted)) = self.skin_slots.place(location.clone()) else {
            return;
        };
        if let Some(old) = evicted {
            self.gui.atlas.sprites.remove(&old);
        }
        let Some(mut rect) = self
            .gui
            .atlas
            .sprites
            .get(&format!("horizon/skin{slot}"))
            .copied()
        else {
            return;
        };
        let (w, h) = (self.gui.atlas.width as f32, self.gui.atlas.height as f32);
        self.map_patches.push(horizon_client::font::Patch {
            x: (rect.u0 * w).round() as u32,
            y: (rect.v0 * h).round() as u32,
            width: 64,
            height: 64,
            rgba,
        });
        // A cape is a 64x32 texture in the top half of the slot (skins::process_cape).
        if location.starts_with("capes/") {
            rect.v1 = (rect.v0 + rect.v1) / 2.0;
        }
        self.gui.atlas.sprites.insert(location, rect);
    }
    /// Whether a downloaded skin or cape is in the atlas to draw, marking it
    /// drawn; if not (new, or its slot taken), its image is queued for upload
    /// again and it is drawn from the next frame.
    fn again_skin(&mut self, location: &str) -> bool {
        let mut downloads = self.skin_downloads.borrow_mut();
        // A slot without its sprite (the atlas was rebuilt, or the upload
        // never happened) counts as missing, so the skin is stored again.
        if self.skin_slots.contains(location) && self.gui.atlas.sprites.contains_key(location) {
            downloads.drawn(location);
            true
        } else {
            downloads.again(location);
            false
        }
    }
    /// A player's skin and model as drawn: the downloaded skin once it is in
    /// the atlas, until then the profile's default (`DefaultPlayerSkin.get`),
    /// as `PlayerInfo.getSkin` gives before the texture has loaded.
    pub(super) fn drawn_skin(
        &mut self,
        uuid: u128,
        (skin, slim): (String, bool),
    ) -> (String, bool) {
        if (skin.starts_with("skins/") || skin == "horizon/offline_skin") && !self.again_skin(&skin)
        {
            let skin = horizon_client::tab_list::default_skin(uuid);
            return (skin.to_owned(), skin.contains("/slim/"));
        }
        (skin, slim)
    }
    /// `drawn_skin` for a face in the player list or spectator menu.
    pub(super) fn face_skin(&mut self, uuid: u128, skin: &str) -> String {
        self.drawn_skin(uuid, (skin.to_owned(), false)).0
    }
    /// ChatType.Bound.decorate (`Decoration::Chat`) and `decorateNarration`
    /// (`Decoration::Narration`): the chat type's decoration from the
    /// server's registry, applied to `content`.
    pub(super) fn decorate_chat(
        &self,
        bound: &play::ChatTypeBound,
        content: &serde_json::Value,
        decoration: Decoration,
    ) -> horizon_client::text::Text {
        use horizon_client::scoreboard::comp_text;
        let Some(r) = self.remote.as_ref() else {
            return comp_text(content);
        };
        let chat_type = r.registries.data("minecraft:chat_type", bound.chat_type);
        comp_text(&chat_type_decorate(chat_type, decoration, bound, content))
    }
    fn remote_item_data(&mut self, id: i32, stack: &Option<Stack>) {
        let Some(stack) = self.to_stack(stack) else {
            return;
        };
        let position = self
            .remote
            .as_mut()
            .and_then(|r| r.pending_items.remove(&id));
        if let Some(item) = self.dropped.iter_mut().find(|d| d.id == id as u32) {
            item.stack = stack;
            return;
        }
        let Some((p, velocity)) = position else {
            return;
        };
        let at = Vec3::new(p[0], p[1], p[2]);
        // ItemEntity's constructor draws bobOffs from the item's own random.
        let mut random = horizon_client::mobs::entity_random(id as u32, at);
        let bob_offset = random.next_float() * std::f32::consts::PI * 2.0;
        self.dropped.push(ItemEntity {
            random,
            ..ItemEntity::new(id as u32, stack, at, velocity, bob_offset)
        });
    }
    /// `Entity.lerpMotion` (and `recreateFromPacket`'s motion) for the
    /// entities whose own tick moves them on a client: dropped items, experience
    /// orbs, falling blocks and primed TNT.
    fn remote_motion(&mut self, id: i32, velocity: [f64; 3]) {
        self.remote_dragon_motion(id, velocity);
        let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
        // `falling` and `tnts` are searched from the back: AddEntity calls this right after
        // appending the block or TNT, so a spawn doesn't walk the whole list.
        if let Some(m) = self.mob_index.get_mut(&mut self.mobs, id as u32) {
            // Squid.aiStep derives its body pitch and roll from network motion.
            m.velocity = v;
        } else if let Some(d) = self.dropped.iter_mut().find(|d| d.id == id as u32) {
            d.velocity = v;
        } else if let Some(i) = self.falling.iter().rposition(|f| f.id == id as u32) {
            self.falling.update(i, |f| f.velocity = v);
        } else if let Some(t) = self.tnts.iter_mut().rev().find(|t| t.id == id as u32) {
            t.velocity = v;
        } else if let Some(o) = self.orbs.iter_mut().find(|o| o.id == id as u32) {
            o.velocity = v;
        } else if let Some(i) = self.carts.iter().position(|c| c.id == id as u32) {
            self.carts.update(i, |c| c.velocity = v);
        } else if let Some(i) = self
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id == id as u32)
        {
            self.creatures.projectiles.update(i, |p| {
                p.velocity = v;
                // AbstractArrow.lerpMotion: the stuck timer restarts, and an
                // arrow the server set moving leaves the block it was stuck in.
                if let Some(st) = p.arrow.as_mut() {
                    st.life = 0;
                    if st.in_ground && v.length_squared() > 0.0 {
                        st.in_ground = false;
                        p.stuck = false;
                    }
                }
            });
        } else if let Some(t) = self.tridents.iter_mut().find(|t| t.id == id as u32) {
            t.velocity = v;
            // AbstractArrow.lerpMotion (ThrownTrident keeps it): `life`
            // restarts, and a trident set moving leaves its block.
            t.in_ground_time = 0;
            if t.in_ground && v.length_squared() > 0.0 {
                t.in_ground = false;
            }
        } else if let Some(r) = self.rockets.iter_mut().find(|r| r.id == id as u32) {
            r.velocity = v;
        } else if self.remote_hook_motion(id as u32, v) {
        } else if let Some(p) = self
            .remote
            .as_mut()
            .and_then(|r| r.pending_items.get_mut(&id))
        {
            p.1 = v;
        } else if !self.potions.remote_thrown_motion(id as u32, v) {
            self.end.remote_eye_motion(id as u32, velocity);
        }
    }
    fn apply_remote_health(&mut self) {
        let Some((health, food, saturation)) = self.remote.as_ref().map(|r| r.health) else {
            return;
        };
        if let Some(p) = self.player.as_mut() {
            if (p.health.health() - health).abs() > f32::EPSILON {
                let _ = p.set_health(health.max(0.0), false);
            }
            p.health.food.food = food;
            p.health.food.saturation = saturation;
        }
    }
    /// `ServerPlayer.dismountTo` lands a rider at the ghast's own (x, maxY, z)
    /// (`HappyGhast.getDismountLocationForPassenger`). A vanilla client
    /// simulates the ghast it rides, so its copy is where the server has it;
    /// this window's copy trails the server's by the stepped interpolation and
    /// would rise into the rider's feet, which lets the rider fall through.
    /// Put the copy under the landing spot.
    fn place_ghast_under(&mut self, id: u32, spot: Vec3) {
        let Some(m) = self
            .mobs
            .iter_mut()
            .find(|m| m.id == id && m.species() == horizon_client::mobs::Species::HappyGhast)
        else {
            return;
        };
        let at = Vec3::new(spot.x, spot.y - f64::from(m.height()), spot.z);
        // Only a teleport that is that dismount: right above the ghast.
        if (at.x - m.position.x).powi(2) + (at.z - m.position.z).powi(2) > 25.0
            || (at.y - m.position.y).abs() > 5.0
        {
            return;
        }
        m.position = at;
        m.previous = at;
        if let Some(handler) = self
            .remote
            .as_mut()
            .and_then(|r| r.stepped.get_mut(&(id as i32)))
        {
            handler.cancel();
        }
    }
    /// `ClientPacketListener.handleLookAt`: `Entity.lookAt` from this
    /// player's feet or eyes, towards the packet's entity (its feet or eyes)
    /// when this client tracks it, else towards the packet's point.
    fn remote_look_at(&mut self, p: &play::PlayerLookAt) {
        // `EntityAnchorArgument.Anchor`: 0 feet, 1 eyes.
        const EYES: i32 = 1;
        let Some(player) = self.player.as_ref() else {
            return;
        };
        let mut from = [player.position.x, player.position.y, player.position.z];
        if p.from == EYES {
            from[1] += f64::from(player.dimensions.eye_height);
        }
        let to = p
            .entity
            .as_ref()
            .and_then(|e| {
                let m = self.mobs.iter().find(|m| m.id == e.id as u32)?;
                let y = if e.anchor == EYES {
                    m.eye_y()
                } else {
                    m.position.y
                };
                Some([m.position.x, y, m.position.z])
            })
            .unwrap_or(p.target);
        (self.yaw, self.pitch) = look_at_rotation(from, to);
    }
    /// ClientPacketListener.handleMovePlayer.
    fn remote_teleport(&mut self, p: &play::PlayerPosition) -> Result<()> {
        use play::relative as rel;
        let (had_player, current) = self
            .player
            .as_ref()
            .map(|pl| (true, pl.position))
            .unwrap_or((false, Vec3::ZERO));
        let velocity = self
            .player
            .as_ref()
            .map(|pl| pl.velocity)
            .unwrap_or(Vec3::ZERO);
        let pick = |bit: i32, base: f64, v: f64| if p.relatives & bit != 0 { base + v } else { v };
        let position = Vec3::new(
            pick(rel::X, current.x, p.position[0]),
            pick(rel::Y, current.y, p.position[1]),
            pick(rel::Z, current.z, p.position[2]),
        );
        let yaw = if p.relatives & rel::Y_ROT != 0 {
            self.yaw + p.yaw
        } else {
            p.yaw
        };
        let relative_pitch = p.relatives & rel::X_ROT != 0;
        let pitch = position_packet_pitch(self.pitch, p.pitch, relative_pitch);
        let delta = horizon_client::player_sync::teleport_velocity(
            velocity,
            (self.yaw, self.pitch),
            (yaw, pitch),
            p,
        );
        // BlockStatePredictionHandler.onTeleport: a prior prediction keeps its block, not its remembered player position.
        if let Some(r) = self.remote.as_mut() {
            r.last_teleport_sequence = r.sequence;
        }
        // The server's answer to a passage crossed (`cross_passage`), or any other.
        self.passages.crossing = None;
        // handleMovePlayer: a passenger keeps its own position and look (its vehicle
        // carries it), so a vehicle a datapack teleports every tick doesn't snap the
        // rider's view; the teleport is still accepted, with where the player is.
        if self.riding.is_some()
            && let Some(player) = self.player.as_ref()
        {
            let at = player.position;
            self.send(S::AcceptTeleportation(play::AcceptTeleportation {
                id: p.teleport,
                x: at.x,
                y: at.y,
                z: at.z,
                yaw: self.yaw,
                pitch: self.pitch,
            }));
            self.stop_remote_mining();
            self.mining = None;
            return Ok(());
        }
        match self.player.as_mut() {
            Some(player) => {
                player.position = position;
                player.velocity = delta;
            }
            None => {
                let mut player = PlayerMotion::new(self.motion, position)?;
                // The id the server gave, for the hooks the motion fires.
                player.entity_id = self.own_entity_id();
                // Player.tick derives noPhysics from the current game mode.
                player.spectator = self.spectator();
                player.velocity = delta;
                self.take_carried_attributes(&mut player)?;
                self.hook.discard();
                self.player_state.attachments.retire();
                self.resident_entities.local_player.renew();
                self.player = Some(player);
                self.bind_local_player_residency();
                self.input_player_replaced();
                self.sway = Default::default();
                self.eye = 1.62;
                self.previous_eye = 1.62;
                self.clock.reset();
                self.apply_remote_health();
                self.apply_remote_abilities();
            }
        }
        entity_section_steps::player_teleport(self, had_player, current, position);
        if !self.replay.playing() {
            self.previous_position = position;
        }
        self.yaw = yaw;
        self.pitch = pitch;
        // setValuesFromPositionPacket: setOldPosAndRot gives xRotO the same
        // change, so a relative pitch keeps the model's ease and an absolute
        // one snaps it.
        if let Some(m) = self.player_model.as_mut() {
            m.previous_head_pitch =
                position_packet_pitch(m.previous_head_pitch, p.pitch, relative_pitch);
            m.head_pitch = pitch;
        }
        if let Some(id) = self.remote.as_mut().and_then(|r| r.ghast_dismount.take()) {
            self.place_ghast_under(id, position);
        }
        // handleMovePlayer does not change LocalPlayer.sendPosition's baseline:
        // the next tick must still report the corrected state.
        self.send(S::AcceptTeleportation(play::AcceptTeleportation {
            id: p.teleport,
            x: position.x,
            y: position.y,
            z: position.z,
            yaw,
            pitch,
        }));
        // Then MultiPlayerGameMode.stopDestroyBlock: a dig the teleport (an
        // anticheat's setback, a /tp) interrupts is aborted on the wire, and the
        // attack strength starts again.
        self.stop_remote_mining();
        self.mining = None;
        Ok(())
    }

    /// `ClientPacketListener.tick`'s `LevelLoadTracker.tickClientLoad`: the
    /// player's section is shown once its column is lit; when the level is
    /// ready the loading screen closes and the server hears `PlayerLoaded`
    /// (`notifyPlayerLoaded`).
    fn tick_client_load(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };
        if !self
            .remote
            .as_ref()
            .is_some_and(|r| r.level_load.is_active())
        {
            return;
        }
        // `LevelRenderer`'s playerCompiledSectionCallback: the section holding
        // the camera is there (the column is lit, as meshing needs).
        let (cx, cz) = (
            (player.position.x.floor() as i32) >> 4,
            (player.position.z.floor() as i32) >> 4,
        );
        let section_ready = self.streaming.is_lit(cx, cz);
        let camera_y = (player.position.y + f64::from(self.eye)).floor() as i32;
        let shape = self.world.shape;
        let check = horizon_client::level_loading::PlayerCheck {
            outside_build_height: camera_y < shape.min_y || camera_y >= shape.min_y + shape.height,
            spectator: player.spectator,
            alive: player.health.health() > 0.0,
        };
        let now = client_millis();
        // LevelLoadingScreen.tick.
        self.tick_load_progress();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        if section_ready && r.level_load.wants_player_section() {
            r.level_load.player_section_ready();
        }
        r.level_load.tick_client_load(now, check);
        if !r.level_load.is_level_ready(now) {
            return;
        }
        r.level_load.finish();
        if !r.loaded_sent {
            r.loaded_sent = true;
            self.send(S::PlayerLoaded(Empty));
        }
        // LevelLoadingScreen.onClose.
        self.say_system_now(&horizon_client::text::Text::translate(
            "narrator.ready_to_play",
            &[],
        ));
        self.capture_loaded_world();
    }

    /// One client tick while connected (Minecraft.tick with a ClientLevel).
    pub(super) fn tick_remote(&mut self) -> Result<()> {
        self.tick_remote_level()?;
        // The rockets boosting this player tick after it, also while it waits
        // for chunks.
        self.tick_own_rockets();
        self.tick_book_snapshot_retry();
        // Far tiles the samplers asked for (`lod_net`).
        if let Some(r) = &self.remote {
            r.session.send_far_terrain();
        }
        self.tick_chat_session();
        // Minecraft.tick sends this even when the player cannot tick yet,
        // including while waiting for chunks. Keep it outside early returns.
        if self.remote.as_ref().is_some_and(Remote::has_level) {
            self.send(S::ClientTickEnd(Empty));
        }
        Ok(())
    }

    fn tick_remote_player(&mut self, running: bool, hold: bool) -> Result<()> {
        // Inventory.tick's ItemStack.inventoryTick: the pop time runs down.
        if !hold {
            for pop in &mut self.hotbar_pop {
                *pop = pop.saturating_sub(1);
            }
        }
        // ClientLevel.tick: the border's size change, while the level runs.
        if running {
            self.border.tick();
        }
        // ClientLevel.tick: the End's sky flash, frozen or not.
        self.tick_end_flash();
        self.player_border();
        // The entities added after the player tick after it: their pushes
        // (from where they ended last tick, onto where the player ended)
        // reach its next move, this one.
        let pushed_after = self.remote_pushes(false);
        self.tick_remote_entities(running);
        self.tick_remote_arm_poses();
        self.tick_random_textures();
        // Breeze.tick's block dust: drawn by each client for the breezes it sees.
        if running {
            self.tick_breeze_effects();
        }
        self.tick_remote_guardian_eyes(running);
        // Level.tickBlockEntities: a frozen level ticks no block entities.
        // A step advances the moving pistons created by the block event once,
        // rather than finishing them during the frozen ticks between steps.
        if running {
            self.tick_block_entity_anims();
            self.tick_remote_spawners();
            self.client_tick_beacons();
            self.tick_remote_potent_sulfur();
            self.with_level(|b, level, ticks| {
                b.client_tick_block_entities(level, ticks);
            });
            let pushes = std::mem::take(&mut self.block_ticks.piston_pushes);
            self.push_piston_entities(pushes);
            self.sync_moving_pistons();
        }
        self.tick_hand();
        if running {
            self.tick_particles();
        } else if self.clock.still() {
            self.settle_rain_fog();
        }
        self.tick_frame_visuals(running);
        // Under `HORIZON_STILL` they wait for the world too: endermen's and blazes' particles
        // draw on the particles' random, and effects count down, on every tick a frozen
        // client runs, as many as the run took to get there.
        if !hold {
            self.tick_client_ambient();
            self.tick_client_effects();
        }
        // Gui.tick: the titles, the action bar and the held item's name.
        if !hold {
            self.tick_hud();
        } else {
            // Only its countdowns wait: lying down still opens the bed's chat, waking closes it.
            self.tick_in_bed_chat();
        }
        self.tick_subtitles();
        self.tick_client_load();
        let entity_collisions = self.player_entity_collisions();
        // Those added before it tick before it: their pushes reach this move.
        let pushed = pushed_after.add_vector(self.remote_pushes(true));
        let sprint_window = self.settings.sprint_window;
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        let (cx, cz) = (
            (player.position.x.floor() as i32) >> 4,
            (player.position.z.floor() as i32) >> 4,
        );
        if !self.streaming.is_lit(cx, cz) {
            // LocalPlayer.tick can wait for chunks, but Minecraft.tick still
            // calls sendChanges: rotations and the 20-tick position reminder
            // must continue, as well as moves into the unloaded area.
            if let Some(r) = self.remote.as_ref()
                && r.loaded_sent
            {
                let input = r.last_input;
                self.remote_send_state(input);
            }
            return Ok(());
        }
        // LocalPlayer.tick reaches Player.tick only once the client has loaded
        // and the player's chunk is there; its endpoints bracket that work
        // alone, also on the death screen, not the level pass before it.
        let player = self
            .remote
            .as_ref()
            .is_some_and(|r| r.loaded_sent)
            .then(|| {
                (mod_api::__private::listened(mod_api::HookId::PlayerTickPre)
                    || mod_api::__private::listened(mod_api::HookId::PlayerTickPost))
                .then(|| self.local_player())
                .flatten()
            })
            .flatten();
        let player_from = entity_section_steps::player_before(self);
        if let Some(player) = player {
            self.native_player_tick_pre(mod_api::__private::PlayerTickPreCall { player });
        }
        let result = self.tick_local_player(entity_collisions, pushed, sprint_window);
        entity_section_steps::player_after(self, player_from);
        // `EntityEvent.Size` for the pose the tick set.
        self.native_local_player_size();
        if let Some(player) = player {
            self.native_player_tick_post(mod_api::__private::PlayerTickPostCall { player });
        }
        result
    }

    /// `LocalPlayer.tick`'s `Player.tick` work: input, riding, motion, sleep
    /// and the state the server hears.
    fn tick_local_player(
        &mut self,
        entity_collisions: Vec<horizon_core::geometry::Aabb>,
        pushed: Vec3,
        sprint_window: i32,
    ) -> Result<()> {
        // Player.tick's ItemCooldowns.tick, after Pre and after
        // handleKeybinds: this path replaces `tick_world`'s for a server's
        // client (multiplayer and the default single player), so the entries
        // advance here or the slot stays shaded until the server says the
        // entry ended. `onCooldownEnded` is the plain ItemCooldowns' no-op on
        // a client. Ticking it after the keybinds keeps a held use from
        // trying again a tick before the server's entry ends.
        self.cooldowns.tick();
        // Player.tick's sleep counter (the darkening and the wake-up fade).
        self.sleep_timer = super::tick_sleep_timer(self.sleep_timer, self.sleeping.is_some());
        let stick = self.pad_move();
        if self.player.is_none() {
            return Ok(());
        }
        // The LevelLoadingScreen takes the input until the level is ready.
        let loading = self
            .remote
            .as_ref()
            .is_some_and(|r| r.level_load.is_active());
        let playing = self.screen == Screen::Playing && !loading && !self.dialog_open();
        let held = |action| playing && self.keys.held(action);
        let mut keys = Keys {
            forward: held(Action::Forward),
            backward: held(Action::Back),
            left: held(Action::Left),
            right: held(Action::Right),
            jump: held(Action::Jump),
            shift: held(Action::Sneak),
            sprint: held(Action::Sprint),
            sprint_suppressed: false,
            analog: None,
        }
        .with_stick(stick.filter(|_| playing));
        // On a server (not this game's own server thread), the stick moves as the keys it sets do
        // (`KeyboardInput.tick`: 8 directions, full speed). The server only hears those keys
        // (`ServerboundPlayerInputPacket`), and an anticheat that predicts from them pulls back
        // a player moving at the stick's own angle and speed.
        if self.integrated.is_none() && self.server_address().is_some() {
            keys.analog = None;
        }
        let keys = self.mod_movement_keys(keys);
        let input = u8::from(keys.forward)
            | u8::from(keys.backward) << 1
            | u8::from(keys.left) << 2
            | u8::from(keys.right) << 3
            | u8::from(keys.jump) << 4
            | u8::from(keys.shift) << 5
            | u8::from(keys.sprint) << 6;
        // LocalPlayer.applyInput and aiStep: watching through another
        // entity, the player does not move itself; its keys still reach the
        // server, whose sneak check lets go of the camera.
        let keys = if self.controls_camera() {
            keys
        } else {
            Keys::default()
        };
        // The held lead's end eases from the look at the start of the tick
        // (`yRotO`/`xRotO`), as the local tick does; `tick` returns before it
        // reaches its own write when this window is a server client.
        self.rope_rot_old = (self.yaw, self.pitch);
        // A passenger moves with its vehicle (Entity.rideTick), not by itself.
        let riding = self.screen != Screen::Dead && self.remote_ride(keys, sprint_window)?;
        // The server sends no passenger's position: they sit where their
        // vehicles are now.
        self.position_riders();
        // Entity.getFluidInteractionBox asks the vehicle: a boat afloat shelters
        // its passenger's eyes (AbstractBoat.modifyPassengerFluidInteractionBox).
        let boat = self
            .riding
            .and_then(|id| self.boats.iter().find(|b| b.id == id))
            .map(|b| horizon_client::motion::RiddenBoat {
                under_water: b.under_water(),
                max_y: b.bounds().max_position().y,
            });
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        player.fluids.boat = boat;
        if !riding && !self.replay.playing() {
            self.previous_position = player.position;
        }
        let before = (
            player.ground.on_ground,
            [player.velocity.x, player.velocity.y, player.velocity.z],
            [player.position.x, player.position.y, player.position.z],
        );
        let was_flying = player.flying;
        self.sync_player_body();
        // LivingEntity.tickDeath: the local player counts its deathTime once
        // per client tick while dead (the camera's death roll and zoom).
        if self.screen == Screen::Dead {
            self.ui.death_time = self.ui.death_time.saturating_add(1);
        }
        let sleeping = self.sleeping;
        let sleep_position = sleeping.and_then(|head| self.sleeping_position(head));
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        let was_gliding = player.fall_flying;
        let dead = self.screen == Screen::Dead;
        // LivingEntity.isImmobile: a sleeper lies still, even if its bed's
        // shape is unavailable here; only the server decides when to wake.
        if sleeping.is_some() {
            if let Some(at) = sleep_position {
                player.position = at;
            }
            player.velocity = Vec3::ZERO;
            player.pitch = 0.0;
        } else if !dead && !riding {
            player.client_side = true;
            player.velocity = player.velocity.add_vector(pushed);
            // LivingEntity.checkAutoSpinAttack: a Riptide spin that touches a
            // mob ends and bounces back; the server deals the hit.
            player.spin_targets.clear();
            if player.auto_spin_attack_ticks > 0 {
                player.spin_targets.extend(
                    self.mobs
                        .iter()
                        .filter(|m| !m.removed)
                        .map(|m| (m.id, m.bounds())),
                );
            }
            MotionWorld {
                entities: &entity_collisions,
                player,
                scene: &mut self.world,
                assets: &self.pack.assets,
                collision_binding: self.collision,
                resources: self.motion,
                health: &self.health_resources,
                loop_context: &self.loop_context,
                damage_hooks: &self.damage_hooks,
                tag_items: None,
                block_registries: None,
            }
            .controlled_simulation_tick_with_sprint_window(keys, sprint_window)?;
        } else {
            // LivingEntity.baseTick counts hurtTime down for a dead or riding
            // player too (controlled_simulation_tick does it otherwise): left
            // at 10, the hurt tilt restarted every frame and shook the camera.
            player.health.advance_clock();
            if riding && !dead {
                player.client_side = true;
                let mut passenger = MotionWorld {
                    entities: &entity_collisions,
                    player: &mut *player,
                    scene: &mut self.world,
                    assets: &self.pack.assets,
                    collision_binding: self.collision,
                    resources: self.motion,
                    health: &self.health_resources,
                    loop_context: &self.loop_context,
                    damage_hooks: &self.damage_hooks,
                    tag_items: None,
                    block_registries: None,
                };
                passenger.passenger_pose_tick()?;
                // LivingEntity.aiStep: a passenger freezes in powder snow too, and the
                // frostbite overlay shows it (#8156).
                passenger.passenger_block_effects_tick()?;
            }
        }
        // The server decides fire the window cannot predict (blaze fireballs,
        // fire aspect, lightning): Entity.isOnFire reads the synched flag, so
        // the predicted tick's countdown must not end it while it is set.
        player.fire_ticks = hold_server_fire(
            player.fire_ticks,
            self.remote.as_ref().is_some_and(|r| r.own_on_fire),
        );
        // LocalPlayer.onUpdateAbilities: flight turned on or off (a double
        // jump, a spectator, landing) is sent to the server.
        let flying = player.flying;
        // LocalPlayer.aiStep: a glide this client started is sent to the
        // server (ServerboundPlayerCommandPacket START_FALL_FLYING).
        let started_gliding = player.fall_flying && !was_gliding;
        let events = std::mem::take(&mut player.events);
        self.previous_eye = self.eye;
        self.eye += (player.dimensions.eye_height - self.eye) * 0.5;
        for event in events {
            match event {
                horizon_client::motion::MotionEvent::Emission {
                    position,
                    state,
                    muffled,
                } => {
                    let at = [position.x, position.y, position.z];
                    let sounds = self.own_sound_type(at, state, mod_api::BlockSoundCause::Step);
                    // Player.playStepSound: muffled under the swim sound in water.
                    let (volume, pitch) = if muffled {
                        (sounds.volume * 0.05, sounds.pitch * 0.8)
                    } else {
                        (sounds.volume * 0.15, sounds.pitch)
                    };
                    self.play_client_player_sound(&sounds.step, volume, pitch);
                }
                horizon_client::motion::MotionEvent::Swim { volume, pitch } => {
                    self.play_client_player_sound("minecraft:entity.player.swim", volume, pitch);
                }
                // Entity.doWaterSplashEffect on the client's own level: the
                // server's splash skips this player (Player.playSound), and
                // its particles are the client's own.
                horizon_client::motion::MotionEvent::Splash {
                    high_speed,
                    volume,
                    pitch,
                } => {
                    let name = if high_speed {
                        "minecraft:entity.player.splash.high_speed"
                    } else {
                        "minecraft:entity.player.splash"
                    };
                    self.play_client_player_sound(name, volume, pitch);
                }
                horizon_client::motion::MotionEvent::SplashParticle {
                    bubble,
                    position,
                    velocity,
                } => {
                    use horizon_client::particles::{ParticleOptions, ParticleType};
                    let kind = if bubble {
                        ParticleType::Bubble
                    } else {
                        ParticleType::Splash
                    };
                    self.particles.add(
                        &ParticleOptions::simple(kind),
                        [position.x, position.y, position.z],
                        [velocity.x, velocity.y, velocity.z],
                    );
                }
                horizon_client::motion::MotionEvent::Chime { volume, pitch } => {
                    self.play_client_player_sound(
                        "minecraft:block.amethyst_block.chime",
                        volume,
                        pitch,
                    );
                }
                // NetherPortalBlock.entityInside on the client's own level:
                // the portal overlay and its trigger sound (the server moves
                // the player).
                horizon_client::motion::MotionEvent::PowderSnowMelt { pos } => {
                    self.powder_snow_melt(pos);
                }
                horizon_client::motion::MotionEvent::InsidePortal { .. } => {
                    self.client_inside_nether_portal();
                }
                // HoneyBlock.maybeDoSlideEffects on the client's own level:
                // the slide sound, heard here (LocalPlayer.playSound); the
                // particles come from the server's event 53.
                horizon_client::motion::MotionEvent::HoneySlide { sound: true, .. } => {
                    self.play_client_player_sound("minecraft:block.honey_block.slide", 1.0, 1.0);
                }
                // LivingEntity.causeFallDamage and HoneyBlock.fallOn on the
                // client's own level: the fall and honey sounds are heard here
                // (LocalPlayer.playSound); the server's copy skips this player
                // (Player.playSound).
                horizon_client::motion::MotionEvent::Fall(event) => self.block_fall(event),
                _ => {}
            }
        }
        // A stand riding the player follows this move (tickPassenger).
        self.seat_own_stands();
        // Other players on this player (possibly through a plugin seat) follow this tick too;
        // the full pass only when something rides this player.
        if self.remote.as_ref().is_some_and(|r| {
            let me = r.entity_id as u32;
            r.passenger_vehicles.values().any(|&v| v == me)
        }) {
            self.position_riders();
        }
        // The server owns health and hunger.
        self.apply_remote_health();
        self.tick_avatar(before, false);
        // LivingEntity.tick: a sleeper's xRot is 0.
        if sleeping.is_some() {
            self.pitch = 0.0;
            self.snap_player_model_pitch();
        }
        // The player's own model (third person, the inventory's preview).
        if !self.replay.playing() {
            self.tick_player_model();
        }
        self.client_hooks_tick();
        if !dead {
            self.remote_tick_using();
        }
        if flying != was_flying {
            self.send(S::PlayerAbilities(play::PlayerAbilitiesRequest {
                flags: if flying { 2 } else { 0 },
            }));
        }
        if started_gliding {
            let id = self.remote.as_ref().map_or(0, |r| r.entity_id);
            self.send(S::PlayerCommand(play::PlayerCommand {
                id,
                action: 6,
                data: 0,
            }));
        }
        // LocalPlayer.sendChanges sends `input.keyPresses`, which an auto-jump's `makeJump` set to
        // jump for this tick: the server's prediction plays the jump from it. Sent as held, the jump
        // was the client's alone, and the server saw a jump its keys didn't allow.
        let auto_jumped = self.player.as_ref().is_some_and(|p| p.was_auto_jump);
        self.remote_send_state(input | u8::from(auto_jumped) << 4);
        Ok(())
    }

    /// The last `ClientboundPlayerAbilities` the server sent, on a client.
    pub(super) fn remote_abilities(&self) -> Option<&play::PlayerAbilities> {
        self.remote.as_ref()?.abilities.as_ref()
    }

    /// The local player's share of every touching entity's
    /// `LivingEntity.pushEntities`, and of the boats' pushes: on a client
    /// `EntitySelector.pushableBy` lets entities push only the local player,
    /// so the server never sends these pushes. `doPush(player)` is
    /// `player.push(entity)`. `before_player`: the mobs that tick before the
    /// player (`ticked_before_player`) and the boats, else the mobs that tick
    /// after it.
    fn remote_pushes(&self, before_player: bool) -> Vec3 {
        let (Some(p), Some(r)) = (self.player.as_ref(), self.remote.as_ref()) else {
            return Vec3::ZERO;
        };
        // LivingEntity.isPushable: alive, no spectator, off ladders.
        let feet = [
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        ];
        if self.screen == Screen::Dead
            || p.spectator
            || self.named_block_in_tag("minecraft:climbable", &self.block_name(feet))
        {
            return Vec3::ZERO;
        }
        let bounds = p.bounds();
        let teams = self.mob_teams();
        // `Entity.getTeam` is `Scoreboard.getPlayersTeam(getScoreboardName())`:
        // a player's score holder is its profile name, every other entity's
        // its UUID text. `mob_teams` keys by UUID and names the local player
        // `PLAYER_NAME`, which misses a server's teams for players.
        let board = &self.score.board;
        let player_team = board.players_team(&r.profile_name);
        let mut push = Vec3::ZERO;
        let order = |id: u32| r.ticked_before_player.contains(&id) == before_player;
        for m in self.mobs.iter().filter(|m| order(m.id)) {
            // Bats never push, nor passengers of one vehicle, nor a spectator; the
            // box test comes before the lookups that far mobs never need (#14053).
            if !m.alive()
                || m.species() == horizon_client::mobs::Species::Bat
                || self
                    .riding
                    .is_some_and(|v| v == m.id || m.extra.mount == Some(v))
                || !m.bounds().intersects(bounds)
            {
                continue;
            }
            let listed = r
                .level_players
                .get(&(m.id as i32))
                .and_then(|uuid| r.tab_players.get(uuid));
            let mob_team =
                listed.map_or_else(|| teams.of(Who::Mob(m.id)), |t| board.players_team(&t.name));
            if listed.is_some_and(|t| t.game_mode == SPECTATOR)
                || !horizon_client::scoreboard::pushable_by(mob_team, player_team, true)
            {
                continue;
            }
            // Entity.push(Entity), `this` the player.
            let (mut xa, mut za) = (m.position.x - p.position.x, m.position.z - p.position.z);
            let mut dd = xa.abs().max(za.abs());
            if dd < f64::from(0.01_f32) {
                continue;
            }
            dd = dd.sqrt();
            xa /= dd;
            za /= dd;
            let pow = (1.0 / dd).min(1.0);
            xa *= pow;
            za *= pow;
            xa *= f64::from(0.05_f32);
            za *= f64::from(0.05_f32);
            push = Vec3::new(push.x - xa, push.y, push.z - za);
        }
        // AbstractBoat.push(player) from the boat's own tick (its box grown
        // by 0.2 sideways) and from the player's pushEntities (its box).
        let area = Vec3::new(f64::from(0.2_f32), f64::from(-0.01_f32), f64::from(0.2_f32));
        // The boats' pushes stay where they were measured: with the entities
        // that tick before the player.
        let boats = if before_player {
            &self.boats[..]
        } else {
            &[][..]
        };
        for b in boats.iter().filter(|b| self.riding != Some(b.id)) {
            let bb = b.bounds();
            let times = usize::from(
                bounds.intersects(bb.inflate(area))
                    && horizon_client::scoreboard::pushable_by(None, player_team, true),
            ) + usize::from(
                bounds.intersects(bb)
                    && horizon_client::scoreboard::pushable_by(player_team, None, true),
            );
            if times > 0
                && super::boat_passengers::boat_pushes(bb, bounds, false)
                && let Some(v) = super::boat_passengers::push_impulse(b.position, p.position)
            {
                push = push.add_vector(v.scale(times as f64));
            }
        }
        push
    }
    /// ClientLevel.tickEntities; a frozen level only ticks the players
    /// (`TickRateManager.isEntityFrozen`).
    /// `GuardianRenderer.getEntityToLookAt`: a guardian's eye follows its
    /// synched attack target, else the camera.
    fn tick_remote_guardian_eyes(&mut self, running: bool) {
        use horizon_client::mobs::{Species, Who, guardian};
        if !running {
            return;
        }
        let camera = self.player.as_ref().map(|p| {
            [
                p.position.x,
                p.position.y + f64::from(p.dimensions.eye_height),
                p.position.z,
            ]
        });
        let mut looks = std::mem::take(&mut self.guardian_look_scratch);
        looks.clear();
        looks.extend(
            self.mobs
                .iter()
                .enumerate()
                .filter(|(_, m)| {
                    matches!(m.species(), Species::Guardian | Species::ElderGuardian) && m.alive()
                })
                .filter_map(|(i, m)| {
                    let look = match m.ai.target.filter(|_| m.extra.flag) {
                        Some(Who::Mob(id)) => self
                            .mobs
                            .iter()
                            .find(|t| t.id == id)
                            .map(|t| [t.position.x, t.eye_y(), t.position.z]),
                        _ => camera,
                    };
                    look.map(|look| (i, look))
                }),
        );
        for &(i, look) in &looks {
            guardian::look_at(&mut self.mobs[i], look);
        }
        self.guardian_look_scratch = looks;
    }
    /// Humanoid held arm poses, once the tick has set equipment and use state.
    pub(in crate::game) fn tick_remote_arm_poses(&mut self) {
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if !horizon_client::mobs::anim::extracts_humanoid(m) {
                continue;
            }
            // Extraction also runs while the world is frozen: derive the use
            // animation from the current equipment rather than last tick's state.
            if &*m.kind.id != "minecraft:player" {
                let slot = if m.extra.using_off_hand {
                    horizon_items::equipment::OFFHAND
                } else {
                    horizon_items::equipment::MAINHAND
                };
                let animation = (m.extra.using_item > 0)
                    .then(|| m.extra.slot_stack(slot))
                    .flatten()
                    .map_or_else(Default::default, |stack| self.item_use_animation(stack));
                self.mobs[i].extra.use_animation = animation;
            }
            let m = &self.mobs[i];
            let poses = self.held_arm_poses(m.id, m);
            let charge_duration = (&*m.kind.id != "minecraft:player").then(|| {
                let using_slot = if m.extra.using_off_hand {
                    horizon_items::equipment::OFFHAND
                } else {
                    horizon_items::equipment::MAINHAND
                };
                m.extra
                    .slot_stack(using_slot)
                    .map_or(25, |stack| Self::charge_ticks(stack).max(1)) as i32
            });
            self.mobs[i].extra.held_arm_pose = poses;
            if let Some(duration) = charge_duration {
                self.mobs[i].extra.ints[5] = duration;
            }
        }
    }
    fn tick_remote_entities(&mut self, running: bool) {
        if running {
            self.tick_text_displays();
            self.tick_remote_ominous_item_spawners();
        }
        // EntityFluidInteraction.update for the entities this window only displays: the
        // server never sends the fluid contact, and the renderers read it (cod, salmon and
        // tropical fish lie on their side without `in_water`, #1591).
        // A frozen level ticks only the players, as below.
        // Entity.isInRain for the wolves, which dry off by their own `Wolf.tick`.
        let (mut fluids, mut rain, mut standings, mut water_fx) = {
            let r = self
                .remote
                .as_mut()
                .expect("remote entity tick without remote state");
            (
                std::mem::take(&mut r.tick_fluids),
                std::mem::take(&mut r.tick_rain),
                std::mem::take(&mut r.tick_standings),
                std::mem::take(&mut r.tick_water_fx),
            )
        };
        let thundering = {
            let w = self.mob_world_base(&[]);
            let players = self.remote.as_ref().map(|r| &r.level_players);
            fluids.clear();
            fluids.extend(self.mobs.iter().map(|m| {
                (running || players.is_some_and(|p| p.contains_key(&(m.id as i32))))
                    .then(|| w.fluids(m.bounds(), m.eye_y()))
            }));
            rain.clear();
            rain.extend(self.mobs.iter().map(|m| {
                m.species() == horizon_client::mobs::Species::Wolf
                    && horizon_client::mobs::tame::in_rain(m, &w)
            }));
            // The block under each mob the packs' random-model rules read (a grazing
            // cow), one hair below the feet like `Entity.getBlockStateOn`; nothing
            // without rules.
            standings.clear();
            standings.extend(self.mobs.iter().map(|m| {
                horizon_client::mobs::cem::tracks_standing(&m.kind.id).then(|| {
                    let (x, y, z) = (
                        m.position.x.floor() as i32,
                        (m.position.y - 0.0001).floor() as i32,
                        m.position.z.floor() as i32,
                    );
                    w.block_name(x, y, z).to_owned()
                })
            }));
            w.thundering
        };
        // The window's own player already splashes from its motion tick.
        let own = self.remote.as_ref().map(|r| r.entity_id);
        for i in 0..self.mobs.len() {
            let m = &mut self.mobs[i];
            let fluids = fluids[i].take();
            let standing = standings[i].take();
            if let Some(fluids) = fluids {
                let was = m.fluids.in_water;
                let first = m.first_tick;
                m.fluids = fluids;
                // Entity.doWaterSplashEffect on the client copy. Particles
                // only: ClientLevel draws addParticle, and playSeededSound
                // drops Entity.playSound (the server broadcasts that).
                if m.fluids.in_water && !was && !first && Some(m.id as i32) != own {
                    horizon_client::mobs::living::client_water_splash(m, &mut water_fx);
                }
                m.first_tick = false;
            }
            if let Some(name) = standing {
                m.standing_on = Some(name);
            }
        }
        let game_time = self.world.game_time;
        // `AbstractPiglin.isConverting`: the dimension is not piglin safe.
        let nether = !self.dimension_type().piglins_zombify;
        // What a wither's side heads can look at, by network entity id: the
        // mobs and the player (position and eye height).
        let wither_targets: Vec<(i32, [f64; 3], f64)> = if self
            .mobs
            .iter()
            .any(|m| m.species() == horizon_client::mobs::Species::Wither)
        {
            let own = self
                .remote
                .as_ref()
                .zip(self.eye_position())
                .map(|(r, eye)| (r.entity_id, eye, eye[1]));
            self.mobs
                .iter()
                .map(|m| {
                    (
                        m.id as i32,
                        [m.position.x, m.position.y, m.position.z],
                        m.eye_y(),
                    )
                })
                .chain(own)
                .collect()
        } else {
            Vec::new()
        };
        // ExperienceOrb.followNearbyPlayer: the players an orb may go for,
        // their feet and the point `eyeHeight / 2` above them.
        let followed: Vec<([f64; 3], [f64; 3])> = {
            let own = self
                .player
                .as_ref()
                .filter(|p| !p.health.dead_or_dying() && !self.spectator())
                .map(|p| {
                    let feet = [p.position.x, p.position.y, p.position.z];
                    let eye = f64::from(p.dimensions.eye_height);
                    (feet, [feet[0], feet[1] + eye / 2.0, feet[2]])
                });
            let others = self.remote.as_ref().into_iter().flat_map(|r| {
                self.mobs
                    .iter()
                    .filter(move |m| m.alive() && r.level_players.contains_key(&(m.id as i32)))
                    .filter(move |m| {
                        let listed = r.level_players.get(&(m.id as i32));
                        !listed
                            .and_then(|uuid| r.tab_players.get(uuid))
                            .is_some_and(|t| t.game_mode == SPECTATOR)
                    })
                    .map(|m| {
                        let feet = [m.position.x, m.position.y, m.position.z];
                        (
                            feet,
                            [feet[0], feet[1] + (m.eye_y() - feet[1]) / 2.0, feet[2]],
                        )
                    })
            });
            own.into_iter().chain(others).collect()
        };
        // The mount this client moves ticks in `remote_ride` instead, as its
        // authoritative side (LivingEntity.tick with travelRidden).
        let driven = self.driven_mount().map(|m| m.id);
        // Cat.handleLieDown: the sleeping players (another's, drawn lying in
        // a bed, and this client's own) a lying cat can lie on top of.
        let sleepers: Vec<horizon_core::geometry::Aabb> = self
            .remote
            .as_ref()
            .map(|r| {
                self.mobs
                    .iter()
                    .filter(|m| {
                        r.level_players.contains_key(&(m.id as i32)) && m.extra.ints[1] != 0
                    })
                    .map(|m| {
                        let p = m.position;
                        horizon_core::geometry::Aabb::new(
                            horizon_core::geometry::Vec3::new(p.x - 0.1, p.y, p.z - 0.1),
                            horizon_core::geometry::Vec3::new(p.x + 0.1, p.y + 0.2, p.z + 0.1),
                        )
                    })
                    .chain(self.sleeping.map(|bed| {
                        let p = horizon_core::geometry::Vec3::new(
                            f64::from(bed[0]) + 0.5,
                            f64::from(bed[1]) + 0.6875,
                            f64::from(bed[2]) + 0.5,
                        );
                        horizon_core::geometry::Aabb::new(
                            horizon_core::geometry::Vec3::new(p.x - 0.1, p.y, p.z - 0.1),
                            horizon_core::geometry::Vec3::new(p.x + 0.1, p.y + 0.2, p.z + 0.1),
                        )
                    }))
                    .collect()
            })
            .unwrap_or_default();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut lerps = std::mem::take(&mut r.lerps);
        let mut cart_lerps = std::mem::take(&mut r.cart_lerps);
        let mut using = Vec::new();
        let mut mob_fx = water_fx;
        let tick_listened = mod_api::__private::listened(mod_api::HookId::PlayerTickPre)
            || mod_api::__private::listened(mod_api::HookId::PlayerTickPost);
        for (i, rain) in rain.iter().copied().enumerate() {
            let id = self.mobs[i].id;
            let player = self
                .remote
                .as_ref()
                .unwrap()
                .level_players
                .contains_key(&(id as i32));
            if (!running && !player) || driven == Some(id) {
                continue;
            }
            let animation = if player && self.mobs[i].extra.using_item > 0 {
                let extra = &self.mobs[i].extra;
                let slot = if extra.using_off_hand {
                    horizon_items::equipment::OFFHAND
                } else {
                    horizon_items::equipment::MAINHAND
                };
                extra
                    .slot_stack(slot)
                    .map_or_else(Default::default, |stack| self.item_use_animation(stack))
            } else {
                Default::default()
            };
            let player_ref = tick_listened
                .then(|| {
                    self.remote.as_ref().and_then(|r| {
                        let uuid = *r.level_players.get(&(id as i32))?;
                        Some(mod_api::__private::game::player_ref(
                            mod_api::EntityRef::new(self.native_lifecycle.epoch()?, id),
                            uuid,
                        ))
                    })
                })
                .flatten();
            let r = self.remote.as_mut().unwrap();
            let stepped = &mut r.stepped;
            let players = &r.level_players;
            let tab_players = &r.tab_players;
            let m = &mut self.mobs[i];
            let section_from = m.position;
            if player {
                // AvatarRenderer.extractRenderState: isSpectator, from the player list.
                m.extra.spectator = players
                    .get(&(m.id as i32))
                    .and_then(|uuid| tab_players.get(uuid))
                    .is_some_and(|t| t.game_mode == SPECTATOR);
            }
            m.previous = m.position;
            // `Entity.deltaMovementOnPreviousTick`: the elytra's flight yaw
            // lerps from it, so another player's model does not snap when a
            // new tick's velocity arrives.
            m.extra.previous_velocity = m.velocity;
            m.previous_yaw = m.yaw;
            m.previous_body_yaw = m.body_yaw;
            m.previous_head_yaw = m.head_yaw;
            m.previous_head_pitch = m.head_pitch;
            m.previous_limb_speed = m.limb_speed;
            m.age += 1;
            if m.species() == horizon_client::mobs::Species::Rabbit {
                horizon_client::mobs::rabbit::base_tick(m);
            }
            if matches!(
                m.species(),
                horizon_client::mobs::Species::Horse
                    | horizon_client::mobs::Species::Donkey
                    | horizon_client::mobs::Species::Mule
                    | horizon_client::mobs::Species::SkeletonHorse
                    | horizon_client::mobs::Species::ZombieHorse
            ) {
                horizon_client::mobs::equine::tail_tick(m);
            }
            if m.species() == horizon_client::mobs::Species::Creeper {
                horizon_client::mobs::hostile::creeper_client_tick(m);
            }
            if matches!(
                m.species(),
                horizon_client::mobs::Species::Guardian
                    | horizon_client::mobs::Species::ElderGuardian
            ) {
                horizon_client::mobs::guardian::client_tick(m);
            }
            // PolarBear.tick: the stand animation follows the synched flag.
            if m.species() == horizon_client::mobs::Species::PolarBear {
                horizon_client::mobs::polar_bear::client_tick(m);
            }
            // The particles these mobs' own ticks draw on the client side.
            {
                use horizon_client::mobs::{
                    Species as S, cube, illager, illusioner, nautilus, tame, wither,
                };
                match m.species() {
                    S::Wolf => tame::client_tick(m, rain, &mut mob_fx),
                    S::Slime | S::MagmaCube | S::SulfurCube => cube::client_tick(m, &mut mob_fx),
                    S::Nautilus | S::ZombieNautilus => nautilus::client_tick(m, &mut mob_fx),
                    S::Illusioner => {
                        illusioner::client_tick(m, &mut mob_fx);
                        illager::cast_particles(m, &mut mob_fx);
                    }
                    S::Evoker => illager::cast_particles(m, &mut mob_fx),
                    S::Wither => {
                        wither::client_smoke(m, &mut mob_fx);
                        // `getInvulnerableTicks() > 0`: the charge's cloud,
                        // after the heads' smoke as in `aiStep`.
                        wither::charging_particles(m, &mut mob_fx);
                    }
                    _ => {}
                }
            }
            if m.hurt_time > 0 {
                m.hurt_time -= 1;
            }
            if !m.alive() {
                m.death_time += 1;
            }
            // LivingEntity.updateSwingTime.
            m.anim.attack_anim_o = m.anim.attack_anim;
            horizon_client::mobs::ai::tick_swing(m);
            // Entity.tick: the SteppedInterpolationHandler's step.
            if let Some(at) = stepped
                .get_mut(&(m.id as i32))
                .and_then(|h| h.interpolate())
            {
                place_interpolated(m, at);
            }
            // `ClientLevel.tickNonPassenger`'s `commonTick` interpolated the
            // entity before `Player.tick`, which `Pre` precedes.
            if let Some(player) = player_ref {
                self.native_player_tick_pre(mod_api::__private::PlayerTickPreCall { player });
            }
            let head_lerps = &mut self.remote.as_mut().unwrap().head_lerps;
            let m = &mut self.mobs[i];
            // LivingEntity.aiStep: the head's own interpolation.
            if let Some((yaw, steps)) = head_lerps.get_mut(&(m.id as i32))
                && *steps > 0
            {
                m.head_yaw += wrap_degrees(*yaw - m.head_yaw) / *steps as f32;
                *steps -= 1;
            }
            // Fox.tick: the head tilt and crouch follow the synched flags.
            if m.species() == horizon_client::mobs::Species::Fox {
                horizon_client::mobs::fox::client_tick(m);
            }
            // Cat.tick: the lying and relaxing amounts follow the synched flags.
            if m.species() == horizon_client::mobs::Species::Cat {
                m.anim.vo = m.anim.v;
                horizon_client::mobs::tame::cat_amounts(m);
                horizon_client::mobs::tame::cat_lying_on_sleeper(m, &sleepers);
            }
            // Chicken.aiStep: the wings follow the server's on-ground flag.
            if m.species() == horizon_client::mobs::Species::Chicken {
                horizon_client::mobs::farm::chicken_flap(m);
            }
            // Parrot.calculateFlapping: the wings follow the server's
            // on-ground flag and the shoulder it sits on (the glide is the
            // server's).
            if m.species() == horizon_client::mobs::Species::Parrot {
                horizon_client::mobs::parrot::flap(m);
            }
            // LivingEntity.updatingUsingItem: the ticks the item has been
            // used before this one. Another player's use particles; a mob's
            // drawn bow only needs the count (its pull follows it).
            if player {
                // AvatarRenderer.getArmPose: the used item's use animation
                // poses the arm.
                m.extra.use_animation = animation;
            }
            if m.extra.using_item > 0 {
                if player {
                    let item = if m.extra.using_off_hand {
                        m.extra.slot_stack(horizon_items::equipment::OFFHAND)
                    } else {
                        m.extra.slot_stack(horizon_items::equipment::MAINHAND)
                    }
                    .map(|s| s.item.clone());
                    if let Some(item) = item {
                        let eye = m.eye_height();
                        let at = [m.position.x, m.position.y, m.position.z];
                        using.push((
                            item,
                            m.extra.using_item as u32 - 1,
                            at,
                            eye,
                            m.head_pitch,
                            m.yaw,
                        ));
                    }
                }
                m.extra.using_item += 1;
            }
            // LivingEntity.tick's tickHeadTurn: a mob's BodyRotationControl,
            // a player's body following its walk and look.
            if player {
                remote_player_head_turn(m);
                // LivingEntity.tick: the glide count, wings and swim amount.
                m.tick_avatar_pose(m.fluids.in_water);
                // LivingEntity.tick: a sleeper's xRot is 0.
                if m.extra.ints[1] != 0 {
                    m.head_pitch = 0.0;
                    m.previous_head_pitch = 0.0;
                }
            } else if m.alive() {
                m.body_rotation_tick();
            }
            // Squid.aiStep; WalkAnimationState.update from the horizontal distance.
            horizon_client::mobs::aquatic::squid_client_step(m, &mut mob_fx);
            let d = (m.position.x - m.previous.x).hypot(m.position.z - m.previous.z) as f32;
            let (target, factor) = if m.species() == horizon_client::mobs::Species::Frog {
                (horizon_client::mobs::frog::walk_target(m, d), 0.4)
            } else if horizon_client::mobs::camel::is_camel(m.species()) {
                horizon_client::mobs::camel::walk_target(m, d)
            } else {
                ((d * 4.0).min(1.0), 0.4)
            };
            m.limb_speed += (target - m.limb_speed) * factor;
            m.limb_position += m.limb_speed;
            if m.species() == horizon_client::mobs::Species::Rabbit {
                horizon_client::mobs::rabbit::post_tick(m);
            }
            if m.species() == horizon_client::mobs::Species::Bat {
                horizon_client::mobs::bat::setup_animation_states(m);
            }
            // The animation states a server computes in its own `ai_step`,
            // and the particles vanilla draws on the client.
            horizon_client::mobs::sync::client_tick(m, game_time, &mut mob_fx);
            match m.species() {
                // WitherBoss.aiStep: the side heads turn toward their targets.
                horizon_client::mobs::Species::Wither => {
                    horizon_client::mobs::wither::client_tick(m, |id| {
                        wither_targets
                            .iter()
                            .find(|t| t.0 == id)
                            .map(|t| (t.1, t.2))
                    });
                }
                // Hoglin and piglin `isConverting` outside the Nether.
                horizon_client::mobs::Species::Hoglin
                | horizon_client::mobs::Species::Piglin
                | horizon_client::mobs::Species::PiglinBrute => {
                    horizon_client::mobs::piglin::client_converting(m, nether);
                }
                horizon_client::mobs::Species::Panda => {
                    horizon_client::mobs::panda::client_tick(m, thundering);
                }
                // Shulker.tick's onPeekAmountChange: the lid moves this
                // client's player, whose movement is its own.
                horizon_client::mobs::Species::Shulker => {
                    horizon_client::mobs::shulker::client_push(m, &mut mob_fx);
                }
                _ => {}
            }
            let section_id = m.id;
            let section_to = m.position;
            if let Some(player) = player_ref {
                self.native_player_tick_post(mod_api::__private::PlayerTickPostCall { player });
            }
            if section_from != section_to {
                self.remote_entity_section_step(section_id, section_from, section_to);
            }
        }
        // ExperienceOrb.tick runs on a client too (all but the merges and the
        // push out of a block, which are the server's): gravity, buoyancy,
        // collisions and the pull of the nearest player between the server's
        // updates; the server removes it on pickup.
        let (world, facts, collision) = (&self.world, self.res.facts(), self.collision);
        let border = Some(self.border.geometry());
        let mut lava_random = None;
        let mut next_float = || {
            lava_random
                .get_or_insert_with(random_source::entity)
                .next_float()
        };
        if running {
            self.orbs.retain_mut(|o| {
                let distance = |feet: &[f64; 3]| -> f64 {
                    (feet[0] - o.position.x).powi(2)
                        + (feet[1] - o.position.y).powi(2)
                        + (feet[2] - o.position.z).powi(2)
                };
                let target = followed
                    .iter()
                    .filter(|(feet, _)| distance(feet) < 64.0)
                    .min_by(|a, b| distance(&a.0).total_cmp(&distance(&b.0)))
                    .map(|(_, target)| *target);
                // Entity.commonTick: setOldPosAndRot, then
                // LinearInterpolationHandler.doInterpolate moves 1/remaining of
                // the way to the server's position, so the frame draws the step
                // from the old position.
                let old = o.position;
                if let Some(l) = lerps.get_mut(&(o.id as i32)).filter(|l| l.steps > 0) {
                    let alpha = 1.0 / f64::from(l.steps);
                    o.position = Vec3::new(
                        o.position.x + (l.target[0] - o.position.x) * alpha,
                        o.position.y + (l.target[1] - o.position.y) * alpha,
                        o.position.z + (l.target[2] - o.position.z) * alpha,
                    );
                    l.steps -= 1;
                }
                let before = o.position;
                let kept = o.tick(
                    world,
                    facts,
                    collision,
                    target,
                    false,
                    &mut next_float,
                    border,
                    &self.res.motion().fluids,
                    &self.loop_context,
                    &mut |_, _| {},
                );
                // `tick` set `previous` to the stepped position: keep the
                // pre-step one, as `xo` is in vanilla.
                o.previous = old;
                // AbstractInterpolationHandler.applyPredictedMovement: the orb's
                // own move carries the target along (vanilla also checks the
                // moved target for collisions; that check is left out here).
                if let Some(l) = lerps.get_mut(&(o.id as i32)).filter(|l| l.steps > 0) {
                    l.target[0] += o.position.x - before.x;
                    l.target[1] += o.position.y - before.y;
                    l.target[2] += o.position.z - before.z;
                }
                kept
            });
        }
        // ItemEntity.tick runs on a client too: gravity, collisions and
        // friction move the item between the server's updates (every 20
        // ticks). Ordinary removal remains server-authoritative. Mods can opt
        // into predicting consumption at block contact.
        self.remote
            .as_mut()
            .unwrap()
            .head_lerps
            .retain(|_, (_, steps)| *steps > 0);
        let mut entered = Vec::new();
        if running {
            entity_section_steps::tick_items(self, &mut entered);
        }
        // Entity.doWaterSplashEffect on this window: Level.addParticle draws
        // the bubbles and splashes. ClientLevel.playSeededSound drops
        // Entity.playSound, so the sound stays the server's.
        for splash in entered {
            self.apply_item_splash(splash);
        }
        let _ = self.handle_mob_events_reusable(&mut mob_fx);
        if let Some(r) = self.remote.as_mut() {
            r.tick_fluids = fluids;
            r.tick_rain = rain;
            r.tick_standings = standings;
            r.tick_water_fx = mob_fx;
        }
        for (item, ticks, at, eye, pitch, yaw) in using {
            self.remote_player_eating(&item, ticks, at, eye, pitch, yaw);
        }
        // ClientLevel.tick: the sky flash fades in a frozen level too.
        self.flash = self.flash.saturating_sub(1);
        if running {
            self.guardian_bubbles();
            // The dragon: LivingEntity's interpolation, inside its aiStep.
            let dragon_lerp = self
                .dragon
                .as_ref()
                .and_then(|d| lerps.get_mut(&(d.id as i32)))
                .filter(|l| l.steps > 0)
                .map(|l| {
                    let steps = l.steps;
                    l.steps -= 1;
                    (l.target, l.rotation.map(|r| r.0), steps)
                });
            self.tick_remote_dragon(dragon_lerp);
            // Armor stands: ClientLevel.tickNonPassenger's setOldPosAndRot,
            // then their SteppedInterpolationHandler's step, as the mobs'.
            if let Some(r) = self.remote.as_mut() {
                for a in &mut self.armor_stands {
                    let step = r
                        .stepped
                        .get_mut(&(a.id as i32))
                        .and_then(|h| h.interpolate());
                    a.interpolate(step.map(|at| {
                        let p = at.position;
                        (Vec3::new(p[0], p[1], p[2]), at.yaw)
                    }));
                }
            }
            // The yaw of a boat eases toward the server's over the same steps
            // as its position, read before `step` counts one off.
            let turns: HashMap<i32, (f32, u32)> = lerps
                .iter()
                .filter(|(_, l)| l.steps > 0)
                .filter_map(|(id, l)| l.rotation.map(|r| (*id, (r.0, l.steps))))
                .collect();
            // Boats, minecarts, projectiles and the rest ease toward the
            // server's position over three ticks (`lerps`).
            let mut step = |id: u32, at: Vec3| match lerps.get_mut(&(id as i32)) {
                Some(l) if l.steps > 0 => {
                    let n = f64::from(l.steps);
                    l.steps -= 1;
                    Vec3::new(
                        at.x + (l.target[0] - at.x) / n,
                        at.y + (l.target[1] - at.y) / n,
                        at.z + (l.target[2] - at.z) / n,
                    )
                }
                _ => at,
            };
            self.tick_remote_objects(
                &mut step,
                |id, yaw| {
                    turns.get(&(id as i32)).map_or(yaw, |&(target, steps)| {
                        super::remote_objects::lerp_yaw_step(yaw, target, steps)
                    })
                },
                |c| {
                    cart_lerps
                        .get_mut(&(c.id as i32))
                        .is_some_and(|l| l.tick(c))
                },
            );
            self.tick_remote_projectiles(&mut step);
            // LightningBolt.tick on a client.
            self.tick_bolts();
        }
        lerps.retain(|_, l| l.steps > 0);
        if let Some(r) = self.remote.as_mut() {
            for (id, l) in lerps {
                r.lerps.entry(id).or_insert(l);
            }
            for (id, l) in cart_lerps {
                r.cart_lerps.entry(id).or_insert(l);
            }
        }
    }
    /// `AbstractMinecartRenderer.newExtractState`: sample the same weighted
    /// steps at the frame's partial tick, including corners within one tick.
    pub(super) fn remote_minecart_lerp(
        &self,
        id: u32,
        partial: f32,
    ) -> Option<([f64; 3], f32, f32)> {
        let lerp = self.remote.as_ref()?.cart_lerps.get(&(id as i32))?;
        if lerp.current.is_empty() {
            return None;
        }
        let (p, _, yaw, pitch) = lerp.at(partial);
        Some(([p.x, p.y, p.z], yaw, pitch))
    }

    /// `ClientPacketListener.handleMinecartAlongTrack`: the steps join the ones
    /// the minecart has queued, if it runs `NewMinecartBehavior`.
    fn remote_move_minecart(&mut self, p: play::MoveMinecartAlongTrack) {
        if !self.minecart_improvements() || !self.carts.iter().any(|c| c.id == p.id as u32) {
            return;
        }
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let at = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
        r.cart_lerps
            .entry(p.id)
            .or_insert_with(CartLerp::new)
            .pending
            .extend(p.steps.iter().map(|s| CartStep {
                position: at(s.position),
                movement: at(s.movement),
                yaw: horizon_protocol::buf::unpack_degrees(s.yaw),
                pitch: horizon_protocol::buf::unpack_degrees(s.pitch),
                weight: s.weight,
            }));
    }
    /// `Consumable.onConsumeTick` for another player using a consumable: the
    /// crumbs at their mouth every 4 ticks once 21.875% of its use has passed
    /// (`shouldEmitParticlesAndSounds`; the sounds come from the server).
    fn remote_player_eating(
        &mut self,
        item: &str,
        ticks: u32,
        at: [f64; 3],
        eye: f64,
        pitch: f32,
        yaw: f32,
    ) {
        let Some(consumable) = self
            .res
            .items()
            .get(item)
            .and_then(|i| i.consumable.as_ref())
            .filter(|c| c.particles)
        else {
            return;
        };
        let duration = consumable.ticks().max(0) as u32;
        let remaining = duration - ticks.min(duration);
        if !consumable.should_emit(remaining as i32) {
            return;
        }
        let options = self.item_particle(&ItemStack::new(item, 1));
        let mut spawned = Vec::new();
        horizon_client::particles::item_use_particles(
            &mut self.world.random,
            pitch,
            yaw,
            at,
            at[1] + eye,
            5,
            |p, v| spawned.push((p, v)),
        );
        for (p, v) in spawned {
            self.particles.add(&options, p, v);
        }
    }
    /// The end of the player's tick: `AbstractBoat.tick`'s paddles (the vehicle ticks before its
    /// passenger's `sendChanges`), then `LocalPlayer.sendChanges`: the input, and a passenger's look,
    /// vehicle and sprint, or `sendPosition`'s sprint and move. The held slot is not sent here:
    /// `MultiPlayerGameMode.tick` sends it at the start of the next tick (`remote_handle_input`).
    fn remote_send_state(&mut self, input: u8) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let (pos, on_ground, collision, sprinting) = (
            [p.position.x, p.position.y, p.position.z],
            p.ground.on_ground,
            p.collision.horizontal,
            p.sprinting,
        );
        let (yaw, pitch) = (self.yaw, self.pitch);
        let riding = self.riding.is_some();
        let controls_camera = self.controls_camera();
        let (paddles, vehicle): (Vec<S>, Vec<S>) = self
            .remote_vehicle_packets()
            .into_iter()
            .partition(|p| matches!(p, S::PaddleBoat(_)));
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut out = paddles;
        if input != r.last_input {
            r.last_input = input;
            out.push(S::PlayerInput(play::PlayerInput { flags: input }));
        }
        // LocalPlayer.sendIsSprintingIfNeeded.
        let sprint = |r: &mut Remote, out: &mut Vec<S>| {
            if sprinting != r.was_sprinting {
                r.was_sprinting = sprinting;
                out.push(S::PlayerCommand(play::PlayerCommand {
                    id: r.entity_id,
                    action: if sprinting { 1 } else { 2 },
                    data: 0,
                }));
            }
        };
        let mut moved_or_rotated = false;
        if riding {
            // LocalPlayer.sendChanges: a passenger sends its look, then the
            // vehicle it controls and, only then, its sprint.
            out.push(S::MovePlayerRot(play::MovePlayerRot {
                yaw,
                pitch,
                flags: u8::from(on_ground) | u8::from(collision) << 1,
            }));
            if !vehicle.is_empty() {
                out.extend(vehicle);
                sprint(r, &mut out);
            }
            moved_or_rotated = true;
        } else {
            sprint(r, &mut out);
            // LocalPlayer.sendPosition: no move while the view is another
            // entity's (the server keeps the player on it).
            if controls_camera
                && let Some(packet) =
                    r.position_sender
                        .update(pos, (yaw, pitch), on_ground, collision)
            {
                out.push(packet);
                moved_or_rotated = true;
            }
        }
        if moved_or_rotated && r.session.is_recording() {
            let pkt = horizon_protocol::packets::play::Clientbound::PlayerPosition(
                horizon_protocol::packets::play::PlayerPosition {
                    teleport: 0,
                    position: pos,
                    delta: [0.0, 0.0, 0.0],
                    yaw,
                    pitch,
                    relatives: 0,
                },
            );
            r.session.record_clientbound(&pkt);
        }
        if r.session.is_recording() {
            let is_using = self.using.is_some();
            let living_flags: u8 = if is_using {
                1 | if self.hand == Hand::Off { 2 } else { 0 }
            } else {
                0
            };
            let crouching = p.local_crouching || matches!(p.pose, crate::pose::Pose::Crouching);
            let mut shared: u8 = 0;
            if crouching {
                shared |= 0x02;
            }
            if sprinting {
                shared |= 0x08;
            }
            if p.swimming {
                shared |= 0x10;
            }
            if p.fall_flying {
                shared |= 0x80;
            }
            if shared != r.was_shared_flags
                || living_flags != r.was_living_flags
                || r.last_recorded_pose != Some(p.pose)
            {
                r.was_shared_flags = shared;
                r.was_living_flags = living_flags;
                r.last_recorded_pose = Some(p.pose);
                use horizon_protocol::entity_data::{Value, serializer};
                let pkt = horizon_protocol::packets::play::Clientbound::SetEntityData(
                    horizon_protocol::packets::play::SetEntityData {
                        id: r.entity_id,
                        values: vec![
                            (0, serializer::BYTE, Value::Byte(shared as i8)),
                            (6, serializer::POSE, Value::Id(p.pose.id())),
                            (8, serializer::BYTE, Value::Byte(living_flags as i8)),
                        ],
                    },
                );
                r.session.record_clientbound(&pkt);
            }
        }
        // Creative inventory edits are sent slot by slot (SetCreativeModeSlot).
        let mut creative_slots = Vec::new();
        if self.creative {
            // A server's open menu takes clicks, which it applies itself: what
            // this client predicts for them isn't an edit to send (it would
            // overwrite the server's stacks whenever the prediction is wrong).
            let in_menu = r.menu.as_ref().is_some();
            let stacks = creative_sync_stacks(&self.inventory, &self.offhand, &self.armor);
            for (i, stack) in stacks.into_iter().enumerate() {
                if stack != r.synced[i] {
                    r.synced[i] = stack.clone();
                    if !in_menu {
                        creative_slots.push((creative_sync_slot(i), stack));
                    }
                }
            }
        }
        for packet in out {
            self.send(packet);
        }
        for (slot, stack) in creative_slots {
            let item = self.to_network(&stack);
            self.send(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
                slot,
                item,
            }));
        }
    }
    pub(in crate::game) fn creative_stacks(&self) -> Vec<Option<ItemStack>> {
        creative_sync_stacks(&self.inventory, &self.offhand, &self.armor)
    }
    /// `CreativeInventoryListener.slotChanged`: `handleCreativeModeItemAdd` for every slot the
    /// player's inventory menu changed. `CreativeModeInventoryScreen.slotClicked` ends in
    /// `broadcastChanges`, so a click's slots go out with the click, right behind a thrown
    /// stack, and not at the end of the tick: a slot update the server sends meanwhile (the
    /// pickup of what a held Q threw) can then never land on a change still unsent and undo it.
    pub(super) fn send_creative_slots(&mut self) {
        if !self.creative {
            return;
        }
        let stacks = self.creative_stacks();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut changed = Vec::new();
        for (i, stack) in stacks.into_iter().enumerate() {
            if stack != r.synced[i] {
                r.synced[i] = stack.clone();
                changed.push((creative_sync_slot(i), stack));
            }
        }
        for (slot, stack) in changed {
            let item = self.to_network(&stack);
            self.send(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
                slot,
                item,
            }));
        }
    }
    /// Inventory as the server last described it.
    fn remote_synced(&mut self) {
        let stacks = self.creative_stacks();
        if let Some(r) = self.remote.as_mut() {
            r.synced = stacks;
        }
    }
    /// The slots a server packet changed since `before` are as the server has them; the
    /// others keep what was last sent, so a creative click made since still goes out.
    pub(in crate::game) fn remote_synced_since(&mut self, before: &[Option<ItemStack>]) {
        let now = self.creative_stacks();
        if let Some(r) = self.remote.as_mut() {
            mark_synced(&mut r.synced, before, now);
        }
    }

    /// `MultiPlayerGameMode.stopDestroyBlock`: aborts the break and starts the attack strength
    /// again, if one is running (`if (!isDestroying) return;`), and says whether it did. Only the
    /// progress goes: `destroyBlockPos` and `destroyingItem` keep the target, so a dig resumed on
    /// the same block sends no START and the server's clock runs on (#4124).
    pub(super) fn stop_remote_mining(&mut self) -> bool {
        let kept = self.remote.as_mut().and_then(|r| {
            if !std::mem::take(&mut r.is_destroying) {
                return None;
            }
            let (pos, face, _) = r.destroying?;
            r.destroying = Some((pos, face, 0.0));
            r.destroying
        });
        let Some((pos, _, _)) = kept else {
            return false;
        };
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.tutorial_destroy(state, -1.0);
        self.send(S::PlayerAction(play::PlayerAction {
            action: 2,
            pos,
            direction: 0,
            sequence: 0,
        }));
        self.reset_attack_strength();
        true
    }

    /// MultiPlayerGameMode.startDestroyBlock / continueDestroyBlock.
    fn remote_mining(&mut self, start: bool) -> bool {
        let debug_stick = self
            .held()
            .is_some_and(|stack| &*stack.item == "minecraft:debug_stick");
        if !self
            .res
            .items()
            .can_destroy_block(self.held(), self.creative)
        {
            self.stop_remote_mining();
            self.mining = None;
            return true;
        }
        let Some(r) = self.remote.as_mut() else {
            return true;
        };
        if !start && r.destroy_delay > 0 {
            r.destroy_delay -= 1;
            return true;
        }
        let Some(hit) = self.pick() else {
            self.mining = None;
            return true;
        };
        let face = face_direction(hit.face);
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if self.res.facts().is_air(state) {
            return true;
        }
        // startDestroyBlock refuses a restricted break before predicting it.
        if self.remote_break_restricted(hit.pos) {
            self.mining = None;
            return true;
        }
        if self.creative {
            if self.veto_mining_prediction(hit.pos, hit.face, true, 1.0) {
                return false;
            }
            let seq = self.next_sequence();
            // DebugStickItem.canDestroyBlock never lets the client break the
            // block; the server only selects the property on this START.
            if !debug_stick {
                self.predict_break(hit.pos, state, seq);
            }
            self.send(S::PlayerAction(play::PlayerAction {
                action: 0,
                pos: hit.pos,
                direction: face,
                sequence: seq,
            }));
            if let Some(r) = self.remote.as_mut() {
                r.destroy_delay = 5;
            }
            return true;
        }
        // sameDestroyTarget: the same block and still the item the break began with.
        let same = self.remote.as_ref().is_some_and(|r| {
            r.destroying.is_some_and(|(p, _, _)| p == hit.pos)
                && self.same_destroying_item(&r.destroying_item)
        });
        // startDestroyBlock leaves only a break that is still running alone (`isDestroying &&
        // sameDestroyTarget`); a click on the block a stopped break still names starts it again,
        // so the server re-anchors its clock. A held button carries the same target on, and that
        // one keeps the clock (#4124).
        let running = self.remote.as_ref().is_some_and(|r| r.is_destroying);
        if start && same && running {
            return true;
        }
        // The server's own getDestroyProgress: under water, off the ground,
        // with haste, fatigue and efficiency, so the client stops digging
        // when the server is done too (#1254).
        let progress = self.destroy_progress(hit.pos);
        if !same || start {
            let proposed = if progress >= 1.0 { 1.0 } else { 0.0 };
            if self.veto_mining_prediction(hit.pos, hit.face, true, proposed) {
                return false;
            }
            // startDestroyBlock aborts only a break that is running; a target left by a stop that
            // ended one is just replaced by the START below.
            let aborted = self.remote.as_mut().and_then(|r| {
                if !std::mem::take(&mut r.is_destroying) {
                    return None;
                }
                r.destroying.take()
            });
            if let Some((old, _, _)) = aborted {
                let old_state = self.world.block(old[0], old[1], old[2]);
                self.tutorial_destroy(old_state, -1.0);
                self.send(S::PlayerAction(play::PlayerAction {
                    action: 2,
                    pos: old,
                    direction: face,
                    sequence: 0,
                }));
            }
            let seq = self.next_sequence();
            self.send(S::PlayerAction(play::PlayerAction {
                action: 0,
                pos: hit.pos,
                direction: face,
                sequence: seq,
            }));
            self.tutorial_destroy(state, 0.0);
            // startDestroyBlock: BlockState.attack on the client, so a redstone
            // ore's dust (RedStoneOreBlock.attack) comes before the break.
            self.redstone_ore_particles(hit.pos);
            if progress >= 1.0 {
                // startDestroyBlock: instant survival breaks have no destroy delay.
                if !debug_stick {
                    self.predict_break(hit.pos, state, seq);
                }
                return true;
            }
            let held = self.main_hand_key();
            if let Some(r) = self.remote.as_mut() {
                (r.destroying, r.destroying_item) = (Some((hit.pos, face, 0.0)), held);
                r.is_destroying = true;
            }
            // startDestroyBlock plays the first hit and crack itself; the
            // server's level events 2019 and 2020 do the rest.
            self.breaking_block_effects(hit.pos, hit.face, true);
            self.mining = Some(Mining {
                pos: hit.pos,
                progress: 0.0,
                per_tick: 0.0,
                ticks: 0,
            });
            return true;
        }
        let Some((pos, last_face, done)) = self.remote.as_ref().and_then(|r| r.destroying) else {
            return true;
        };
        let done = done + progress;
        if self.veto_mining_prediction(pos, hit.face, false, done) {
            return false;
        }
        self.tutorial_destroy(state, done.clamp(0.0, 1.0));
        let ticks = self.mining.as_ref().map_or(0, |m| m.ticks) + 1;
        if done >= 1.0 {
            let seq = self.next_sequence();
            // DebugStickItem.canDestroyBlock: the server selects the property
            // and sends the block update instead; never predict a break.
            if !debug_stick {
                self.predict_break(pos, state, seq);
            }
            self.send(S::PlayerAction(play::PlayerAction {
                action: 3,
                pos,
                direction: face,
                sequence: seq,
            }));
            if let Some(r) = self.remote.as_mut() {
                // `destroyBlockPos` remains the last target after the break;
                // if that position is filled again before the next pick, a
                // held attack resumes it without sending a second START.
                r.destroying = Some((pos, face, 0.0));
                r.is_destroying = false;
                r.destroy_delay = 5;
            }
            self.mining = None;
        } else {
            if last_face != face {
                self.send(S::PlayerAction(play::PlayerAction {
                    action: 1,
                    pos,
                    direction: face,
                    sequence: 0,
                }));
            }
            if let Some(r) = self.remote.as_mut() {
                r.destroying = Some((pos, face, done));
            }
            self.mining = Some(Mining {
                pos,
                progress: done,
                per_tick: 0.0,
                ticks,
            });
        }
        true
    }

    fn veto_mining_prediction(
        &self,
        pos: [i32; 3],
        face: [i32; 3],
        starting: bool,
        progress: f32,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ClientMiningPredict) {
            return false;
        }
        let mut canceled = false;
        self.native_client_mining_predict(
            mod_api::ClientMiningPrediction {
                pos,
                face,
                starting,
                progress: progress.clamp(0.0, 1.0),
            },
            &mut canceled,
        );
        canceled
    }
    /// `Player.blockActionRestricted` for the block at `pos`: a spectator
    /// never breaks, an adventure player only with a main-hand item whose
    /// `can_break` matches the block (`ItemStack.canBreakBlockInAdventureMode`).
    /// A predicate this client could not decode (exact components) only
    /// tells that the server gave one, and the server sends the block back
    /// if it does not match.
    fn remote_break_restricted(&self, pos: [i32; 3]) -> bool {
        use horizon_protocol::item::{WirePatch, component::CAN_BREAK};
        if self
            .held()
            .and_then(|s| s.data())
            .is_some_and(|d| d.other("minecraft:can_break").is_some())
        {
            return self.block_action_restricted(pos);
        }
        self.spectator()
            || (!self.may_build()
                && !self
                    .held()
                    .and_then(|s| s.wire_patch())
                    .is_some_and(|p| WirePatch::adds(&p, CAN_BREAK)))
    }

    /// `Minecraft.startUseItem`'s block turn for `hand`: a use that swung drops
    /// the hand (`LocalPlayer.itemUsed`) when it spent the held item or the
    /// player has infinite materials.
    fn remote_use_block(&mut self, hand: Hand) -> bool {
        let before = self.with_hand(hand, |g| g.held().map(|s| s.count));
        let swings = self.swings_started;
        let done = self.remote_use_item_on(hand);
        if done
            && self.swings_started != swings
            && let Some(after) = self.with_hand(hand, |g| g.held().map(|s| s.count))
            && (Some(after) != before || self.creative)
        {
            self.heights.item_used(hand);
        }
        done
    }

    /// `Item.useOn` on this client's level for the items whose effects are
    /// the clicker's own (`Level.levelEvent(player, ...)` and
    /// `playSound(player, ...)`: the server sends them to the others only):
    /// honeycomb waxing copper, and an axe, hoe or shovel transforming a
    /// block (stripping, scraping, removing wax, tilling, making a path).
    /// A cushion succeeds where it may be placed and spawns on the server, a
    /// firework rocket wherever it may be used, and the server launches it.
    /// Whether the item's use succeeded, which ends the click.
    /// The caller has selected Item.useOn after cooldown and adventure gates.
    fn predict_item_on_block(&mut self, stack: &ItemStack, hit: &super::Hit) -> bool {
        // FireworkRocketItem.useOn: SUCCESS unless gliding, when Item.use
        // boosts instead. The phase's cooldown/adventure gates already ran.
        if &*stack.item == "minecraft:firework_rocket" {
            return !self.player.as_ref().is_some_and(|p| p.fall_flying);
        }
        let at = [
            f64::from(hit.pos[0]) + 0.5,
            f64::from(hit.pos[1]) + 0.5,
            f64::from(hit.pos[2]) + 0.5,
        ];
        if &*stack.item == "minecraft:honeycomb" {
            // HoneycombItem.useOn.
            let registry = self.pack.assets.registry;
            let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let old = registry.state(clicked);
            let waxes = old
                .and_then(|old| self.res.transformers().waxed(registry, old))
                .is_some();
            if waxes {
                self.level_event_at(3003, hit.pos, 0);
                self.play_client_local_sound(
                    LevelSound::new("minecraft:item.honeycomb.wax_on", at, 1.0, 1.0)
                        .in_source(mod_api::SoundSource::Blocks),
                );
                // The other half of a double chest: its particles, and a second
                // sound from the clicked block.
                if let Some(other) =
                    old.and_then(|old| super::storage::chest_connected_pos(old, hit.pos))
                {
                    self.level_event_at(3003, other, 0);
                    self.play_client_local_sound(
                        LevelSound::new("minecraft:item.honeycomb.wax_on", at, 1.0, 1.0)
                            .in_source(mod_api::SoundSource::Blocks),
                    );
                }
            }
            waxes
        } else if let Some((_, sound, _, _, particle, _)) = self.block_transform(stack, hit) {
            self.transform_particle(&particle, hit.pos);
            self.play_client_local_sound(
                LevelSound::new(&sound, at, 1.0, 1.0).in_source(mod_api::SoundSource::Blocks),
            );
            true
        } else if self
            .res
            .items()
            .get(&stack.item)
            .is_some_and(|i| i.cushion_color.is_some())
        {
            // CushionItem.useOn on a client level: SUCCESS once
            // `canBePlacedAt` holds, before the server's own checks.
            self.cushion_spawn(hit).is_some()
        } else if &*stack.item == "minecraft:painting" {
            // HangingEntityItem.useOn on a client level: SUCCESS once
            // `Painting.create` finds a surviving variant, before the
            // server spawns it.
            self.painting_fits(hit, stack)
        } else {
            false
        }
    }

    /// `MapItem.useOn` on this client: a map used on a banner is SUCCESS
    /// whatever its data says (the server toggles the marker, or fails
    /// quietly), so this hand swings and the click ends. The caller applies
    /// ItemStack.useOn's adventure predicate before selecting this phase.
    fn predict_map_on_banner(&self, stack: &ItemStack, hit: &super::Hit) -> bool {
        super::maps::map_id(stack).is_some()
            && self.named_block_in_tag("minecraft:banners", &self.block_name(hit.pos))
    }

    /// `BoneMealItem.useOn` with the acting hand's `stack` on this client's
    /// level (a copy: nothing grows here): whether it succeeds.
    fn remote_bone_meal_succeeds(&mut self, hit: &super::Hit, stack: &ItemStack) -> bool {
        let pos = horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
        let face = horizon_core::geometry::Direction::ALL[face_direction(hit.face) as usize];
        let (ask, _) = self.held_bonemeal(pos, stack);
        self.with_level(|b, level, _| {
            b.bone_meal_succeeds_on_client(
                &mut DryLevel::new(level),
                pos,
                face,
                ask.crop,
                ask.check,
            )
        })
    }

    /// MultiPlayerGameMode.useItemOn inside startPrediction: the blocks the
    /// use changed on this client's level at once, each kept as predicted
    /// under the use's sequence until the server acknowledges it (a use the
    /// server refused goes back). Only the blocks a hand opens or flips: a
    /// wooden or copper door, trapdoor or fence gate, a lever or a button;
    /// whatever else the use changes waits for the server.
    fn predict_use(&mut self, (changes, sounds): &DryUse, seq: i32) {
        let facts = self.res.facts();
        let predicted = |state: BlockStateId| hand_operated(&facts.get(state).name);
        if changes.is_empty()
            || !changes
                .iter()
                .all(|(p, s)| predicted(*s) && predicted(self.world.block(p.x, p.y, p.z)))
        {
            return;
        }
        self.predict_blocks(changes, seq);
        // `level.playSound(player, ...)`: the clicker hears it from its own use; the
        // server sends it to the others only.
        for (pos, sound, volume, pitch) in sounds {
            let at = [
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 0.5,
                f64::from(pos.z) + 0.5,
            ];
            let name = if sound.contains(':') {
                sound.to_string()
            } else {
                format!("minecraft:{sound}")
            };
            self.play_client_local_sound(
                LevelSound::new(&name, at, *volume, *pitch).in_source(mod_api::SoundSource::Blocks),
            );
        }
    }

    /// Another player of this level, whose copy passes every click (`Player.interactOn`).
    pub(super) fn is_level_player(&self, id: i32) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|r| r.level_players.contains_key(&id))
    }

    /// Whether a main-hand right click with `held` would end on the block hit, for the gameplay
    /// guide: what `remote_block_reacts` answers on its dry level.
    pub(super) fn block_takes_click(&mut self, hit: &super::Hit, held: Option<&ItemStack>) -> bool {
        self.remote_block_reacts(hit, held).0.consumes_action()
    }
    /// `InteractionResult.CONSUME` for this click: the block keeps it and does nothing, so the
    /// item in hand never gets it either (an empty flower pot, a pressed button).
    pub(super) fn block_swallows_click(
        &mut self,
        hit: &super::Hit,
        held: Option<&ItemStack>,
    ) -> bool {
        matches!(
            self.remote_block_reacts(hit, held).0,
            InteractionResult::Consume
        )
    }

    /// Whether the block hit takes a main-hand right click with `held`
    /// (`BlockState.useItemOn`, then `useWithoutItem`), once the caller let
    /// the block react (no sneaking with full hands): a door, a button, a
    /// container, and the blocks it changed. Vanilla's client runs it on its
    /// own level first; this runs it on a copy, and `predict_use` keeps the
    /// changes it predicts.
    fn remote_block_reacts(
        &mut self,
        hit: &super::Hit,
        held: Option<&ItemStack>,
    ) -> (InteractionResult, DryUse) {
        // AbstractCauldronBlock.useItemOn (CauldronInteractions): a bucket,
        // bottle or dye the cauldron takes fills or lowers it, and the click
        // ends there instead of falling through to Item.use (which would
        // empty a water bucket into the face of the cauldron).
        if let Some(reacts) = self.remote_cauldron_reacts(hit, held) {
            return reacts;
        }
        if self.jukebox_takes(hit.pos, held) {
            return (InteractionResult::Success, DryUse::default());
        }
        // RespawnAnchorBlock.useItemOn: PASS when the off hand's glowstone is to
        // charge the anchor, so useWithoutItem doesn't run and the off hand's turn
        // comes (glowstone in this hand charges it in `use_item_on`).
        if !self.anchor_takes(hit.pos, held) && self.anchor_takes(hit.pos, self.offhand.as_ref()) {
            return (InteractionResult::Pass, DryUse::default());
        }
        // BeehiveBlock.useItemOn: shears or a glass bottle on a full hive
        // (honey level 5) is SUCCESS, so the click ends after UseItemOn and
        // no UseItem follows (the server does the harvest, see `use_beehive`).
        if held.is_some_and(|s| matches!(&*s.item, "minecraft:shears" | "minecraft:glass_bottle"))
            && self
                .hive_state(hit.pos)
                .is_some_and(|(honey, _)| honey == Some(5))
        {
            return (InteractionResult::Success, DryUse::default());
        }
        // ComposterBlock.useItemOn: a compostable item on a composter below
        // level 8 is SUCCESS on the client too (only the server adds the
        // layer), so the other hand (a torch) is never tried.
        if self.composter_takes(hit.pos, held) {
            return (InteractionResult::Success, DryUse::default());
        }
        self.remote_block_template(hit, held, false)
    }

    /// The shared dry-level boundary for a normal block use or the empty
    /// method requested by a canceled `BLOCK` phase's `TryWithEmptyHand`.
    pub(super) fn remote_block_template(
        &mut self,
        hit: &super::Hit,
        held: Option<&ItemStack>,
        without_item: bool,
    ) -> (InteractionResult, DryUse) {
        use horizon_core::geometry::Direction;
        let face = match hit.face {
            [0, 1, 0] => Direction::Up,
            [0, -1, 0] => Direction::Down,
            [0, 0, -1] => Direction::North,
            [0, 0, 1] => Direction::South,
            [-1, 0, 0] => Direction::West,
            _ => Direction::East,
        };
        let player = self.use_context();
        self.with_block_use_level(|b, level, _| {
            let mut level = DryLevel::new(level);
            let hit = horizon_core::geometry::BlockHit {
                block: horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
                direction: face,
                location: horizon_core::geometry::Vec3::new(
                    hit.point[0],
                    hit.point[1],
                    hit.point[2],
                ),
                inside: hit.inside,
            };
            let mut ticks = horizon_world::level::Ticks::default();
            let result = if without_item {
                use horizon_world::level::LevelAccess as _;
                let state = level.block_state(hit.block);
                b.use_without_item(&mut level, &mut ticks, state, hit, player)
            } else {
                b.use_item_on(&mut level, &mut ticks, hit, player, held.map(|s| &*s.item))
            };
            (result, (level.changed.into_iter().collect(), level.sounds))
        })
    }

    /// MultiPlayerGameMode.useItemOn inside startPrediction, for a block item:
    /// the blocks `BlockItem.place` set at `pos` are placed on this client's
    /// level at once, so the next click of a fast bridge aims at them rather
    /// than past them (the server would refuse it). Its place sound is the
    /// placer's own (`playSound(player, ...)`: the server sends it to the
    /// others only).
    fn predict_place(&mut self, (pos, changes): &DryPlace, seq: i32) {
        let ticks = self.ticks;
        if let Some(r) = self.remote.as_mut() {
            // set_block plays the place sound of the block at `placing`.
            r.placing = Some(([pos.x, pos.y, pos.z], ticks));
        }
        self.predict_blocks(changes, seq);
    }
    /// The blocks a use changed, set on this client's level and kept as
    /// predicted under the use's sequence until the server acknowledges it
    /// (`BlockStatePredictionHandler.retainKnownServerState`): a use the
    /// server refused goes back.
    pub(super) fn predict_blocks(&mut self, changes: &DryChanges, seq: i32) {
        for &(p, state) in changes {
            let pos = [p.x, p.y, p.z];
            let before = self.world.block(p.x, p.y, p.z);
            let here = self.player_position();
            if let Some(r) = self.remote.as_mut() {
                r.predictions
                    .entry(pos)
                    .and_modify(|e| e.sequence = seq)
                    .or_insert(Prediction {
                        sequence: seq,
                        state: before.0 as i32,
                        player_pos: here,
                    });
            }
            self.set_block(pos, state.0 as i32, true);
        }
    }

    /// Run BlockItem.place on a disposable level: where it placed and the
    /// blocks it set. The real level supplies collision shapes and
    /// building-blocking entities; random draws, scheduled ticks and events
    /// stay in the preview.
    pub(super) fn remote_can_place(
        &mut self,
        hit: &super::Hit,
        stack: &ItemStack,
    ) -> Option<DryPlace> {
        if !self.may_build() {
            return None;
        }
        let ctx = horizon_world::level::PlaceContext {
            clicked_pos: horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
            face: horizon_core::geometry::Direction::ALL[face_direction(hit.face) as usize],
            hit: Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
            inside: hit.face == [0; 3],
            yaw: self.yaw,
            pitch: self.pitch,
            sneaking: self.remote_shift_down(),
            item: stack.item.to_string(),
            can_use_game_master_blocks: self.can_use_game_master_blocks(),
        };
        self.with_level(|b, level, _| {
            let mut level = DryLevel::new(level);
            match b.place_block(
                &mut level,
                &mut horizon_world::level::Ticks::default(),
                &ctx,
            ) {
                horizon_world::level::PlaceOutcome::Placed { pos, .. } => {
                    Some((pos, level.changed.into_iter().collect()))
                }
                horizon_world::level::PlaceOutcome::Failed => None,
            }
        })
    }

    /// `MultiPlayerGameMode.useItem`'s local `ItemStack.use` for a bucket:
    /// `BucketItem.use` picks its own target and runs `pickupBlock` or
    /// `emptyContents` on this client's level inside `startPrediction`, so
    /// its success (which decides `itemUsed`) and the blocks it changes are
    /// the client's own. A fill into a waterloggable block changes nothing
    /// here: `SimpleWaterloggedBlock.placeLiquid` tests `isClientSide` and
    /// leaves the block update to the server.
    fn remote_use_bucket(
        &mut self,
        stack: &ItemStack,
    ) -> (bool, Option<DryChanges>, Vec<DrySound>) {
        // `BucketItem.use`: `level.mayInteract` and `mayUseItemAt`.
        // `Player.mayUseItemAt` tests the clicked block against `can_place_on`.
        if !self.may_build() {
            let aim = if &*stack.item == "minecraft:bucket" {
                self.bucket_hit()
            } else {
                self.pick()
            };
            if !aim.is_some_and(|hit| self.can_place_on_in_adventure(hit.pos)) {
                return (false, None, Vec::new());
            }
        }
        if &*stack.item == "minecraft:bucket" {
            let Some(hit) = self.bucket_hit() else {
                return (false, None, Vec::new());
            };
            let pos = horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
            let creative = self.creative;
            return self.with_level(|b, level, ticks| {
                let mut level = DryLevel::new(level);
                let picked = b.bucket_pickup(&mut level, ticks, pos, creative).is_some();
                (
                    picked,
                    Some(level.changed.into_iter().collect()),
                    std::mem::take(&mut level.sounds),
                )
            });
        }
        let Some((content, sound, _)) = self.predicted_bucket_contents(stack) else {
            return (false, None, Vec::new());
        };
        let Some(hit) = self.pick() else {
            return (false, None, Vec::new());
        };
        let target = self.bucket_target(content, &hit);
        let pos = horizon_core::position::BlockPos::new(target[0], target[1], target[2]);
        let [x, y, z] = hit.beside();
        let beside = Some(horizon_core::position::BlockPos::new(x, y, z));
        let sneaking = self.remote_shift_down();
        let creative = self.creative;
        self.with_level(|b, level, ticks| {
            let mut level = DryLevel::new(level);
            let emptied = b.bucket_empty(
                &mut level, ticks, pos, beside, content, sound, sneaking, creative,
            );
            // Water poured into a liquid container (where it went, which may
            // be the block beside a refusing one) fills it server-side only.
            let client_change = !(content.is_water()
                && emptied.is_some_and(|at| {
                    use horizon_world::level::LevelAccess as _;
                    b.is_liquid_container(level.block_state(at))
                }));
            (
                emptied.is_some(),
                client_change.then(|| level.changed.into_iter().collect()),
                std::mem::take(&mut level.sounds),
            )
        })
    }

    /// Whether `held`'s `Item.use` needs a target this client sees none of
    /// (not SUCCESS): BucketItem.use (a fluid source or powder snow to pick
    /// up through `ClipContext.Fluid.SOURCE_ONLY`, a block to empty a full
    /// one against, `mayUseItemAt`), BottleItem.use (water to fill from) and
    /// BoatItem.use (a block or fluid surface with room for the boat).
    fn use_finds_no_target(&self, held: &ItemStack) -> bool {
        let id = &*held.item;
        let facts = |pos: [i32; 3]| {
            self.res
                .facts()
                .get(self.world.block(pos[0], pos[1], pos[2]))
        };
        let source = |want: Option<Fluid>| {
            let fluid_hit = self.pick_fluid();
            // pick_fluid steps through waterlogged blocks, but their water is
            // a source the SOURCE_ONLY ray stops at (and a BucketPickup).
            if let Some(hit) = self.pick()
                && facts(hit.pos).waterlogged
            {
                let eye = self.aim().map_or(hit.point, |(eye, _)| eye);
                let d = |p: [f64; 3]| (0..3).map(|a| (p[a] - eye[a]).powi(2)).sum::<f64>();
                if fluid_hit
                    .as_ref()
                    .is_none_or(|fluid| d(fluid.point) >= d(hit.point))
                {
                    return want.is_none_or(|want| want == Fluid::Water);
                }
            }
            fluid_hit.is_some_and(|hit| {
                let f = facts(hit.pos);
                let fluid = f.fluid != Fluid::None && f.fluid_level == 8 && !f.fluid_falling;
                match want {
                    Some(want) => fluid && f.fluid == want,
                    None => fluid || self.is_powder_snow(hit.pos),
                }
            })
        };
        // SolidBucketItem only has useOn: its use is Item.use's PASS.
        if id == "minecraft:powder_snow_bucket" {
            return true;
        }
        if id == "minecraft:bucket" {
            return !self.may_build() || !source(None);
        }
        if horizon_client::items::bucket_contents(id).is_some() {
            return !self.may_build() || self.pick().is_none();
        }
        if id == "minecraft:glass_bottle" {
            // A dragon's breath cloud never counts here: its owner is not
            // synced, so this client's BottleItem.use finds none and PASSes;
            // the packet still goes and the server fills the bottle.
            return !source(Some(Fluid::Water));
        }
        if super::boats::any_boat_wood(id).is_some() {
            return self.boat_to_place(held).is_none();
        }
        false
    }
    /// `Item.use` runs on the client too (MultiPlayerGameMode.useItem): an
    /// item held in use until the button lets go starts being used here, so
    /// that letting go sends RELEASE_USE_ITEM (Minecraft.handleKeybinds) and
    /// the server ends the use: a bow or a trident is let go, a crossbow's
    /// charge ends, a shield is lowered, a spyglass stops scoping. Their
    /// sounds, loaded arrows and thrown tridents come from the server.
    /// False when the use FAILs here: a bow or an uncharged crossbow with
    /// nothing to shoot.
    fn remote_start_held_use(&mut self, held: &ItemStack) -> bool {
        if self.using.is_some() {
            return true;
        }
        let bundle = self.items().get(&held.item).is_some_and(|i| i.bundle);
        let start = match &*held.item {
            // BundleItem.use: always starts; the server drops the contents.
            _ if bundle => true,
            // BowItem.use: with an arrow, or in creative.
            "minecraft:bow" => self.get_projectile(held).is_some() || self.creative,
            // CrossbowItem.use: charging when not charged and with ammunition;
            // a charged one shoots on the server.
            "minecraft:crossbow" => {
                if held.data().is_some_and(|d| !d.charged.is_empty()) {
                    return true;
                }
                self.get_projectile(held).is_some()
            }
            // TridentItem.use: not about to break, Riptide only in water or rain.
            "minecraft:trident" => {
                // FAIL: no dip, and the off hand gets its turn.
                return self.use_trident(held);
            }
            // SpyglassItem.use (its sound is the server's).
            "minecraft:spyglass" => true,
            // InstrumentItem.use: the horn plays here too (`playSound(player, ...)`
            // skips the player's own client), the server's copy tells the others.
            "minecraft:goat_horn" => {
                self.remote_use_instrument(held);
                return true;
            }
            // EnderEyeItem.use: startUsingItem unless an End portal frame is
            // aimed at, with or without a stronghold to throw the eye at.
            "minecraft:ender_eye" => !self.aims_at_end_portal_frame(),
            // Item.use: a kinetic weapon starts being used.
            id if horizon_client::spear::Kinetic::of(id).is_some() => true,
            // ShieldItem.use: anything that blocks attacks, off cooldown.
            // Item.use: a consumable or `blocks_attacks` the server's patch
            // adds starts the use (Consumable.startConsuming, startUsingItem).
            _ => {
                if !self.raise_shield(self.held_slot()) && self.patched_held_use(held) {
                    self.start_using_item();
                }
                return true;
            }
        };
        if start {
            self.start_using_item();
        }
        start || !matches!(&*held.item, "minecraft:bow" | "minecraft:crossbow")
    }
    /// `InstrumentItem.use` on this client: unless the horn is on cooldown,
    /// it is held up, plays its sound at the player and starts its cooldown.
    fn remote_use_instrument(&mut self, held: &ItemStack) {
        let Some(instrument) = self.res.items().instrument(held) else {
            return;
        };
        let group = horizon_client::cooldowns::group(held, None);
        if self.cooldowns.is_on_cooldown(&group) {
            return;
        }
        self.start_using_item();
        self.play_player_bound_sound(&instrument.sound, 2, instrument.range / 16.0, 1.0, true);
        if instrument.use_duration > 0.0 {
            self.add_cooldown(group, instrument.use_ticks());
        }
    }
    /// Whether `stack` stays in use until the button lets go (bows,
    /// crossbows, tridents, spyglasses, shields, eyes of ender, whose
    /// `getUseDuration` 0 never counts down to the end) or the server ends
    /// it (a mod's item): its use only counts ticks here.
    fn held_until_released(&self, stack: &ItemStack) -> bool {
        if self.mod_use(stack) {
            return true;
        }
        matches!(
            &*stack.item,
            "minecraft:bow"
                | "minecraft:crossbow"
                | "minecraft:trident"
                | "minecraft:spyglass"
                | "minecraft:ender_eye"
        ) || horizon_client::spear::Kinetic::of(&stack.item).is_some()
            || self.blocks_attacks(stack).is_some()
            || self.patched_held_use(stack)
    }
    /// Whether the server's patch gives `stack`, an item with no consumable
    /// of its own, a `consumable` with a use time or `blocks_attacks` (a
    /// sword made to block): `Item.use` starts a use of it
    /// (`Consumable.startConsuming`, `startUsingItem`) that lasts until the
    /// button lets go or the server stops it, as the client's
    /// `LivingEntity.updateUsingItem` never completes one.
    fn patched_held_use(&self, stack: &ItemStack) -> bool {
        let (ticks, blocks) = horizon_client::item_sync::patched_use(stack);
        ticks.map_or(blocks, |ticks| ticks > 0)
            && self
                .items()
                .get(&stack.item)
                .is_none_or(|i| i.consumable.is_none())
    }

    /// The rest of Minecraft.startUseItem once the main hand's target passed:
    /// the main hand's item, then the off hand's turn (its block, its item).
    /// The server does each packet's part with that packet's hand only.
    fn remote_use_hands(&mut self, attempt: mod_api::InteractionAttempt) {
        // Nothing in the crosshair: an empty hand's click is heard, by mods only.
        let miss = mod_api::__private::listened(mod_api::HookId::RightClickEmpty)
            && !self.entity_targeted()
            && self.pick().is_none();
        if miss && self.held().is_none() {
            self.native_right_click_empty(Hand::Main);
        }
        // A successful Item.use always drops the hand (LocalPlayer.itemUsed).
        if self.remote_use_item(Hand::Main) {
            self.heights.item_used(Hand::Main);
            return;
        }
        // The off hand's turn: the event again, then the entity or the block.
        if let Some(swing) = self
            .native_interaction(InteractionKey::Use, Hand::Off, attempt)
            .canceled_swing()
        {
            if swing {
                self.swing_hand(Hand::Off, SwingAnimation::DEFAULT);
            }
            return;
        }
        let off_target = if self.remote_use_targets_entity() {
            self.remote_use_entity(Hand::Off)
        } else {
            self.remote_use_block(Hand::Off)
        };
        if off_target {
            return;
        }
        if miss && self.offhand.is_none() {
            self.native_right_click_empty(Hand::Off);
        }
        if self.remote_use_item(Hand::Off) {
            self.heights.item_used(Hand::Off);
        }
    }
    /// `LivingEntity.updatingUsingItem` on a server client: an item held in
    /// use (`held_until_released`), and a consumable's eating animation and
    /// its crumbs until the server's USE_ITEM_COMPLETE. The server sends the
    /// stack, its hunger and the sounds.
    fn remote_tick_using(&mut self) {
        let Some((slot, ticks)) = self.using else {
            // A mod's use the game reset (the server stopping the use...).
            self.end_mod_use();
            return;
        };
        let stack = self.stack_in(slot).cloned();
        // The use stops when the hand no longer holds the item it began with
        // (ItemStack.isSameItem).
        let item = stack.as_ref().map(|s| Arc::clone(&s.item));
        if let Some(r) = self.remote.as_mut() {
            if ticks == 0 {
                r.use_item = item;
            } else if r.use_item != item {
                self.using = None;
                self.end_mod_use();
                return;
            }
        }
        // Another hotbar slot picked: LivingEntity.updatingUsingItem sees a
        // different stack in the hand. The off hand keeps its item.
        let in_hand = slot == self.selected || slot == super::shield::OFFHAND_SLOT;
        // A mod's item: held until the button lets go, counted for the mod.
        if self.mod_use_tick(slot, ticks, !self.right || !in_hand) {
            return;
        }
        // updatingUsingItem runs here too: a use a mod's tick ran out stays
        // frozen until let go (RELEASE_USE_ITEM) or the server finishes it.
        if let Some(s) = stack.as_ref().filter(|_| in_hand)
            && !self.use_item_ticked(s, ticks)
        {
            return;
        }
        // BrushItem.onUseTick: with no block aimed the use ends (the server
        // ends it too); a stroke shows its dust. Its sounds are the server's.
        if stack
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:brush")
        {
            let elapsed = ticks + 1;
            let hit = self.pick().filter(|_| in_hand);
            if let Some(hit) = &hit
                && elapsed % 10 == 5
            {
                self.brush_dust(hit, slot == super::shield::OFFHAND_SLOT);
            }
            if hit.is_none() && in_hand {
                self.send_action(6);
            }
            self.using = hit.and(Some((slot, elapsed))).filter(|_| elapsed < 200);
            return;
        }
        // A bundle's 200-tick use ends with the server's entity event 9, not here.
        if stack
            .as_ref()
            .is_some_and(|s| self.items().get(&s.item).is_some_and(|i| i.bundle))
        {
            self.using = in_hand.then_some((slot, ticks + 1));
            return;
        }
        // A drawn bow, a charging crossbow, a raised shield...: held until
        // the button lets go.
        if let Some(s) = stack.as_ref().filter(|s| self.held_until_released(s)) {
            if in_hand && (self.mod_use(s) || self.blocks_attacks(s).is_some()) {
                self.item_on_use_tick(s, ticks);
            }
            self.using = in_hand.then_some((slot, ticks + 1));
            return;
        }
        // A goat horn is held up for its instrument's use duration.
        if let Some(duration) = stack
            .as_ref()
            .and_then(|s| self.res.items().instrument(s))
            .map(|i| i.use_ticks().max(0) as u32)
        {
            self.using = (in_hand && ticks + 1 < duration).then_some((slot, ticks + 1));
            return;
        }
        let consumable = stack.as_ref().and_then(|s| self.consumable(s));
        let (Some(stack), Some(consumable)) = (stack, consumable) else {
            self.using = None;
            return;
        };
        // Button release belongs to the next handleKeybinds phase, which
        // sends RELEASE_USE_ITEM. Do not silently clear a use begun by a
        // quick press/release between ticks.
        if !in_hand {
            self.using = None;
            return;
        }
        // Consumable.startConsuming eats one with no use time at once, without
        // a use (nor a USE_ITEM_COMPLETE).
        if consumable.ticks() <= 0 {
            self.using = None;
            return;
        }
        // Consumable.shouldEmitParticlesAndSounds, as in tick_used_item. The
        // countdown goes on past zero (LivingEntity.updateUsingItem completes
        // only on the server) until the server's USE_ITEM_COMPLETE.
        // getUseItemRemainingTicks: the duration this use started with.
        let remaining = self.use_duration - ticks as i32;
        if consumable.should_emit(remaining) {
            // `emit_consume` plays the eat sound and spawns the bites (when the
            // item has consume particles), as `tick_used_item` does on the
            // player's own server.
            self.emit_consume(&consumable, &stack, 5);
        }
        self.item_on_use_tick(&stack, ticks);
        self.using = Some((slot, ticks + 1));
    }
    /// `Player.handleEntityEvent(9)`: `completeUsingItem` on the client. A consumable's last crumbs, and FoodProperties.onConsume,
    /// which runs on the client too (Consumable.onConsume calls its
    /// ConsumableListeners on both sides): the client's FoodData eats at once,
    /// so a held right click finds `Player.canEat` false instead of starting
    /// another bite before the server's ClientboundSetHealth arrives.
    fn remote_complete_using(&mut self) {
        let Some((slot, _)) = self.using.take() else {
            return;
        };
        let Some(stack) = self.stack_in(slot).cloned() else {
            return;
        };
        let Some(info) = self.items().get(&stack.item).cloned() else {
            return;
        };
        // Consumable.onConsume's emitParticlesAndSounds(16): the last crumbs
        // and bite, which the server plays for the others only.
        if let Some(consumable) = self.consumable(&stack) {
            self.emit_consume(&consumable, &stack, 16);
        }
        if let Some((nutrition, saturation)) = info.food
            && let Some(r) = self.remote.as_mut()
        {
            // FoodData.eat(nutrition, saturation), as finish_consuming does;
            // the next ClientboundSetHealth replaces it.
            let (_, food, sat) = &mut r.health;
            *food = (*food + nutrition as i32).clamp(0, 20);
            *sat = (*sat + saturation).clamp(0.0, *food as f32);
            self.apply_remote_health();
        }
        // applyAfterUseComponentSideEffects, then LivingEntityUseItemEvent.Finish.
        self.remote_use_item_finished(slot, &stack, &info);
    }
    pub(super) fn request_remote_mining_stop(&mut self) {
        if let Some(r) = self.remote.as_mut() {
            r.stop_mining = true;
        }
    }

    fn remote_handle_input(&mut self) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut input = std::mem::take(&mut r.input);
        let stop = std::mem::take(&mut r.stop_mining);
        // Minecraft.tick runs handleKeybinds only while `gui.screen() == null`: a
        // book, a sign editor, a dialog or a prompt over the world (each a
        // `Screen` in vanilla) stops the attack and use mappings from acting,
        // and `KeyMapping.releaseAll` let go of the buttons when it opened
        // (`track_toggle_screens`), so a dig held into a book ends (#9729).
        let ready = r.loaded_sent
            && self.screen == Screen::Playing
            && self.sleeping.is_none()
            && !self.screen_over_playing();
        // MultiPlayerGameMode.tick, first in Minecraft.tick, screen or not: the slot
        // chosen since the last tick (the wheel, the last tick's number keys, the
        // server's SetHeldSlot) goes out before anything else of this tick.
        self.remote_send_carried();
        if stop || !ready {
            self.stop_remote_mining();
        }
        if !ready {
            // A screen's opening released the keys (`KeyMapping.releaseAll`).
            self.hotbar_presses.clear();
            return;
        }
        let (attack, use_item) = (self.left, self.right);
        input.tick(&mut RemoteInputWorld(self), attack, use_item);
        self.remote_queued_keys();
        if let Some(r) = self.remote.as_mut() {
            input.merge_queued(std::mem::take(&mut r.input));
            r.input = input;
        }
    }

    /// `MultiPlayerGameMode.ensureHasSentCarriedItem`: the server hears of a
    /// new selected slot before anything the slot acts with.
    fn remote_send_carried(&self) {
        if let Some(r) = &self.remote
            && r.carried.get() != Some(self.selected)
        {
            r.carried.set(Some(self.selected));
            self.send_play(S::SetCarriedItem(play::SetCarriedItem {
                slot: self.selected as i16,
            }));
        }
    }

    fn remote_queued_keys(&mut self) {
        // handleKeybinds' keybind polls: the queued presses act now, in tick
        // order, so no packet goes out between CLIENT_TICK_END and a pong.
        let pending = self
            .remote
            .as_mut()
            .map(|r| std::mem::take(&mut r.pending_keys))
            .unwrap_or_default();
        for key in pending {
            match key {
                KeyCode::KeyF if !self.spectator() => self.remote_swap_hands(),
                KeyCode::KeyQ if !self.spectator() => {
                    // Minecraft.hasControlDown: the physical keys, so a toggled
                    // sprint on Left Control does not make Q drop the stack.
                    let all = self.has_control_down() || self.input_takes_stack_drop();
                    self.remote_drop(all);
                }
                KeyCode::KeyE
                    if !self.in_portal_transition()
                        && (self.riding_mount_with_menu() || self.riding_chest_boat()) =>
                {
                    let id = self.remote.as_ref().map_or(0, |r| r.entity_id);
                    self.send(S::PlayerCommand(play::PlayerCommand {
                        id,
                        action: 5,
                        data: 0,
                    }));
                }
                _ => {}
            }
        }
        if self.clicks.miss_time > 0 {
            self.clicks.miss_time -= 1;
        }
    }

    /// `MultiPlayerGameMode.attack` runs `Player.attack` on this client too: its
    /// `isSweepAttack` for a damaging hit on `target` (`living`: a
    /// `LivingEntity`, which may be hit critically), heard by native mods on
    /// both sides. Only the server sweeps.
    fn remote_sweep_event(&mut self, target: &str, living: bool) {
        if mod_api::__private::mods().is_empty() {
            return;
        }
        let speed = self
            .own_attribute("minecraft:attack_speed")
            .unwrap_or_else(|| self.main_hand_attack_speed());
        let scale = self.hand_strength.scale(0.5, speed);
        let spin = self.player.as_ref().map_or(0.0, |p| p.auto_spin_attack_dmg);
        let damage = if spin > 0.0 {
            spin
        } else {
            self.own_attribute("minecraft:attack_damage")
                .map_or(1.0, |v| v as f32)
        };
        let magic = scale * (self.enchanted_damage(target, damage, None) - damage);
        if damage <= 0.0 && magic <= 0.0 {
            return;
        }
        let crit = living && self.critical_attack(scale);
        self.sweep_decision(scale, crit, target);
    }
    /// The entity type of the boat, minecart, armor stand or End crystal with
    /// server id `id` that an attack reaches `Player.attack`'s sweep check on,
    /// and whether it is living. A painting or item frame is not: on this
    /// client `skipAttackInteraction` hurts it and stops the attack there.
    fn remote_attacked_object(&self, id: i32) -> Option<(String, bool)> {
        let id = id as u32;
        if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            return Some((b.entity_type(), false));
        }
        if let Some(c) = self.carts.iter().find(|c| c.id == id) {
            return Some((c.kind.id().to_owned(), false));
        }
        if self.armor_stands.iter().any(|a| a.id == id) {
            return Some(("minecraft:armor_stand".to_owned(), true));
        }
        self.crystals
            .iter()
            .any(|c| c.id == id)
            .then(|| ("minecraft:end_crystal".to_owned(), false))
    }
    /// `MultiPlayerGameMode.attack` runs `Player.attack` on this client too.
    /// For a mob `hurtOrSimulate` fails here (`Entity.hurtClient`), so the
    /// hit has no attacker side; but `RemotePlayer.hurtClient` is true, so a
    /// hit on another player with extra knockback runs the attacker's side of
    /// `causeExtraKnockback`: the 0.6 horizontal speed cut and the sprint
    /// stop. Without it the next move outruns the server's player.
    fn remote_player_attack(&mut self, mob: usize) {
        // Java dispatches this through the target's RemotePlayer type. The
        // level_players map is profile bookkeeping and can lag the entity;
        // mobs must stay out because their hurtClient returns false.
        if self.mobs[mob].kind.id != "minecraft:player" {
            return;
        }
        // `getAttackStrengthScale(0.5)`: here the ticker is `hand_strength`'s
        // (`attack_ticks` only counts in the local world), and the attack
        // speed is the player's synced attribute, which a server may change.
        let speed = self
            .own_attribute("minecraft:attack_speed")
            .unwrap_or_else(|| self.main_hand_attack_speed());
        let strength = self.hand_strength.scale(0.5, speed);
        // Player.attack's damage: a riptide spin's, or this client's own
        // ATTACK_DAMAGE, which the held item and effects don't change here.
        let spin = self.player.as_ref().map_or(0.0, |p| p.auto_spin_attack_dmg);
        let damage = if spin > 0.0 {
            spin
        } else {
            self.own_attribute("minecraft:attack_damage")
                .map_or(1.0, |v| v as f32)
        };
        let knockback = self
            .own_attribute("minecraft:attack_knockback")
            .map_or(0.0, |v| v as f32);
        let sprinting = self.player.as_ref().is_some_and(|p| p.sprinting);
        // attackVisualEffects: a RemotePlayer's hurtClient is true, so a
        // critical hit (full strength, canCriticalAttack) with damage left
        // after baseDamageScaleFactor runs LocalPlayer.crit, this client's
        // own CRIT emitter beside the one the server's Animate brings.
        // magicBoost is 0 on a client, so no ENCHANTED_HIT one.
        if damage * (0.2 + strength * strength * 0.8) > 0.0 && self.critical_attack(strength) {
            use horizon_client::particles::{ParticleOptions, ParticleType};
            let m = &self.mobs[mob];
            self.particles.tracking_emitter(
                ParticleOptions::simple(ParticleType::Crit),
                TrackingTarget::Entity(m.id as i32),
                mob_tracking_body(m),
            );
        }
        if !horizon_client::attack_strength::attacker_slows_after_hit(
            damage, knockback, strength, sprinting,
        ) {
            return;
        }
        if let Some(p) = self.player.as_mut() {
            let slowdown = horizon_client::attack_strength::ATTACKER_SLOWDOWN;
            p.velocity = Vec3::new(
                p.velocity.x * slowdown,
                p.velocity.y,
                p.velocity.z * slowdown,
            );
        }
        self.stop_sprint_after_extra_knockback();
    }
    /// The latency the server's player list gives this player
    /// (`PlayerInfo.getLatency`), none before the server lists it. A new entry reads 0 until the
    /// server's first latency update (every 600 ticks).
    pub(in crate::game) fn own_latency(&self) -> Option<i32> {
        let r = self.remote.as_ref()?;
        r.tab_players.get(&r.profile).map(|me| me.latency)
    }

    /// An attribute's value on this client's player, if its map has the attribute.
    fn own_attribute(&mut self, id: &str) -> Option<f64> {
        let p = self.player.as_mut()?;
        let key = p.attributes.key(id)?;
        p.attributes.lookup(Some(&key))?;
        p.attributes
            .value(Some(key), horizon_world::attribute_map::ValueKind::Computed)
            .ok()
    }

    /// Keys with a multiplayer meaning; returns true when consumed.
    pub(super) fn remote_key(&mut self, key: KeyCode) -> Result<bool> {
        if self.screen != Screen::Playing {
            if (key == KeyCode::KeyE || key == KeyCode::Escape) && self.screen == Screen::Inventory
            {
                self.remote_close_container();
                self.set_screen(Screen::Playing);
                self.capture(true);
                return Ok(true);
            }
            return Ok(false);
        }
        match key {
            KeyCode::KeyT | KeyCode::Slash | KeyCode::Enter => {
                // ChatScreen, over the server's command tree.
                self.open_chat_method(if key == KeyCode::Slash { "/" } else { "" });
                // `Minecraft.handleKeybinds` opens it in the call that then
                // releases an active use (Gui.handleKeybinds runs before the
                // isUsingItem branch).
                self.release_use_for_keybind_screen();
                Ok(true)
            }
            KeyCode::KeyQ => {
                if self.spectator() {
                    return Ok(true);
                }
                if let Some(r) = self.remote.as_mut() {
                    r.pending_keys.push(key);
                }
                Ok(true)
            }
            // Open the ridden vehicle's own screen (`HasCustomInventoryScreen`)
            // instead of the player's inventory: riding a horse, llama or
            // chest boat, the inventory key sends
            // ServerboundPlayerCommandPacket OPEN_INVENTORY.
            KeyCode::KeyE if self.riding_mount_with_menu() || self.riding_chest_boat() => {
                if self.in_portal_transition() {
                    return Ok(true);
                }
                if let Some(r) = self.remote.as_mut() {
                    r.pending_keys.push(key);
                }
                Ok(true)
            }
            KeyCode::KeyE => {
                if self.in_portal_transition() {
                    return Ok(true);
                }
                // The inventory opens without telling the server.
                if self.tutorial.open_inventory() {
                    self.persist_tutorial_step();
                }
                self.recipe_ui.reset_screen();
                if self.creative {
                    self.set_screen(Screen::Creative);
                    self.refresh_creative_tabs();
                    self.creative_screen.reopen();
                    self.refresh_saved_hotbars();
                } else {
                    self.set_screen(Screen::Inventory);
                }
                self.craft_size = 2;
                self.capture(false);
                // `Minecraft.handleKeybinds` opens the inventory in the same
                // call that then releases an active use.
                self.release_use_for_keybind_screen();
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    /// `Minecraft.handleKeybinds`: a key that opens a screen runs
    /// `Gui.setScreen`, whose `KeyMapping.releaseAll` clears the use key, and
    /// the same invocation then releases an active use
    /// (`player.isUsingItem() && !keyUse.isDown()` → `releaseUsingItem`).
    /// Only the screens opened from those keybinds do this (inventory, chat,
    /// advancements); the pause menu, server containers, mount inventories and
    /// dialogs release on the first tick after closing instead, as vanilla
    /// does.
    pub(super) fn release_use_for_keybind_screen(&mut self) {
        // Only a server's client holds a use to release.
        if self.remote.is_none() {
            return;
        }
        // `KeyMapping.releaseAll` clears key.use; without this the button still
        // reads held and a use would start again on the first tick back.
        self.right = false;
        horizon_client::interaction_input::release_on_screen_open(&mut RemoteInputWorld(self));
    }
    /// `MultiPlayerGameMode.releaseUsingItem`: sends `RELEASE_USE_ITEM` and
    /// runs the client half of `Item.releaseUsing` (a Riptide launch moves the
    /// player itself). Vanilla reaches it both when the use key is let go and,
    /// in the same tick, when a keybind opens a screen.
    fn release_remote_use(&mut self) {
        self.send_action(6);
        // a mod's Stop may skip the item's release, and a mod's own release follows.
        let hand = self.used_hand();
        if let Some((slot, ticks)) = self.using.take()
            && let Some(hand) = hand
            && let Some(stack) = self.stack_in(slot).cloned()
        {
            self.with_hand(hand, |g| {
                if !g.use_item_stopped(&stack, ticks) {
                    return;
                }
                if &*stack.item == "minecraft:trident" {
                    g.release_trident(g.ticks_held(&stack, ticks));
                }
                g.mod_release_using(&stack, ticks);
            });
        }
    }
    /// ClientPacketListener.sendChat / sendCommand, signed with a chat
    /// session (game/network/remote_chat.rs). The Chat option's Hidden and System
    /// levels keep chat out and Hidden keeps the commands out (ChatAbilities).
    pub(super) fn send_signed_chat_input(&mut self, message: &str) {
        let message = message.trim();
        if message.is_empty() || !self.chat_abilities().allows(message) {
            return;
        }
        if let Some(command) = message.strip_prefix('/') {
            if self.studio_command(command) {
                return;
            }
            self.send_command(command);
        } else {
            self.send_signed_chat(message);
        }
    }
    /// ChatScreen.handleChatInput fires the client event for chat, not commands.
    pub(super) fn send_chat_from_screen(&mut self, message: &str) {
        if message.starts_with('/') || !mod_api::__private::listened(mod_api::HookId::ClientChat) {
            self.send_signed_chat_input(message);
        } else if let Some(uuid) = self.remote.as_ref().map(|r| r.profile) {
            let id = self.own_entity_id();
            let sent = chat_after_event(message, |outgoing, cancel| {
                self.native_client_chat(id, uuid, message, outgoing, cancel);
            });
            if let Some(outgoing) = sent {
                self.send_signed_chat(&outgoing);
            }
        }
    }
    /// MultiPlayerGameMode.dropItem (key.drop): predict the removal, then send
    /// the action; swing only if a nonempty stack was removed.
    pub(super) fn remote_drop(&mut self, all: bool) {
        // ensureHasSentCarriedItem: the server drops from the slot just chosen.
        self.remote_send_carried();
        let dropped =
            horizon_client::items::take_held_drop(&mut self.inventory[self.selected], all, false);
        self.send_action(if all { 4 } else { 5 });
        if dropped.is_some() {
            self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        }
    }
    /// Minecraft.handleKeybinds, key.swapOffhand: the server swaps the hands
    /// (ServerboundPlayerActionPacket SWAP_ITEM_WITH_OFFHAND at BlockPos.ZERO, facing down) and
    /// sends both slots back; nothing is predicted.
    pub(super) fn remote_swap_hands(&mut self) {
        self.send_action(7);
        // Equal stacks swap with no slot update to wait for.
        let (main, off) = (self.stack_in(self.selected), self.offhand.as_ref());
        let moves = !super::hand::matches_for_swap(main, off);
        if let Some(r) = self.remote.as_mut() {
            r.swap_answer = [moves; 2];
        }
    }
    /// The server set inventory `slot` (`shield::OFFHAND_SLOT` for the off
    /// hand), `changed` from the stack this client held there. The stack
    /// that answers a swap (`setItemInHand`) is another object, which the
    /// hand shows after the swap animation (`hand_stack_replaced`); other
    /// updates of a hand's slot count as the held stack changing in place,
    /// as this client doesn't predict every use the way vanilla's does
    /// (a thrown snowball's stack shrinks only when the server says so).
    fn remote_hand_slot_set(&mut self, slot: usize, changed: bool) {
        let hand = if slot == super::shield::OFFHAND_SLOT {
            1
        } else if slot == self.selected {
            0
        } else {
            return;
        };
        let answer = self
            .remote
            .as_mut()
            .is_some_and(|r| std::mem::take(&mut r.swap_answer[hand]));
        if answer && changed {
            self.hand_stack_replaced(slot);
        }
    }
    /// A container click: the server answers with the authoritative slots.
    pub(super) fn remote_click(&mut self, right: bool) {
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let batch = HudBatch::new(size, self.settings.gui_scale_inputs());
        let Some(internal) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else {
            return;
        };
        let input = if self.has_shift_down() {
            Input::QuickMove
        } else {
            Input::Pickup
        };
        if self.screen == Screen::Creative {
            // CreativeModeInventoryScreen.slotClicked: the Survival Inventory tab clicks the
            // player's own InventoryMenu, and its CreativeInventoryListener sends each slot that
            // changed (SetCreativeModeSlot). No ContainerClick: a server holding the carried
            // stack would give it back after the Destroy Item slot cleared it (#3307).
            self.click_open_inventory(Some(internal), i32::from(right), input);
            return;
        }
        self.remote_menu_ops(&[(Some(internal), i32::from(right), input)]);
    }
    /// `AbstractContainerMenu.setItem`/`initializeContents`: the state id a
    /// click answers with is the open menu's, not another window's.
    fn remote_menu_state(&mut self, container: i32, state: i32) {
        let open = self.remote_menu().0;
        if let Some(r) = self.remote.as_mut().filter(|_| container == open) {
            r.state_id = state;
        }
    }
    /// The server's open menu: its window id and menu type (0, "inventory"
    /// when none is open).
    pub(super) fn remote_menu(&self) -> (i32, String) {
        match self.remote.as_ref().and_then(|r| r.menu.as_ref().cloned()) {
            Some((id, name)) => (id, name),
            None => (0, "inventory".to_owned()),
        }
    }
    /// `MultiPlayerGameMode.handleContainerInput` for each click of `ops` on
    /// the server's open menu (a quick craft drag is several): the verified
    /// menu engine predicts them on this client's copy, then each goes out as
    /// a `ContainerClick` with the slots it changed and the carried stack
    /// after it; the server corrects anything it sees differently. A menu the
    /// engine doesn't model sends the clicks unpredicted. False when a click's
    /// slot isn't one of the menu's.
    pub(super) fn remote_menu_ops(&mut self, ops: &[(Option<usize>, i32, Input)]) -> bool {
        // CrafterScreen.slotClicked: a slot toggle goes out before the click.
        if let [(Some(slot), button, input)] = ops {
            self.crafter_slot_clicked(*slot, *button, *input);
        }
        let (container, menu) = self.remote_menu();
        let mut clicks = Vec::with_capacity(ops.len());
        for &(slot, button, input) in ops {
            let slot = match slot {
                None => -999,
                Some(internal) => match Self::menu_slot_index(&menu, internal) {
                    Some(s) => s as i16,
                    None => return false,
                },
            };
            clicks.push((slot, button, input));
        }
        // Unpredicted: nothing changed, the carried stack as it is.
        let mut sent: Vec<Predicted> = clicks
            .iter()
            .map(|&(slot, button, input)| (slot, button, input, Vec::new(), self.carried.clone()))
            .collect();
        let mut threw = false;
        // The engine's slot indices are the menu's own, as on the network.
        self.menu_session(|m, env, _| {
            let Some((predicted, dropped)) = predict_clicks(m, env, &clicks) else {
                // The server throws on it too: nothing is predicted.
                return Some(false);
            };
            sent = predicted;
            threw = dropped;
            Some(true)
        });
        self.send_clicks(container, sent);
        // LivingEntity.drop(.., Prediction.PREDICTED): the client swings its own
        // main hand; the server shows the swing only to the other players.
        if threw {
            self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        }
        true
    }
    /// A QUICK_MOVE on the server's open menu with nothing predicted: the
    /// client's slots stay as they are and every slot it moved comes back from
    /// the server (`broadcastChanges`), the answer the next step waits for.
    /// A prediction the server agrees with gets no answer at all.
    pub(super) fn remote_quick_move_unpredicted(&mut self, internal: usize) -> bool {
        let (container, menu) = self.remote_menu();
        let Some(slot) = Self::menu_slot_index(&menu, internal) else {
            return false;
        };
        let carried = self.carried.clone();
        let click = (slot as i16, 0, Input::QuickMove, Vec::new(), carried);
        self.send_clicks(container, vec![click]);
        true
    }
    /// A Shift double click on a server (`AbstractContainerScreen.mouseReleased`):
    /// a predicted QUICK_MOVE for every slot the engine moves, each sent.
    pub(super) fn remote_quick_move_all(&mut self, slot: usize, moved: &ItemStack) {
        let (container, _) = self.remote_menu();
        let mut sent: Vec<Predicted> = Vec::new();
        self.menu_session(|m, env, to_menu| {
            let clicked = to_menu(slot)?;
            let mut before = menu_items(m);
            let mut each = |m: &horizon_client::menu::Menu, t: usize| {
                let changed = changed_slots(&before, m);
                sent.push((t as i16, 0, Input::QuickMove, changed, m.carried.clone()));
                before = menu_items(m);
            };
            Some(
                m.quick_move_all_each(env, clicked, moved, &mut each)
                    .is_ok(),
            )
        });
        self.send_clicks(container, sent);
    }
    fn send_clicks(&mut self, container: i32, clicks: Vec<Predicted>) {
        // A result the engine claims to have changed is the one the server
        // agrees to (`click_agreed`), so it sends none: keep it. A result the
        // engine doesn't claim (a recipe it doesn't know) the server sends.
        if matches!(
            self.remote_menu().1.as_str(),
            "inventory" | "minecraft:crafting"
        ) && let Some(r) = self.remote.as_mut()
        {
            for (.., changed, _) in &clicks {
                if let Some((_, result)) = changed.iter().find(|(slot, _)| *slot == 0) {
                    r.craft_result = result.clone();
                }
            }
        }
        let state = self.remote.as_ref().map_or(0, |r| r.state_id);
        let hashed = |g: &Self, s: &Option<ItemStack>| g.hashed_stack(s);
        for (slot, button, input, changed, carried) in clicks {
            let changed = changed.iter().map(|(i, s)| (*i, hashed(self, s))).collect();
            let carried = hashed(self, &carried);
            self.send(S::ContainerClick(play::ContainerClick {
                container,
                state,
                slot,
                button: button as i8,
                input: input_ordinal(input),
                changed,
                carried,
            }));
        }
    }
    /// The server's open menu is the inventory or a chest-like container
    /// (chests, barrels, ender chests: `generic_9xN`; shulker boxes).
    pub(super) fn remote_storage_open(&self) -> bool {
        match self.remote.as_ref().and_then(|r| r.menu.as_ref()) {
            None => true,
            Some((_, name)) => {
                name == "inventory"
                    || name.starts_with("minecraft:generic_9x")
                    || name == "minecraft:shulker_box"
            }
        }
    }
    /// A click outside a server's window (slot -999): predicted like
    /// `MultiPlayerGameMode.handleContainerInput` (the carried stack after
    /// `doClick`; the server drops the stack and swings the hand).
    pub(super) fn remote_throw(&mut self, right: bool, input: horizon_client::menu::Input) {
        use horizon_client::menu::Input;
        let container = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map_or(0, |m| m.0);
        if input == Input::Pickup
            && let Some(c) = self.carried.as_mut()
        {
            if right && c.count > 1 {
                c.count -= 1;
            } else {
                self.carried = None;
            }
            self.swing_arm();
        }
        let carried = self.hashed_stack(&self.carried);
        let state = self.remote.as_ref().map_or(0, |r| r.state_id);
        self.send(S::ContainerClick(play::ContainerClick {
            container,
            state,
            slot: -999,
            button: i8::from(right),
            input: if input == Input::Throw { 4 } else { 0 },
            changed: Vec::new(),
            carried,
        }));
    }
    /// `CreativeModeInventoryScreen.slotClicked` outside the window: the server knows nothing of
    /// a stack picked from the creative screen, so the carried stack (an item tab's right click:
    /// one of it) is dropped through `handleCreativeModeItemDrop`, a SetCreativeModeSlot to slot
    /// -1, and the hand swings as `Player.drop` predicts.
    pub(super) fn remote_creative_drop(&mut self, right: bool) {
        let one = right && !self.creative_inventory_open();
        let Some(carried) = self.carried.as_mut() else {
            return;
        };
        let thrown = if one && carried.count > 1 {
            carried.count -= 1;
            ItemStack {
                count: 1,
                ..carried.clone()
            }
        } else {
            self.carried.take().expect("carried")
        };
        self.remote_creative_throw(thrown);
    }
    /// `MultiPlayerGameMode.handleCreativeModeItemDrop`: a stack thrown out of the creative
    /// screen goes to the server as a SetCreativeModeSlot to slot -1, which drops it from the
    /// player's hand; the client only swings (`Player.drop` on a client).
    pub(super) fn remote_creative_throw(&mut self, thrown: ItemStack) {
        if !self.creative {
            return;
        }
        self.swing_arm();
        let item = self.to_network(&Some(thrown));
        self.send(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
            slot: -1,
            item,
        }));
    }
    pub(super) fn remote_close_container(&mut self) {
        let id = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref().map(|m| m.0))
            .unwrap_or(0);
        self.send(S::ContainerClose(play::ContainerClose { container: id }));
        self.remote_close_local();
    }
    /// `MultiPlayerGameMode.handleInventoryButtonClick`: button `button` of
    /// the menu the server opened goes to it (`ContainerButtonClick`), whose
    /// menu decides what it does (`clickMenuButton`). No sound: that is the
    /// screen's. False while no such menu is open.
    pub(super) fn remote_button_click(&self, button: i32) -> bool {
        let Some(container) = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map(|(w, _)| *w)
        else {
            return false;
        };
        self.send(S::ContainerButtonClick(play::ContainerButtonClick {
            container,
            button,
        }));
        true
    }
    fn remote_reject_container(&mut self, container: i32) {
        self.remote_close_local();
        if self.screen == Screen::Inventory {
            self.set_screen(Screen::Playing);
            self.capture(true);
        }
        self.send(S::ContainerClose(play::ContainerClose { container }));
    }
    fn remote_close_local(&mut self) {
        // `Gui.setScreen` clears the focus of the screen it takes down: the
        // container's keyboard focus must not outlive it (a Tab bound to the
        // inventory moves the focus and closes the container in one press).
        self.pad_clear_focus();
        self.recipe_ui.reset_screen();
        if let Some(r) = self.remote.as_mut() {
            r.menu.set(None);
            r.set_menu_title(None);
            r.craft_result = None;
        }
        self.mount_menu_reset();
        // The client's menu has no level access: what it holds is the server's.
        self.station = None;
        // AbstractContainerMenu.removed: the server gives the grid and the
        // carried stack back to the inventory and syncs it; the copies here
        // are stale the moment the window closes.
        self.craft_grid.fill(None);
        self.carried = None;
        self.craft_size = 2;
        if let Some(key) = self.container.take() {
            // Server containers stay renderable but lose their copied slots.
            if let Some(entity) = self.world.block_entities.get_mut(&key) {
                for s in entity.slots_mut() {
                    *s = None;
                }
            }
        }
    }
    /// A server's trading window (MerchantScreen over a ClientSideMerchant).
    fn remote_open_merchant(
        &mut self,
        container: i32,
        title: String,
        text: horizon_client::text::Text,
    ) {
        if let Some(r) = self.remote.as_mut() {
            r.menu
                .set(Some((container, "minecraft:merchant".to_owned())));
            r.set_menu_title(Some(text));
        }
        let session = horizon_client::merchant::Session::new(Vec::new(), true);
        self.open_remote_merchant(session, (!title.is_empty()).then_some(title));
    }
    /// ClientboundMerchantOffersPacket.
    fn remote_merchant_offers(&mut self, p: play::MerchantOffers) {
        if self.open_menu_name(p.container) != "minecraft:merchant" {
            return;
        }
        let cost = |g: &Self, c: &play::NetCost| {
            g.to_stack(&Some(c.clone().into()))
                .map(|stack| horizon_client::merchant::ItemCost { stack })
        };
        let offers: Vec<_> = p
            .offers
            .iter()
            .filter_map(|o| {
                let mut offer = horizon_client::merchant::MerchantOffer {
                    base_cost_a: cost(self, &o.cost_a)?,
                    cost_b: match &o.cost_b {
                        Some(b) => Some(cost(self, b)?),
                        None => None,
                    },
                    result: self.to_stack(&Some(o.result.clone()))?,
                    uses: o.uses,
                    max_uses: o.max_uses,
                    reward_exp: true,
                    special_price_diff: o.special_price,
                    demand: o.demand,
                    price_multiplier: o.price_multiplier,
                    xp: o.xp,
                };
                // setToOutOfStock.
                if o.out_of_stock {
                    offer.uses = offer.max_uses;
                }
                Some(offer)
            })
            .collect();
        if let Some(m) = self.station.as_mut().and_then(|s| s.merchant.as_mut()) {
            m.session.offers = offers;
            m.session.level = p.level;
            m.session.xp = p.xp;
            m.session.villager = p.show_progress;
        }
    }
    /// ClientboundOpenScreenPacket for the menus Horizon draws.
    fn remote_open_screen(&mut self, p: play::OpenScreen) {
        // A new RecipeBookComponent owns fresh ghost slots and an overlay,
        // even when the server replaces a menu without a close packet.
        self.recipe_ui.reset_screen();
        let menu = self
            .remote
            .as_ref()
            .and_then(|r| {
                r.builtin
                    .get("minecraft:menu")?
                    .name(p.menu)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let target = self
            .remote
            .as_mut()
            .and_then(|r| r.use_target.take())
            .map(|t| (t[0], t[1], t[2]));
        // The server names a mod menu's block itself: it, not the last
        // clicked block, is the one the menu is of. Only for this window:
        // any other's is stale.
        let extra = self
            .remote
            .as_mut()
            .and_then(|r| r.menu_extra.take())
            .filter(|e| e.window == p.container);
        let target = extra
            .as_ref()
            .map_or(target, |e| e.pos.map(|p| (p[0], p[1], p[2])));
        if menu == "minecraft:lectern" {
            // LecternScreen: the book and page come in the menu's content and data.
            if let Some(r) = self.remote.as_mut() {
                r.menu.set(Some((p.container, menu)));
                r.set_menu_title(None);
            }
            self.remote_open_lectern(target.unwrap_or_default());
            return;
        }
        if let Some(kind) = Self::remote_station_kind(&menu) {
            let title = menu_title(&p.title);
            let pos = target.map_or([0, 0, 0], |t| [t.0, t.1, t.2]);
            self.remote_open_station(p.container, menu, kind, title, pos);
            return;
        }
        if menu == "minecraft:merchant" {
            // A trading window: its offers follow in a MerchantOffers packet.
            let title = plain(
                &p.title,
                &self
                    .remote
                    .as_ref()
                    .map(|r| r.lang.clone())
                    .unwrap_or_default(),
            );
            self.remote_open_merchant(p.container, title, menu_title(&p.title));
            return;
        }
        let mod_type = horizon_client::containers::mod_menu_type(&menu);
        let entity = match menu.as_str() {
            "minecraft:crafting" => None,
            "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => {
                let kind = match menu.as_str() {
                    "minecraft:blast_furnace" => FurnaceKind::BlastFurnace,
                    "minecraft:smoker" => FurnaceKind::Smoker,
                    _ => FurnaceKind::Furnace,
                };
                Some(BlockEntity::Furnace(Furnace {
                    kind,
                    items: [None, None, None],
                    lit_remaining: 0,
                    lit_total: 0,
                    cooking_timer: 0,
                    cooking_total: 0,
                    speed: 1.0,
                    recipes_used: std::collections::HashMap::new(),
                }))
            }
            // HopperMenu, and DispenserMenu (MenuType.GENERIC_3x3: dispensers
            // and droppers).
            "minecraft:hopper" => Some(BlockEntity::Chest(vec![None; 5])),
            // BrewingStandMenu: its times come in ContainerSetData.
            "minecraft:brewing_stand" => Some(BlockEntity::Brewing(Box::new(
                horizon_client::brewing::BrewingStand::new(),
            ))),
            "minecraft:generic_3x3" => Some(BlockEntity::Chest(vec![None; 9])),
            // CrafterMenu: the disabled slots and triggered come in ContainerSetData.
            "minecraft:crafter_3x3" => Some(BlockEntity::Crafter(Default::default())),
            m if m.starts_with("minecraft:generic_9x") || m == "minecraft:shulker_box" => {
                let rows = m
                    .strip_prefix("minecraft:generic_9x")
                    .and_then(|r| r.parse::<usize>().ok())
                    .unwrap_or(3);
                Some(BlockEntity::Chest(vec![None; rows * 9]))
            }
            // A content mod's menu type: its container's slots, which its
            // mod's screen places.
            _ if mod_type.is_some() => Some(BlockEntity::Mod(Box::new(
                horizon_client::containers::ModBlockEntity::new(
                    String::new(),
                    horizon_client::containers::ModBlockEntityFields {
                        items: vec![None; mod_type.map_or(0, |t| t.slots)],
                        ..Default::default()
                    },
                ),
            ))),
            _ => {
                // Unsupported menu: tell the server we closed it.
                self.remote_reject_container(p.container);
                self.add_chat_line(
                    horizon_client::text::Text::literal(format!(
                        "(Horizon cannot show {menu} screens yet)"
                    )),
                    horizon_client::chat::ChatSource::Client,
                );
                return;
            }
        };
        let title = menu_title(&p.title);
        if let Some(r) = self.remote.as_mut() {
            r.menu.set(Some((p.container, menu.clone())));
            r.set_menu_title(Some(title));
        }
        self.craft_size = if menu == "minecraft:crafting" { 3 } else { 2 };
        self.container = None;
        if let Some(entity) = entity {
            // The clicked block shows the menu when it holds that container;
            // an ender chest's or a double chest's slots are no one block's,
            // so they go under a key no block has.
            let holds = |k: &(i32, i32, i32)| {
                let name = &self.res.facts().get(self.world.block(k.0, k.1, k.2)).name;
                BlockEntity::for_block(name).is_some_and(|e| {
                    e.id() == entity.id() && e.slots().len() == entity.slots().len()
                })
            };
            let key = target.filter(holds).unwrap_or((i32::MIN, 0, 0));
            self.world.block_entities.insert(key, entity);
            self.container = Some(key);
            self.open_native_screen(key, target, p.container, &menu, extra);
            // A mod menu type shows through its mod's screen only: without one
            // (`MenuScreens.create` finds no screen) the window closes again.
            if self.mod_screen.is_none() && mod_type.is_some() {
                self.world.block_entities.remove(&key);
                self.container = None;
                if let Some(r) = self.remote.as_mut() {
                    r.menu.set(None);
                    r.set_menu_title(None);
                }
                self.send(S::ContainerClose(play::ContainerClose {
                    container: p.container,
                }));
                return;
            }
        }
        self.set_screen(Screen::Inventory);
        self.capture(false);
    }
    /// `ClientPacketListener.handleMountScreenOpen`: a horse's or llama's
    /// inventory; for another entity the window is closed again.
    fn remote_open_mount(&mut self, p: play::MountScreenOpen) {
        let columns = usize::try_from(p.columns).unwrap_or(0).min(5);
        if !self.open_remote_mount_menu(p.entity as u32, columns) {
            self.remote_reject_container(p.container);
            return;
        }
        if let Some(r) = self.remote.as_mut() {
            r.menu
                .set(Some((p.container, format!("{MOUNT_MENU}{columns}"))));
            r.set_menu_title(None);
        }
    }
    /// The type of the menu a server opened (`MenuType`'s id).
    pub(super) fn remote_menu_type(&self) -> Option<&str> {
        Some(self.remote.as_ref()?.menu.as_ref()?.1.as_str())
    }
    /// The title the server gave the menu it opened.
    pub(super) fn remote_menu_title(&self) -> Option<horizon_client::text::Text> {
        self.remote.as_ref()?.menu_title.clone()
    }
    /// A language change reaches the server-sent components too: their titles
    /// are resolved when the packet arrives, with the session's language.
    pub(super) fn refresh_remote_language(&mut self) {
        if let Some(lang) = horizon_client::lang::current()
            && let Some(r) = self.remote.as_mut()
        {
            r.lang = Arc::new(lang.map().clone());
        }
    }
    /// A server's chest-like menu (`MenuType.GENERIC_9xN`): its rows and title.
    pub(super) fn remote_chest_menu(&self) -> Option<(usize, horizon_client::text::Text)> {
        let r = self.remote.as_ref()?;
        let rows = r
            .menu
            .as_ref()?
            .1
            .strip_prefix("minecraft:generic_9x")?
            .parse()
            .ok()?;
        Some((
            rows,
            r.menu_title
                .clone()
                .unwrap_or_else(horizon_client::text::Text::empty),
        ))
    }
    /// ServerboundPlayerCommandPacket STOP_SLEEPING.
    pub(super) fn remote_stop_sleeping(&mut self) {
        let id = self.remote.as_ref().map_or(0, |r| r.entity_id);
        self.send(S::PlayerCommand(play::PlayerCommand {
            id,
            action: 0,
            data: 0,
        }));
    }
    pub(super) fn remote_respawn(&mut self) {
        self.send(S::ClientCommand(play::ClientCommand { action: 0 }));
    }

    /// A connection's screen is over the world: connecting, reconfiguring, or
    /// the level loading screen, a portal's included (`remote_overlay`).
    pub(super) fn remote_screen_up(&self) -> bool {
        !self.replay.playing()
            && self
                .remote
                .as_ref()
                .is_some_and(|r| self.player.is_none() || r.level_load.is_active())
    }
    /// Connecting / loading screens; true when it covered the frame.
    pub(super) fn remote_overlay(&mut self, batch: &mut HudBatch) -> bool {
        if !self.remote_screen_up() {
            return false;
        }
        let Some(r) = &self.remote else { return false };
        let loading = r.level_load.is_active();
        if self.player.is_some() && !loading {
            return false;
        }
        let (w, h) = (batch.width, batch.height);
        if let Some(since) = r.reconfiguring {
            // ServerReconfigScreen narrates its title.
            horizon_client::widgets::begin_screen(|| {
                horizon_client::widgets::ScreenInfo::new(
                    "reconfiguring",
                    super::screen_info::tr("connect.reconfiguring", &[]),
                )
            });
            self.draw_reconfiguring(batch, since);
            return true;
        }
        if r.joined || loading {
            // LevelLoadingScreen: the world's terrain comes in. A server's
            // chunk status view and progress are the integrated server's alone
            // (`LevelLoadTracker.statusView`, `hasProgress`), and so is what
            // the narrator says of it.
            let progress = self
                .integrated
                .as_ref()
                .map(|i| i.load_view())
                .filter(|v| v.has_progress())
                .map(|v| v.server_progress());
            horizon_client::widgets::begin_screen(|| {
                super::screen_info::level_loading(progress, &super::screen_info::tr)
            });
            if self.draw_dimension_transition_screen(
                batch,
                &r.level_load,
                progress.map(|_| self.loading_progress),
            ) {
                return true;
            }
            self.draw_level_loading_background(batch, r.level_load.reason);
            let atlas = &self.gui.atlas;
            match self.integrated.as_ref().map(|i| i.load_view()) {
                Some(view) => {
                    let status = |x: i32, z: i32| view.status(x, z);
                    let progress = view.has_progress().then_some(self.loading_progress);
                    horizon_client::level_loading::draw(batch, atlas, Some(&status), progress);
                }
                None => horizon_client::level_loading::draw(batch, atlas, None, None),
            }
            return true;
        }
        let cancel = self.connect_cancel((w, h)).is_some();
        // ConnectScreen.extractRenderState: "Joining the world" every 2 s (a
        // server's: single player's isn't joined through ConnectScreen).
        if self.integrated.is_none()
            && let Some(r) = self.remote.as_mut()
            && r.joining_narrated
                .is_none_or(|t| t.elapsed() > Duration::from_millis(2000))
        {
            r.joining_narrated = Some(Instant::now());
            self.say_system_now(&horizon_client::text::Text::translate(
                "narrator.joining",
                &[],
            ));
        }
        let Some(r) = &self.remote else { return false };
        // ConnectScreen: `GameNarrator.NO_TITLE`, then its Cancel; single
        // player's server is joined under its LevelLoadingScreen.
        let single_player = self.integrated.is_some();
        horizon_client::widgets::begin_screen(|| {
            if single_player {
                super::screen_info::level_loading(None, &super::screen_info::tr)
            } else {
                horizon_client::widgets::ScreenInfo::new("connect", "")
            }
        });
        let atlas = &self.gui.atlas;
        let status = horizon_client::text::Text::untrusted(r.status.as_str());
        horizon_client::level_loading::draw_connecting(batch, atlas, &status, self.cursor, cancel);
        true
    }
    /// Connected or connecting with no player yet: `ConnectScreen`, then
    /// `LevelLoadingScreen`, over the panorama with its blur.
    pub(super) fn remote_loading(&self) -> bool {
        if self.replay.playing() {
            return false;
        }
        use horizon_client::level_loading::Reason;
        self.remote.as_ref().is_some_and(|r| {
            if r.level_load.is_active() {
                // The portal screens have no panorama, so no blur.
                r.level_load.reason == Reason::Other
            } else {
                self.player.is_none()
            }
        })
    }
    /// `ServerReconfigScreen`: a vertical layout of cells padded by 10, the
    /// title (9 high) over Disconnect (150 x 20), centred. The title's and the
    /// button's top left corners.
    fn reconfigure_layout(&self, (w, h): (f32, f32)) -> ((f32, f32), (f32, f32)) {
        let title = horizon_client::text::Text::translate("connect.reconfiguring", &[]);
        let title_w = self.gui.atlas.font.width_text(&title).ceil() as i32;
        let width = (title_w + 20).max(170);
        let left = ((w as i32 - width) as f32 * 0.5) as i32;
        let top = ((h as i32 - 69) as f32 * 0.5) as i32;
        let across = |child: i32| (10.0 + ((width - child - 10) as f32 - 10.0) * 0.5) as i32;
        (
            ((left + across(title_w)) as f32, (top + 10) as f32),
            ((left + across(150)) as f32, (top + 39) as f32),
        )
    }
    /// `ServerReconfigScreen` over the panorama; Disconnect wakes up after
    /// `DISCONNECT_TIME` (600 ticks).
    fn draw_reconfiguring(&self, batch: &mut HudBatch, since: u64) {
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        horizon_client::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        let title = horizon_client::text::Text::translate("connect.reconfiguring", &[]);
        let ((tx, ty), (bx, by)) = self.reconfigure_layout((w, h));
        batch.text_styled(atlas, &title, tx, ty, u32::MAX, true);
        // CommonComponents.GUI_DISCONNECT
        let label = horizon_client::text::Text::translate("menu.disconnect", &[]);
        let active = self.ticks.saturating_sub(since) >= 600;
        let hover = horizon_client::ui::hovered(self.cursor, bx, by, 150.0, 20.0);
        horizon_client::ui::button(batch, atlas, bx, by, 150.0, 20.0, &label, active, hover);
    }
    /// `ConnectScreen`'s Cancel: a server's (single player's server isn't
    /// joined through `ConnectScreen`), until the login is done.
    fn connect_cancel(&self, (w, h): (f32, f32)) -> Option<(f32, f32)> {
        let r = self.remote.as_ref()?;
        (self.player.is_none() && !r.joined && self.integrated.is_none())
            .then(|| horizon_client::level_loading::connect_cancel(w, h))
    }
    /// A click on `ConnectScreen`: Cancel aborts the connection
    /// (`connect.aborted`) and goes back. Escape does nothing there
    /// (`shouldCloseOnEsc` is false).
    pub(super) fn connect_screen_click(&mut self) -> Result<()> {
        // ServerReconfigScreen's Disconnect, once it is active.
        if let Some(since) = self.remote.as_ref().and_then(|r| r.reconfiguring) {
            let (_, (x, y)) = self.reconfigure_layout(self.gui_size());
            if self.ticks.saturating_sub(since) >= 600
                && horizon_client::ui::hovered(self.cursor, x, y, 150.0, 20.0)
            {
                self.audio.play_button_click();
                self.leave_server(None)?;
            }
            return Ok(());
        }
        let Some((x, y)) = self.connect_cancel(self.gui_size()) else {
            return Ok(());
        };
        if horizon_client::ui::hovered(self.cursor, x, y, 200.0, 20.0) {
            self.audio.play_button_click();
            self.leave_server(None)?;
        }
        Ok(())
    }
    /// The server's death message; chat uses the shared ChatComponent.
    pub(super) fn remote_hud(&self, batch: &mut HudBatch) {
        let Some(r) = &self.remote else { return };
        let atlas = &self.gui.atlas;
        let w = batch.width;
        if self.screen == Screen::Dead
            && let Some(message) = &r.death_message
        {
            // DeathScreen draws the styled cause component.
            let tw = atlas.font.width_text(message);
            batch.text_styled(
                atlas,
                message,
                ((w - tw) / 2.0).floor(),
                85.0,
                u32::MAX,
                true,
            );
        }
    }
    /// Shared flag 6 of an entity on a server's client (`None` without a server).
    pub(super) fn remote_glowing(&self, id: u32) -> Option<bool> {
        let r = self.remote.as_ref()?;
        Some(r.glowing.contains(&(id as i32)))
    }
    /// The profile name of another player the server sent, by entity id.
    pub(super) fn remote_player_name(&self, id: u32) -> Option<&str> {
        let r = self.remote.as_ref()?;
        r.level_player_name(id as i32, &r.player_uuid(id)?)
    }
    pub(super) fn remote_player_uuid(&self, id: u32) -> Option<u128> {
        let remote = self.remote.as_ref()?;
        if self.player_model.as_ref().is_some_and(|m| m.id == id) {
            Some(remote.profile)
        } else {
            remote.level_players.get(&(id as i32)).copied()
        }
    }
    /// Status for `horizon-ctl status`.
    /// The other players' entities in the level and their names (their
    /// profiles', which outlive the player list's entries).
    pub(super) fn remote_player_names(&self) -> Vec<(u32, String)> {
        let Some(r) = self.remote.as_ref() else {
            return Vec::new();
        };
        r.level_players
            .iter()
            .filter(|(_, uuid)| **uuid != r.profile)
            .filter_map(|(id, uuid)| Some((*id as u32, r.level_player_name(*id, uuid)?.to_owned())))
            .collect()
    }
    /// `ClientClockManager.tick`: the clock moves on by the game time that
    /// passed since its last tick, at its rate. A game time the server sets
    /// forward (its sync every second) moves the clock with it.
    fn remote_clock_tick(&mut self) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let delta = self.world.game_time - r.clock_game_time;
        r.clock_game_time = self.world.game_time;
        for clock in r.clocks.values_mut() {
            let _ = client_level_tick::advance_clock(clock, delta);
        }
        self.world.day_time = r.clock_time(r.default_clock);
    }
    pub(super) fn remote_status(&self) -> Option<serde_json::Value> {
        let r = self.remote.as_ref()?;
        Some(serde_json::json!({
            "address": horizon_client::net::shown_address(&r.session.address),
            "name": r.session.name,
            "joined": r.joined,
            // ServerboundPlayerLoadedPacket sent: the level around the
            // player is ready, and the client ticks and sends its moves.
            "loaded": r.loaded_sent,
            "status": r.status,
            "entity_id": r.entity_id,
            "dimension": r.dimension.name,
            "game_mode": r.game_mode,
            "chunks": self.world.chunks.len(),
            "entities": r.entities.values().fold(std::collections::BTreeMap::<&str, u32>::new(), |mut m, t| {
                *m.entry(t.kind.as_str()).or_default() += 1;
                m
            }),
            "players": r.players.values().collect::<Vec<_>>(),
            // Where the other players' entities are (harnesses check movement).
            // What the other players hold and wear, as this client draws them.
            "player_equipment": self
                .mobs
                .iter()
                .filter(|m| r.level_players.contains_key(&(m.id as i32)))
                .map(|m| {
                    let e = &m.extra;
                    let armor = horizon_items::equipment::ARMOR.map(|slot| e.slot_id(slot));
                    serde_json::json!({
                        "id": m.id,
                        "swinging": m.ai.swinging,
                        "using": m.extra.using_item,
                        "armor": armor,
                        "main_hand": e.main_hand_id(),
                        "off_hand": e.off_hand_id(),
                    })
                })
                .collect::<Vec<_>>(),
            "player_positions": r
                .entities
                .values()
                .filter(|t| t.kind == "minecraft:player")
                .map(|t| t.position)
                .collect::<Vec<_>>(),
            // The other players this client draws dead (lying down, after EntityEvent DEATH).
            "dead_players": self
                .mobs
                .iter()
                .filter(|m| r.level_players.contains_key(&(m.id as i32)) && !m.alive())
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            // The other players this client draws lying in a bed (SLEEPING_POS).
            "lying_players": self
                .mobs
                .iter()
                .filter(|m| r.level_players.contains_key(&(m.id as i32)) && m.extra.ints[1] != 0)
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            // The player's own model exists (third person and the inventory draw it).
            "player_model": self.player_model.is_some(),
            // The world view draws its body now (third person, or lying in a bed;
            // not while invisible).
            "own_body": self.player_model_shown() && self.own_body_visible(),
            "text_display_vertices": self.text_display_vertices,
            "text_displays": self.text_displays.len(),
            "health": r.health.0,
            "food": r.health.1,
            "decode_errors": r.decode_errors,
            "chat": r.chat.iter().rev().take(10).rev().map(|(t, _)| t.clone()).collect::<Vec<_>>(),
        }))
    }
}

/// `Entity.lookAt`'s rotation from `from` towards `to`, as (yaw, pitch).
fn look_at_rotation(from: [f64; 3], to: [f64; 3]) -> (f32, f32) {
    use horizon_core::geometry::trig::atan2;
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let horizontal = (dx * dx + dz * dz).sqrt();
    // `* 180.0F / (float)Math.PI` on the double.
    let degrees = |r: f64| r * 180.0 / f64::from(std::f32::consts::PI);
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
pub(crate) mod tests {
    use super::super::api_values::TryIntoApi;

    mod predicted_block_notice_tests {
        include!("predicted_block_notice_tests.rs");
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn menu_title_snapshot_tracks_open_replacement_and_close() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let (client, _server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(client, "menu-title-test").unwrap();
            let remote = game.remote.as_mut().unwrap();
            for value in [Some("first"), Some("replacement"), None] {
                remote.set_menu_title(value.map(horizon_client::text::Text::literal));
                assert_eq!(
                    remote.menu_title_snapshot(),
                    remote.menu_title.try_into_api().unwrap().as_ref()
                );
            }
        });
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn menu_title_snapshot_follows_packet_and_close_paths() {
        use horizon_core::nbt::Tag;
        use horizon_protocol::packets::play::{self, Clientbound as C};
        use std::sync::Arc;

        super::super::native_entity_refs::tests::with_client_game(|game| {
            let (client, _server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(client, "menu-title-paths").unwrap();
            let menu_id = |game: &super::Game<'_>, name: &str| {
                game.remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:menu")
                    .unwrap()
                    .id(name)
                    .unwrap()
            };
            let chest = menu_id(game, "minecraft:generic_9x3");
            let lectern = menu_id(game, "minecraft:lectern");
            let open = |game: &mut super::Game<'_>, menu, title: &str| {
                game.handle_packet(C::OpenScreen(play::OpenScreen {
                    container: 7,
                    menu,
                    title: Tag::String(title.into()),
                }))
                .unwrap();
            };
            let snapshot = |game: &super::Game<'_>| {
                let shared = game
                    .remote
                    .as_ref()
                    .unwrap()
                    .shared_menu_title_snapshot()
                    .unwrap();
                let observed = game.test_observed_title().unwrap();
                assert!(Arc::ptr_eq(&observed, &shared));
                shared
            };
            open(game, chest, "first");
            let first = snapshot(game);
            open(game, chest, "replacement");
            let replacement = snapshot(game);
            assert!(!Arc::ptr_eq(&first, &replacement));
            assert_ne!(first, replacement);
            game.handle_packet(C::ContainerClose(play::ContainerClose { container: 7 }))
                .unwrap();
            assert!(
                game.remote
                    .as_ref()
                    .unwrap()
                    .shared_menu_title_snapshot()
                    .is_none()
            );
            assert!(game.test_observed_title().is_none());
            open(game, chest, "again");
            snapshot(game);
            game.remote_close_container();
            assert!(
                game.remote
                    .as_ref()
                    .unwrap()
                    .shared_menu_title_snapshot()
                    .is_none()
            );
            open(game, chest, "before reject");
            game.remote_reject_container(7);
            assert!(
                game.remote
                    .as_ref()
                    .unwrap()
                    .shared_menu_title_snapshot()
                    .is_none()
            );
            open(game, chest, "before dimension change");
            spawn_menu_test(game);
            assert!(
                game.remote
                    .as_ref()
                    .unwrap()
                    .shared_menu_title_snapshot()
                    .is_none()
            );
            open(game, chest, "before lectern");
            snapshot(game);
            open(game, lectern, "lectern");
            assert!(
                game.remote
                    .as_ref()
                    .unwrap()
                    .shared_menu_title_snapshot()
                    .is_none()
            );
            game.remote_lectern_page(1);
            assert!(
                game.remote
                    .as_ref()
                    .unwrap()
                    .shared_menu_title_snapshot()
                    .is_none()
            );
            game.handle_packet(C::ContainerClose(play::ContainerClose { container: 7 }))
                .unwrap();
            assert!(
                game.remote
                    .as_ref()
                    .unwrap()
                    .shared_menu_title_snapshot()
                    .is_none()
            );
            assert!(game.test_observed_title().is_none());
        });
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn stopping_a_retained_client_dig_aborts_only_once() {
        use crate::game::{Hit, native_entity_refs::tests::with_client_game};
        use horizon_client::{chunk::Chunk, motion::PlayerMotion};
        use horizon_core::geometry::Vec3;
        use horizon_protocol::connection::Pipe;
        use mod_api::{
            ClientHost, ClientMiningPredictEvent, ClientPacketSendEvent, Flow, Hooks, ModEntry,
        };
        use mod_platform::testing::with_mods;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        struct CountActions {
            packets: Arc<AtomicUsize>,
            vetoes: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for CountActions {
            fn client_mining_predict(
                &self,
                _: &dyn ClientHost,
                event: &mut ClientMiningPredictEvent,
            ) -> Flow {
                assert!(!event.prediction.starting);
                assert_eq!(event.prediction.pos, [0, 64, 0]);
                assert!(event.prediction.progress > 0.5);
                self.vetoes.fetch_add(1, Ordering::Relaxed);
                Flow::Cancel
            }

            fn client_packet_send(
                &self,
                _: &dyn ClientHost,
                event: &mut ClientPacketSendEvent<'_>,
            ) -> Flow {
                if event.packet.id.path() == "player_action" {
                    self.packets.fetch_add(1, Ordering::Relaxed);
                }
                Flow::Continue
            }
        }
        let count = Arc::new(AtomicUsize::new(0));
        let vetoes = Arc::new(AtomicUsize::new(0));
        let mods = vec![ModEntry::new(
            "count_actions",
            Box::new(CountActions {
                packets: count.clone(),
                vetoes: vetoes.clone(),
            }),
        )]
        .into();
        with_mods(mods, || {
            with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "MiningTest").unwrap();
                let pos = [0, 64, 0];
                game.set_mode_id(0);
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 3.5)).unwrap());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                game.tick_hit.set(Some(Some(Hit {
                    pos,
                    face: [0, 1, 0],
                    point: [0.5, 65.0, 0.5],
                    inside: false,
                })));
                let held = game.main_hand_key();
                let remote = game.remote.as_mut().unwrap();
                remote.destroying = Some((pos, 1, 0.5));
                remote.destroying_item = held;
                remote.is_destroying = true;
                game.remote_continue_attack(true);
                assert_eq!(vetoes.load(Ordering::Relaxed), 1);
                let remote = game.remote.as_ref().unwrap();
                assert!(remote.is_destroying);
                assert_eq!(remote.destroying, Some((pos, 1, 0.5)));
                assert_eq!(count.load(Ordering::Relaxed), 0);
                game.remote_continue_attack(false);
                game.remote_continue_attack(false);
                assert_eq!(
                    game.remote.as_ref().unwrap().destroying,
                    Some((pos, 1, 0.0))
                );
            })
        });
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }
    /// The remote player animation uses the scoped item answer once per tick.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn remote_player_use_animation_dispatches_once() {
        use mod_api::{ClientHost, Hooks, ItemPart, ModEntry, Mods, Registrar, UseAnimationEvent};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        #[derive(Clone)]
        struct Part(Arc<AtomicUsize>);
        #[mod_api::horizon_part]
        impl ItemPart for Part {
            fn use_animation(&self, _: &dyn ClientHost, event: &mut UseAnimationEvent<'_>) {
                self.0.fetch_add(1, Ordering::Relaxed);
                *event.animation = mod_api::UseAnimation::Drink;
            }
        }
        struct Mod(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let item = r.item_key("minecraft:bow");
                r.item_part(item, Part(self.0.clone()));
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: Mods = vec![ModEntry::new(
            "remote_animation",
            Box::new(Mod(calls.clone())),
        )]
        .into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                let (client, _server) = horizon_protocol::connection::Pipe::pair();
                game.connect_local(client, "animation-client").unwrap();
                let remote = game.remote.as_mut().unwrap();
                remote.entity_id = 42;
                remote.level_players.insert(66, 789);
                let kind = remote.player_type.as_ref().unwrap().clone();
                let mut mob = horizon_client::mobs::Mob::new(
                    66,
                    kind,
                    horizon_core::geometry::Vec3::ZERO,
                    0.0,
                );
                mob.extra.using_item = 1;
                mob.extra.set_slot_stack(
                    horizon_items::equipment::MAINHAND,
                    Some(horizon_client::items::ItemStack::new("minecraft:bow", 1)),
                );
                game.mobs.push(mob);
                game.tick_remote_entities(false);
                assert_eq!(
                    game.mobs[0].extra.use_animation,
                    mod_api::UseAnimation::Drink
                );
                assert_eq!(calls.load(Ordering::Relaxed), 1);
            });
        });
    }

    /// After `ClientboundUpdateAttributes`, the local player's `LivingRef`
    /// reads its attribute value, base, modifiers and attribute list from the
    /// client's own copy of the synced instances. A mob the server tracks
    /// keeps no attribute state of its own on the client, so its reads are
    /// `None` there.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn update_attributes_answers_the_local_players_living_reads() {
        use super::super::native_entity_refs::tests::with_client_game;
        use horizon_client::{mobs::Mob, motion::PlayerMotion};
        use horizon_core::geometry::Vec3;
        use horizon_protocol::{
            buf::Reader,
            connection::Pipe,
            packets::play::{self, Clientbound as C},
        };

        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "attributes-client").unwrap();
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player.as_mut().unwrap().entity_id = 42;
            game.mobs.push(Mob::new(60, zombie, Vec3::ZERO, 0.0));
            let remote = game.remote.as_mut().unwrap();
            remote.entity_id = 42;
            remote.logged_in = true;
            remote.entities.insert(
                60,
                super::Tracked {
                    kind: "minecraft:zombie".into(),
                    position: [0.0; 3],
                    codec_base: [0.0; 3],
                    silent: false,
                    vehicle: None,
                },
            );

            let health = "minecraft:max_health";
            let living = game.living(game.entity(42).unwrap()).unwrap();
            // Before the server says anything, the client's own map holds the
            // type's defaults, as a vanilla client's does before its sync.
            assert_eq!(living.attribute_value(game, health), Some(20.0));
            assert_eq!(living.attribute_modifiers(game, health), Some(Vec::new()));

            let registry_id = game.motion.attributes.id(health).unwrap().registry_id();
            let packet = C::UpdateAttributes(play::UpdateAttributes {
                id: 42,
                values: vec![play::AttributeSnapshot {
                    attribute: registry_id,
                    base: 30.0,
                    modifiers: vec![play::AttributeModifier {
                        id: "test:bonus".into(),
                        amount: 5.0,
                        operation: 0,
                    }],
                }],
            });
            let body = packet.encode();
            let mut reader = Reader::new(&body);
            let id = reader.varint().unwrap();
            game.handle_packet(C::decode(id, reader.rest()).unwrap())
                .unwrap();

            assert_eq!(living.attribute_value(game, health), Some(35.0));
            assert_eq!(living.attribute_base(game, health), Some(30.0));
            assert_eq!(
                living.attribute_modifiers(game, health),
                Some(vec![mod_api::AttributeModifier::new(
                    mod_api::Identifier::parse("test:bonus").unwrap(),
                    5.0,
                    mod_api::AttributeOperation::AddValue,
                )])
            );
            let listed = living.attributes(game).unwrap();
            assert!(listed.contains(&mod_api::Identifier::parse(health).unwrap()));

            // The mob's attributes are the server's business: no client state,
            // no read.
            let mob = mod_api::LivingRef::new(game.entity(60).unwrap());
            assert_eq!(mob.attribute_value(game, health), None);
            assert_eq!(mob.attribute_base(game, health), None);
            assert_eq!(mob.attribute_modifiers(game, health), None);
            assert_eq!(mob.attributes(game), None);
        });
    }

    /// Sleeping metadata and the local tick use the loaded bed's shape, not
    /// a red-bed constant. Missing client data must not replace a server pose
    /// or decide to wake the player.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn client_sleep_metadata_and_tick_preserve_shape_and_authoritative_positions() {
        use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
        use horizon_client::{chunk::Chunk, mobs::Mob, motion::PlayerMotion};
        use horizon_core::geometry::Vec3;
        use horizon_protocol::{
            buf::Reader,
            connection::Pipe,
            entity_data::{Value, serializer},
            packets::play::{self, Clientbound as C},
        };

        let deliver = |game: &mut super::super::Game<'_>, packet: C| {
            let body = packet.encode();
            let mut reader = Reader::new(&body);
            let id = reader.varint().unwrap();
            game.handle_packet(C::decode(id, reader.rest()).unwrap())
                .unwrap();
        };
        let metadata = |id, bed: Option<[i32; 3]>| {
            C::SetEntityData(play::SetEntityData {
                id,
                values: vec![
                    (
                        6,
                        serializer::POSE,
                        Value::Id(if bed.is_some() { 2 } else { 0 }),
                    ),
                    (14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(bed)),
                ],
            })
        };
        let teleport = |position: Vec3| {
            C::PlayerPosition(play::PlayerPosition {
                teleport: 1,
                position: [position.x, position.y, position.z],
                delta: [0.0; 3],
                yaw: 0.0,
                pitch: 30.0,
                relatives: 0,
            })
        };

        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            let (client, server) = Pipe::pair();
            game.connect_local(client, "sleep-client").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.loaded_sent = true;
            remote.entity_id = 42;
            remote.level_players.insert(66, 789);
            let kind = remote.player_type.as_ref().unwrap().clone();
            let authoritative = Vec3::new(8.5, 70.0, 8.5);
            game.player = Some(PlayerMotion::new(game.motion, authoritative).unwrap());
            game.mobs.push(Mob::new(66, kind, authoritative, 0.0));
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(0, 0));
            let registry = game.pack.assets.registry;
            let air = registry.default_state("minecraft:air").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();

            // Straw's foot is 4/16 high; red's shape is 9/16. These expected
            // positions are independent of sleeping_position and its helpers.
            for (name, complete, head, expected) in [
                (
                    "minecraft:straw_bed",
                    true,
                    [5, 64, 5],
                    Some(Vec3::new(5.5, 64.375, 5.5)),
                ),
                (
                    "minecraft:red_bed",
                    true,
                    [5, 64, 5],
                    Some(Vec3::new(5.5, 64.6875, 5.5)),
                ),
                ("minecraft:straw_bed", false, [5, 64, 5], None),
                ("minecraft:stone", false, [5, 64, 5], None),
                ("minecraft:straw_bed", false, [37, 64, 5], None),
            ] {
                let case = (name, complete, head);
                game.world.set(5, 64, 5, air).unwrap();
                game.world.set(5, 64, 6, air).unwrap();
                if head[0] == 5 {
                    let state = if name == "minecraft:stone" {
                        stone
                    } else {
                        registry
                            .resolve(&horizon_world::chunk::BlockState {
                                name: name.into(),
                                properties: [
                                    ("facing", "north"),
                                    ("occupied", "false"),
                                    ("part", "head"),
                                ]
                                .into_iter()
                                .map(|(k, v)| (k.into(), v.into()))
                                .collect(),
                            })
                            .unwrap()
                    };
                    game.world.set(5, 64, 5, state).unwrap();
                    if complete {
                        let mut foot = registry.state(state).unwrap().clone();
                        foot.properties.insert("part".into(), "foot".into());
                        game.world
                            .set(5, 64, 6, registry.id(&foot).unwrap())
                            .unwrap();
                    }
                } else {
                    assert!(!game.world.chunks.contains_key(&(2, 0)));
                }
                deliver(game, teleport(authoritative));
                let other = game.mob_index.get_mut(&mut game.mobs, 66).unwrap();
                other.position = authoritative;
                other.previous = authoritative;
                other.head_pitch = 30.0;
                other.previous_head_pitch = 30.0;
                game.sleep_timer = 17;
                deliver(game, metadata(66, Some(head)));
                deliver(game, metadata(42, Some(head)));
                let at = expected.unwrap_or(authoritative);
                assert_eq!(
                    game.player.as_ref().unwrap().position,
                    at,
                    "own metadata: {case:?}"
                );
                assert_eq!(game.previous_position, at, "own old position: {case:?}");
                let other = game.mob_index.get(&game.mobs, 66).unwrap();
                assert_eq!(other.position, at, "other metadata: {case:?}");
                assert_eq!(other.previous, at, "other old position: {case:?}");
                assert_eq!((other.head_pitch, other.previous_head_pitch), (0.0, 0.0));
                assert_eq!(game.sleeping, Some(head));
                assert_eq!(game.sleep_timer, 0);

                // A correct authoritative teleport cannot be overwritten by
                // the next sleeping tick, even while this client's bed is absent.
                deliver(game, teleport(at));
                game.player.as_mut().unwrap().velocity = Vec3::new(0.3, -0.4, 0.2);
                for timer in 1..=2 {
                    game.tick_remote_player(true, false).unwrap();
                    assert_eq!(game.sleep_timer, timer, "local tick must execute: {case:?}");
                    assert_eq!(game.sleeping, Some(head));
                    let player = game.player.as_ref().unwrap();
                    assert_eq!(player.position, at, "own tick: {case:?}");
                    assert_eq!(player.velocity, Vec3::ZERO);
                    assert_eq!(player.pitch, 0.0);
                    assert_eq!(
                        game.mob_index.get(&game.mobs, 66).unwrap().position,
                        at,
                        "other tick: {case:?}"
                    );
                }

                deliver(game, metadata(66, None));
                deliver(game, metadata(42, None));
                assert_eq!(game.sleeping, None);
                assert_eq!(
                    game.player.as_ref().unwrap().position,
                    at,
                    "clearing metadata does not relocate"
                );
                let other = game.mob_index.get(&game.mobs, 66).unwrap();
                assert_eq!(other.position, at);
                assert_eq!(other.extra.ints[1], 0);
                assert_eq!(other.extra.synced_pose, 0);
                deliver(game, C::Animate(play::Animate { id: 42, action: 0 }));
                assert_eq!(game.sleep_timer, 100);
                let wake = Vec3::new(9.5, 70.1, 9.5);
                deliver(game, teleport(wake));
                assert_eq!(game.player.as_ref().unwrap().position, wake);
                game.tick_remote_player(true, false).unwrap();
                assert_eq!(game.sleeping, None);
                assert_eq!(game.sleep_timer, 101);
                let player = game.player.as_ref().unwrap();
                assert_eq!(
                    (player.position.x, player.position.z),
                    (wake.x, wake.z),
                    "wake tick must not snap to bed: {case:?}"
                );
            }
            server.shutdown();
        });
    }

    /// `ClientChatReceivedEvent`: a mod's replacement is the line the chat
    /// shows, a cancelled system line never shows, and a cancelled message
    /// held by Chat Delay lets the next one through in the same tick.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mods_replace_and_cancel_received_chat() {
        use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
        use horizon_core::nbt::Tag;
        use horizon_protocol::{
            connection::Pipe,
            packets::play::{self, Clientbound as C},
        };

        struct Censor;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Censor {
            fn client_chat_received(
                &self,
                _: &mut dyn mod_api::ClientHost,
                event: &mut mod_api::ClientChatReceivedEvent<'_>,
            ) -> mod_api::Flow {
                use mod_api::text::TextContent;
                let text = match &event.message.content {
                    TextContent::Literal(text) => text,
                    // Player chat arrives decorated with the sender and message.
                    TextContent::Translatable { key, arguments, .. } if key == "chat.type.text" => {
                        let TextContent::Literal(text) = &arguments[1].content else {
                            panic!("expected literal player message fixture");
                        };
                        text
                    }
                    _ => return mod_api::Flow::Continue,
                };
                if text.contains("drop") {
                    return mod_api::Flow::Cancel;
                }
                // A system line carries no player message, so only it gets the tag.
                if event.system && event.player_message.is_none() {
                    *event.message = mod_api::Text::literal(format!("[server] {text}"));
                }
                mod_api::Flow::Continue
            }
        }
        mod_platform::__private::install(std::sync::Arc::from([mod_api::ModEntry::new(
            "test-chat-received",
            Box::new(Censor),
        )]));

        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "ChatTest").unwrap();
            let lines = |game: &super::super::Game| -> Vec<String> {
                let chat = &game.remote.as_ref().unwrap().chat;
                chat.iter().map(|(line, _)| line.clone()).collect()
            };
            let system = |content: &str| {
                C::SystemChat(play::SystemChat {
                    content: Tag::String(content.into()),
                    overlay: false,
                })
            };
            game.handle_packet(system("hello")).unwrap();
            game.handle_packet(system("drop me")).unwrap();
            assert_eq!(lines(game), ["[server] hello"]);

            game.settings.chat_delay = 1.0;
            let disguised = |message: &str| {
                C::DisguisedChat(play::DisguisedChat {
                    message: Tag::String(message.into()),
                    chat_type: play::ChatTypeBound {
                        chat_type: 0,
                        name: Tag::String("Steve".into()),
                        target: None,
                    },
                })
            };
            game.handle_packet(disguised("drop this")).unwrap();
            game.handle_packet(disguised("hi")).unwrap();
            assert_eq!(lines(game).len(), 1, "Chat Delay holds both");
            game.tick_chat_queue();
            let shown = lines(game);
            assert_eq!(shown.len(), 2, "{shown:?}");
            assert!(
                shown[1].contains("hi") && !shown[1].contains("drop"),
                "{shown:?}"
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mod_chat_needs_a_joined_player_and_what_the_chat_option_allows() {
        use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
        use horizon_protocol::connection::Pipe;

        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_err());
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "ChatTest").unwrap();
            assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_err());
            game.remote.as_mut().unwrap().joined = true;
            assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "  ").is_err());
            // Full, then Commands Only, then Hidden.
            game.settings.chat_visibility = 0;
            assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "hello").is_ok());
            assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_ok());
            game.settings.chat_visibility = 1;
            assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "hello").is_err());
            assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_ok());
            game.settings.chat_visibility = 2;
            assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_err());
        });
    }

    /// `ChatComponent.addClientSystemMessage`: the line joins this client's
    /// chat history and no packet leaves for the server.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mod_chat_line_shows_locally_and_sends_nothing() {
        use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
        use horizon_protocol::connection::{Connection, Pipe};

        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            let (client, server) = Pipe::pair();
            game.connect_local(client, "ChatLineTest").unwrap();
            // Whatever connecting wrote is not this line's; drain it until the
            // client waits for the login answer, so the check below is about
            // `chat_line` alone.
            let (mut reader, _writer) = Connection::local(server).split();
            reader
                .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                .unwrap();
            while reader.recv().is_ok() {}
            mod_api::prelude::NetworkClientHost::chat_line(
                game,
                mod_api::Text::literal("local only"),
            )
            .unwrap();
            let history: Vec<_> = game
                .remote
                .as_ref()
                .unwrap()
                .chat
                .iter()
                .map(|(line, _)| line.clone())
                .collect();
            assert_eq!(history, ["local only"]);
            assert!(
                reader.recv().unwrap_err().is_timed_out(),
                "a local chat line sends nothing to the server"
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn rejected_menu_packets_retire_the_previous_client_menu() {
        use super::super::{
            Screen, execution_role::ExecutionRole, native_entity_refs::tests::with_game,
        };
        use horizon_client::items::ItemStack;
        use horizon_core::nbt::Tag;
        use horizon_protocol::{
            connection::Pipe,
            packets::play::{Clientbound as C, MountScreenOpen, OpenScreen},
        };

        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "MenuTest").unwrap();
            for packet in [
                C::OpenScreen(OpenScreen {
                    container: 7,
                    menu: -1,
                    title: Tag::String("unsupported".into()),
                }),
                C::MountScreenOpen(MountScreenOpen {
                    container: 7,
                    columns: 0,
                    entity: i32::MAX,
                }),
            ] {
                let remote = game.remote.as_mut().unwrap();
                remote.menu.set(Some((7, "minecraft:generic_9x3".into())));
                let generation = remote.menu.generation;
                game.set_screen(Screen::Inventory);
                game.carried = Some(ItemStack::new("minecraft:stone", 1));
                game.handle_packet(packet).unwrap();
                let remote = game.remote.as_ref().unwrap();
                assert!(remote.menu.as_ref().is_none());
                assert_ne!(remote.menu.generation, generation);
                assert!(game.carried.is_none());
                assert!(game.screen == Screen::Playing);
            }
        });
    }

    /// `LivingEntity.handleEntityEvent(55)` is `swapHandItems`: the whole
    /// stacks trade hands, not just their ids.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_hand_swap_event_keeps_whole_stacks() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::{items::ItemStack, mobs::Mob};
        use horizon_core::geometry::Vec3;
        use horizon_items::equipment::{MAINHAND, OFFHAND};
        use horizon_protocol::packets::play::{Clientbound as C, EntityEvent};

        with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
            sword.damage = 40;
            sword.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("Biter".into()));
            let mut shield = ItemStack::new("minecraft:shield", 1);
            shield.damage = 12;
            let mut mob = Mob::new(31, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            mob.extra.set_slot_stack(MAINHAND, Some(sword.clone()));
            mob.extra.set_slot_stack(OFFHAND, Some(shield.clone()));
            game.mobs.push(mob);
            let swap = |game: &mut super::super::Game| {
                game.handle_packet(C::EntityEvent(EntityEvent { id: 31, event: 55 }))
                    .unwrap();
            };
            swap(game);
            let m = game.mobs.iter().find(|m| m.id == 31).unwrap();
            assert_eq!(m.extra.slot_stack(MAINHAND), Some(&shield));
            assert_eq!(m.extra.slot_stack(OFFHAND), Some(&sword));
            swap(game);
            let m = game.mobs.iter().find(|m| m.id == 31).unwrap();
            assert_eq!(m.extra.slot_stack(MAINHAND), Some(&sword));
            assert_eq!(m.extra.slot_stack(OFFHAND), Some(&shield));
        });
    }

    #[test]
    fn client_menu_generation_changes_on_reused_window_ids() {
        let mut menu = super::ClientMenu::default();
        let inventory = menu.generation;
        menu.set(Some((7, "minecraft:generic_9x3".to_owned())));
        let first = menu.generation;
        assert_ne!(inventory, first);
        menu.set(None);
        let closed = menu.generation;
        assert_ne!(first, closed);
        assert!(menu.as_ref().is_none());
        menu.set(Some((7, "minecraft:generic_9x3".to_owned())));
        assert_ne!(closed, menu.generation);
        assert_ne!(first, menu.generation);
        assert_eq!(menu.as_ref().map(|entry| entry.0), Some(7));
        let before_replacement = menu.generation;
        menu.set(Some((7, "minecraft:generic_9x3".to_owned())));
        assert_ne!(before_replacement, menu.generation);
    }

    #[test]
    fn client_menu_slot_mapping_covers_inventory_and_containers() {
        for (name, count, first, last) in [
            ("inventory", 46, 200, 504),
            ("minecraft:crafting", 46, 200, 8),
            ("minecraft:generic_9x3", 63, 300, 8),
            ("minecraft:hopper", 41, 300, 8),
            ("minecraft:furnace", 39, 300, 8),
            ("minecraft:brewing_stand", 41, 300, 8),
        ] {
            assert_eq!(super::Game::inventory_screen_slot(name, 0), Some(first));
            assert_eq!(
                super::Game::inventory_screen_slot(name, count - 1),
                Some(last)
            );
            assert_eq!(super::Game::inventory_screen_slot(name, count), None);
        }
        assert_eq!(
            super::Game::inventory_screen_slot("minecraft:unknown", 0),
            None
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn transactional_bucket_hands_reach_the_owning_client() {
        use super::super::{
            native_entity_refs::tests::with_client_game, native_guest_turn_tests::with_guests,
        };
        use horizon_client::{
            items::ItemStack,
            lan::{Out, PlayerSender},
        };
        use horizon_protocol::{buf::Reader, packets::play::Clientbound as C};
        use mod_api::{HostTransaction, ITEM_FLUIDS, ItemPlace, ServerHost, move_resources};

        with_guests(|server| {
            let (tx, rx) = std::sync::mpsc::channel();
            server.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                guest.sender = PlayerSender::new(tx, std::sync::Arc::default());
                guest.selected = 2;
                guest.inventory[2] = Some(ItemStack::new("minecraft:bucket", 1));
                guest.offhand = Some(ItemStack::new("minecraft:water_bucket", 1));
            });
            let main = ItemPlace::PlayerHand {
                player: 12,
                hand: horizon_client::first_person::Hand::Main,
            };
            let off = ItemPlace::PlayerHand {
                player: 12,
                hand: horizon_client::first_person::Hand::Off,
            };
            with_client_game(|client| {
                let (_reader, _writer) = connect_menu_test(client);
                spawn_menu_test(client);
                client.selected = 2;
                // Consume actual server packets through the client's ordinary handler.
                let deliver = |client: &mut super::Game<'_>| {
                    let mut slots = Vec::new();
                    for out in rx.try_iter() {
                        let bodies = match out {
                            Out::Packet(body) => vec![body],
                            Out::Packets(bodies) => bodies,
                            Out::Disconnect(_) => panic!("owner disconnected"),
                        };
                        for body in bodies {
                            let mut reader = Reader::new(&body);
                            let id = reader.varint().unwrap();
                            let packet = C::decode(id, reader.rest()).unwrap();
                            // Other tick packets concern entity/chunk state, not this contract.
                            if let C::ContainerSetSlot(ref slot) = packet {
                                slots.push(slot.slot);
                                client.handle_packet(packet).unwrap();
                            }
                        }
                    }
                    slots
                };
                server.tick_lan(true);
                deliver(client);
                assert_eq!(
                    client.inventory[2],
                    Some(ItemStack::new("minecraft:bucket", 1))
                );
                assert_eq!(
                    client.offhand,
                    Some(ItemStack::new("minecraft:water_bucket", 1))
                );

                for commit in [false, true] {
                    let host: &mut dyn ServerHost = server;
                    let result = host.transaction(|host, tx| {
                        let source = off.capability(host, &ITEM_FLUIDS).unwrap();
                        let target = main.capability(host, &ITEM_FLUIDS).unwrap();
                        assert_eq!(
                            move_resources(host, &source, &target, |_| true, 1000, tx).unwrap(),
                            1000
                        );
                        if commit { Ok(()) } else { Err(()) }
                    });
                    assert_eq!(result.is_ok(), commit);
                    server.tick_lan(true);
                    let slots = deliver(client);
                    if commit {
                        assert!(slots.contains(&38), "selected hotbar slot published");
                        assert!(slots.contains(&45), "offhand published");
                    } else {
                        assert!(slots.is_empty(), "abort publishes no tentative inventory");
                    }
                    let (held, other) = if commit {
                        ("minecraft:water_bucket", "minecraft:bucket")
                    } else {
                        ("minecraft:bucket", "minecraft:water_bucket")
                    };
                    assert_eq!(
                        client.inventory[client.selected],
                        Some(ItemStack::new(held, 1))
                    );
                    assert_eq!(client.offhand, Some(ItemStack::new(other, 1)));
                    let guest = &server.lan.as_ref().unwrap().hub.guests[0];
                    assert_eq!(guest.inventory[2], client.inventory[2]);
                    assert_eq!(guest.offhand, client.offhand);
                }
                server.tick_lan(true);
                assert!(
                    deliver(client).is_empty(),
                    "settled slots are not sent twice"
                );
            });
        });
    }

    /// A client connecting through a local pipe and through configuration, as
    /// the client menu tests drive it: the server end's reader (what the
    /// client sends) and writer.
    pub(crate) fn connect_menu_test(
        game: &mut super::Game<'_>,
    ) -> (
        horizon_protocol::connection::ConnReader,
        horizon_protocol::connection::ConnWriter,
    ) {
        use super::super::execution_role::ExecutionRole;
        use horizon_protocol::{
            connection::{Connection, Pipe},
            packets::{Empty, configuration as cfg, login},
        };
        game.options.execution_role = ExecutionRole::Client;
        let (client, server) = Pipe::pair();
        game.connect_local(client, "MenuReadTest").unwrap();
        let (mut reader, mut writer) = Connection::local(server).split();
        reader
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        reader.recv().unwrap();
        reader.recv().unwrap();
        writer
            .send(
                &login::Clientbound::LoginFinished(login::LoginFinished {
                    profile: login::GameProfile {
                        uuid: 42,
                        name: "MenuReadTest".into(),
                        properties: vec![],
                    },
                    session: 1,
                })
                .encode(),
            )
            .unwrap();
        let (id, body) = reader.recv().unwrap();
        assert!(matches!(
            login::Serverbound::decode(id, &body).unwrap(),
            login::Serverbound::LoginAcknowledged(_)
        ));
        reader.recv().unwrap();
        reader.recv().unwrap();
        writer
            .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
            .unwrap();
        let (id, body) = reader.recv().unwrap();
        assert!(matches!(
            cfg::Serverbound::decode(id, &body).unwrap(),
            cfg::Serverbound::FinishConfiguration(_)
        ));
        (reader, writer)
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn dimension_transition_screen_is_selected_at_entry_drawn_and_dropped() {
        use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
        use horizon_client::{hud::HudBatch, level_loading::Reason};
        use mod_api::__private::game::RegistrationContextNew;
        use mod_api::{DimensionTransition, DimensionTransitionScreen};

        struct Arrival;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Arrival {
            fn register_dimension_transition_screen(
                &self,
                _: &mod_api::RegistrationContext,
                event: &mut mod_api::RegisterDimensionTransitionScreenEvent<'_>,
            ) {
                event.screens.push(DimensionTransitionScreen {
                    screen: "transition_test:arrival".into(),
                    transition: DimensionTransition::Conditional {
                        to: "transition_test:b".into(),
                        from: "transition_test:a".into(),
                    },
                });
            }
            fn dimension_transition_screen_draw(
                &self,
                _: &dyn mod_api::ClientHost,
                event: &mut mod_api::DimensionTransitionScreenDrawEvent<'_>,
            ) {
                assert_eq!(event.screen, "transition_test:arrival");
                assert_eq!(event.reason, mod_api::DimensionTransitionReason::Other);
                assert_eq!(event.progress, Some(0.25));
                event.draw.fill([0.0, 0.0, 80.0, 10.0], 0xFF102030);
            }
        }
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("transition_test", Box::new(Arrival))].into(),
            || {
                let mut registrations = Vec::new();
                super::Game::native_register_dimension_transition_screen(
                    &mod_api::RegistrationContext::new(),
                    &mut registrations,
                );
                horizon_client_render::dimension_transition::register(registrations);
                with_game(|game| {
                    game.options.execution_role = ExecutionRole::Client;
                    let (client, _server) = horizon_protocol::connection::Pipe::pair();
                    game.connect_local(client, "TransitionTest").unwrap();
                    let mut spawn = horizon_protocol::packets::play::SpawnInfo {
                        dimension_type: 0,
                        dimension: "transition_test:a".into(),
                        seed: 0,
                        game_type: 0,
                        previous_game_type: None,
                        debug: false,
                        flat: true,
                        last_death: None,
                        portal_cooldown: 0,
                        sea_level: 63,
                    };
                    game.enter_remote_dimension(&spawn, false, Reason::Other);
                    assert_eq!(
                        game.remote.as_ref().unwrap().level_load.transition_screen(),
                        None
                    );
                    spawn.dimension = "transition_test:b".into();
                    game.enter_remote_dimension(&spawn, true, Reason::Other);
                    let tracker = game.remote.as_ref().unwrap().level_load;
                    assert!(tracker.transition_screen().is_some());
                    let mut batch = HudBatch::scratch([320, 240]);
                    assert!(game.draw_dimension_transition_screen(
                        &mut batch,
                        &tracker,
                        Some(0.25)
                    ));
                    assert!(!batch.vertices.is_empty());
                    game.remote.as_mut().unwrap().level_load.finish();
                    let tracker = game.remote.as_ref().unwrap().level_load;
                    assert!(!game.draw_dimension_transition_screen(&mut batch, &tracker, None));
                    spawn.dimension = "transition_test:c".into();
                    game.enter_remote_dimension(&spawn, true, Reason::Other);
                    let tracker = game.remote.as_ref().unwrap().level_load;
                    let mut vanilla = HudBatch::scratch([320, 240]);
                    assert!(!game.draw_dimension_transition_screen(&mut vanilla, &tracker, None));
                    game.draw_level_loading_background(&mut vanilla, tracker.reason);
                    assert!(!vanilla.vertices.is_empty());
                });
            },
        );
    }

    /// Spawns the connected client in the overworld as entity 7, profile 42.
    fn spawn_menu_test(game: &mut super::Game<'_>) -> horizon_protocol::packets::play::SpawnInfo {
        use horizon_client::level_loading::Reason;
        use horizon_protocol::packets::play::SpawnInfo;
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, false, Reason::Other);
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        remote.profile = 42;
        spawn
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn client_menu_reads_follow_packets_and_lifetimes() {
        use super::super::{
            Screen, execution_role::ExecutionRole, native_entity_refs::tests::with_game,
        };
        use horizon_client::level_loading::Reason;
        use horizon_core::nbt::Tag;
        use horizon_protocol::{
            connection::Pipe,
            item::Stack,
            packets::play::{self, Clientbound as C},
        };

        with_game(|game| {
            let (mut reader, _writer) = connect_menu_test(game);
            assert!(
                game.local_player()
                    .and_then(|player| game.player_container_menu(player))
                    .is_none()
            );
            let spawn = spawn_menu_test(game);
            let menu = game
                .local_player()
                .and_then(|player| game.player_container_menu(player))
                .unwrap();
            assert_eq!(game.menu_slot_count(menu), Some(46));
            assert_eq!(game.menu_slot(menu, 46), None);
            assert_eq!(game.menu_slot(menu, 36), Some(None));
            let stone = Some(Stack::new(game.item_id("minecraft:stone"), 3));
            game.handle_packet(C::SetPlayerInventory(play::SetPlayerInventory {
                slot: 0,
                item: stone.clone(),
            }))
            .unwrap();
            assert_eq!(game.menu_slot(menu, 36).unwrap().unwrap().count, 3);
            use mod_api::{DragKind, MenuClick, MouseButton, SwapWith};
            let pickup = |slot| MenuClick::Pickup {
                slot: Some(slot),
                button: MouseButton::Left,
            };
            for invalid in [
                pickup(46),
                pickup(usize::MAX),
                MenuClick::Swap {
                    slot: 36,
                    with: SwapWith::Hotbar(9),
                },
                MenuClick::Clone { slot: 36 },
                MenuClick::Drag {
                    kind: DragKind::Clone,
                    slots: vec![36],
                },
                MenuClick::Drag {
                    kind: DragKind::Split,
                    slots: vec![36, usize::MAX],
                },
            ] {
                assert!(!menu.click(game, &[invalid]));
                assert_eq!(menu.slot(game, 36).unwrap().unwrap().count(), 3);
                assert_eq!(menu.carried(game), Some(None));
            }
            game.options.execution_role = ExecutionRole::IntegratedServer;
            assert!(!menu.click(game, &[pickup(36)]));
            game.options.execution_role = ExecutionRole::Client;
            let screen = game.screen;
            game.screen = Screen::Creative;
            assert!(!menu.click(game, &[pickup(36)]));
            game.screen = screen;
            game.remote.as_mut().unwrap().state_id = 17;
            assert!(menu.click(game, &[pickup(36)]));
            let (id, body) = reader.recv().unwrap();
            let play::Serverbound::ContainerClick(sent) =
                play::Serverbound::decode(id, &body).unwrap()
            else {
                panic!("expected the first valid click, not a refused request");
            };
            assert_eq!(
                (
                    sent.container,
                    sent.state,
                    sent.slot,
                    sent.button,
                    sent.input
                ),
                (0, 17, 36, 0, 0)
            );
            assert_eq!(sent.changed.len(), 1);
            assert_eq!(sent.changed[0].0, 36);
            assert!(sent.changed[0].1.is_none());
            assert_eq!(sent.carried.unwrap().count, 3);
            assert_eq!(menu.slot(game, 36), Some(None));
            assert_eq!(menu.carried(game).unwrap().unwrap().count(), 3);
            assert!(!menu.click(game, &[pickup(36), pickup(usize::MAX),]));
            assert_eq!(menu.slot(game, 36), Some(None));
            assert_eq!(menu.carried(game).unwrap().unwrap().count(), 3);
            assert!(menu.click(
                game,
                &[MenuClick::Drag {
                    kind: DragKind::Split,
                    slots: vec![37]
                },]
            ));
            assert_eq!(menu.slot(game, 37).unwrap().unwrap().count(), 3);
            assert_eq!(menu.carried(game), Some(None));
            assert!(menu.click(game, &[pickup(37)]));
            assert!(menu.click(game, &[pickup(36)]));
            assert_eq!(menu.carried(game), Some(None));
            assert_eq!(menu.slot(game, 36).unwrap().unwrap().count(), 3);
            game.handle_packet(C::SetPlayerInventory(play::SetPlayerInventory {
                slot: 0,
                item: Some(Stack::new(game.item_id("minecraft:stone"), 2)),
            }))
            .unwrap();
            assert_eq!(menu.slot(game, 36).unwrap().unwrap().count(), 2);
            assert!(menu.click(game, &[MenuClick::QuickMove { slot: 36 }]));
            assert_eq!(menu.slot(game, 36), Some(None));
            assert_eq!(menu.slot(game, 9).unwrap().unwrap().count(), 2);
            assert!(menu.click(
                game,
                &[MenuClick::Swap {
                    slot: 9,
                    with: SwapWith::Hotbar(1)
                }]
            ));
            assert_eq!(menu.slot(game, 9), Some(None));
            assert_eq!(menu.slot(game, 37).unwrap().unwrap().count(), 2);
            assert!(menu.click(
                game,
                &[MenuClick::Throw {
                    slot: 37,
                    whole_stack: true
                }]
            ));
            assert_eq!(menu.slot(game, 37), Some(None));
            game.handle_packet(C::SetCursorItem(play::SetCursorItem {
                item: stone.clone(),
            }))
            .unwrap();
            assert_eq!(game.menu_carried(menu).unwrap().unwrap().count, 3);
            let crafting_id = game
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:menu")
                .unwrap()
                .id("minecraft:crafting")
                .unwrap();
            let open = || {
                C::OpenScreen(play::OpenScreen {
                    container: 7,
                    menu: crafting_id,
                    title: Tag::String("Crafting".into()),
                })
            };
            game.handle_packet(open()).unwrap();
            assert_eq!(game.menu_slot_count(menu), None);
            assert!(!menu.click(game, &[pickup(36)]));
            let crafting = game
                .local_player()
                .and_then(|player| game.player_container_menu(player))
                .unwrap();
            assert_eq!(game.menu_slot_count(crafting), Some(46));
            game.handle_packet(C::ContainerSetSlot(play::ContainerSetSlot {
                container: 7,
                state: 0,
                slot: 1,
                item: stone.clone(),
            }))
            .unwrap();
            assert_eq!(game.menu_slot(crafting, 1).unwrap().unwrap().count, 3);
            game.handle_packet(C::ContainerSetSlot(play::ContainerSetSlot {
                container: 7,
                state: 0,
                slot: 0,
                item: stone,
            }))
            .unwrap();
            assert_eq!(game.menu_slot(crafting, 0).unwrap().unwrap().count, 3);
            game.handle_packet(open()).unwrap();
            assert_eq!(game.menu_slot_count(crafting), None);
            assert!(!crafting.click(game, &[pickup(1)]));
            let crafting = game
                .local_player()
                .and_then(|player| game.player_container_menu(player))
                .unwrap();
            game.handle_packet(C::Respawn(play::Respawn {
                spawn: spawn.clone(),
                keep: 0,
            }))
            .unwrap();
            assert_eq!(game.menu_slot_count(crafting), None);
            assert!(!crafting.click(game, &[pickup(1)]));
            let inventory = game
                .local_player()
                .and_then(|player| game.player_container_menu(player))
                .unwrap();
            assert_ne!(inventory, menu);
            let chest_id = game
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:menu")
                .unwrap()
                .id("minecraft:generic_9x3")
                .unwrap();
            game.handle_packet(C::OpenScreen(play::OpenScreen {
                container: 7,
                menu: chest_id,
                title: Tag::String("Chest".into()),
            }))
            .unwrap();
            let chest = game
                .local_player()
                .and_then(|player| game.player_container_menu(player))
                .unwrap();
            assert_eq!(game.menu_slot_count(chest), Some(63));
            let stone = Some(Stack::new(game.item_id("minecraft:stone"), 4));
            game.handle_packet(C::ContainerSetSlot(play::ContainerSetSlot {
                container: 7,
                state: 0,
                slot: 2,
                item: stone,
            }))
            .unwrap();
            assert_eq!(game.menu_slot(chest, 2).unwrap().unwrap().count, 4);
            assert_eq!(game.menu_slot(chest, 3), Some(None));
            game.handle_packet(C::SetCursorItem(play::SetCursorItem { item: None }))
                .unwrap();
            assert!(chest.click(game, &[pickup(2)]));
            assert_eq!(chest.slot(game, 2), Some(None));
            assert_eq!(chest.carried(game).unwrap().unwrap().count(), 4);
            assert!(chest.click(game, &[pickup(2)]));
            assert_eq!(chest.slot(game, 2).unwrap().unwrap().count(), 4);
            assert_eq!(chest.carried(game), Some(None));
            for expected in [
                (0, 17, -999, 0, 5),
                (0, 17, 37, 1, 5),
                (0, 17, -999, 2, 5),
                (0, 17, 37, 0, 0),
                (0, 17, 36, 0, 0),
                (0, 17, 36, 0, 1),
                (0, 17, 9, 1, 2),
                (0, 17, 37, 1, 4),
                (7, 0, 2, 0, 0),
                (7, 0, 2, 0, 0),
            ] {
                let sent = (0..32)
                    .find_map(|_| {
                        let (id, body) = reader.recv().unwrap();
                        match play::Serverbound::decode(id, &body).unwrap() {
                            play::Serverbound::ContainerClick(sent) => Some(sent),
                            _ => None,
                        }
                    })
                    .expect("expected click packet");
                assert_eq!(
                    (
                        sent.container,
                        sent.state,
                        sent.slot,
                        sent.button,
                        sent.input
                    ),
                    expected
                );
                if expected == (0, 17, -999, 2, 5) {
                    assert!(sent.carried.is_none());
                    assert_eq!(sent.changed.len(), 1);
                    assert_eq!(sent.changed[0].0, 37);
                    assert_eq!(sent.changed[0].1.as_ref().unwrap().count, 3);
                }
            }
            reader
                .set_read_timeout(Some(std::time::Duration::from_millis(20)))
                .unwrap();
            assert!(reader.recv().unwrap_err().is_timed_out());
            game.remote
                .as_mut()
                .unwrap()
                .menu
                .set(Some((7, "minecraft:unknown".into())));
            assert!(
                game.local_player()
                    .and_then(|player| game.player_container_menu(player))
                    .is_none()
            );
            assert_eq!(game.menu_slot_count(inventory), None);
            assert_eq!(game.menu_slot_count(chest), None);
            game.remote.as_mut().unwrap().menu.set(None);
            let before_replacement = game
                .local_player()
                .and_then(|player| game.player_container_menu(player))
                .unwrap();
            let mut other = spawn;
            other.dimension = "test:other_overworld".into();
            game.enter_remote_dimension(&other, true, Reason::Other);
            assert_eq!(game.menu_slot_count(before_replacement), None);
            assert!(!before_replacement.click(game, &[pickup(36)]));
            let before_logout = game
                .local_player()
                .and_then(|player| game.player_container_menu(player))
                .unwrap();
            game.client_logging_out();
            assert_eq!(game.menu_slot_count(before_logout), None);
            assert!(!before_logout.click(game, &[pickup(36)]));
            assert!(
                game.local_player()
                    .and_then(|player| game.player_container_menu(player))
                    .is_none()
            );
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "MenuReadTest").unwrap();
            game.enter_remote_dimension(&other, false, Reason::Other);
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            let reconnected = game
                .local_player()
                .and_then(|player| game.player_container_menu(player))
                .unwrap();
            assert_eq!(game.menu_slot_count(before_logout), None);
            assert_eq!(game.menu_slot_count(reconnected), Some(46));
            assert_ne!(reconnected, before_logout);
        });
    }

    /// `MenuRef::close` and `click_button` go through the screen's own close
    /// and the button packet, and refuse a menu that is not open or stale.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn client_menu_close_and_button_click_reach_the_server() {
        use super::super::{
            Screen, execution_role::ExecutionRole, native_entity_refs::tests::with_game,
        };
        use horizon_core::nbt::Tag;
        use horizon_protocol::packets::play::{self, Clientbound as C};

        with_game(|game| {
            let (mut reader, _writer) = connect_menu_test(game);
            spawn_menu_test(game);
            let sent = |reader: &mut horizon_protocol::connection::ConnReader| {
                (0..32).find_map(|_| {
                    let (id, body) = reader.recv().ok()?;
                    match play::Serverbound::decode(id, &body).unwrap() {
                        p @ (play::Serverbound::ContainerClose(_)
                        | play::Serverbound::ContainerButtonClick(_)) => Some(p),
                        _ => None,
                    }
                })
            };
            // The inventory menu while playing: no screen to close, no buttons.
            let inventory = game
                .local_player()
                .and_then(|player| game.player_container_menu(player))
                .unwrap();
            assert_eq!(game.screen, Screen::Playing);
            assert!(!inventory.close(game));
            assert!(!inventory.click_button(game, 0));
            let loom = game
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:menu")
                .unwrap()
                .id("minecraft:loom")
                .unwrap();
            let open = |game: &mut super::Game<'_>| {
                game.handle_packet(C::OpenScreen(play::OpenScreen {
                    container: 5,
                    menu: loom,
                    title: Tag::String("Loom".into()),
                }))
                .unwrap();
                game.local_player()
                    .and_then(|player| game.player_container_menu(player))
                    .unwrap()
            };
            let menu = open(game);
            assert_eq!(game.screen, Screen::Inventory);
            // The old inventory handle is stale now that the loom replaced it.
            assert!(!inventory.close(game));
            assert!(!inventory.click_button(game, 0));
            game.options.execution_role = ExecutionRole::IntegratedServer;
            assert!(!menu.click_button(game, 3));
            assert!(!menu.close(game));
            game.options.execution_role = ExecutionRole::Client;
            assert_eq!(game.screen, Screen::Inventory);
            assert!(menu.click_button(game, 3));
            let Some(play::Serverbound::ContainerButtonClick(click)) = sent(&mut reader) else {
                panic!("expected the button click, not a refused request");
            };
            assert_eq!((click.container, click.button), (5, 3));
            assert!(menu.close(game));
            assert_eq!(game.screen, Screen::Playing);
            let Some(play::Serverbound::ContainerClose(close)) = sent(&mut reader) else {
                panic!("expected the close");
            };
            assert_eq!(close.container, 5);
            assert!(game.remote.as_ref().unwrap().menu.as_ref().is_none());
            // Closed: the handle is stale and nothing more is sent.
            assert!(!menu.close(game));
            assert!(!menu.click_button(game, 3));
            // A server-side close leaves a reopened menu's old handle stale too.
            let first = open(game);
            game.handle_packet(C::ContainerClose(play::ContainerClose { container: 5 }))
                .unwrap();
            let second = open(game);
            assert!(!first.close(game));
            assert!(!first.click_button(game, 3));
            assert!(second.close(game));
            reader
                .set_read_timeout(Some(std::time::Duration::from_millis(20)))
                .unwrap();
            let Some(play::Serverbound::ContainerClose(close)) = sent(&mut reader) else {
                panic!("expected the second close");
            };
            assert_eq!(close.container, 5);
            assert!(sent(&mut reader).is_none());
        });
    }

    /// A client names the level it shows by the server's key, even where `Dim` has
    /// no name for it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn client_handles_reach_a_custom_level() {
        use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
        use horizon_client::level_loading::Reason;
        use horizon_protocol::{connection::Pipe, packets::play::SpawnInfo};
        use mod_api::{DimensionKey, LevelRef};
        use std::sync::Arc;

        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            let (client, _server) = Pipe::pair();
            use horizon_world::dimension_type::DimensionFacts;
            let stale_key = horizon_core::LevelKey::parse("test:stale_guest_facts").unwrap();
            let mut stale = DimensionFacts::overworld().clone();
            stale.coordinate_scale = 19.0;
            DimensionFacts::set_levels(vec![(stale_key, stale)]);
            game.connect_local(client, "CustomLevelTest").unwrap();
            assert_eq!(game.dimension_facts_for(stale_key).coordinate_scale, 1.0);
            let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
            let json = serde_json::json!({"min_y": 0, "height": 256,
                "coordinate_scale": 4.0, "ambient_light": 0.3,
                "attributes": {"minecraft:gameplay/fast_lava": true}});
            registries.registries.insert(
                "minecraft:dimension_type".into(),
                vec![(
                    "test:type".into(),
                    Some(horizon_client::scoreboard::json_to_tag(&json)),
                )],
            );
            game.set_registries(Arc::new(registries));
            let spawn = SpawnInfo {
                dimension_type: 0,
                dimension: "test:pocket".into(),
                seed: 0,
                game_type: 1,
                previous_game_type: None,
                debug: false,
                flat: true,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            };
            game.enter_remote_dimension(&spawn, false, Reason::Other);
            game.remote.as_mut().unwrap().logged_in = true;

            let level = game.level("test:pocket").unwrap();
            assert_ne!(level.dimension(), DimensionKey::OVERWORLD);
            assert_eq!(level.dimension().id(game).as_deref(), Some("test:pocket"));
            assert!(game.level("minecraft:overworld").is_none());
            assert!(level.height(game).is_some());
            let facts = level.dimension_type(game).unwrap();
            assert_eq!(facts.id, "test:type");
            assert_eq!(facts.coordinate_scale, 4.0);
            assert_eq!(facts.ambient_light, 0.3);
            assert!(facts.fast_lava);
            assert_eq!(game.dimension_type().coordinate_scale, 4.0);
            let overworld = LevelRef::new(level.epoch(), DimensionKey::OVERWORLD);
            assert_eq!(overworld.height(game), None);
            // An equal JSON body with a different type name must not hit the old cache.
            let mut renamed = (*game.remote.as_ref().unwrap().registries).clone();
            renamed
                .registries
                .get_mut("minecraft:dimension_type")
                .unwrap()[0]
                .0 = "test:renamed".into();
            game.set_registries(Arc::new(renamed));
            game.enter_remote_dimension(&spawn, false, Reason::Other);
            assert_eq!(game.dimension_type().id, "test:renamed");
            // Reject invalid server numbers and enums without failing the join.
            for (field, value) in [
                ("coordinate_scale", serde_json::json!(0.0)),
                ("coordinate_scale", serde_json::json!("NaN")),
                ("coordinate_scale", serde_json::Value::Null),
                ("ambient_light", serde_json::json!("NaN")),
                ("ambient_light", serde_json::Value::Null),
                ("monster_spawn_block_light_limit", serde_json::json!(16)),
                ("monster_spawn_block_light_limit", serde_json::json!(-1)),
                ("skybox", serde_json::json!("invalid")),
            ] {
                let mut invalid = json.clone();
                invalid[field] = value;
                let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
                registries.registries.insert(
                    "minecraft:dimension_type".into(),
                    vec![(
                        "minecraft:the_nether".into(),
                        Some(horizon_client::scoreboard::json_to_tag(&invalid)),
                    )],
                );
                game.set_registries(Arc::new(registries));
                game.enter_remote_dimension(&spawn, false, Reason::Other);
                assert_eq!(game.dimension_type().coordinate_scale, 8.0, "{field}");
                assert!(game.dimension_type().fast_lava);
            }
            let min = f64::from(1.0E-5_f32);
            let max = 30_000_000.0_f64;
            for (value, valid) in [
                (min.next_down(), false),
                (min, true),
                (min.next_up(), true),
                (max.next_down(), true),
                (max, true),
                (max.next_up(), false),
                (f64::from_bits(1), false),
            ] {
                let mut boundary = json.clone();
                boundary["coordinate_scale"] = serde_json::json!(value);
                // Vanilla uses Codec.FLOAT, not a unit-interval ambient-light codec.
                boundary["ambient_light"] = serde_json::json!(1.1);
                let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
                registries.registries.insert(
                    "minecraft:dimension_type".into(),
                    vec![(
                        "minecraft:the_nether".into(),
                        Some(horizon_client::scoreboard::json_to_tag(&boundary)),
                    )],
                );
                game.set_registries(Arc::new(registries));
                game.enter_remote_dimension(&spawn, false, Reason::Other);
                assert_eq!(
                    game.dimension_type().coordinate_scale,
                    if valid { value } else { 8.0 }
                );
                if valid {
                    assert_eq!(game.dimension_type().ambient_light, 1.1);
                }
            }
            for ambient in [1.5_f32, -0.5_f32] {
                let mut legal = json.clone();
                legal["ambient_light"] = serde_json::json!(ambient);
                let block_limit = if ambient > 0.0 { 15 } else { 0 };
                legal["monster_spawn_block_light_limit"] = serde_json::json!(block_limit);
                let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
                registries.registries.insert(
                    "minecraft:dimension_type".into(),
                    vec![(
                        "test:type".into(),
                        Some(horizon_client::scoreboard::json_to_tag(&legal)),
                    )],
                );
                game.set_registries(Arc::new(registries));
                game.enter_remote_dimension(&spawn, false, Reason::Other);
                assert_eq!(game.dimension_type().coordinate_scale, 4.0);
                assert_eq!(game.dimension_type().ambient_light, ambient);
                assert_eq!(
                    game.dimension_type().monster_spawn_block_light_limit,
                    block_limit
                );
                let level = game.level("test:pocket").unwrap();
                assert_eq!(level.dimension_type(game).unwrap().ambient_light, ambient);
            }
            // Disconnect drops the connection-owned table; the server table is untouched.
            let lifetime =
                Arc::downgrade(game.remote.as_ref().unwrap().level_facts.as_ref().unwrap());
            game.leave_server(None).unwrap();
            assert!(
                lifetime.upgrade().is_none(),
                "connection facts must be released"
            );
            assert_eq!(DimensionFacts::of_level(stale_key).coordinate_scale, 19.0);
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "SecondFactsSession").unwrap();
            let key = horizon_core::LevelKey::parse("test:pocket").unwrap();
            assert_eq!(game.dimension_facts_for(key).coordinate_scale, 1.0);
            DimensionFacts::set_levels(Vec::new());
        });
    }

    /// The `both` handle reads answer on a client from its own copy: the local
    /// player's health, effects, equipment and last death, the blocks and time
    /// of day of the level it shows and the entities it tracks.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn client_handle_reads_answer_from_the_clients_copy() {
        use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
        use horizon_client::{
            chunk::Chunk, first_person::Hand, level_loading::Reason, motion::PlayerMotion,
        };
        use horizon_core::geometry::Vec3;
        use horizon_protocol::{
            connection::Pipe,
            packets::play::{self, Clientbound as C, SpawnInfo},
        };
        use mod_api::{EntityRef, EquipmentSlot, GlobalPos};
        use std::sync::Arc;

        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "HandleReadTest").unwrap();
            let spawn = SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 0,
                game_type: 1,
                previous_game_type: None,
                debug: false,
                flat: true,
                last_death: Some(play::GlobalPos {
                    dimension: "minecraft:the_nether".into(),
                    pos: [4, 70, -9],
                }),
                portal_cooldown: 0,
                sea_level: 63,
            };
            game.enter_remote_dimension(&spawn, false, Reason::Other);
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(1.5, 65.0, -2.5)).unwrap());
            game.yaw = 90.0;
            game.pitch = 10.0;

            // The local player and the level it shows.
            let player = game.local_player().unwrap();
            let me = player.entity();
            let living = game.living(me).unwrap();
            let level = game.level("minecraft:overworld").unwrap();
            assert!(game.level("minecraft:the_nether").is_none());
            assert_eq!(me.position(game), Some([1.5, 65.0, -2.5]));
            assert_eq!(me.rotation(game), Some([90.0, 10.0]));
            game.player.as_mut().unwrap().velocity = Vec3::new(0.0, -0.5, 0.25);
            assert_eq!(me.velocity(game), Some([0.0, -0.5, 0.25]));

            // Last death: the login/respawn packet's, as a GlobalPos.
            let died = player.last_death_location(game).unwrap();
            assert_eq!(died.dimension().to_string(), "minecraft:the_nether");
            let pos = died.pos();
            assert_eq!([pos.x, pos.y, pos.z], [4, 70, -9]);
            let _: GlobalPos = died;

            // Health, maximum health, effects and equipment come from the packets.
            game.handle_packet(C::SetHealth(play::SetHealth {
                health: 14.0,
                food: 20,
                saturation: 5.0,
            }))
            .unwrap();
            assert_eq!(living.health(game), Some(14.0));
            assert_eq!(living.max_health(game), Some(20.0));
            let speed = game
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:mob_effect")
                .unwrap()
                .id("minecraft:speed")
                .unwrap();
            assert_eq!(living.effects(game), Some(Vec::new()));
            assert!(
                living
                    .effect(
                        game,
                        mod_platform::__private::part_tests::effect_key("minecraft:speed")
                    )
                    .is_none()
            );
            game.handle_packet(C::UpdateMobEffect(play::UpdateMobEffect {
                id: 7,
                effect: speed,
                amplifier: 1,
                duration: 600,
                flags: 2,
            }))
            .unwrap();
            let effect = living
                .effect(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed"),
                )
                .unwrap();
            assert_eq!((effect.duration, effect.amplifier), (600, 1));
            assert_eq!(living.effects(game).unwrap().len(), 1);
            let stone = Some(horizon_protocol::item::Stack::new(
                game.item_id("minecraft:stone"),
                3,
            ));
            game.handle_packet(C::SetPlayerInventory(play::SetPlayerInventory {
                slot: 0,
                item: stone,
            }))
            .unwrap();
            let held = living.hand_item(game, Hand::Main).unwrap();
            assert_eq!(
                (
                    mod_api::__private::item_key_id(held.item()).unwrap(),
                    held.count()
                ),
                ("minecraft:stone", 3)
            );
            assert_eq!(living.equipment(game, EquipmentSlot::Head), None);
            assert_eq!(living.hand_item(game, Hand::Off), None);

            // Blocks of the chunks the client holds; nothing loads another.
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let chunks = game.world.chunks.len();
            let at = [3, 64, 5];
            assert!(level.is_loaded(game, at));
            assert!(!level.is_loaded(game, [160, 64, 0]));
            game.handle_packet(C::BlockUpdate(play::BlockUpdate { pos: at, state: 1 }))
                .unwrap();
            assert_eq!(
                level
                    .block(game, at)
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:stone")
            );
            assert!(level.block_state(game, at).is_some());
            assert_eq!(level.block(game, [160, 64, 0]), None);
            assert_eq!(level.block_property(game, [160, 64, 0], "facing"), None);
            assert_eq!(game.world.chunks.len(), chunks);

            // The time of day, as the server's clock last set it.
            game.set_registries({
                let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
                registries.registries.insert(
                    "minecraft:world_clock".into(),
                    vec![("minecraft:overworld".into(), None)],
                );
                Arc::new(registries)
            });
            game.handle_packet(C::SetTime(play::SetTime {
                game_time: 100,
                clocks: vec![play::ClockState {
                    clock: 0,
                    total_ticks: 30_000,
                    partial_tick: 0.0,
                    rate: 1.0,
                }],
            }))
            .unwrap();
            assert_eq!(level.day_time(game), Some(30_000));

            // An entity the server told the client of; others read None.
            let pig = game
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id("minecraft:pig")
                .unwrap();
            game.handle_packet(C::AddEntity(play::AddEntity {
                id: 31,
                uuid: 9,
                kind: pig,
                position: [10.0, 64.0, 12.0],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 64,
                head_yaw: 64,
                data: 0,
            }))
            .unwrap();
            let other = game.entity(31).unwrap();
            assert_eq!(other.position(game), Some([10.0, 64.0, 12.0]));
            assert!(other.velocity(game).is_some());
            assert!(other.rotation(game).is_some());
            assert!(game.living(other).is_none(), "only the local player");
            assert!(game.entity(99).is_none());
            let untracked = EntityRef::new(me.epoch(), 99);
            assert_eq!(untracked.position(game), None);
            assert_eq!(untracked.velocity(game), None);

            // A replaced level retires the handles.
            let mut other_level = spawn.clone();
            other_level.dimension = "test:other_overworld".into();
            game.enter_remote_dimension(&other_level, true, Reason::Other);
            assert_eq!(me.position(game), None);
            assert_eq!(living.health(game), None);
            assert_eq!(level.block(game, at), None);
            assert!(!level.is_loaded(game, at));
            assert_eq!(level.day_time(game), None);
            assert_eq!(player.last_death_location(game), None);

            // The server's role keeps its own reads: a client handle names nothing there.
            game.options.execution_role = ExecutionRole::IntegratedServer;
            assert_eq!(game.native_entity_target(untracked), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn client_day_time_reads_overworld_clock_across_packets_and_levels() {
        use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
        use horizon_protocol::{
            connection::Pipe,
            packets::play::{self, Clientbound as C},
        };
        use mod_api::LevelRef;
        use std::sync::Arc;

        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "ClockReadTest").unwrap();
            let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
            let dimensions = vec![
                ("minecraft:overworld".into(), None),
                ("minecraft:the_end".into(), None),
                ("minecraft:the_nether".into(), None),
            ];
            registries
                .registries
                .insert("minecraft:world_clock".into(), dimensions[..2].to_vec());
            registries
                .registries
                .insert("minecraft:dimension_type".into(), dimensions);
            let registries = Arc::new(registries);
            game.set_registries(registries.clone());
            let mut spawn = play::SpawnInfo {
                dimension_type: 1,
                dimension: "minecraft:the_end".into(),
                seed: 0,
                game_type: 1,
                previous_game_type: None,
                debug: false,
                flat: true,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            };
            let login = |spawn| {
                C::Login(play::Login {
                    player_id: 7,
                    hardcore: false,
                    levels: vec!["minecraft:overworld".into(), "minecraft:the_end".into()],
                    max_players: 1,
                    chunk_radius: 2,
                    simulation_distance: 2,
                    reduced_debug_info: false,
                    show_death_screen: true,
                    limited_crafting: false,
                    spawn,
                    online_mode: false,
                    enforces_secure_chat: false,
                })
            };
            let state = |clock, total_ticks, partial_tick, rate| play::ClockState {
                clock,
                total_ticks,
                partial_tick,
                rate,
            };
            game.handle_packet(login(spawn.clone())).unwrap();
            let end = game.level("minecraft:the_end").unwrap();
            let wrong_level = LevelRef::new(end.epoch(), mod_api::DimensionKey::OVERWORLD);
            assert_eq!(wrong_level.day_time(game), None);
            game.handle_packet(C::SetTime(play::SetTime {
                game_time: 100,
                clocks: vec![state(0, 30_000, 0.0, 1.0), state(1, 100, 0.0, 1.0)],
            }))
            .unwrap();
            assert_eq!(end.day_time(game), Some(30_000));
            assert_eq!(
                game.world.day_time, 100,
                "the sky follows the End default clock"
            );

            // An Overworld-only update must not select it as the End sky clock.
            game.handle_packet(C::SetTime(play::SetTime {
                game_time: 104,
                clocks: vec![state(0, 40_000, 0.75, 0.5)],
            }))
            .unwrap();
            assert_eq!(end.day_time(game), Some(40_000));
            assert_eq!(game.world.day_time, 104);
            game.tick_remote_level().unwrap();
            assert_eq!(end.day_time(game), Some(40_001));
            assert_eq!(game.world.day_time, 105);
            game.tick_remote_level().unwrap();
            assert_eq!(end.day_time(game), Some(40_001));
            assert_eq!(game.world.day_time, 106);

            // Empty time syncs advance every retained clock by the game-time delta.
            game.handle_packet(C::SetTime(play::SetTime {
                game_time: 110,
                clocks: vec![],
            }))
            .unwrap();
            assert_eq!(end.day_time(game), Some(40_003));
            assert_eq!(game.world.day_time, 110);
            // A paused clock stays put even while the client's game time runs.
            game.handle_packet(C::SetTime(play::SetTime {
                game_time: 110,
                clocks: vec![state(0, 50_000, 0.25, 0.0)],
            }))
            .unwrap();
            game.tick_remote_level().unwrap();
            assert_eq!(end.day_time(game), Some(50_000));
            assert_eq!(game.world.day_time, 111);

            // A frozen world ticks neither clock; packets still catch both up.
            game.handle_packet(C::SetTime(play::SetTime {
                game_time: 111,
                clocks: vec![state(0, 60_000, 0.0, 2.0)],
            }))
            .unwrap();
            game.handle_packet(C::TickingState(play::TickingState {
                rate: 20.0,
                frozen: true,
            }))
            .unwrap();
            game.tick_remote_level().unwrap();
            assert_eq!(game.world.game_time, 111);
            assert_eq!(end.day_time(game), Some(60_000));
            assert_eq!(game.world.day_time, 111);
            game.handle_packet(C::SetTime(play::SetTime {
                game_time: 114,
                clocks: vec![],
            }))
            .unwrap();
            assert_eq!(end.day_time(game), Some(60_006));
            assert_eq!(game.world.day_time, 114);
            // Backward corrections use the same signed delta and fractional state.
            game.handle_packet(C::SetTime(play::SetTime {
                game_time: 113,
                clocks: vec![],
            }))
            .unwrap();
            assert_eq!(end.day_time(game), Some(60_004));
            assert_eq!(game.world.day_time, 113);
            game.handle_packet(C::TickingState(play::TickingState {
                rate: 20.0,
                frozen: false,
            }))
            .unwrap();

            // Same-level respawn keeps the epoch and clocks, but replaces the
            // local player and its mood handler, with or without entity data.
            for keep in [0, 2] {
                game.ambience.moodiness = 0.375;
                game.handle_packet(C::Respawn(play::Respawn {
                    spawn: spawn.clone(),
                    keep,
                }))
                .unwrap();
                assert_eq!(game.ambience.moodiness, 0.0);
                assert_eq!(end.day_time(game), Some(60_004));
            }
            spawn.dimension_type = 0;
            spawn.dimension = "minecraft:overworld".into();
            game.handle_packet(C::Respawn(play::Respawn {
                spawn: spawn.clone(),
                keep: 0,
            }))
            .unwrap();
            assert_eq!(end.day_time(game), None);
            let overworld = game.level("minecraft:overworld").unwrap();
            assert_eq!(overworld.day_time(game), Some(60_004));
            assert_eq!(
                game.world.day_time, 60_004,
                "select the new default from retained clocks"
            );
            game.tick_remote_level().unwrap();
            assert_eq!(overworld.day_time(game), Some(60_006));
            assert_eq!(game.world.day_time, 60_006);

            // A dimension without a default clock must not select the prior one.
            spawn.dimension_type = 2;
            spawn.dimension = "minecraft:the_nether".into();
            game.handle_packet(C::Respawn(play::Respawn {
                spawn: spawn.clone(),
                keep: 0,
            }))
            .unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            assert_eq!(overworld.day_time(game), None);
            assert_eq!(nether.day_time(game), Some(60_006));
            assert_eq!(game.world.day_time, 0);

            // Login starts a new listener epoch, even on the same connection.
            game.handle_packet(login(spawn.clone())).unwrap();
            assert_eq!(nether.day_time(game), None);
            let fresh = game.level("minecraft:the_nether").unwrap();
            assert_eq!(fresh.day_time(game), Some(0));
            game.handle_packet(C::SetTime(play::SetTime {
                game_time: 1,
                clocks: vec![],
            }))
            .unwrap();
            assert_eq!(
                fresh.day_time(game),
                Some(1),
                "the new clock anchor and rate are fresh too"
            );
            game.handle_packet(C::SetTime(play::SetTime {
                game_time: 1,
                clocks: vec![state(0, 7_000, 0.0, 1.0)],
            }))
            .unwrap();
            assert_eq!(fresh.day_time(game), Some(7_000));
            assert_eq!(game.world.day_time, 0);
            game.client_logging_out();
            assert_eq!(fresh.day_time(game), None);
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "ClockReadTest").unwrap();
            assert_eq!(fresh.day_time(game), None);
            game.set_registries(registries);
            game.handle_packet(login(spawn)).unwrap();
            let reconnected = game.level("minecraft:the_nether").unwrap();
            assert_eq!(fresh.day_time(game), None);
            assert_eq!(reconnected.day_time(game), Some(0));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn client_camera_entity_returns_to_player_after_removal() {
        use super::super::{
            Game, native_entity_refs::tests::with_client_game, native_lifecycle::NativeLifecycle,
        };
        use horizon_client::{level_loading::Reason, motion::PlayerMotion};
        use horizon_core::geometry::Vec3;
        use horizon_protocol::{
            connection::Pipe,
            packets::play::{self, Clientbound as C, SpawnInfo},
        };
        use mod_api::ClientHost;

        with_client_game(|game| {
            let camera = |game: &Game<'_>| (game as &dyn ClientHost).camera_entity();
            assert!(camera(game).is_none(), "the fixture has no player");
            game.native_lifecycle = NativeLifecycle::default();
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "CameraReadTest").unwrap();
            let mut spawn = SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 0,
                game_type: 3,
                previous_game_type: None,
                debug: false,
                flat: true,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            };
            game.enter_remote_dimension(&spawn, false, Reason::Other);
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            assert!(camera(game).is_none(), "a level alone has no player");
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let local = camera(game).unwrap();
            assert_eq!(
                local.id(),
                7,
                "use the remote identity, not PlayerMotion's default"
            );
            assert_eq!(local.position(game), Some([0.0; 3]));

            let creeper = game
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id("minecraft:creeper")
                .unwrap();
            game.handle_packet(C::AddEntity(play::AddEntity {
                id: 31,
                uuid: 9,
                kind: creeper,
                position: [10.0, 64.0, 12.0],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            }))
            .unwrap();
            game.remote_set_camera(31);
            assert!(!game.controls_camera());
            assert!(game.hides_as_camera(31));
            assert!(game.spectated_post_effect.is_some());
            let attached = camera(game).unwrap();
            game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![99] }))
                .unwrap();
            assert_eq!(camera(game), Some(attached));
            assert_eq!(attached.id(), 31);
            assert_eq!(attached.position(game), Some([10.0, 64.0, 12.0]));
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 99 }))
                .unwrap();
            assert_eq!(
                camera(game),
                Some(attached),
                "unknown packet targets are ignored"
            );
            game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![31] }))
                .unwrap();
            assert_eq!(camera(game), Some(local));
            assert!(
                game.controls_camera(),
                "movement input and position packets are enabled"
            );
            assert!(!game.hides_as_camera(31));
            assert!(game.spectated_post_effect.is_none());
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 7 }))
                .unwrap();
            assert_eq!(camera(game), Some(local));

            game.title = Some(game.new_title(game.save.root.clone()));
            assert!(
                camera(game).is_none(),
                "the title screen has no camera entity"
            );
            game.title = None;
            assert_eq!(camera(game), Some(local));

            spawn.dimension = "test:other_overworld".into();
            game.handle_packet(C::Respawn(play::Respawn { spawn, keep: 0 }))
                .unwrap();
            assert!(
                camera(game).is_none(),
                "replacement waits for its new player"
            );
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let fresh = camera(game).unwrap();
            assert_eq!(fresh.id(), local.id());
            assert_ne!(fresh.epoch(), local.epoch());
            assert_eq!(local.position(game), None, "old level handles stay stale");
            assert_eq!(fresh.position(game), Some([0.0; 3]));
            game.client_logging_out();
            assert!(camera(game).is_none());
            assert_eq!(fresh.position(game), None);
        });
    }

    /// The local player's own state reads answer on a client from what the
    /// server sent it (`LocalPlayer`); another player reads None there.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn client_player_reads_answer_from_the_local_players_copy() {
        use super::super::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
        use horizon_client::{level_loading::Reason, motion::PlayerMotion};
        use horizon_core::geometry::Vec3;
        use horizon_protocol::{
            connection::Pipe,
            packets::play::{self, Clientbound as C, SpawnInfo},
        };
        use mod_api::{EntityRef, GameMode};

        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "PlayerReadTest").unwrap();
            let spawn = SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 0,
                game_type: 0,
                previous_game_type: None,
                debug: false,
                flat: true,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            };
            game.enter_remote_dimension(&spawn, false, Reason::Other);
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 65.0, 0.5)).unwrap());
            let player = game.local_player().unwrap();

            // Survival, as the login packet left it: building allowed, no flight.
            assert_eq!(player.game_mode(game), Some(GameMode::Survival));
            assert_eq!(player.may_build(game), Some(true));
            assert_eq!(player.abilities(game).map(|a| a.may_fly), Some(false));

            // CHANGE_GAME_MODE to adventure: no building; then the abilities packet.
            game.handle_packet(C::GameEvent(play::GameEvent {
                event: 3,
                param: 2.0,
            }))
            .unwrap();
            assert_eq!(player.game_mode(game), Some(GameMode::Adventure));
            assert_eq!(player.is_creative(game), Some(false));
            assert_eq!(player.is_spectator(game), Some(false));
            assert_eq!(player.may_build(game), Some(false));
            game.handle_packet(C::PlayerAbilities(play::PlayerAbilities {
                flags: 4 | 2,
                flying_speed: 0.1,
                walking_speed: 0.2,
            }))
            .unwrap();
            let abilities = player.abilities(game).unwrap();
            assert!(abilities.may_fly && abilities.flying && !abilities.invulnerable);
            assert!(!abilities.may_build);
            assert_eq!((abilities.fly_speed, abilities.walk_speed), (0.1, 0.2));

            // Food from SetHealth; the client keeps no exhaustion.
            game.handle_packet(C::SetHealth(play::SetHealth {
                health: 20.0,
                food: 5,
                saturation: 1.5,
            }))
            .unwrap();
            let food = player.food(game).unwrap();
            assert_eq!(
                (food.level, food.saturation, food.exhaustion),
                (5, 1.5, 0.0)
            );
            assert_eq!(player.needs_food(game), Some(true));
            assert_eq!(player.has_enough_food(game), Some(false));

            // The hotbar slot and the experience the server set.
            game.handle_packet(C::SetHeldSlot(play::SetHeldSlot { slot: 4 }))
                .unwrap();
            assert_eq!(player.selected_slot(game), Some(4));
            game.handle_packet(C::SetExperience(play::SetExperience {
                progress: 0.25,
                level: 3,
                total: 30,
            }))
            .unwrap();
            let xp = player.experience(game).unwrap();
            assert_eq!((xp.level, xp.progress, xp.total), (3, 0.25, 30));
            assert_eq!(player.xp_needed_for_next_level(game), Some(13));

            // The main inventory retains typed stacks and their damage, in slot order.
            let mut stack = horizon_client::items::ItemStack::new("minecraft:diamond_pickaxe", 1);
            stack.damage = 7;
            game.inventory.fill(None);
            game.inventory[3] = Some(stack.clone());
            game.inventory[8] = Some(horizon_client::items::ItemStack::new("minecraft:stone", 12));
            assert_eq!(
                player.inventory(game),
                Some(
                    vec![
                        stack,
                        horizon_client::items::ItemStack::new("minecraft:stone", 12)
                    ]
                    .try_into_api()
                    .unwrap()
                )
            );

            // Another player: the client isn't sent their state.
            let epoch = player.entity().epoch();
            let other = mod_api::__private::game::player_ref(EntityRef::new(epoch, 8), 43);
            assert_eq!(other.game_mode(game), None);
            assert_eq!(other.food(game), None);
            assert_eq!(other.selected_slot(game), None);
            assert_eq!(other.inventory(game), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn client_epochs_follow_level_replacement_and_logout() {
        use super::super::{
            execution_role::ExecutionRole, native_entity_refs::tests::with_game,
            native_lifecycle::NativeLifecycle,
        };
        use horizon_client::{level_loading::Reason, mobs::Mob, motion::PlayerMotion};
        use horizon_core::geometry::Vec3;
        use horizon_protocol::{
            connection::Pipe,
            packets::play::{Clientbound, Respawn, SpawnInfo},
        };
        use mod_api::{CrosshairTarget, EntityRef};

        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            game.native_lifecycle = NativeLifecycle::default();
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "EpochTest").unwrap();
            // A transport alone has no ClientLevel yet.
            assert!(game.native_lifecycle.epoch().is_none());
            let mut spawn = SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 0,
                game_type: 1,
                previous_game_type: None,
                debug: false,
                flat: true,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            };
            game.enter_remote_dimension(&spawn, false, Reason::Other);
            let joined = game.native_lifecycle.epoch().unwrap();
            let old = EntityRef::new(joined, 12);
            let remote = game.remote.as_mut().unwrap();
            remote.menu.set(Some((7, "minecraft:generic_9x3".into())));
            let open_generation = remote.menu.generation;
            game.handle_packet(Clientbound::Respawn(Respawn {
                spawn: spawn.clone(),
                keep: 0,
            }))
            .unwrap();
            assert_eq!(game.native_lifecycle.epoch(), Some(joined));
            let remote = game.remote.as_mut().unwrap();
            assert!(remote.menu.as_ref().is_none());
            assert_ne!(remote.menu.generation, open_generation);
            let inventory_generation = remote.menu.generation;
            game.handle_packet(Clientbound::Respawn(Respawn {
                spawn: spawn.clone(),
                keep: 0,
            }))
            .unwrap();
            assert_eq!(game.native_lifecycle.epoch(), Some(joined));
            let remote = game.remote.as_mut().unwrap();
            assert!(remote.menu.as_ref().is_none());
            assert_ne!(remote.menu.generation, inventory_generation);
            remote.menu.set(Some((7, "minecraft:generic_9x3".into())));
            assert_ne!(remote.menu.generation, open_generation);
            assert_ne!(remote.menu.generation, inventory_generation);

            // A different level key replaces the world even with the same
            // dimension type; IDs can be reused but old handles cannot.
            spawn.dimension = "test:other_overworld".into();
            game.enter_remote_dimension(&spawn, true, Reason::Other);
            let replaced = game.native_lifecycle.epoch().unwrap();
            assert_ne!(replaced, joined);
            let current = EntityRef::new(replaced, 12);
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs
                .push(Mob::new(12, kind, Vec3::new(0.0, 1.0, 2.0), 0.0));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.yaw = 0.0;
            game.pitch = 0.0;
            let Some(CrosshairTarget::Entity { entity, .. }) = game.crosshair_target() else {
                panic!("the crosshair hits the pig in front of the player");
            };
            assert_eq!(entity, current);
            assert!(
                game.native_entity_target(current).is_none(),
                "still server-only"
            );
            // Isolate the existing epoch check from the logical-server guard.
            game.options.execution_role = ExecutionRole::IntegratedServer;
            assert!(game.native_entity_target(current).is_some());
            assert!(game.native_entity_target(old).is_none());
            game.options.execution_role = ExecutionRole::Client;

            game.client_logging_out();
            assert!(game.native_lifecycle.epoch().is_none());
            assert!(game.crosshair_target().is_none());
            game.client_logging_out();
            assert!(game.native_lifecycle.epoch().is_none());
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "EpochTest").unwrap();
            assert!(game.native_lifecycle.epoch().is_none());
            game.enter_remote_dimension(&spawn, false, Reason::Other);
            assert_ne!(game.native_lifecycle.epoch(), Some(replaced));
            game.client_logging_out();
            assert!(game.native_lifecycle.epoch().is_none());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn connection_and_dimension_replacement_clear_every_particle_group() {
        use super::super::{
            execution_role::ExecutionRole, native_entity_refs::tests::with_game,
            native_hooks::particle_count_tests::fill,
        };
        use horizon_client::level_loading::Reason;
        use horizon_protocol::{connection::Pipe, packets::play::SpawnInfo};
        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            fill(game);
            // Include pending particles as well as live groups before replacing the connection.
            game.particles.elder_guardian();
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "ParticleReset").unwrap();
            assert_eq!(game.client_particle_count(), 0);
            assert!(game.pickups.live.is_empty());
            game.tick_particles();
            assert_eq!(game.client_particle_count(), 0);
            let spawn = SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 0,
                game_type: 1,
                previous_game_type: None,
                debug: false,
                flat: true,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            };
            fill(game);
            game.particles.elder_guardian();
            game.enter_remote_dimension(&spawn, false, Reason::Other);
            assert_eq!(game.client_particle_count(), 0);
            assert!(game.pickups.live.is_empty());
            game.tick_particles();
            assert_eq!(
                game.client_particle_count(),
                0,
                "pending groups cannot return in the new level"
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn forgetting_and_closing_remote_columns_clear_their_data() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::{chunk::Chunk, containers::BlockEntity};

        with_game(|game| {
            for x in 0..2 {
                game.streaming
                    .insert_remote(&mut game.world, Chunk::empty(x, 0));
                game.world
                    .block_entities
                    .insert((x * 16, 64, 0), BlockEntity::Chest(vec![]));
            }
            // A same-position resend replaces the data without removing the column.
            game.streaming
                .replace_remote(&mut game.world, Chunk::empty(0, 0));
            assert_eq!(game.world.chunks.len(), 2);
            assert!(game.world.chunks.contains_key(&(0, 0)));
            assert!(game.streaming.is_lit(0, 0));
            let mut forgotten = [(0, 0)].into_iter().collect();
            game.forget_remote_columns(&mut forgotten, true);
            assert!(!game.world.chunks.contains_key(&(0, 0)));
            assert!(!game.world.block_entities.contains_key(&(0, 64, 0)));
            assert!(!game.streaming.is_lit(0, 0));
            assert!(game.world.chunks.contains_key(&(1, 0)));
            game.forget_remote_columns(&mut [(0, 0)].into_iter().collect(), true);
            for x in [7, 13] {
                game.streaming
                    .insert_remote(&mut game.world, Chunk::empty(x, 0));
                game.world
                    .block_entities
                    .insert((x * 16, 64, 0), BlockEntity::Chest(vec![]));
            }
            // Cache-centre packet: move to (15, 0), at radius 10.
            game.drop_remote_chunks_out_of_range((15, 0), 10);
            assert!(!game.world.chunks.contains_key(&(1, 0)));
            assert!(game.world.chunks.contains_key(&(7, 0)));
            assert!(game.world.chunks.contains_key(&(13, 0)));
            // Cache-radius packet: shrinking to 2 keeps radius 5 around (15, 0).
            game.drop_remote_chunks_out_of_range((15, 0), 2);
            assert!(!game.world.chunks.contains_key(&(7, 0)));
            assert!(game.world.chunks.contains_key(&(13, 0)));
            assert!(!game.world.block_entities.contains_key(&(7 * 16, 64, 0)));
            // A later forget for an evicted column is harmless.
            game.forget_remote_columns(&mut [(7, 0)].into_iter().collect(), true);
            game.forget_all_remote_columns();
            assert!(game.world.chunks.is_empty());
            assert!(game.world.block_entities.is_empty());
        });
    }

    /// The client chunk seam (game/network/remote_chunks.rs) across every free
    /// route, as the freed set each route returns: a forget packet, the
    /// eager centre-move drop (a cache-centre packet), the radius-shrink
    /// drop (a cache-radius packet) and the client level going. One freed
    /// entry per column, in the order the routes free them, and a repeat
    /// forget for an already-freed column frees nothing: adopting a column
    /// is a chunk packet's doing (`remote_chunk`), not the streaming
    /// level's.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn every_free_route_returns_the_columns_it_freed_once() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::chunk::Chunk;

        with_game(|game| {
            for x in [0, 1] {
                game.streaming
                    .insert_remote(&mut game.world, Chunk::empty(x, 0));
            }
            // A forget packet frees its column once; the same forget again,
            // for a column already freed, frees nothing.
            let mut gone = [(0, 0)].into_iter().collect();
            assert_eq!(game.forget_remote_columns(&mut gone, true), [(0, 0)]);
            let mut gone = [(0, 0)].into_iter().collect();
            assert!(game.forget_remote_columns(&mut gone, true).is_empty());
            for x in [7, 13] {
                game.streaming
                    .insert_remote(&mut game.world, Chunk::empty(x, 0));
            }
            // The centre move to (15, 0) at radius 10 frees (1, 0) only.
            assert_eq!(game.drop_remote_chunks_out_of_range((15, 0), 10), [(1, 0)]);
            // Shrinking the radius to 2 frees (7, 0) only.
            assert_eq!(game.drop_remote_chunks_out_of_range((15, 0), 2), [(7, 0)]);
            // The client level going frees what is left.
            assert_eq!(game.forget_all_remote_columns(), [(13, 0)]);
            assert!(game.world.chunks.is_empty());
            assert!(game.world.block_entities.is_empty());
        });
    }

    /// The seam sits at the packet level: streaming adopting, replacing or
    /// re-sending a column keeps it resident — a re-sent position loads
    /// again (`remote_chunk`), never an unload and a load — so a forget
    /// returns exactly it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn streaming_adoption_and_resend_stay_resident_until_the_forget() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::chunk::Chunk;

        with_game(|game| {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(0, 0));
            // A re-send of a resident, lit column takes replace_remote.
            game.streaming
                .replace_remote(&mut game.world, Chunk::empty(0, 0));
            assert!(game.world.chunks.contains_key(&(0, 0)));
            let mut gone = [(0, 0)].into_iter().collect();
            assert_eq!(game.forget_remote_columns(&mut gone, true), [(0, 0)]);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn joining_a_server_empties_the_off_hand_too() {
        // ClientPacketListener.handleLogin makes a new LocalPlayer, whose whole Inventory is
        // empty: a server that never sends slot 45 must not leave the last server's item.
        use crate::game::{
            Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
        };
        use horizon_client::items::ItemStack;
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::Client;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        let (first, _first_server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(first, "HopTest").unwrap();
        let stack = || Some(ItemStack::new("minecraft:shield", 1));
        game.inventory[0] = stack();
        game.armor[0] = stack();
        game.offhand = stack();
        let (second, _second_server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(second, "HopTest").unwrap();
        assert!(game.inventory[0].is_none() && game.armor[0].is_none());
        assert!(game.offhand.is_none());
    }

    #[test]
    fn the_chunk_storage_takes_any_radius_and_centre_a_server_sends() {
        use super::in_storage_range;
        // Vanilla's storage: radius at least 2, plus 3, each way from the centre.
        assert!(in_storage_range(10, (0, 0), 13, -13));
        assert!(!in_storage_range(10, (0, 0), 14, 0));
        assert!(in_storage_range(-5, (100, 100), 105, 95));
        assert!(!in_storage_range(0, (100, 100), 106, 100));
        // Out of i32's range, as a server may send: no overflow, and the sums a wrap got wrong.
        assert!(in_storage_range(i32::MAX, (0, 0), i32::MAX, i32::MIN + 1));
        assert!(!in_storage_range(10, (i32::MIN, 0), i32::MAX, 0));
        assert!(in_storage_range(
            10,
            (i32::MIN, i32::MAX),
            i32::MIN + 13,
            i32::MAX - 13
        ));
    }

    use super::{
        boat_interact_passes, chat_after_event, client_has_level, hold_server_fire,
        item_frame_interact_passes, keep_client_state, open_hash_order, remote_object_swings,
        remote_use_swings, shape_meets, snap_target, synced_local_use,
    };

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and scratch homes/cwd"]
    fn tracking_emitters_follow_the_packet_target() {
        use crate::game::{
            Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole,
            load_resources, totem::TOTEM_EVENT,
        };
        use horizon_client::{motion::PlayerMotion, particles::ParticleType, pose::Pose};
        use horizon_core::geometry::Vec3;
        use horizon_protocol::{
            entity_data::{Value, serializer},
            packets::play::{self, Clientbound as C},
        };

        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::Client;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        let (client, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(client, "EmitterTest").unwrap();
        game.remote.as_mut().unwrap().entity_id = 7;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap());
        game.particles.seed_randoms(42, 43, 44);

        let check = |game: &Game<'_>, ty, feet: [f64; 3], height: f32| {
            let mut seen = false;
            for (options, at, velocity) in game.particles.requested.as_ref().unwrap() {
                if options.ty != ty {
                    continue;
                }
                seen = true;
                let y = feet[1] + f64::from(height) * (0.5 + (velocity[1] - 0.2) / 4.0);
                assert!((at[0] - feet[0]).abs() <= f64::from(0.6_f32) / 4.0);
                assert!(
                    (at[1] - y).abs() < 1e-10,
                    "{ty:?} at {at:?}, expected y {y}"
                );
                assert!((at[2] - feet[2]).abs() <= f64::from(0.6_f32) / 4.0);
            }
            assert!(seen, "no {ty:?} emissions");
        };
        let types = [
            ParticleType::Crit,
            ParticleType::EnchantedHit,
            ParticleType::TotemOfUndying,
        ];
        let target = 40;
        let start = [5.0, 64.0, 7.0];
        game.spawn_entity(target, "minecraft:player", start, [0.0; 3]);
        game.particles.requested = Some(Vec::new());
        for action in [1, 2] {
            game.handle_packet(C::Animate(play::Animate { id: target, action }))
                .unwrap();
        }
        game.handle_packet(C::EntityEvent(play::EntityEvent {
            id: target,
            event: TOTEM_EVENT,
        }))
        .unwrap();
        for ty in types {
            check(&game, ty, start, 1.8);
        }

        let moved = [15.0, 65.0, -2.0];
        game.particles.requested = Some(Vec::new());
        game.handle_packet(C::SetEntityData(play::SetEntityData {
            id: target,
            values: vec![(6, serializer::POSE, Value::Id(Pose::Crouching.id()))],
        }))
        .unwrap();
        game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
            id: target,
            path: play::PositionPath::Linear(moved),
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
        }))
        .unwrap();
        for _ in 0..4 {
            game.tick_remote_entities(true);
        }
        game.tick_particles();
        for ty in types {
            check(&game, ty, moved, 1.5);
        }

        // The old entity moved again before unpairing. Re-pairing its id before
        // the next particle tick keeps emitting at the old entity's final body.
        game.particles.requested = Some(Vec::new());
        let last = [18.0, 65.0, -2.0];
        game.mob_index
            .get_mut(&mut game.mobs, target as u32)
            .unwrap()
            .position = Vec3::new(last[0], last[1], last[2]);
        game.handle_packet(C::RemoveEntities(play::RemoveEntities {
            ids: vec![target],
        }))
        .unwrap();
        game.spawn_entity(target, "minecraft:player", [28.0, 67.0, 1.0], [0.0; 3]);
        game.tick_particles();
        for ty in types {
            check(&game, ty, last, 1.5);
        }

        game.particles.clear();
        game.particles.requested = Some(Vec::new());
        game.handle_packet(C::EntityEvent(play::EntityEvent {
            id: 7,
            event: TOTEM_EVENT,
        }))
        .unwrap();
        check(&game, ParticleType::TotemOfUndying, [0.0, 64.0, 0.0], 1.8);
        game.particles.requested = Some(Vec::new());
        let player = game.player.as_mut().unwrap();
        player.position = Vec3::new(-4.0, 68.0, 8.0);
        player.set_pose(Pose::Swimming).unwrap();
        game.tick_particles();
        check(&game, ParticleType::TotemOfUndying, [-4.0, 68.0, 8.0], 0.6);
    }

    /// The vanilla 26.3 client's ServerboundContainerClickPacket for a shift-click from hotbar slot 37
    /// into slot 10 listed 37 first (wire stream harness, `inventory`). The key 0 always leads.
    #[test]
    fn changed_slots_go_out_in_the_order_vanilla_s_hash_map_iterates() {
        assert_eq!(open_hash_order(&[10, 37]), [37, 10]);
        assert_eq!(open_hash_order(&[0, 1, 2, 36, 44])[0], 0);
        // Past 24 keys the table grows; every key is still listed once.
        let all: Vec<i16> = (0..46).collect();
        let mut out = open_hash_order(&all);
        out.sort_unstable();
        assert_eq!(out, all);
    }

    #[test]
    fn typed_chat_is_sent_as_the_client_chat_event_leaves_it() {
        assert_eq!(chat_after_event("hello", |_, _| {}), Some("hello".into()));
        assert_eq!(
            chat_after_event("hello", |m, _| m.push_str(" world")),
            Some("hello world".into())
        );
        assert_eq!(chat_after_event("hello", |_, c| *c = true), None);
        assert_eq!(chat_after_event("hello", |m, _| m.clear()), None);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn block_alias_survival_consumption_and_client_prediction() {
        use crate::game::{Audio, Game, Options, Progress, Settings, World, load_resources};
        use horizon_client::{
            chunk::Chunk,
            items::ItemStack,
            mod_content::{ContentMod, build},
        };
        use horizon_mod_package::content::{Block, Content, Item};
        use std::{path::PathBuf, sync::Arc};
        let base = PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
        );
        let root = std::env::temp_dir().join(format!("horizon-alias-game-{}", std::process::id()));
        let mut content = Content::declared("ruby");
        content
            .declare(
                &[Block {
                    id: "ruby:frame".into(),
                    template: "minecraft:stone".into(),
                    item: false,
                    ..Default::default()
                }],
                &["empty", "filled"].map(|name| Item {
                    id: format!("ruby:{name}"),
                    template: "minecraft:stick".into(),
                    block: Some("ruby:frame".into()),
                    ..Default::default()
                }),
            )
            .unwrap();
        let mods = [ContentMod {
            id: "ruby".into(),
            content,
            jars: Vec::new(),
        }];
        let inventory = horizon_mod_package::enabled::EnabledMods::new([(
            "ruby".to_owned(),
            mod_api::Side::Both,
        )]);
        build(&base, std::path::Path::new(""), &mods, &root, &inventory).unwrap();
        let (registry, _) = horizon_world::level::data::load_registry_with_flags(&root).unwrap();
        crate::game::api_values::register_game_keys((0..registry.len()).map(|id| {
            registry
                .state(horizon_world::registry::BlockStateId(id as u32))
                .unwrap()
                .name
                .clone()
        }));
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.world = World::new(game.res.facts().air, 1);
        for x in -1..=1 {
            for z in -1..=1 {
                game.world
                    .chunks
                    .insert((x, z), Arc::new(Chunk::empty(x, z)));
            }
        }
        game.sync_tick_chunks();
        game.set_mode_id(0);
        game.player = None;
        game.selected = 0;
        let target = game
            .pack
            .assets
            .registry
            .default_state("ruby:frame")
            .unwrap();
        for (i, name) in ["ruby:empty", "ruby:filled"].iter().enumerate() {
            let pos = [i as i32 * 2, 70, 0];
            let hit = crate::game::Hit {
                pos,
                face: [0, 1, 0],
                inside: false,
                point: [f64::from(pos[0]) + 0.5, 70.0, 0.5],
            };
            let stack = ItemStack::new(name, 2);
            assert_eq!(game.items().get(name).unwrap().block, Some(target));
            let preview = game
                .remote_can_place(&hit, &stack)
                .expect("client predicts the alias");
            assert_eq!([preview.0.x, preview.0.y, preview.0.z], pos);
            assert_eq!(
                game.world.block(pos[0], pos[1], pos[2]),
                game.res.facts().air,
                "prediction is dry"
            );
            *game.held_mut() = Some(stack);
            assert!(game.use_hand(Some(&hit)).unwrap());
            assert_eq!(game.world.block(pos[0], pos[1], pos[2]), target);
            let held = game.held().unwrap();
            assert_eq!((&*held.item, held.count), (*name, 1));
        }
        let blocked = crate::game::Hit {
            pos: [0, 70, 0],
            face: [0, 1, 0],
            inside: false,
            point: [0.5, 71.0, 0.5],
        };
        game.set_mode_id(2);
        let before = game.held().cloned();
        game.use_hand(Some(&blocked)).unwrap();
        assert_eq!(
            game.held(),
            before.as_ref(),
            "adventure refusal keeps the alias stack"
        );
        drop(game);
        std::fs::remove_dir_all(root).unwrap();
    }

    use horizon_client::{
        containers::{Beacon, BlockEntity},
        items::ItemStack,
    };
    use horizon_core::nbt::Tag;

    /// #12040: a server's client moves at the speed the server's attributes give it, never its
    /// effects' own. Vanilla's server removes a Speed that runs out in ServerPlayer.doTick, after
    /// the tick's sendDirtyEntityData: ClientboundRemoveMobEffectPacket goes out at once and the
    /// movement speed without the effect's modifier a tick later, so for that tick the client still
    /// runs fast (a potion drunk starts the same way). A client a tick off is what GrimAC's
    /// Simulation flagged.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_servers_client_takes_its_speed_from_the_attributes_not_the_effect() {
        use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        use horizon_protocol::packets::play::{
            AttributeModifier, AttributeSnapshot, Clientbound as C, RemoveMobEffect,
            UpdateAttributes, UpdateMobEffect,
        };
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::Client,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.init_remote_for_replay().unwrap();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        let me = game.remote.as_ref().unwrap().entity_id;
        let speed = game.remote_effect_network_id("speed").unwrap();
        let attribute = {
            let p = game.player.as_ref().unwrap();
            let key = p.attributes.key("minecraft:movement_speed").unwrap();
            key.attribute().registry_id()
        };
        // Player.createAttributes: 0.1F.
        let walking = f64::from(0.1f32);
        // Speed II: 0.2 (a float) a level, multiplying the total.
        let amount = f64::from(0.2f32) * 2.0;
        let attributes = |sped: bool| {
            C::UpdateAttributes(UpdateAttributes {
                id: me,
                values: vec![AttributeSnapshot {
                    attribute,
                    base: walking,
                    modifiers: sped
                        .then(|| AttributeModifier {
                            id: "minecraft:effect.speed".into(),
                            amount,
                            operation: 2,
                        })
                        .into_iter()
                        .collect(),
                }],
            })
        };
        // Each step: the packets of one server tick, then a client tick.
        let mut step = |packets: Vec<C>| {
            for packet in packets {
                game.handle_packet(packet).unwrap();
            }
            game.tick_client_effects();
            game.player
                .as_mut()
                .unwrap()
                .attribute("minecraft:movement_speed")
                .unwrap()
        };
        let effect = C::UpdateMobEffect(UpdateMobEffect {
            id: me,
            effect: speed,
            amplifier: 1,
            duration: 60,
            flags: 2 | 4,
        });
        let fast = walking * (1.0 + amount);
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let seen = [
            step(vec![effect]),
            step(vec![attributes(true)]),
            step(Vec::new()),
            step(vec![C::RemoveMobEffect(RemoveMobEffect {
                id: me,
                effect: speed,
            })]),
            step(vec![attributes(false)]),
        ];
        let expected = [walking, fast, fast, fast, walking];
        assert!(
            seen.iter().zip(expected).all(|(&s, e)| close(s, e)),
            "speed on each tick: {seen:?}, vanilla's client: {expected:?}"
        );
    }

    /// `ItemFrame.interact`, `AbstractBoat.interact`, and `Minecart.interact`:
    /// interactions that pass leave the click to the held item without swinging;
    /// vehicle boarding answers CONSUME and ends the click without a predicted swing (#9120).
    #[test]
    fn object_interactions_pass_and_swing_vanilla() {
        // ItemFrame: fixed frame or empty frame + empty hand passes.
        assert!(item_frame_interact_passes(true, true, true));
        assert!(item_frame_interact_passes(true, false, false));
        assert!(item_frame_interact_passes(false, false, false));
        // ItemFrame: inserting or rotating item does not pass.
        assert!(!item_frame_interact_passes(false, true, false)); // rotates
        assert!(!item_frame_interact_passes(false, false, true)); // inserts
        assert!(!item_frame_interact_passes(false, true, true)); // rotates

        // Boat: sneaking or out of control passes.
        assert!(boat_interact_passes(false, true, false, true)); // sneaking
        assert!(boat_interact_passes(false, false, true, true)); // out of control
        // Boat: the client answers SUCCESS, room or not (startRiding is the server's).
        assert!(!boat_interact_passes(false, false, false, true));
        assert!(!boat_interact_passes(false, false, false, false));

        // Chest boat: sneaking opens the chest; boarding answers SUCCESS.
        assert!(!boat_interact_passes(true, true, false, true)); // opens chest
        assert!(!boat_interact_passes(true, false, false, false)); // boards (client)
        // Chest boat out of control: passes with room, else opens the chest.
        assert!(boat_interact_passes(true, false, true, true));
        assert!(!boat_interact_passes(true, false, true, false));

        // Swings: boarding a rideable cart answers CONSUME (no swing).
        assert!(!remote_object_swings(None, true));
        // Swings: other object interactions, a boat's boarding too, answer SUCCESS.
        assert!(remote_object_swings(None, false));
        // Interaction entities honor response.
        assert!(remote_object_swings(Some(true), false));
        assert!(!remote_object_swings(Some(false), false));
    }

    /// A mod's pre-filled chat opens only over the world, with its text
    /// typed in and nothing sent.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mod_open_chat_fills_the_box_only_over_the_world() {
        use super::super::{
            Audio, Game, Options, Progress, Screen, Settings, execution_role::ExecutionRole,
            load_resources,
        };

        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES")
                .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!("no menu handoff"),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::Client;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();

        // No player: the title screen.
        game.player = None;
        assert!(game.open_chat("/tp ").is_err());
        assert!(game.ui.chat_screen.is_none());
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(
                game.motion,
                horizon_core::geometry::Vec3::ZERO,
            )
            .unwrap(),
        );
        game.screen = Screen::Paused;
        assert!(game.open_chat("/tp ").is_err());
        assert!(game.ui.chat_screen.is_none());
        game.screen = Screen::Playing;
        assert!(game.open_chat("/tp ").is_ok());
        assert!(game.screen == Screen::Chat);
        assert_eq!(game.ui.chat_screen.as_ref().unwrap().input.value, "/tp ");
        // The chat is a screen too: a second call doesn't replace what's typed.
        assert!(game.open_chat("hello").is_err());
        assert_eq!(game.ui.chat_screen.as_ref().unwrap().input.value, "/tp ");
    }

    /// The packet riding graph must retain invisible intermediary entities. A custom
    /// sit plugin can use an area-effect cloud, whose server position stops updating
    /// while it rides a player, with a player or marker stand as its own passenger.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn plugin_player_stack_follows_network_moves_and_local_carriers() {
        use super::super::{
            Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
        };
        use horizon_client::{motion::PlayerMotion, sprint::Keys};
        use horizon_core::geometry::Vec3;
        use horizon_protocol::packets::play::{self, Clientbound as C};
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            ExecutionRole::Client,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::Client;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        let (client, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(client, "SeatTest").unwrap();
        game.remote.as_mut().unwrap().entity_id = 7;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.spawn_entity(40, "minecraft:player", [2.0, 64.0, 3.0], [0.0; 3]);
        game.spawn_entity(41, "minecraft:player", [0.0, 64.0, 0.0], [0.0; 3]);
        game.spawn_entity(42, "minecraft:area_effect_cloud", [0.0; 3], [0.0; 3]);
        let set = |g: &mut Game<'_>, vehicle, passengers: Vec<i32>| {
            g.handle_packet(C::SetPassengers(play::SetPassengers {
                vehicle,
                passengers,
            }))
            .unwrap();
        };
        set(&mut game, 40, vec![42]);
        set(&mut game, 42, vec![7, 41]);
        let close = |at: Vec3, x: f64, y: f64, z: f64| {
            assert!(
                (at.x - x).abs() < 1e-5 && (at.y - y).abs() < 1e-5 && (at.z - z).abs() < 1e-5,
                "position {at:?}, expected [{x}, {y}, {z}]"
            );
        };
        assert!(game.remote_ride(Keys::default(), 0).unwrap());
        close(game.player.as_ref().unwrap().position, 2.0, 65.7, 3.0);
        close(
            game.mobs.iter().find(|m| m.id == 41).unwrap().position,
            2.0,
            65.7,
            3.0,
        );
        game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
            id: 40,
            path: play::PositionPath::Linear([12.0, 67.0, -4.0]),
            yaw: 45.0,
            pitch: 0.0,
            on_ground: true,
        }))
        .unwrap();
        for _ in 0..4 {
            game.tick_remote_entities(true);
            game.remote_ride(Keys::default(), 0).unwrap();
        }
        close(game.player.as_ref().unwrap().position, 12.0, 68.7, -4.0);
        close(
            game.mobs.iter().find(|m| m.id == 41).unwrap().position,
            12.0,
            68.7,
            -4.0,
        );
        // The intermediary's packet position must remain unchanged for relative decoding.
        assert_eq!(
            game.remote.as_ref().unwrap().tracked(42).unwrap().1,
            [0.0; 3]
        );
        // A remote player riding the local player must follow our own movement too.
        set(&mut game, 42, vec![41]);
        set(&mut game, 7, vec![42]);
        game.player.as_mut().unwrap().position = Vec3::new(20.0, 70.0, 25.0);
        game.position_riders();
        close(
            game.mobs.iter().find(|m| m.id == 41).unwrap().position,
            20.0,
            71.7,
            25.0,
        );
        // A direct stack has no intermediary's 0.5-height offset.
        set(&mut game, 40, vec![7, 41]);
        game.remote_ride(Keys::default(), 0).unwrap();
        close(game.player.as_ref().unwrap().position, 12.0, 68.2, -4.0);
        close(
            game.mobs.iter().find(|m| m.id == 41).unwrap().position,
            12.0,
            68.2,
            -4.0,
        );
        // Removing the carrier detaches passengers even without a final empty packet.
        game.remove_entity(40);
        assert_eq!(game.riding, None);
        assert_eq!(
            game.mobs.iter().find(|m| m.id == 41).unwrap().extra.mount,
            None
        );
    }

    /// `Entity.isColliding` and the snap `ClientLevel.syncBlockState` makes: a
    /// block the server puts back where the player stands sends it to the
    /// position the prediction remembered, and a block that comes back clear of
    /// the player, an empty one, or one the player has been teleported since
    /// leaves it where it is.
    #[test]
    fn a_block_the_server_puts_back_snaps_the_player_out_of_it() {
        use horizon_core::geometry::{Aabb, Vec3, VoxelShape};
        // A player standing in block (0, 64, 0): 0.6 wide, 1.8 tall.
        let player = Aabb::new(Vec3::new(0.2, 64.0, 0.2), Vec3::new(0.8, 65.8, 0.8));
        let cube = VoxelShape::block();
        assert!(
            shape_meets(&cube, [0, 64, 0], player),
            "the block the player is standing in"
        );
        assert!(!shape_meets(&cube, [0, 66, 0], player), "above its head");
        assert!(
            !shape_meets(&cube, [1, 64, 0], player),
            "the block beside it"
        );
        assert!(!shape_meets(&VoxelShape::empty(), [0, 64, 0], player));

        // The snap itself: the remembered position, unless a teleport came after
        // the prediction (`lastTeleportSequence < sequence`).
        let at = [0.5, 64.0, 0.5];
        assert_eq!(snap_target(at, -1, 3, true), Some(at));
        assert_eq!(snap_target(at, -1, 3, false), None, "a block it clears");
        assert_eq!(snap_target(at, 3, 3, true), None, "teleported at 3");
        assert_eq!(
            snap_target(at, 2, 3, true),
            Some(at),
            "teleported before it"
        );
    }

    /// `MultiPlayerGameMode.useItem`'s `shouldSwing`: the SUCCESS answers
    /// swing, CONSUME's do not (#6425).
    #[test]
    fn a_thrown_item_swinging_on_a_server_and_a_charge_not() {
        let stack = |id: &str| ItemStack::new(id, 1);
        // SUCCESS: the throwables, a bucket, a boat, a bundle, a book, a map.
        for id in [
            "minecraft:ender_pearl",
            "minecraft:splash_potion",
            "minecraft:wind_charge",
            "minecraft:snowball",
            "minecraft:experience_bottle",
            "minecraft:bucket",
            "minecraft:oak_boat",
            "minecraft:bundle",
            "minecraft:writable_book",
            "minecraft:map",
        ] {
            assert!(remote_use_swings(&stack(id), false), "{id} swings");
        }
        // CONSUME: the charges and startUsingInstantly's uses, a drinkable
        // potion among them (`consumable` set).
        for id in [
            "minecraft:bow",
            "minecraft:crossbow",
            "minecraft:trident",
            "minecraft:spyglass",
            "minecraft:goat_horn",
            "minecraft:ender_eye",
        ] {
            assert!(!remote_use_swings(&stack(id), false), "{id} does not swing");
        }
        // The kinetic weapons charge like a trident.
        assert!(!remote_use_swings(&stack("minecraft:iron_spear"), false));
        // A consumable (food, a drink) starts its use without a swing.
        assert!(!remote_use_swings(&stack("minecraft:apple"), true));
    }

    /// Minecraft.tick: CLIENT_TICK_END only while a level exists. None before
    /// the first Login (configuration has finished, the server has not placed
    /// the player yet), and none from StartConfiguration until the next Login.
    #[test]
    fn tick_ends_need_the_level_the_login_makes() {
        assert!(!client_has_level(false, None), "joined, waiting for Login");
        assert!(client_has_level(true, None), "after Login");
        assert!(!client_has_level(true, Some(40)), "reconfiguring");
    }

    /// #12148: a proxy moves the player to its next server. The last server's
    /// columns stay until the next Login, but Reconfigured already put in the
    /// next server's biome table, shorter when only the last server had a
    /// biome the game lacks: the columns hold an id past it ("the len is 67
    /// but the index is 67"). Vanilla's clearClientLevel leaves no player, so
    /// getSituationalMusic is MENU, and ServerReconfigScreen (no
    /// LevelLoadingScreen) lets MusicManager.tick start it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_reconfiguring_client_plays_menu_music_and_never_indexes_past_the_biomes() {
        use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
        use horizon_client::{chunk::Chunk, music::Music};
        use horizon_core::geometry::Vec3;
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources required"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::Client,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.init_remote_for_replay().unwrap();
        // The last server's column at the origin: its extra biome came after
        // the built-in ones, one past the table the next server leaves.
        let stale = world_biomes!(game).list.len() as u16;
        let mut column = Chunk::empty(0, 0);
        // Quart 16 above the column's bottom at -64: block y 0.
        column.set_biome_cell(0, 16, 0, stale);
        game.world
            .chunks
            .insert((0, 0), std::sync::Arc::new(column));
        assert_eq!(game.biome_at([0.0; 3]), "minecraft:plains");
        assert_eq!(
            game.biome_name_at(Vec3::new(0.5, 0.5, 0.5)).as_deref(),
            Some("minecraft:plains")
        );

        // StartConfiguration after a Login: no player, ServerReconfigScreen.
        let ticks = game.ticks;
        let r = game.remote.as_mut().unwrap();
        r.logged_in = true;
        r.reconfiguring = Some(ticks);
        game.player = None;
        assert_eq!(game.situational_music().0, Some(Music::menu()));
        // A new manager waits its first 100 ticks, then starts the menu track.
        let menu = (0..=100).any(|_| {
            game.tick_music();
            game.music.current(&game.audio.engine) == Some("minecraft:music.menu")
        });
        assert!(menu, "the menu music starts under ServerReconfigScreen");

        // With a level again the camera probe reads that column: plains' music.
        game.remote.as_mut().unwrap().reconfiguring = None;
        let (music, _) = game.situational_music();
        assert!(music.is_some_and(|m| m != Music::menu()));
    }

    /// The server's on-fire flag keeps the predicted countdown (which
    /// `fluid_base_tick` runs down) from ending the fire; a clear flag leaves
    /// the window's own contact fire alone.
    #[test]
    fn the_servers_fire_flag_holds_the_burning_state_through_the_countdown() {
        let mut ticks = 1;
        for _ in 0..5 {
            ticks = hold_server_fire(ticks - 1, true);
            assert!(ticks > 0, "still burning");
        }
        assert_eq!(hold_server_fire(300, true), 300, "a longer fire is not cut");
        assert_eq!(hold_server_fire(7, false), 7, "predicted lava fire stays");
        assert_eq!(hold_server_fire(0, false), 0);
    }

    #[test]
    fn a_beacon_update_tag_keeps_the_beam_the_client_walked() {
        let old = BlockEntity::Beacon(Box::new(Beacon {
            levels: 1,
            primary: Some("minecraft:speed".into()),
            sections: vec![(0xF9FFFE, 2)],
            checking: vec![(0xF9FFFE, 1)],
            last_check_y: 70,
            ..Default::default()
        }));
        // BeaconBlockEntity.getUpdateTag: saveCustomOnly.
        let tag = Tag::compound([
            ("id", Tag::String("minecraft:beacon".into())),
            ("x", Tag::Int(0)),
            ("y", Tag::Int(64)),
            ("z", Tag::Int(0)),
            ("primary_effect", Tag::String("minecraft:haste".into())),
            (
                "secondary_effect",
                Tag::String("minecraft:regeneration".into()),
            ),
            ("Levels", Tag::Int(4)),
        ]);
        let (_, mut new) = horizon_client::save::decode_block_entity(&tag).unwrap();
        keep_client_state(Some(&old), &mut new);
        let BlockEntity::Beacon(b) = new else {
            panic!("not a beacon");
        };
        assert_eq!(b.sections, vec![(0xF9FFFE, 2)]);
        assert_eq!(b.checking, vec![(0xF9FFFE, 1)]);
        assert_eq!(b.last_check_y, 70);
        // The powers are the tag's; the levels stay the client's own (not read).
        assert_eq!(b.primary.as_deref(), Some("minecraft:haste"));
        assert_eq!(b.secondary.as_deref(), Some("minecraft:regeneration"));
        assert_eq!(b.levels, 1);
    }

    #[test]
    fn the_server_stopping_the_use_lowers_the_shield() {
        // Player.disableShield's stopUsingItem: flags 0 or 2 while the
        // off-hand shield is raised (LocalPlayer.onSyncedDataUpdated →
        // stopUsingItem). stopUsingItem clears bit 0 and leaves bit 1, so a
        // main-hand stop is 0 and an off-hand stop is 2.
        let off = super::super::shield::OFFHAND_SLOT;
        assert_eq!(synced_local_use(Some((off, 12)), 0, 3, |_| true), None);
        assert_eq!(synced_local_use(Some((3, 5)), 0, 3, |_| true), None);
        assert_eq!(synced_local_use(Some((off, 12)), 2, 3, |_| true), None);
    }

    #[test]
    fn the_server_starting_a_use_starts_it_in_that_hand() {
        let off = super::super::shield::OFFHAND_SLOT;
        assert_eq!(synced_local_use(None, 1, 4, |_| true), Some((4, 0)));
        assert_eq!(synced_local_use(None, 3, 4, |_| true), Some((off, 0)));
        // LivingEntity.startUsingItem needs an item in the hand.
        assert_eq!(synced_local_use(None, 1, 4, |_| false), None);
    }

    /// tooBigToInterpolate: a position sync measures from where the entity
    /// is, a teleport from where its interpolation is headed.
    #[test]
    fn a_far_move_snaps_by_the_packets_own_measure() {
        use super::{MovePacket, Path, PosRot, SteppedInterpolation};
        let at = |x: f64| PosRot {
            position: [x, 0.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
        };
        let mut handler = SteppedInterpolation::new(3);
        // At x 0, headed for x 50.
        handler.interpolate_to(at(0.0), Some(Path::Linear([50.0, 0.0, 0.0])), None);
        let to = [100.0, 0.0, 0.0];
        assert!(MovePacket::PositionSync.snaps(&handler, at(0.0), to));
        assert!(!MovePacket::Teleport.snaps(&handler, at(0.0), to));
        assert!(!MovePacket::Move.snaps(&handler, at(0.0), to));
        // With nothing in flight, both measure from the entity.
        let idle = SteppedInterpolation::new(3);
        assert!(MovePacket::Teleport.snaps(&idle, at(0.0), to));
        assert!(!MovePacket::Teleport.snaps(&idle, at(0.0), [60.0, 0.0, 0.0]));
    }

    /// A far sync puts the entity there and drops the steps it was walking;
    /// a rotation alone keeps the position it is headed for and turns there
    /// over the update interval.
    #[test]
    fn a_living_entitys_move_snaps_or_steps() {
        use super::{MovePacket, Path, PosRot, SteppedInterpolation, stepped_move};
        let at = |x: f64, yaw: f32| PosRot {
            position: [x, 0.0, 0.0],
            yaw,
            pitch: 0.0,
        };
        let mut handler = SteppedInterpolation::new(3);
        // Walking to x 5 when the far sync comes.
        handler.interpolate_to(at(0.0, 0.0), Some(Path::Linear([5.0, 0.0, 0.0])), None);
        assert!(handler.active());
        let far = [100.0, 0.0, 0.0];
        let snap = stepped_move(
            &mut handler,
            MovePacket::PositionSync,
            at(0.0, 0.0),
            far,
            Some(Path::Linear(far)),
            Some((90.0, 0.0)),
        );
        assert_eq!(snap, Some(at(100.0, 90.0)));
        assert!(!handler.active());
        assert_eq!(handler.interpolate(), None, "no step back toward x 5");
        // A rotation-only move packet.
        let turn = stepped_move(
            &mut handler,
            MovePacket::Move,
            at(100.0, 90.0),
            far,
            None,
            Some((180.0, 0.0)),
        );
        assert_eq!(turn, None);
        let steps: Vec<_> = std::iter::from_fn(|| handler.interpolate()).collect();
        assert_eq!(
            steps.iter().map(|s| s.yaw).collect::<Vec<_>>(),
            [120.0, 150.0, 180.0]
        );
        assert!(steps.iter().all(|s| s.position == far));
    }

    #[test]
    fn matching_flags_leave_a_running_use_alone() {
        assert_eq!(synced_local_use(Some((2, 7)), 1, 2, |_| true), Some((2, 7)));
        assert_eq!(synced_local_use(None, 0, 2, |_| true), None);
    }

    /// `ChatType.Bound.decorate` and `decorateNarration`: a private message
    /// shows as "Steve whispers to you: hi" and is narrated "Steve says hi";
    /// a chat type the registry lacks is `minecraft:chat`'s.
    #[test]
    fn chat_is_narrated_with_its_chat_types_narration_decoration() {
        use super::{Decoration, chat_type_decorate, play};
        use horizon_core::nbt::{JavaString, Tag, parse_snbt};
        use serde_json::json;
        let bound = play::ChatTypeBound {
            chat_type: 0,
            name: Tag::String(JavaString::from("Steve")),
            target: None,
        };
        let content = json!("hi");
        let (incoming, _) = parse_snbt(
            r#"{chat:{translation_key:"commands.message.display.incoming",parameters:["sender","content"],style:{color:"gray",italic:1b}},narration:{translation_key:"chat.type.text.narrate",parameters:["sender","content"]}}"#,
        )
        .unwrap();
        let chat = chat_type_decorate(Some(&incoming), Decoration::Chat, &bound, &content);
        assert_eq!(chat["translate"], "commands.message.display.incoming");
        assert_eq!(chat["color"], "gray");
        assert_eq!(chat["with"], json!(["Steve", "hi"]));
        assert_eq!(
            chat_type_decorate(Some(&incoming), Decoration::Narration, &bound, &content),
            json!({"translate": "chat.type.text.narrate", "with": ["Steve", "hi"]})
        );
        assert_eq!(
            chat_type_decorate(None, Decoration::Narration, &bound, &content),
            json!({"translate": "chat.type.text.narrate", "with": ["Steve", "hi"]})
        );
        assert_eq!(
            chat_type_decorate(None, Decoration::Chat, &bound, &content)["translate"],
            "chat.type.text"
        );
    }

    /// A cauldron's fill level is a use this client predicts, and a guest's
    /// host forwards its sound: every cauldron state counts, not only the
    /// doors and levers.
    #[test]
    fn every_cauldron_counts_as_hand_operated() {
        for name in [
            "minecraft:cauldron",
            "minecraft:water_cauldron",
            "minecraft:lava_cauldron",
            "minecraft:powder_snow_cauldron",
        ] {
            assert!(super::hand_operated(name), "{name}");
        }
        assert!(!super::hand_operated("minecraft:water"));
    }

    /// An NPC plugin lists the NPC, spawns it, and takes the entry off the list:
    /// the entity keeps the skin it found; a player spawned before its entry
    /// arrives takes the entry once it comes.
    #[test]
    fn a_player_entity_keeps_the_skin_its_entry_gave() {
        use horizon_client::skins::Textures;
        use std::collections::HashMap;
        let npc = Textures {
            skin: Some("http://textures.minecraft.net/texture/npc".into()),
            signed: true,
            ..Textures::default()
        };
        let (mut listed, mut kept) = (HashMap::from([(7u128, npc.clone())]), HashMap::new());
        let mut players = HashMap::from([(2i32, 7u128), (3, 8)]);
        assert_eq!(
            super::skin_entries(&players, &listed, &mut kept),
            [(7, npc.clone())]
        );
        listed.remove(&7);
        assert_eq!(
            super::skin_entries(&players, &listed, &mut kept),
            [(7, npc.clone())]
        );
        // Player 8's entry comes after its entity.
        let late = Textures {
            skin: Some("http://textures.minecraft.net/texture/late".into()),
            ..npc.clone()
        };
        listed.insert(8, late.clone());
        let mut entries = super::skin_entries(&players, &listed, &mut kept);
        entries.sort_by_key(|(u, _)| *u);
        assert_eq!(entries, [(7, npc), (8, late)]);
        // A removed entity forgets its profile with it.
        players.remove(&2);
        kept.remove(&2);
        listed.clear();
        assert_eq!(super::skin_entries(&players, &listed, &mut kept).len(), 1);
    }

    #[test]
    fn tab_publication_fingerprint_tracks_all_inputs() {
        let fingerprint = |ids: &[u128], skins, atlas| {
            (
                ids.len(),
                ids.iter().fold(0, |xor, uuid| xor ^ uuid),
                skins,
                atlas,
            )
        };
        let published = fingerprint(&[7, 8], 1, 2);
        assert_eq!(published, fingerprint(&[8, 7], 1, 2));
        assert_ne!(published, fingerprint(&[7, 9], 1, 2));
        assert_ne!(published, fingerprint(&[7, 8], 2, 2));
        assert_ne!(published, fingerprint(&[7, 8], 1, 3));
    }

    #[test]
    fn tab_object_skin_publication_tracks_changes_and_atlas_clear() {
        let mut published = std::collections::HashMap::new();
        let mut writes = 0;
        for skin in ["skins/first", "skins/first", "skins/second"] {
            for key in ["id:7", "name:player"] {
                if super::object_skin_changed(&published, key, skin) {
                    published.insert(key.to_owned(), skin.to_owned());
                    writes += 1;
                }
            }
        }
        assert_eq!(writes, 4, "unchanged skins do not publish either entry");
        published.clear();
        assert!(super::object_skin_changed(
            &published,
            "id:7",
            "skins/second"
        ));
        assert!(super::object_skin_changed(
            &published,
            "name:player",
            "skins/second"
        ));
    }

    /// An NPC plugin's scale: a base the plugin sets, or a modifier on 1.
    #[test]
    fn attribute_values_add_then_multiply_as_vanilla() {
        use horizon_protocol::packets::play::{AttributeModifier, AttributeSnapshot};
        let modifier = |amount, operation| AttributeModifier {
            id: "minecraft:proof".into(),
            amount,
            operation,
        };
        let snapshot = |base, modifiers| AttributeSnapshot {
            attribute: 0,
            base,
            modifiers,
        };
        assert_eq!(super::snapshot_value(&snapshot(2.0, vec![])), 2.0);
        assert_eq!(
            super::snapshot_value(&snapshot(1.0, vec![modifier(1.0, 0)])),
            2.0
        );
        // (1 + 1) * (1 + 0.5) from the base, then * (1 + 1) of the total.
        let all = vec![modifier(1.0, 0), modifier(0.5, 1), modifier(1.0, 2)];
        assert_eq!(super::snapshot_value(&snapshot(1.0, all)), 6.0);
    }

    use super::Game;

    /// `Entity.lookAt`: yaw 0 faces +Z, -90 faces +X; pitch -90 looks up.
    #[test]
    fn look_at_turns_towards_the_target() {
        let close = |(yaw, pitch): (f32, f32), want: (f32, f32)| {
            assert!(
                (yaw - want.0).abs() < 1e-3 && (pitch - want.1).abs() < 1e-3,
                "{yaw} {pitch}, want {want:?}"
            );
        };
        let at = [10.0, 64.0, -5.0];
        let off = |d: [f64; 3]| [at[0] + d[0], at[1] + d[1], at[2] + d[2]];
        close(
            super::look_at_rotation(at, off([0.0, 0.0, 3.0])),
            (0.0, 0.0),
        );
        close(
            super::look_at_rotation(at, off([3.0, 0.0, 0.0])),
            (-90.0, 0.0),
        );
        close(
            super::look_at_rotation(at, off([-3.0, 0.0, 0.0])),
            (90.0, 0.0),
        );
        close(
            super::look_at_rotation(at, off([0.0, 0.0, -3.0])),
            (-180.0, 0.0),
        );
        close(
            super::look_at_rotation(at, off([0.0, 2.0, 2.0])),
            (0.0, -45.0),
        );
        close(
            super::look_at_rotation(at, off([0.0, -4.0, 4.0])),
            (0.0, 45.0),
        );
    }

    /// `handleMovePlayer` moves the player's pitch and the model's old pitch
    /// (`xRotO`) alike: a relative change keeps the ease a tick began, each
    /// end clamped on its own, and an absolute one snaps both (#3844).
    #[test]
    fn a_position_packet_moves_both_pitch_ends() {
        use super::position_packet_pitch as pitch;
        // The tick is laying the head down from 30 to 0; the packet turns 10.
        assert_eq!(
            (pitch(30.0, 10.0, true), pitch(0.0, 10.0, true)),
            (40.0, 10.0)
        );
        // Each end clamps on its own.
        assert_eq!(
            (pitch(80.0, 20.0, true), pitch(60.0, 20.0, true)),
            (90.0, 80.0)
        );
        // An absolute pitch ignores both ends.
        assert_eq!(
            (pitch(30.0, -45.0, false), pitch(0.0, -45.0, false)),
            (-45.0, -45.0)
        );
        assert_eq!(pitch(0.0, 120.0, false), 90.0);
    }

    /// The hurt sound of `ClientboundDamageEventPacket` follows its damage
    /// type (`Player.getHurtSound`: the type's effects), with the codec's
    /// `hurt` default for an unknown type (#4149).
    #[test]
    fn the_hurt_sound_follows_the_damage_events_type() {
        use super::hurt_sound_for;
        use horizon_protocol::registry::Dynamic;
        use horizon_world::damage_source::Catalog;
        let types = [
            ("minecraft:in_fire", "burning"),
            ("minecraft:drown", "drowning"),
            ("minecraft:sweet_berry_bush", "poking"),
            ("minecraft:freeze", "freezing"),
            ("minecraft:arrow", "hurt"),
        ];
        // The types the server synchronized, in registry order.
        let mut registries = Dynamic::default();
        registries.add(horizon_protocol::packets::configuration::RegistryData {
            registry: "minecraft:damage_type".into(),
            entries: types
                .iter()
                .map(
                    |(id, _)| horizon_protocol::packets::configuration::RegistryEntry {
                        id: (*id).into(),
                        data: None,
                    },
                )
                .collect(),
        });
        // The damage catalog: one entry per type above, each with its effects.
        let bits = |f: f32| format!("{:08x}", f.to_bits());
        let catalog = Catalog::from_json(
            &serde_json::json!({
                "data_version": i64::from(horizon_world::DATA_VERSION),
                "types": types.iter().map(|(id, effects)| serde_json::json!({
                    "id": id,
                    "message_id": id.trim_start_matches("minecraft:"),
                    "exhaustion": bits(0.1),
                    "scaling": "when_caused_by_living_non_player",
                    "effects": effects,
                    "death_message_type": "default",
                })).collect::<Vec<_>>(),
            }),
            &serde_json::json!({}),
        )
        .unwrap();
        for (id, (name, effects)) in types.iter().enumerate() {
            assert_eq!(
                hurt_sound_for(&registries, &catalog, id as i32),
                horizon_client::health::hurt_sound(effects),
                "{name}"
            );
        }
        // An unknown id falls back to the codec's default effects.
        assert_eq!(
            hurt_sound_for(&registries, &catalog, types.len() as i32),
            "minecraft:entity.player.hurt"
        );
        // A server-defined damage type with inline NBT effects needs no entry in
        // the client's built-in catalog.
        let mut custom = horizon_core::nbt::Compound::new();
        custom.insert(
            "effects".into(),
            horizon_core::nbt::Tag::String("burning".into()),
        );
        registries.add(horizon_protocol::packets::configuration::RegistryData {
            registry: "minecraft:damage_type".into(),
            entries: vec![horizon_protocol::packets::configuration::RegistryEntry {
                id: "custom:lava_splash".into(),
                data: Some(horizon_core::nbt::Tag::Compound(custom)),
            }],
        });
        assert_eq!(
            hurt_sound_for(&registries, &catalog, 0),
            "minecraft:entity.player.hurt_on_fire"
        );
    }
    #[test]
    fn synchronized_damage_defaults_override_a_different_local_definition() {
        let local = horizon_world::damage_source::Catalog::from_json(&serde_json::json!({
            "data_version": horizon_world::DATA_VERSION,
            "types": [{"id":"demo:heat", "message_id":"heat", "exhaustion":"00000000", "scaling":"never", "effects":"burning", "death_message_type":"default"}]
        }), &serde_json::json!({})).unwrap();
        let mut registries = horizon_protocol::registry::Dynamic::default();
        registries.add(horizon_protocol::packets::configuration::RegistryData {
            registry: "minecraft:damage_type".into(),
            entries: vec![horizon_protocol::packets::configuration::RegistryEntry {
                id: "demo:heat".into(),
                data: Some(horizon_core::nbt::Tag::Compound(Default::default())),
            }],
        });
        assert_eq!(
            super::hurt_sound_for(&registries, &local, 0),
            "minecraft:entity.player.hurt"
        );
    }
    /// `ClientPacketListener.handleSetEntityPassengersPacket`: the vehicle
    /// ejects its passengers, then each listed entity `startRiding`s it, so
    /// `isVehicle` holds while the list is not empty, whatever the passenger's
    /// type: a server's item display riding a player hides that player's name
    /// tag like a text display does (#5666).
    #[test]
    fn any_tracked_passenger_makes_its_vehicle_a_vehicle() {
        use std::collections::{HashMap, HashSet};
        let tracked = |kind: &str| super::Tracked {
            kind: kind.to_owned(),
            position: [0.0, 64.0, 0.0],
            codec_base: [0.0, 64.0, 0.0],
            silent: false,
            vehicle: None,
        };
        let mut entities = HashMap::from([
            (42, tracked("minecraft:player")),
            (43, tracked("minecraft:player")),
            (9, tracked("minecraft:item_display")),
            (10, tracked("minecraft:block_display")),
        ]);
        let vehicles = |e: &HashMap<i32, super::Tracked>| -> HashSet<u32> {
            e.values().filter_map(|t| t.vehicle).collect()
        };
        super::seat_passengers(&mut entities, 42, &[9, 10]);
        assert_eq!(vehicles(&entities), HashSet::from([42]));
        // The list replaces the old one: 10 got off, 9 still rides.
        super::seat_passengers(&mut entities, 42, &[9]);
        assert_eq!(entities[&10].vehicle, None);
        assert_eq!(entities[&9].vehicle, Some(42));
        // startRiding leaves the old vehicle first.
        super::seat_passengers(&mut entities, 43, &[9]);
        assert_eq!(vehicles(&entities), HashSet::from([43]));
        // An empty list: nobody rides 43 any more.
        super::seat_passengers(&mut entities, 43, &[]);
        assert!(vehicles(&entities).is_empty());
    }
    /// `ClientboundSoundEntityPacket`: the sound is heard where the entity it
    /// names is — the local player included, which is no tracked entity — and
    /// an id this level doesn't have plays nothing at all.
    #[test]
    fn a_sound_entity_is_heard_where_its_entity_is() {
        use std::collections::HashMap;
        let mut tracked = HashMap::new();
        tracked.insert(
            7,
            super::Tracked {
                kind: "minecraft:cow".to_owned(),
                position: [8.5, 64.0, -3.5],
                codec_base: [8.5, 64.0, -3.5],
                silent: false,
                vehicle: None,
            },
        );
        let player = Some((2, [0.5, 70.0, 0.5], false));
        assert_eq!(
            super::entity_sound_state(2, player, None, &tracked, None)
                .filter(|s| !s.silent)
                .map(|s| s.position),
            Some([0.5, 70.0, 0.5]),
            "the local player is named by Remote.entity_id"
        );
        assert_eq!(
            super::entity_sound_state(7, player, None, &tracked, None)
                .filter(|s| !s.silent)
                .map(|s| s.position),
            Some([8.5, 64.0, -3.5]),
            "another entity the server spawned"
        );
        assert_eq!(
            super::entity_sound_state(9, player, None, &tracked, None)
                .filter(|s| !s.silent)
                .map(|s| s.position),
            None,
            "getEntity is null: handleSoundEntityEvent returns"
        );
        assert_eq!(
            super::entity_sound_state(2, None, None, &tracked, None)
                .filter(|s| !s.silent)
                .map(|s| s.position),
            None,
            "a player with no position yet"
        );
    }
    #[test]
    fn a_sound_entity_retains_silence_for_players_and_non_mob_targets() {
        use std::collections::HashMap;
        let mut tracked = HashMap::new();
        tracked.insert(
            7,
            super::Tracked {
                kind: "minecraft:arrow".to_owned(),
                position: [8.5, 64.0, -3.5],
                codec_base: [8.5, 64.0, -3.5],
                silent: true,
                vehicle: None,
            },
        );
        let state = super::entity_sound_state(7, None, None, &tracked, None).unwrap();
        assert!(state.silent);
        assert!(!state.removed);
        assert_eq!(state.position, [8.5, 64.0, -3.5]);
        let player =
            super::entity_sound_state(2, Some((2, [1.0, 2.0, 3.0], true)), None, &tracked, None)
                .unwrap();
        assert!(player.silent);
        assert!(super::entity_sound_state(9, None, None, &tracked, None).is_none());
    }
    #[test]
    fn a_dragon_sound_uses_live_movement_without_losing_mob_metadata() {
        use horizon_client::sound_engine::SoundEntityState;
        let stale = SoundEntityState {
            position: [0.0, 64.0, 0.0],
            silent: true,
            removed: false,
        };
        let tracked = std::collections::HashMap::new();
        for position in [[24.0, 80.0, -16.0], [25.5, 81.0, -18.0]] {
            let state =
                super::entity_sound_state(7, None, Some(stale), &tracked, Some((7, position)))
                    .unwrap();
            assert_eq!(state.position, position);
            assert!(state.silent);
            assert!(!state.removed);
        }
        let unrelated =
            super::entity_sound_state(7, None, Some(stale), &tracked, Some((8, [99.0; 3])))
                .unwrap();
        assert_eq!(unrelated.position, stale.position);
        assert!(super::entity_sound_state(7, None, None, &tracked, Some((7, [99.0; 3]))).is_none());
    }

    /// Q over a slot of the inventory on a server (#3357): the predicted THROW
    /// drops the stack, so the player's hand swings, as `LivingEntity.drop`
    /// does for `Prediction.PREDICTED`; a plain pick up drops nothing.
    #[test]
    fn a_predicted_throw_from_a_slot_swings_the_hand() {
        use horizon_client::{
            banner::Banners,
            brewing::Brewing,
            crafting::Recipes,
            creative::Names,
            enchant::Enchantments,
            items::{ItemStack, Items},
            menu::{Env, Input, Kind, Menu},
            stations::StationRecipes,
        };
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let none = &horizon_world::loot::PackOverlay::NONE;
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items = Items::catalog(&root, none, &defaults).unwrap();
        let recipes = Recipes::load(&root, &items, none, &crate::game::test_server_load()).unwrap();
        let brewing = Brewing::load(&root, &items, none);
        let banners = Banners::load(&root, &defaults);
        let station_recipes =
            StationRecipes::load(&root, &items, none, &crate::game::test_server_load());
        let enchantments = Enchantments::default();
        let names = Names::default();
        let env = Env {
            crafting_remainder: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::game::test_server_load()
                .loop_context(horizon_world::native_load_context::LoopSide::Client, []),
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
        };
        let mut menu = Menu::new(Kind::Inventory);
        let cobblestone = ItemStack {
            item: "minecraft:cobblestone".into(),
            count: 16,
            damage: 0,
            extra: None,
        };
        // The first hotbar slot of InventoryMenu.
        menu.set_slot(&env, 36, Some(cobblestone));
        let (sent, threw) =
            super::predict_clicks(&mut menu, &env, &[(36, 0, Input::Throw)]).unwrap();
        assert!(threw, "Q over a slot drops one and swings");
        assert_eq!(menu.slot(36).map(|s| s.count), Some(15));
        assert_eq!(sent.len(), 1);
        let (_, threw) = super::predict_clicks(&mut menu, &env, &[(36, 0, Input::Pickup)]).unwrap();
        assert!(!threw, "picking the stack up drops nothing");
        let (_, threw) =
            super::predict_clicks(&mut menu, &env, &[(-999, 1, Input::Pickup)]).unwrap();
        assert!(threw, "one of the carried stack outside the window");
    }

    #[test]
    fn placement_preview_checks_obstruction_without_changing_the_level() {
        use super::DryLevel;
        use horizon_core::{position::BlockPos, random::LegacyRandom};
        use horizon_world::{
            level::{
                Behaviours, LevelAccess, PlaceContext, PlaceOutcome, Ticks, data::load_registry,
            },
            registry::BlockStateId,
        };

        struct Level {
            air: BlockStateId,
            stone: BlockStateId,
            power: BlockStateId,
            client_side: bool,
            obstructed: bool,
            loaded: bool,
            ceiling: i32,
        }
        impl horizon_world::level::BlockEntityNeighbors for Level {}
        impl LevelAccess for Level {
            fn block_state(&self, pos: BlockPos) -> BlockStateId {
                if pos == BlockPos::new(0, 0, 0) {
                    self.stone
                } else if pos == BlockPos::new(-1, 1, 0) {
                    self.power
                } else {
                    self.air
                }
            }
            fn set_raw(&mut self, _: BlockPos, _: BlockStateId) -> Option<BlockStateId> {
                panic!("preview must not write to the live level");
            }
            fn is_loaded(&self, _: BlockPos) -> bool {
                self.loaded
            }
            fn is_client_side(&self) -> bool {
                self.client_side
            }
            fn max_y(&self) -> i32 {
                self.ceiling
            }
            fn is_unobstructed(&self, pos: BlockPos, _: BlockStateId) -> bool {
                assert_eq!(pos, BlockPos::new(0, 1, 0));
                !self.obstructed
            }
            fn random(&mut self) -> &mut LegacyRandom {
                panic!("preview must not advance the live level's random source");
            }
            fn game_time(&self) -> i64 {
                0
            }
        }

        // As with the LAN tests, prepared game resources are optional locally.
        let resources = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !resources.join("block-behaviours.json").exists() {
            return;
        }
        let registry = load_registry(&resources).unwrap();
        let behaviours = Behaviours::load(&registry, &resources).unwrap();
        let mut base = Level {
            air: registry.default_state("minecraft:air").unwrap(),
            stone: registry.default_state("minecraft:stone").unwrap(),
            power: registry.default_state("minecraft:redstone_block").unwrap(),
            client_side: true,
            obstructed: false,
            loaded: true,
            ceiling: 320,
        };
        for item in [
            "minecraft:stone",
            "minecraft:oak_slab",
            "minecraft:oak_door",
            "minecraft:tnt",
            "minecraft:piston",
            "minecraft:sticky_piston",
            // GrowingPlantHeadBlock.getStateForPlacement draws a random age.
            "minecraft:twisting_vines",
        ] {
            let ctx = PlaceContext {
                clicked_pos: BlockPos::new(0, 0, 0),
                face: horizon_core::geometry::Direction::Up,
                hit: horizon_core::geometry::Vec3::new(0.5, 1.0, 0.5),
                inside: false,
                yaw: 0.0,
                pitch: 45.0,
                sneaking: false,
                item: item.into(),
                can_use_game_master_blocks: false,
            };
            for (client_side, obstructed, loaded, ceiling) in [
                (true, false, true, 320),
                (true, true, true, 320),
                (true, false, false, 320),
                (true, false, true, 1),
                (false, false, true, 320),
            ] {
                base.client_side = client_side;
                base.obstructed = obstructed;
                base.loaded = loaded;
                base.ceiling = ceiling;
                let valid = !obstructed && loaded && ceiling > 1;
                let mut preview = DryLevel::new(&base);
                let mut ticks = Ticks::default();
                let outcome = behaviours.place_block(&mut preview, &mut ticks, &ctx);
                assert_eq!(
                    matches!(outcome, PlaceOutcome::Placed { .. }),
                    valid,
                    "{item}, obstructed={obstructed}, loaded={loaded}, ceiling={ceiling}"
                );
                assert_eq!(preview.changed.is_empty(), !valid);
                if valid
                    && matches!(
                        item,
                        "minecraft:tnt" | "minecraft:piston" | "minecraft:sticky_piston"
                    )
                {
                    assert_eq!(
                        registry
                            .state(preview.block_state(BlockPos::new(0, 1, 0)))
                            .unwrap()
                            .name,
                        if item == "minecraft:tnt" && !client_side {
                            "minecraft:air"
                        } else {
                            item
                        },
                        "powered {item}, client_side={client_side}"
                    );
                    assert_eq!(
                        ticks.block_events.is_empty(),
                        client_side || item == "minecraft:tnt",
                        "only the server starts a powered piston"
                    );
                }
                assert_eq!(base.block_state(BlockPos::new(0, 1, 0)), base.air);
            }
        }
    }

    #[test]
    fn the_creative_sync_covers_inventory_menu_slots_5_to_45_once_each() {
        // CreativeInventoryListener sends every InventoryMenu slot but the crafting ones: the
        // armour (5..=8), the inventory (9..=44) and the off hand (45).
        let stack = |item: &str| Some(horizon_client::items::ItemStack::new(item, 1));
        let inventory: Vec<_> = (0..36)
            .map(|i| stack(&format!("minecraft:item_{i}")))
            .collect();
        let offhand = stack("minecraft:shield");
        let armor = [
            stack("minecraft:diamond_helmet"),
            stack("minecraft:diamond_chestplate"),
            stack("minecraft:diamond_leggings"),
            stack("minecraft:diamond_boots"),
        ];
        let stacks = super::creative_sync_stacks(&inventory, &offhand, &armor);
        assert_eq!(stacks.len(), super::CREATIVE_SYNCED);
        let mut slots: Vec<i16> = (0..stacks.len()).map(super::creative_sync_slot).collect();
        // Each entry goes to the menu slot that holds that stack.
        for (i, &slot) in slots.iter().enumerate() {
            let game_slot = Game::inventory_screen_slot("inventory", i32::from(slot)).unwrap();
            let expected = match game_slot {
                0..36 => &inventory[game_slot],
                504 => &offhand,
                500..504 => &armor[game_slot - 500],
                _ => panic!("entry {i} went to game slot {game_slot}"),
            };
            assert_eq!(&stacks[i], expected, "entry {i}, menu slot {slot}");
        }
        slots.sort();
        assert_eq!(slots, (5..=45).collect::<Vec<i16>>());
    }

    #[test]
    fn a_server_slot_update_leaves_an_unsent_creative_click_to_be_sent() {
        let stack = |item: &str| Some(horizon_client::items::ItemStack::new(item, 1));
        let none = || vec![None; super::CREATIVE_SYNCED];
        // Synced: nothing. A click puts a helmet on (entry 37, not sent yet), then the server
        // sets hotbar slot 0 (a pickup): only that entry counts as synced.
        let mut synced = none();
        let mut before = none();
        before[37] = stack("minecraft:diamond_helmet");
        let mut now = before.clone();
        now[0] = stack("minecraft:stick");
        super::mark_synced(&mut synced, &before, now.clone());
        assert_eq!(synced[0], now[0]);
        assert_eq!(
            synced[37], None,
            "the helmet still differs, so the next tick sends it"
        );
    }

    #[test]
    fn every_inventory_menu_slot_is_a_game_slot_and_45_is_the_off_hand() {
        // InventoryMenu: result, 2x2 grid, armour, 27 + 9 inventory slots, then the off hand.
        assert_eq!(Game::inventory_screen_slot("inventory", 45), Some(504));
        let mut seen = std::collections::HashSet::new();
        for slot in 0..46 {
            let internal = Game::inventory_screen_slot("inventory", slot);
            assert!(internal.is_some_and(|i| seen.insert(i)), "slot {slot}");
        }
        assert_eq!(Game::inventory_screen_slot("inventory", 46), None);
    }

    #[test]
    fn remote_clicks_find_slots_past_the_old_large_chest_cutoff() {
        for (menu, internal, index) in [
            ("minecraft:generic_9x6", 18, 63),
            ("minecraft:generic_9x6", 19, 64),
            ("minecraft:generic_9x6", 0, 81),
            ("minecraft:generic_9x6", 8, 89),
            ("minecraft:generic_9x6", 353, 53),
            ("minecraft:generic_9x5", 8, 80),
            ("minecraft:generic_9x4", 8, 71),
            ("minecraft:generic_9x3", 8, 62),
            ("inventory", 504, 45),
        ] {
            assert_eq!(
                Game::menu_slot_index(menu, internal),
                Some(index),
                "{menu}, game slot {internal}"
            );
        }
        assert_eq!(Game::menu_slot_index("minecraft:generic_9x6", 354), None);
        assert_eq!(Game::menu_slot_index("minecraft:generic_9x6", 504), None);
        assert_eq!(Game::menu_slot_index("unknown", 0), None);
    }

    /// `FireworkRocketItem.use` is SUCCESS while gliding: the main hand's
    /// rocket ends the click and the off hand's shield is not raised (#3424).
    /// On foot it passes, so the off hand still gets its turn.
    #[test]
    fn a_rocket_used_while_gliding_ends_the_click() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            eprintln!("skipping: no prepared resources in {}", root.display());
            return;
        }
        let registry = horizon_world::level::data::load_registry(&root).unwrap();
        let facts = horizon_client::blocks::BlockFacts::load(&root, &registry).unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items = horizon_client::items::Items::load(
            &root,
            &registry,
            &facts,
            &horizon_world::loot::PackOverlay::NONE,
            &defaults,
        )
        .unwrap();
        let rocket = items.get("minecraft:firework_rocket").unwrap();
        assert!(super::use_ends_click(rocket, true), "gliding");
        assert!(!super::use_ends_click(rocket, false), "on foot");
        // Other items answer the same whether or not the player glides.
        let shield = items.get("minecraft:shield").unwrap();
        let stick = items.get("minecraft:stick").unwrap();
        for gliding in [false, true] {
            assert!(super::use_ends_click(shield, gliding));
            assert!(!super::use_ends_click(stick, gliding));
            // FoodOnAStickItem.use passes on a client: no swing while the boost cools down.
            for rod in [
                "minecraft:carrot_on_a_stick",
                "minecraft:warped_fungus_on_a_stick",
            ] {
                let rod = items.get(rod).unwrap();
                assert!(!super::use_ends_click(rod, gliding), "{}", rod.id);
            }
            // A carved pumpkin has an equip_slot (head) but is not swappable, so it has no Item.use.
            let pumpkin = items.get("minecraft:carved_pumpkin").unwrap();
            assert!(!super::use_ends_click(pumpkin, gliding));
        }
    }

    /// `ClientPacketListener.handleGameEvent`: verify that the game event packet
    /// types for arrow hit, pufferfish sting and limited crafting match vanilla.
    #[test]
    fn game_event_constants_and_types_match_vanilla() {
        use horizon_protocol::packets::play::GameEvent;
        // Event types from ClientboundGameEventPacket:
        // 6 = PLAY_ARROW_HIT_SOUND, 7 = RAIN_LEVEL_CHANGE, 8 = THUNDER_LEVEL_CHANGE,
        // 9 = PUFFER_FISH_STING, 11 = IMMEDIATE_RESPAWN, 12 = LIMITED_CRAFTING.
        let arrow = GameEvent {
            event: 6,
            param: 0.0,
        };
        let rain = GameEvent {
            event: 7,
            param: 0.5,
        };
        let thunder = GameEvent {
            event: 8,
            param: 0.2,
        };
        let puffer = GameEvent {
            event: 9,
            param: 0.0,
        };
        let respawn = GameEvent {
            event: 11,
            param: 1.0,
        };
        let crafting = GameEvent {
            event: 12,
            param: 1.0,
        };

        assert_eq!(arrow.event, 6);
        assert_eq!(rain.event, 7);
        assert_eq!(thunder.event, 8);
        assert_eq!(puffer.event, 9);
        assert_eq!(respawn.event, 11);
        assert_eq!(crafting.event, 12);
        assert!(crafting.param == 1.0);
    }

    /// #14773: Flint and steel and fire charge useOn on a client level ends the
    /// click (returning true), so when held with an unusable off-hand item, the
    /// click does not fall through to UseItem or try the off hand.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn flint_and_steel_use_on_ends_the_click() {
        use super::super::{
            Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
        };
        use crate::game::Hit;
        use horizon_client::{first_person::Hand, motion::PlayerMotion};
        use horizon_core::geometry::Vec3;

        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES")
                .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::Client;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        let (client, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(client, "FlintTest").unwrap();
        game.remote.as_mut().unwrap().entity_id = 7;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        game.inventory[0] = Some(ItemStack::new("minecraft:flint_and_steel", 1));
        game.offhand = Some(ItemStack::new("minecraft:stick", 1));
        game.selected = 0;
        game.forced_hit = Some(Hit {
            pos: [0, 63, 0],
            face: [0, 1, 0],
            point: [0.5, 64.0, 0.5],
            inside: false,
        });

        let done = game.remote_use_block(Hand::Main);
        assert!(done, "flint and steel useOn must end the click");
    }
}

#[cfg(test)]
mod movement_input_tests;
#[cfg(test)]
#[path = "player_tick_tests.rs"]
mod player_tick_tests;
