//! Multiplayer: the game driven by a server connection instead of the local
//! simulation. Mirrors vanilla's ClientPacketListener (chunks, entities,
//! inventory, health, time, weather, chat, respawn and dimension changes),
//! LocalPlayer.sendPosition (movement packets) and MultiPlayerGameMode
//! (block breaking with sequence numbers, use-item, attacks, container
//! clicks). The player still moves with the local physics, as the vanilla
//! client predicts its own movement; the server stays authoritative for
//! everything else.
use super::{Dim, Game, Mining, Screen};
use horizon_client::first_person::{Hand, SwingAnimation};
use horizon_client::{
    Result,
    chunk::{BIOME_STRATEGY, Chunk, Light, MIN_Y, SECTIONS, Section, block_strategy},
    containers::{BlockEntity, Furnace, FurnaceKind},
    entities::{Experience, ExperienceOrb, ItemEntity},
    hud::{HudBatch, rgba},
    items::ItemStack,
    menu::Input,
    mobs::{Mob, MobType, Who},
    motion::{MotionWorld, PlayerMotion},
    net::{Event, Session},
    save::SaveDir,
    sprint::Keys,
    streaming::{ChunkGenerator, Proto, Streaming},
    tab_list::SPECTATOR,
    world::World,
};
use horizon_core::{
    geometry::Vec3,
    paletted::{PalettedContainer, Strategy},
    random::RandomSource,
};
use horizon_protocol::{
    entity_data::{ITEM_ENTITY_ITEM, Value},
    item::{Hashed, Stack},
    packets::{
        Empty,
        play::{self, Clientbound as C, Serverbound as S},
    },
    registry::{Builtin, DimensionType, Dynamic, ceil_log2},
    text::{Language, plain},
};
use horizon_world::{
    nbt::Tag,
    registry::BlockStateId,
    stepped_interpolation::{Path, PosRot, Start, SteppedInterpolation},
};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{event::MouseButton, keyboard::KeyCode};

/// A text component the server sent, for the HUD.
fn remote_text(tag: &Tag) -> horizon_client::text::Text {
    use horizon_client::scoreboard::{comp_text, tag_to_json};
    comp_text(&tag_to_json(tag))
}

/// A section's sky and block light layers from the wire.
type LightLayers = (Option<Vec<u8>>, Option<Vec<u8>>);

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
struct Tracked {
    kind: String,
    position: [f64; 3],
}
/// Smooth an entity toward a new position over three ticks (InterpolationHandler).
#[derive(Clone, Copy)]
/// `LinearInterpolationHandler`'s target: a position and rotation (yRot,
/// xRot) reached in `steps` ticks.
struct Lerp {
    target: [f64; 3],
    rotation: Option<(f32, f32)>,
    steps: u32,
}

/// `SteppedInterpolationHandler.doInterpolate`'s `setPos` and `setRot` on a
/// mob: its yaw kept continuous (the same turn as vanilla's, without the
/// wrap that would spin it in the frame between ticks), its pitch clamped.
fn place_interpolated(m: &mut Mob, at: PosRot) {
    m.position = Vec3::new(at.position[0], at.position[1], at.position[2]);
    m.yaw += wrap_degrees(at.yaw - m.yaw);
    m.head_pitch = (at.pitch % 360.0).clamp(-90.0, 90.0);
}

pub(super) struct Remote {
    pub session: Session,
    pub brand: String,
    pub(super) builtin: Arc<Builtin>,
    lang: Arc<Language>,
    pub(super) registries: Arc<Dynamic>,
    joined: bool,
    status: String,
    pub(super) entity_id: i32,
    dimension: DimensionType,
    biome_map: Vec<u16>,
    biome_bits: u8,
    view_distance: i32,
    sequence: i32,
    position_sender: horizon_client::player_sync::PositionSender,
    last_input: u8,
    was_sprinting: bool,
    carried: Option<usize>,
    /// The last ClientboundPlayerAbilitiesPacket. Vanilla's LocalPlayer
    /// holds its abilities from the login on; here the player is made at
    /// the first teleport, which takes them then.
    abilities: Option<play::PlayerAbilities>,
    loaded_sent: bool,
    /// The view distance last sent in ClientInformation.
    sent_view: i8,
    /// BlockStatePredictionHandler: blocks changed ahead of the server (a predicted break),
    /// with the sequence of the last action that changed them and the state the server
    /// last said they have. Until the server acknowledges that sequence its block updates
    /// only replace the remembered state; then the block takes it.
    predictions: HashMap<[i32; 3], (i32, i32)>,
    /// Open container: (window id, menu name).
    pub(super) menu: Option<(i32, String)>,
    /// The open menu's title as the server named it.
    pub(super) menu_title: Option<String>,
    state_id: i32,
    use_target: Option<[i32; 3]>,
    entities: HashMap<i32, Tracked>,
    lerps: HashMap<i32, Lerp>,
    /// Living entities' `SteppedInterpolationHandler`s.
    stepped: HashMap<i32, SteppedInterpolation>,
    /// `LivingEntity.lerpHeadTo`: the head yaw and the ticks left to reach it.
    head_lerps: HashMap<i32, (f32, u32)>,
    /// Item entities waiting for their item data: where they are and their
    /// motion.
    pending_items: HashMap<i32, ([f64; 3], Vec3)>,
    health: (f32, i32, f32),
    /// `LocalPlayer.flashOnSetHealth`: set by the first health the server
    /// sends; from then on a drop starts the hurt animation.
    flash_on_set_health: bool,
    game_mode: i32,
    pub chat: Vec<(String, Instant)>,
    /// Lines received since joining, for the control socket's events.
    pub chat_received: u64,
    /// The server's command tree (ClientboundCommandsPacket).
    pub commands: Option<horizon_client::command::Dispatcher>,
    players: HashMap<u128, String>,
    /// `ClientPacketListener.playerInfoMap` as player-list entries, and
    /// which of them are listed.
    tab_players: HashMap<u128, horizon_client::tab_list::TabPlayer>,
    /// Each listed profile's unpacked `textures` property.
    skin_textures: HashMap<u128, horizon_client::skins::Textures>,
    /// The skin texture each profile's players show, once its download is ready.
    skins: HashMap<u128, String>,
    listed: std::collections::HashSet<u128>,
    /// `ClientboundLoginPacket.onlineMode`: the player list shows heads.
    online_mode: bool,
    /// Player entities in the level: entity id to profile UUID.
    level_players: HashMap<i32, u128>,
    /// The local player's profile UUID (always in the level).
    profile: u128,
    /// The local player's profile name: its score holder and team member.
    pub(super) profile_name: String,
    destroying: Option<([i32; 3], u8, f32)>,
    destroy_delay: u32,
    input: horizon_client::interaction_input::InteractionInput,
    stop_mining: bool,
    placing: Option<([i32; 3], u64)>,
    death_message: Option<String>,
    player_type: Option<Arc<MobType>>,
    /// Inventory as last synchronised, for creative-mode slot updates.
    synced: Vec<Option<ItemStack>>,
    decode_errors: u32,
    clock: Option<i32>,
    /// `ClientClockManager.ClientClockInstance`'s rate and partial tick for
    /// the level's clock (its total is `world.day_time`), and
    /// `lastTickGameTime`.
    clock_rate: f32,
    clock_partial: f32,
    clock_game_time: i64,
    /// `ClientChunkCache.Storage`'s view centre (ClientboundSetChunkCacheCenterPacket).
    chunk_center: (i32, i32),
    /// Received events a frame left for the next one, in order, and when
    /// the oldest of them first waited (`poll_remote`).
    backlog: VecDeque<Event>,
    backlog_since: Option<Instant>,
    /// `ClientboundTransferPacket`'s target: when the connection ends the
    /// game connects here, the stored cookies carried over (`TransferState`).
    transfer: Option<String>,
}

/// The time a frame spends on chunk packets before it leaves the rest of
/// the received events for the next frame. A batch of 25 to 60 columns
/// (0.1 ms each on an M4 Pro) all in one frame was a 3 to 8 ms frame every
/// time the player crossed a chunk border at 32 chunks.
const CHUNK_PACKETS_PER_FRAME: Duration = Duration::from_millis(1);
/// Events that have waited this long are all handled in the next frame,
/// whatever their cost: the packets after a slow machine's chunks never
/// wait more than a tick.
const BACKLOG_LIMIT: Duration = Duration::from_millis(50);
/// The open menu name of a server's horse inventory, then its chest's
/// columns (the packet names no `MenuType`).
const MOUNT_MENU: &str = "horizon:mount";

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

/// A `minecraft:profile` component as the item data JSON the model reads.
fn profile_tag(p: &horizon_protocol::item::Profile) -> Tag {
    let properties = p
        .properties
        .iter()
        .map(|(name, value, signature)| {
            let mut entries: Vec<(&'static str, Tag)> = vec![
                ("name", Tag::String(name.as_str().into())),
                ("value", Tag::String(value.as_str().into())),
            ];
            if let Some(signature) = signature {
                entries.push(("signature", Tag::String(signature.as_str().into())));
            }
            Tag::compound(entries)
        })
        .collect::<Vec<_>>();
    let mut entries: Vec<(&'static str, Tag)> = Vec::new();
    if let Some(name) = &p.name {
        entries.push(("name", Tag::String(name.as_str().into())));
    }
    entries.push(("properties", Tag::List(properties)));
    Tag::compound(entries)
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
        if self.defer_world_entry(super::PendingWorldEntry::Server {
            address: address.to_owned(),
            name: name.to_owned(),
            cookies: cookies.clone(),
        })? {
            return Ok(());
        }
        self.join_world_rest()?;
        if let Err(error) = self.join_server(address, name, None, cookies) {
            log::warn!("Horizon: could not join server: {error}");
            if self.title.is_none() {
                self.title = Some(horizon_client::title::Title::new(
                    std::env::current_dir()?.join("saves"),
                    &self.options.resources,
                ));
            }
            if let Some(title) = &mut self.title {
                title.failed(error);
            }
            self.capture(false);
        }
        Ok(())
    }
    /// Join single player's integrated server through memory.
    pub(super) fn connect_local(
        &mut self,
        pipe: horizon_protocol::connection::Pipe,
        name: &str,
    ) -> Result<()> {
        self.join_server("local", name, Some(pipe), None)
    }
    fn join_server(
        &mut self,
        address: &str,
        name: &str,
        local: Option<horizon_protocol::connection::Pipe>,
        cookies: Option<horizon_client::net::Cookies>,
    ) -> Result<()> {
        self.perf.world_opening();
        let root = self.options.resources.clone();
        let builtin = Builtin::find(&root)
            .and_then(|p| Builtin::load(&p).ok())
            .ok_or("Resources are incomplete: registries.json is missing or invalid. Click \"Prepare again\" in the launcher (or run horizon-prepare), then try again.")?;
        let lang = Arc::new(horizon_client::net::load_language(&root));
        let view = client_view_distance(self.settings.render_distance);
        if local.is_none() {
            horizon_client::crash::set_detail(
                "Server",
                horizon_client::net::shown_address(address),
            );
        }
        // A signed-in account always plays under its own name.
        let auth = horizon_client::auth::current();
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
            Some(pipe) => Session::connect_local(pipe, name, view, lang.clone()),
            None => {
                self.extras_connecting(address, name);
                match cookies {
                    Some(cookies) => Session::connect_transferred(
                        address,
                        name,
                        auth.clone(),
                        view,
                        lang.clone(),
                        cookies,
                    ),
                    None => Session::connect(address, name, auth.clone(), view, lang.clone()),
                }
            }
        };
        let player_type = super::perspective::player_type(&root);
        let mut streaming = Streaming::new(
            Arc::new(NoTerrain),
            self.pack.context.clone(),
            self.res.threads(),
        );
        streaming.remote = true;
        streaming.radius = self.settings.render_distance;
        self.streaming = streaming;
        self.world = World::new(self.res.facts().air, 0);
        self.save = Arc::new(SaveDir::new(
            std::env::temp_dir().join("horizon-remote-unused"),
        ));
        self.inventory = vec![None; 36];
        self.armor = [None, None, None, None];
        self.craft_grid = vec![None; 9];
        self.craft_size = 2;
        self.player = None;
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
        self.travelling = None;
        self.mining = None;
        self.container = None;
        self.carried = None;
        self.using = None;
        self.clicks.miss_time = 0;
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
        self.screen = Screen::Playing;
        self.title = None;
        self.dim = Dim::Overworld;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections();
        }
        self.apply_settings();
        self.clear_waypoints();
        self.boss_overlay.reset();
        self.remote = Some(Remote {
            session,
            brand: "unknown".into(),
            builtin: Arc::new(builtin),
            lang,
            registries: Arc::new(Dynamic::default()),
            joined: false,
            status: "Connecting to the server...".into(),
            entity_id: 0,
            dimension: DimensionType::vanilla("minecraft:overworld"),
            biome_map: Vec::new(),
            biome_bits: 7,
            view_distance: self.settings.render_distance,
            sequence: 0,
            position_sender: Default::default(),
            last_input: 0,
            was_sprinting: false,
            carried: None,
            abilities: None,
            loaded_sent: false,
            sent_view: view,
            predictions: HashMap::new(),
            menu: None,
            menu_title: None,
            state_id: 0,
            use_target: None,
            entities: HashMap::new(),
            lerps: HashMap::new(),
            stepped: HashMap::new(),
            head_lerps: HashMap::new(),
            pending_items: HashMap::new(),
            health: (20.0, 20, 5.0),
            flash_on_set_health: false,
            game_mode: 0,
            chat: Vec::new(),
            chat_received: 0,
            commands: None,
            players: HashMap::new(),
            tab_players: HashMap::new(),
            skin_textures: HashMap::new(),
            skins: HashMap::new(),
            listed: Default::default(),
            online_mode: false,
            level_players: HashMap::new(),
            profile: 0,
            profile_name: String::new(),
            destroying: None,
            destroy_delay: 0,
            input: Default::default(),
            stop_mining: false,
            placing: None,
            death_message: None,
            player_type,
            synced: vec![None; 36],
            decode_errors: 0,
            clock: None,
            clock_rate: 1.0,
            clock_partial: 0.0,
            clock_game_time: 0,
            chunk_center: (0, 0),
            backlog: VecDeque::new(),
            backlog_since: None,
            transfer: None,
        });
        // The level's TickRateManager follows the server's ticking packets.
        self.clock = horizon_client::clock::FixedStep::client();
        // The player's advancements and statistics are the server's.
        self.adv = Default::default();
        self.stats = Default::default();
        Ok(())
    }

    /// Disconnect and return to the multiplayer screen (DisconnectedScreen).
    pub(super) fn leave_server(&mut self, reason: Option<String>) -> Result<()> {
        if reason.is_none() && self.integrated.as_ref().is_some_and(|s| !s.stopped()) {
            return self.begin_closing_save(false);
        }
        self.finish_perf();
        if reason.is_none() {
            // Leaving on purpose: nothing for auto-reconnect to retry.
            self.extras_left();
        }
        self.clear_waypoints();
        self.boss_overlay.reset();
        if let Some(mut r) = self.remote.take() {
            r.session.disconnect();
        }
        self.server_packs_disconnect();
        // Single player's server thread saves the world before the title shows.
        let single_player = match self.integrated.take() {
            Some(server) => {
                server.stop()?;
                true
            }
            None => false,
        };
        horizon_client::crash::clear_detail("Server");
        self.clock = Default::default();
        self.adv = Default::default();
        self.stats = Default::default();
        self.debug_charts.reset();
        self.tab_list.reset();
        self.score.board = Default::default();
        self.streaming = Streaming::new(Arc::new(NoTerrain), self.pack.context.clone(), 1);
        self.streaming.paused = true;
        self.world = World::new(self.res.facts().air, 0);
        self.player = None;
        self.mobs.clear();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.dropped.clear();
        self.particles.clear();
        self.pickups.clear();
        self.score.board = Default::default();
        self.tab_list = Default::default();
        self.border = Default::default();
        self.screen = Screen::Playing;
        self.container = None;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections();
        }
        let saves = std::env::current_dir()?.join("saves");
        let mut title = horizon_client::title::Title::new(saves.clone(), &self.options.resources);
        title.last_server = self.settings.file.string("lastServer");
        if single_player && reason.is_none() {
            self.title = Some(title);
            self.capture(false);
            return Ok(());
        }
        title.mode = horizon_client::title::Mode::Multiplayer(match reason {
            Some(reason) => horizon_client::multiplayer::Screen::Disconnected {
                reason,
                retry: None,
            },
            None => horizon_client::multiplayer::Screen::list(&saves),
        });
        self.title = Some(title);
        self.capture(false);
        Ok(())
    }

    pub(super) fn send(&self, packet: S) {
        if let Some(r) = &self.remote {
            r.session.send(packet);
        }
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
        let r = self.remote.as_ref()?;
        let name = r.builtin.get("minecraft:mob_effect")?.name(effect)?;
        horizon_client::brewing::effect(name).map(|d| d.id)
    }
    /// ClientPacketListener.handleUpdateMobEffect for the player itself
    /// (`LivingEntity.forceAddEffect`): the effect replaces any of its kind.
    /// Other entities' effects are not shown.
    fn remote_effect(&mut self, p: &play::UpdateMobEffect) {
        if self.remote.as_ref().is_none_or(|r| r.entity_id != p.id) {
            return;
        }
        let Some(id) = self.remote_effect_id(p.effect) else {
            return;
        };
        let mut e = horizon_client::effects::EffectInstance::new(id, p.duration, p.amplifier);
        e.ambient = p.flags & 1 != 0;
        e.visible = p.flags & 2 != 0;
        let list = &mut self.potions.effects.list;
        match list.iter_mut().find(|x| x.id == id) {
            Some(existing) => *existing = e,
            None => list.push(e),
        }
    }
    fn item_id(&self, name: &str) -> i32 {
        let r = self.remote.as_ref().expect("remote");
        r.builtin
            .get("minecraft:item")
            .and_then(|i| i.id(name))
            .unwrap_or(0)
    }
    pub(super) fn to_stack(&self, s: &Option<Stack>) -> Option<ItemStack> {
        let s = s.as_ref()?;
        let mut out = ItemStack {
            item: self.item_name(s.item),
            count: s.count.max(0) as u32,
            damage: 0,
            extra: None,
        };
        // A vanilla `components` compound, then the same decoder the saved
        // and command item NBT use, so tooltips and models see every field.
        let components = self.protocol_components(s);
        let (damage, data) = horizon_client::item_data::decode_components(components.as_ref());
        out.damage = damage;
        if let Some(data) = data {
            *out.data_mut() = *data;
        }
        out.tidy();
        Some(out)
    }
    /// The item's patched components as vanilla's `components` compound.
    fn protocol_components(&self, s: &Stack) -> Option<Tag> {
        use horizon_protocol::item::{Component, component};
        let r = self.remote.as_ref()?;
        let name = |id: i32| r.builtin.get("minecraft:data_component_type")?.name(id);
        let mut entries: Vec<(&str, Tag)> = Vec::new();
        for (kind, c) in &s.added {
            let Some(key) = name(*kind) else {
                continue;
            };
            let value = match c {
                // Read past, not decoded: the item keeps its prototype's value.
                Component::Raw(_) => continue,
                // Trim materials and patterns are synced registries, as enchantments are.
                Component::Trim { material, pattern } => {
                    let (Some(material), Some(pattern)) = (
                        r.registries.name("minecraft:trim_material", *material),
                        r.registries.name("minecraft:trim_pattern", *pattern),
                    ) else {
                        continue;
                    };
                    Tag::compound([
                        ("material", Tag::String(material.into())),
                        ("pattern", Tag::String(pattern.into())),
                    ])
                }
                Component::Int(v) if *kind == component::RARITY => {
                    let rarity = ["common", "uncommon", "rare", "epic"];
                    Tag::String(rarity.get(*v as usize).copied().unwrap_or("common").into())
                }
                Component::Int(v) => Tag::Int(*v),
                Component::Unit => Tag::Compound(Default::default()),
                Component::Bool(v) => Tag::Byte(i8::from(*v)),
                Component::Text(t) | Component::Nbt(t) => t.clone(),
                Component::Ident(v) => Tag::String(v.as_str().into()),
                Component::Lore(lines) => Tag::List(lines.clone()),
                Component::WritableBook(pages) => Tag::compound([(
                    "pages",
                    Tag::List(
                        pages
                            .iter()
                            .map(|p| Tag::String(p.as_str().into()))
                            .collect(),
                    ),
                )]),
                Component::WrittenBook(b) => Tag::compound([
                    ("title", Tag::String(b.title.as_str().into())),
                    ("author", Tag::String(b.author.as_str().into())),
                    ("generation", Tag::Int(b.generation)),
                    ("pages", Tag::List(b.pages.clone())),
                    ("resolved", Tag::Byte(i8::from(b.resolved))),
                ]),
                Component::Enchantments(list) => {
                    // Enchantments are a synced (data-driven) registry: the
                    // ids index the server's RegistryData, not the builtin
                    // report, so a datapack enchantment names correctly.
                    let levels = list.iter().filter_map(|(id, level)| {
                        let name = r.registries.name("minecraft:enchantment", *id)?;
                        Some((name, Tag::Int(*level)))
                    });
                    Tag::compound(levels)
                }
                Component::AttackRange(v) => Tag::compound([
                    ("min_reach", Tag::Float(v[0].to_bits())),
                    ("max_reach", Tag::Float(v[1].to_bits())),
                    ("min_attack_momentum", Tag::Float(v[2].to_bits())),
                    ("max_attack_momentum", Tag::Float(v[3].to_bits())),
                    ("hitbox_margin", Tag::Float(v[4].to_bits())),
                    ("mob_factor", Tag::Float(v[5].to_bits())),
                ]),
                Component::Profile(p) => profile_tag(p),
                // PotionContents' NBT: the potion by name and the custom colour.
                Component::Potion { potion, color, .. } => {
                    let potion = potion.and_then(|id| r.builtin.get("minecraft:potion")?.name(id));
                    Tag::compound(
                        potion
                            .map(|name| ("potion", Tag::String(name.into())))
                            .into_iter()
                            .chain(color.map(|c| ("custom_color", Tag::Int(c)))),
                    )
                }
                // ItemStackTemplate.CODEC: id, count and components.
                Component::Stacks(stacks) => Tag::List(
                    stacks
                        .iter()
                        .map(|t| {
                            let mut entries = vec![
                                ("id", Tag::String(self.item_name(t.item).as_ref().into())),
                                ("count", Tag::Int(t.count)),
                            ];
                            if let Some(c) = self.protocol_components(t) {
                                entries.push(("components", c));
                            }
                            Tag::compound(entries)
                        })
                        .collect(),
                ),
                Component::CustomModelData {
                    floats,
                    flags,
                    strings,
                    colors,
                } => Tag::compound([
                    (
                        "floats",
                        Tag::List(floats.iter().map(|f| Tag::Float(f.to_bits())).collect()),
                    ),
                    (
                        "flags",
                        Tag::List(flags.iter().map(|b| Tag::Byte(i8::from(*b))).collect()),
                    ),
                    (
                        "strings",
                        Tag::List(
                            strings
                                .iter()
                                .map(|s| Tag::String(s.as_str().into()))
                                .collect(),
                        ),
                    ),
                    (
                        "colors",
                        Tag::List(colors.iter().map(|c| Tag::Int(*c)).collect()),
                    ),
                ]),
                Component::TooltipDisplay { hide, hidden } => {
                    let hidden = hidden
                        .iter()
                        .filter_map(|id| Some(Tag::String(name(*id)?.into())));
                    Tag::compound([
                        ("hide_tooltip", Tag::Byte(i8::from(*hide))),
                        ("hidden_components", Tag::List(hidden.collect())),
                    ])
                }
            };
            entries.push((key, value));
        }
        Some(Tag::compound(entries))
    }
    fn to_network(&self, s: &Option<ItemStack>) -> Option<Stack> {
        let s = s.as_ref()?;
        let mut stack = Stack::new(self.item_id(&s.item), s.count as i32);
        // A creative pick keeps its potion (PotionContents by its builtin id).
        if let Some(id) = s.potion().and_then(|p| {
            let potions = self.remote.as_ref()?.builtin.get("minecraft:potion")?;
            potions.id(&horizon_world::datafix::dynamic::namespaced(p))
        }) {
            stack.added.push((
                horizon_protocol::item::component::POTION_CONTENTS,
                horizon_protocol::item::Component::Potion {
                    potion: Some(id),
                    color: None,
                    rest: Vec::new(),
                },
            ));
        }
        if s.damage > 0 {
            stack.added.push((
                horizon_protocol::item::component::DAMAGE,
                horizon_protocol::item::Component::Int(s.damage as i32),
            ));
        }
        Some(stack)
    }
    /// A line in the connection's chat, with its styles, click and hover
    /// events (ChatComponent.addMessage).
    pub(super) fn chat_line(&mut self, content: horizon_client::text::Text) {
        let text = content.plain();
        log::info!("[chat] {text}");
        self.ui.chat.add(
            &self.gui.atlas,
            content,
            None,
            self.ticks,
            self.screen == Screen::Chat,
        );
        if self.mods.running() {
            self.mods_chat(&text);
        }
        if let Some(r) = self.remote.as_mut() {
            r.chat.push((text, Instant::now()));
            r.chat_received += 1;
            if r.chat.len() > 100 {
                r.chat.remove(0);
            }
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
    /// Process what the network thread received (once per frame, like the
    /// client's packet processor), in order. Chunk packets get
    /// `CHUNK_PACKETS_PER_FRAME`: once a frame has spent it, the events from
    /// the next chunk packet on wait for the next frame, unless they have
    /// already waited `BACKLOG_LIMIT`.
    pub(super) fn poll_remote(&mut self) -> Result<()> {
        let Some(r) = self.remote.as_mut() else {
            return Ok(());
        };
        let mut events = std::mem::take(&mut r.backlog);
        events.extend(r.session.poll());
        let overdue = r
            .backlog_since
            .is_some_and(|since| since.elapsed() >= BACKLOG_LIMIT);
        let mut chunks = Duration::ZERO;
        while let Some(event) = events.pop_front() {
            let chunk =
                matches!(&event, Event::Packet(p) if matches!(**p, C::LevelChunkWithLight(_)));
            if chunk && !overdue && chunks >= CHUNK_PACKETS_PER_FRAME {
                events.push_front(event);
                break;
            }
            let started = chunk.then(Instant::now);
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
                    if let Some(r) = self.remote.as_mut() {
                        r.joined = true;
                        r.profile = uuid;
                        r.profile_name = name.clone();
                        // LevelLoadingScreen's line: no player name.
                        r.status = horizon_client::lang::get_or(
                            "multiplayer.downloadingTerrain",
                            "Loading terrain...",
                        );
                    }
                }
                Event::Brand(brand) => {
                    if let Some(remote) = &mut self.remote {
                        remote.brand = brand;
                    }
                }
                Event::Reconfigured(registries) => self.set_registries(registries),
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
                    from_server,
                } => {
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
                    let retry = self.reconnect_after(&reason, from_server);
                    self.leave_server(Some(reason))?;
                    self.offer_reconnect(retry);
                    return Ok(());
                }
                Event::Packet(packet) => self.handle_packet(*packet)?,
            }
            chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
            if self.remote.is_none() {
                break;
            }
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
        let Some((id, command)) = self.ui.chat_screen.as_mut().and_then(|s| s.take_request())
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
            self.chat_server_suggestions(id, range, list);
        }
    }
    fn set_registries(&mut self, registries: Arc<Dynamic>) {
        let biomes: Vec<u16> = registries
            .registries
            .get("minecraft:worldgen/biome")
            .map(|list| {
                list.iter()
                    .map(|(name, _)| self.res.biomes().id(name))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(r) = self.remote.as_mut() {
            r.biome_bits = ceil_log2(biomes.len());
            r.biome_map = biomes;
            r.registries = registries;
        }
    }
    /// Switch to a (possibly new) dimension: a fresh client level.
    fn enter_remote_dimension(&mut self, spawn: &play::SpawnInfo, keep_world: bool) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let dimension = r.registries.dimension(spawn.dimension_type);
        let same = r.dimension.name == dimension.name && keep_world;
        r.dimension = dimension;
        r.game_mode = spawn.game_type;
        r.loaded_sent = false;
        // A replacement LocalPlayer has a fresh sendPosition baseline.
        r.position_sender = Default::default();
        r.predictions.clear();
        self.set_mode_id(spawn.game_type);
        self.dim = Dim::from_id(&spawn.dimension);
        if !same {
            let mut streaming = Streaming::new(
                Arc::new(NoTerrain),
                self.pack.context.clone(),
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
            self.world = World::new(self.res.facts().air, spawn.seed);
            self.world.game_time = time;
            self.world.day_time = day;
            self.world.weather = weather;
            self.mobs.clear();
            self.clear_remote_objects();
            self.clear_remote_projectiles();
            self.dropped.clear();
            self.particles.clear();
            self.pickups.clear();
            if let Some(r) = self.remote.as_mut() {
                r.entities.clear();
                r.lerps.clear();
                r.stepped.clear();
                r.head_lerps.clear();
                r.pending_items.clear();
                // The entity ids are the old world's; the new dimension's
                // player entities re-map to their profiles on AddEntity.
                r.level_players.clear();
                // A new ClientLevel: a new chunk storage, centred on 0, 0.
                r.chunk_center = (0, 0);
            }
            self.clear_lod();
            self.pending_uploads.clear();
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.clear_sections();
            }
        }
        let has_sky = self
            .remote
            .as_ref()
            .is_none_or(|r| r.dimension.has_skylight);
        self.streaming.sky_light = has_sky;
        self.streaming.nether_shading = self.dim == Dim::Nether;
        // The seed in SpawnInfo is already the hashed one the client's BiomeManager holds.
        self.streaming.biome_zoom = horizon_core::biome::BiomeZoom::new(spawn.seed);
        self.player = None;
        self.apply_settings();
    }

    /// LevelChunkSection data into the client's fixed -64..320 column.
    fn remote_chunk(&self, p: &play::LevelChunkWithLight) -> std::result::Result<Chunk, String> {
        let r = self.remote.as_ref().ok_or("no connection")?;
        let sections = p
            .chunk
            .sections(r.dimension.sections(), 16, r.biome_bits)
            .map_err(|e| e.to_string())?;
        let mut chunk = Chunk::empty(p.x, p.z);
        let offset = (r.dimension.min_y - MIN_Y).div_euclid(16);
        // Biome table index per quart cell of the dimension's own sections.
        let mut biome_cells: Vec<[u32; 64]> = Vec::with_capacity(sections.len());
        for (i, s) in sections.iter().enumerate() {
            let target = offset + i as i32;
            let mut cells = [0u32; 64];
            for (j, cell) in cells.iter_mut().enumerate() {
                *cell = u32::from(
                    r.biome_map
                        .get(s.biomes.get(j) as usize)
                        .copied()
                        .unwrap_or(0),
                );
            }
            biome_cells.push(cells);
            if !(0..SECTIONS as i32).contains(&target) {
                continue;
            }
            // A global section is as wide as the server's registry: 16 bits
            // from a vanilla server, wider from a Horizon host with the same
            // content mods. Read it at that width, then widen it to ours.
            let ours = block_strategy();
            let strategy = if ours.wire_has_palette(s.states.bits) {
                ours
            } else {
                Strategy::blocks(s.states.bits)
            };
            let palette = s.states.palette.iter().map(|&p| p as u32).collect();
            let words = s.states.data.iter().map(|&w| w as u64).collect();
            let mut blocks = PalettedContainer::from_wire(strategy, s.states.bits, palette, words)
                .map_err(|e| e.to_string())?;
            if strategy.global_bits != ours.global_bits {
                let mut values = [0u32; 4096];
                blocks.unpack(&mut values);
                // Ids past our registry (a host with mods we lack) read as air.
                let limit = 1u64 << ours.global_bits;
                for v in &mut values {
                    if u64::from(*v) >= limit {
                        *v = 0;
                    }
                }
                blocks = PalettedContainer::from_values(ours, &values);
            }
            let mut section = Section {
                blocks,
                biomes: PalettedContainer::from_values(BIOME_STRATEGY, &cells),
                light: Light::Uniform(0xF0),
            };
            section.compact();
            chunk.sections[target as usize] = Arc::new(section);
        }
        // Sections outside the dimension's range take its nearest biomes
        // (the column's top or bottom cell layer repeated).
        if !biome_cells.is_empty() {
            let last = biome_cells.len() - 1;
            for si in 0..SECTIONS as i32 {
                let i = si - offset;
                if (0..sections.len() as i32).contains(&i) {
                    continue;
                }
                let (edge, layer) = if i < 0 { (0, 0) } else { (last, 3) };
                let mut cells = [0u32; 64];
                for (j, cell) in cells.iter_mut().enumerate() {
                    *cell = biome_cells[edge][layer * 16 + j % 16];
                }
                Arc::make_mut(&mut chunk.sections[si as usize]).biomes =
                    PalettedContainer::from_values(BIOME_STRATEGY, &cells);
            }
        }
        // ClientPacketListener.applyLightData: the column keeps the light
        // the server sent; nothing is relit.
        let (layers, above) = self.wire_light(&p.light);
        let sky = r.dimension.has_skylight;
        horizon_client::light::light_from_layers(&mut chunk, &layers, above, sky);
        Ok(chunk)
    }
    /// `ClientPacketListener.readSectionList`: the sky and block layers of a
    /// light packet, by section of the client's column, and the sky light
    /// above the column's top section (from the light section just above
    /// it, else open sky).
    fn wire_light(
        &self,
        light: &horizon_protocol::chunk::LightData,
    ) -> (Vec<LightLayers>, [u8; 256]) {
        let mut layers: Vec<LightLayers> = vec![(None, None); SECTIONS];
        let Some(r) = self.remote.as_ref() else {
            return (layers, [0; 256]);
        };
        let sky = r.dimension.has_skylight;
        let mut above = [if sky { 15 } else { 0 }; 256];
        // Light section b is the dimension's section b - 1.
        let first = (r.dimension.min_y - MIN_Y).div_euclid(16) - 1;
        for (b, layer) in light
            .layers(r.dimension.sections() + 2)
            .into_iter()
            .enumerate()
        {
            let si = first + b as i32;
            if (0..SECTIONS as i32).contains(&si) {
                layers[si as usize] = layer;
            } else if si == SECTIONS as i32
                && sky
                && let Some(l) = &layer.0
            {
                for (i, a) in above.iter_mut().enumerate() {
                    *a = (l[i >> 1] >> ((i & 1) * 4)) & 15;
                }
            }
        }
        (layers, above)
    }
    /// `handleLightUpdatePacket`: the sections it names take its light.
    fn remote_light_update(&mut self, p: &play::LightUpdate) {
        let Some(sky) = self.remote.as_ref().map(|r| r.dimension.has_skylight) else {
            return;
        };
        if !self.world.chunks.contains_key(&(p.x, p.z)) {
            return;
        }
        let (layers, _) = self.wire_light(&p.light);
        let Some(chunk) = self.world.chunks.get_mut(&(p.x, p.z)) else {
            return;
        };
        let changed =
            horizon_client::light::update_light_layers(Arc::make_mut(chunk), &layers, sky);
        Streaming::light_updated(&mut self.world, p.x, p.z, &changed);
    }
    /// `ClientChunkCache.Storage.inRange`: within the storage around the
    /// view centre, `calculateStorageRange` (the server's radius, at least
    /// two, plus three) chunks each way.
    fn remote_chunk_in_range(&self, x: i32, z: i32) -> bool {
        self.remote.as_ref().is_none_or(|r| {
            let range = r.view_distance.max(2) + 3;
            (x - r.chunk_center.0).abs() <= range && (z - r.chunk_center.1).abs() <= range
        })
    }
    /// The columns a moved or resized storage no longer holds.
    fn drop_remote_chunks_out_of_range(&mut self) {
        let gone: Vec<(i32, i32)> = self
            .world
            .chunks
            .keys()
            .copied()
            .filter(|&(x, z)| !self.remote_chunk_in_range(x, z))
            .collect();
        for (x, z) in gone {
            self.streaming.forget(&mut self.world, x, z);
            self.world
                .block_entities
                .retain(|k, _| (k.0 >> 4, k.2 >> 4) != (x, z));
            if let Some(renderer) = self.renderer.as_mut() {
                for sy in 0..SECTIONS as i32 {
                    renderer.remove((x, sy, z));
                }
            }
        }
    }
    /// `handleBlockEntityData`: the block entity at `pos`, when it is of the
    /// packet's type, loads the tag (`loadWithComponents`); an empty tag
    /// changes nothing.
    fn remote_block_entity_data(&mut self, pos: [i32; 3], kind: i32, tag: &Tag) {
        let key = (pos[0], pos[1], pos[2]);
        let Some(name) = self
            .remote
            .as_ref()
            .and_then(|r| r.builtin.get("minecraft:block_entity_type")?.name(kind))
        else {
            return;
        };
        let Some(entity) = self.world.block_entities.get(&key) else {
            return;
        };
        let Tag::Compound(fields) = tag else {
            return;
        };
        if entity.id() != name || fields.is_empty() {
            return;
        }
        let mut full = fields.clone();
        full.insert("id".into(), Tag::String(name.into()));
        for (k, v) in ["x", "y", "z"].into_iter().zip(pos) {
            full.insert(k.into(), Tag::Int(v));
        }
        if let Some((_, loaded)) = horizon_client::save::decode_block_entity(&Tag::Compound(full)) {
            self.world.block_entities.insert(key, loaded);
            // The section is drawn again with what the block entity shows.
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let _ = self.world.set(pos[0], pos[1], pos[2], state);
        }
    }
    /// `handleAnimate`: a player wakes up, or an entity gets the critical
    /// hit (`CRIT`) or magic critical hit (`ENCHANTED_HIT`) emitter.
    fn remote_animate(&mut self, id: i32, action: u8) {
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let me = self.remote.as_ref().is_some_and(|r| r.entity_id == id);
        let body = if me {
            self.player.as_ref().map(|p| {
                let d = p.dimensions;
                (
                    [p.position.x, p.position.y, p.position.z],
                    f64::from(d.width),
                    f64::from(d.height),
                )
            })
        } else {
            self.mobs.iter().find(|m| m.id == id as u32).map(|m| {
                (
                    [m.position.x, m.position.y, m.position.z],
                    m.kind.width,
                    m.kind.height,
                )
            })
        };
        let Some((feet, width, height)) = body else {
            return;
        };
        let particle = match action {
            // Player.stopSleepInBed.
            0 => {
                if me {
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
            .tracking_emitter(ParticleOptions::simple(particle), feet, width, height);
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
    fn sync_block_entity(&mut self, pos: (i32, i32, i32), state: BlockStateId) {
        let name = self.res.facts().get(state).name.clone();
        match BlockEntity::for_block(&name) {
            Some(entity) => {
                let same = self
                    .world
                    .block_entities
                    .get(&pos)
                    .is_some_and(|e| e.id() == entity.id());
                if !same {
                    self.world.block_entities.insert(pos, entity);
                }
            }
            None => {
                self.world.block_entities.remove(&pos);
            }
        }
    }
    /// ClientLevel.setServerVerifiedBlockState: a block the client predicted keeps its
    /// predicted state until the server acknowledges the action; the server's state is
    /// remembered for then.
    fn server_block(&mut self, pos: [i32; 3], state: i32) {
        if let Some((_, known)) = self
            .remote
            .as_mut()
            .and_then(|r| r.predictions.get_mut(&pos))
        {
            *known = state;
            return;
        }
        self.set_block(pos, state);
    }
    /// BlockStatePredictionHandler.endPredictionsUpTo (ClientboundBlockChangedAckPacket): the
    /// actions up to `sequence` are settled, so the blocks they predicted take the state the
    /// server gave them (a break the server refused comes back).
    fn end_predictions(&mut self, sequence: i32) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let settled: Vec<_> = r
            .predictions
            .iter()
            .filter(|(_, (s, _))| *s <= sequence)
            .map(|(&pos, &(_, state))| (pos, state))
            .collect();
        for (pos, state) in settled {
            if let Some(r) = self.remote.as_mut() {
                r.predictions.remove(&pos);
            }
            if self.world.block(pos[0], pos[1], pos[2]).0 as i32 != state {
                self.set_block(pos, state);
            }
        }
    }
    fn set_block(&mut self, pos: [i32; 3], state: i32) {
        let state = BlockStateId(state.max(0) as u32);
        if self.world.loaded(pos[0], pos[2])
            && self.world.set(pos[0], pos[1], pos[2], state).is_ok()
        {
            self.sync_block_entity((pos[0], pos[1], pos[2]), state);
        }
        // A block we just placed: play its place sound (the server skips the placer).
        if let Some((target, _)) = self.remote.as_ref().and_then(|r| r.placing)
            && target == pos
            && !self.res.facts().is_air(state)
        {
            let s = self.res.facts().get(state).sounds.clone();
            let at = [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            ];
            self.audio
                .play(&s.place, Some(at), (s.volume + 1.0) / 2.0, s.pitch * 0.8);
            if let Some(r) = self.remote.as_mut() {
                r.placing = None;
            }
        }
    }
    /// `recreateFromPacket`: yRot and xRot from the packet, the body and head
    /// both at its head yaw (LivingEntity).
    fn spawn_entity(&mut self, id: i32, kind: &str, position: [f64; 3], rotation: [f32; 3]) {
        let [yaw, pitch, head] = rotation;
        let at = Vec3::new(position[0], position[1], position[2]);
        let mob_type = if kind == "minecraft:player" {
            self.remote.as_ref().and_then(|r| r.player_type.clone())
        } else {
            self.res.mob_types().get(kind).cloned()
        };
        if kind == "minecraft:experience_orb" {
            // Its value (ExperienceOrb.DATA_VALUE) follows in SetEntityData.
            self.orbs.retain(|o| o.id != id as u32);
            self.orbs.push(ExperienceOrb {
                id: id as u32,
                position: at,
                previous: at,
                velocity: Vec3::ZERO,
                value: 1,
                age: 0,
                on_ground: false,
            });
        }
        if let Some(t) = mob_type {
            let mut mob = Mob::new(id as u32, t, at, head);
            super::remote_mob_data::client_defaults(&mut mob);
            (mob.yaw, mob.previous_yaw) = (yaw, yaw);
            (mob.head_pitch, mob.previous_head_pitch) = (pitch, pitch);
            mob.persistent = true;
            self.mobs.retain(|m| m.id != id as u32);
            self.mobs.push(mob);
            if let Some(r) = self.remote.as_mut() {
                r.stepped.remove(&id);
            }
        }
        if let Some(r) = self.remote.as_mut() {
            if kind == "minecraft:item" {
                r.pending_items.insert(id, (position, Vec3::ZERO));
            }
            r.entities.insert(
                id,
                Tracked {
                    kind: kind.to_owned(),
                    position,
                },
            );
        }
    }
    /// A move packet for entity `id`: to the end of `path` (None: a
    /// rotation alone), turned to `rotation` if it has one. `far_snaps`:
    /// an `EntityPositionSync`, which puts an entity more than 64 blocks
    /// away there at once.
    fn move_entity(
        &mut self,
        id: i32,
        path: Option<Path>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
        far_snaps: bool,
    ) {
        let mob = self.mobs.iter_mut().find(|m| m.id == id as u32);
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
            if let Some(m) = mob {
                // A living entity: LivingEntity.createInterpolationHandler's
                // SteppedInterpolationHandler, over its type's update interval.
                let entity = PosRot {
                    position: [m.position.x, m.position.y, m.position.z],
                    yaw: m.yaw,
                    pitch: m.head_pitch,
                };
                let far = far_snaps && {
                    let d = [0, 1, 2].map(|i| position[i] - entity.position[i]);
                    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 4096.0
                };
                let handler = r.stepped.entry(id).or_insert_with(|| {
                    let interval = horizon_client::lan::tracking::settings(&m.kind.id).1;
                    SteppedInterpolation::new(interval as i32)
                });
                let snap = if far {
                    handler.cancel();
                    let (yaw, pitch) = rotation.unwrap_or((m.yaw, m.head_pitch));
                    Some(PosRot {
                        position,
                        yaw,
                        pitch,
                    })
                } else {
                    match handler.interpolate_to(entity, path, rotation) {
                        Start::Snap(at) => Some(at),
                        Start::Interpolating | Start::Unchanged => None,
                    }
                };
                if let Some(at) = snap {
                    place_interpolated(m, at);
                    m.previous = m.position;
                }
                m.on_ground = on_ground;
                return;
            }
            if let Some(p) = self.creatures.projectiles.iter_mut().find(|p| {
                p.id == id as u32
                    && matches!(
                        p.kind,
                        horizon_client::projectiles::ProjectileKind::LargeFireball { .. }
                    )
            }) {
                if path.is_some() {
                    p.position = Vec3::new(position[0], position[1], position[2]);
                }
                if let Some((yaw, pitch)) = rotation {
                    p.yaw = yaw;
                    p.pitch = pitch;
                }
                return;
            }
            if let Some(p) = r.pending_items.get_mut(&id) {
                p.0 = position;
            }
            // ItemEntity and FallingBlockEntity keep Entity's NO_OP
            // InterpolationHandler: moveOrInterpolateTo puts them there at
            // once, and their own tick moves them on between the server's
            // updates (`tick_remote_entities`).
            let at = Vec3::new(position[0], position[1], position[2]);
            if let Some(d) = self.dropped.iter_mut().find(|d| d.id == id as u32) {
                d.position = at;
                d.on_ground = on_ground;
                return;
            }
            if let Some(f) = self.falling.iter_mut().find(|f| f.id == id as u32) {
                f.position = at;
                return;
            }
            // AbstractInterpolationHandler.interpolateTo: without a rotation,
            // the one it was heading for.
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
        self.turn_remote_object(id, rotation.map(|(yaw, _)| yaw), on_ground);
        self.move_remote_text_display(id, position, rotation);
        self.turn_remote_projectile(id, rotation);
    }
    fn remove_entity(&mut self, id: i32) {
        // Entity.setRemoved detaches every passenger, even when no empty
        // SetPassengers packet preceded the vehicle's removal. Otherwise
        // mobs and other players keep their passenger pose after it is gone.
        self.remote_set_passengers(id, &[]);
        self.mobs.retain(|m| m.id != id as u32);
        self.orbs.retain(|o| o.id != id as u32);
        self.dropped.retain(|d| d.id != id as u32);
        self.remove_remote_object(id as u32);
        self.remove_remote_projectile(id as u32);
        if let Some(r) = self.remote.as_mut() {
            r.entities.remove(&id);
            r.lerps.remove(&id);
            r.stepped.remove(&id);
            r.head_lerps.remove(&id);
            r.pending_items.remove(&id);
        }
    }
    /// Map a menu slot to Horizon's inventory screen slot ids.
    fn menu_slot(menu: &str, slot: i32) -> Option<usize> {
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
            // A workstation: its inputs, its result, then the inventory.
            m => {
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
    fn menu_slot_index(menu: &str, internal: usize) -> Option<usize> {
        (0..)
            .map_while(|slot| Self::menu_slot(menu, slot))
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
        let Some(internal) = Self::menu_slot(&menu, slot) else {
            return;
        };
        if internal == 200 {
            return; // the result slot is recomputed from the grid
        }
        *self.slot_mut(internal) = item;
        if internal >= super::stations::INPUT && self.station.is_some() {
            self.remote_station_slot_changed(internal);
        }
    }

    fn handle_packet(&mut self, packet: C) -> Result<()> {
        match packet {
            // ClientCommonPacketListenerImpl.handlePing runs on the game
            // thread, after all preceding packets (especially teleports).
            C::Ping(p) => self.send(S::Pong(p)),
            // PingDebugMonitor.onPongReceived.
            C::PongResponse(p) => {
                let delay = self.debug_millis() - p.time;
                self.debug_charts.ping.log(delay);
            }
            C::Login(p) => {
                self.level.hardcore = p.hardcore;
                if let Some(r) = self.remote.as_mut() {
                    r.online_mode = p.online_mode;
                    r.entity_id = p.player_id;
                    r.view_distance = p.chunk_radius;
                }
                self.streaming.radius =
                    self.settings.render_distance.min(p.chunk_radius + 1).max(2);
                self.enter_remote_dimension(&p.spawn, false);
                let view = client_view_distance(self.settings.render_distance);
                if let Some(r) = self.remote.as_mut() {
                    r.sent_view = view;
                }
                self.send(S::ClientInformation(
                    horizon_protocol::packets::configuration::ClientInformation::new(view),
                ));
            }
            C::Respawn(p) => {
                // A new player, without effects: the server sends them again.
                self.potions.effects.clear();
                self.enter_remote_dimension(&p.spawn, true);
                self.screen = Screen::Playing;
                if let Some(r) = self.remote.as_mut() {
                    r.death_message = None;
                    r.health = (20.0, 20, 5.0);
                }
                self.capture(true);
            }
            C::PlayerPosition(p) => self.remote_teleport(&p)?,
            C::PlayerRotation(p) => {
                self.yaw = if p.relative_yaw {
                    self.yaw + p.yaw
                } else {
                    p.yaw
                };
                self.pitch = (if p.relative_pitch {
                    self.pitch + p.pitch
                } else {
                    p.pitch
                })
                .clamp(-90.0, 90.0);
            }
            // ClientChunkCache.replaceWithPacketData: a column outside the
            // storage around the view centre is ignored.
            C::LevelChunkWithLight(p) if !self.remote_chunk_in_range(p.x, p.z) => {
                log::warn!(
                    "Ignoring chunk since it's not in the view range: {}, {}",
                    p.x,
                    p.z
                );
            }
            C::LevelChunkWithLight(p) => match self.remote_chunk(&p) {
                Ok(chunk) => {
                    let (x, z) = (p.x, p.z);
                    self.streaming.insert_remote(&mut self.world, chunk);
                    self.world
                        .block_entities
                        .retain(|k, _| (k.0 >> 4, k.2 >> 4) != (x, z));
                    for info in &p.chunk.block_entities {
                        let pos = (
                            x * 16 + i32::from(info.packed_xz as u8 >> 4),
                            i32::from(info.y),
                            z * 16 + i32::from(info.packed_xz as u8 & 15),
                        );
                        let state = self.world.block(pos.0, pos.1, pos.2);
                        self.sync_block_entity(pos, state);
                        // LevelChunk.replaceWithPacketData: the block entity loads its update tag.
                        self.remote_block_entity_data([pos.0, pos.1, pos.2], info.kind, &info.tag);
                    }
                }
                Err(e) => log::warn!("Horizon: chunk {} {}: {e}", p.x, p.z),
            },
            C::LightUpdate(p) => self.remote_light_update(&p),
            C::ForgetLevelChunk(p) => {
                let [x, z] = p.pos;
                self.streaming.forget(&mut self.world, x, z);
                self.world
                    .block_entities
                    .retain(|k, _| (k.0 >> 4, k.2 >> 4) != (x, z));
                if let Some(renderer) = self.renderer.as_mut() {
                    for sy in 0..SECTIONS as i32 {
                        renderer.remove((x, sy, z));
                    }
                }
            }
            C::ChunkBatchFinished(_) => {
                // ChunkBatchSizeCalculator's ceiling; meshing runs on workers.
                self.send(S::ChunkBatchReceived(play::ChunkBatchReceived {
                    chunks_per_tick: 64.0,
                }));
            }
            C::BlockUpdate(p) => self.server_block(p.pos, p.state),
            // ClientPacketListener.handleBlockDestruction: another player's crack.
            C::BlockDestruction(p) => {
                self.destroy_block_progress(p.id, p.pos, i32::from(p.progress));
            }
            C::SectionBlocksUpdate(p) => {
                let [sx, sy, sz] = p.section;
                for change in p.changes {
                    let local = (change & 0xFFF) as i32;
                    let pos = [
                        sx * 16 + (local >> 8 & 15),
                        sy * 16 + (local & 15),
                        sz * 16 + (local >> 4 & 15),
                    ];
                    self.server_block(pos, (change >> 12) as i32);
                }
            }
            C::BlockChangedAck(p) => self.end_predictions(p.sequence),
            // ClientLevel.blockEvent: BlockEntity.triggerEvent (chest lids,
            // shulker boxes, bells) when the block is still the one it names.
            C::BlockEvent(p) => {
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
            // ClientLevel.levelEvent: LevelEventHandler, as this client's own
            // level plays them (particles and sounds: a spawner's burst, bone
            // meal, waxing, a block broken by someone else).
            C::LevelEvent(p) => self.level_event_at(p.kind, p.pos, p.data),
            C::LevelParticles(p) => self.remote_level_particles(&p),
            C::Explode(p) => self.remote_explode(&p),
            C::Sound(p) => {
                let name = match &p.sound.1 {
                    Some((location, _)) => Some(location.clone()),
                    None => self.remote.as_ref().and_then(|r| {
                        r.builtin
                            .get("minecraft:sound_event")?
                            .name(p.sound.0)
                            .map(str::to_owned)
                    }),
                };
                if let Some(name) = name {
                    let at = [
                        f64::from(p.x) / 8.0,
                        f64::from(p.y) / 8.0,
                        f64::from(p.z) / 8.0,
                    ];
                    self.audio.play(&name, Some(at), p.volume, p.pitch);
                }
            }
            C::StopSound(p) => self
                .audio
                .stop_sounds(p.sound.as_deref(), p.source.map(|s| s as usize)),
            C::AddEntity(p) => {
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
                    if kind == "minecraft:player" {
                        r.level_players.insert(p.id, p.uuid);
                    } else {
                        r.level_players.remove(&p.id);
                    }
                }
                self.spawn_entity(p.id, &kind, p.position, rotation);
                // Entity.recreateFromPacket: setDeltaMovement(packet.getMovement()).
                self.remote_motion(p.id, p.velocity);
                if !self.spawn_remote_object(p.id, &kind, p.position, rotation[0], p.data) {
                    self.spawn_remote_projectile(
                        p.id,
                        &kind,
                        p.position,
                        (rotation[0], rotation[1]),
                        p.velocity,
                        p.data,
                    );
                }
                self.move_remote_text_display(p.id, p.position, Some((rotation[0], rotation[1])));
            }
            C::RemoveEntities(p) => {
                for id in p.ids {
                    if let Some(r) = self.remote.as_mut() {
                        r.level_players.remove(&id);
                    }
                    self.remove_entity(id);
                }
            }
            C::MoveEntityPos(play::MoveEntityPos(m))
            | C::MoveEntityPosRot(play::MoveEntityPosRot(m))
            | C::MoveEntityRot(play::MoveEntityRot(m)) => {
                let base = self
                    .remote
                    .as_ref()
                    .and_then(|r| r.entities.get(&m.id))
                    .map(|t| t.position);
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
                self.move_entity(m.id, path, rotation, m.on_ground, false);
            }
            C::EntityPositionSync(p) => {
                let path = match p.path {
                    play::PositionPath::Linear(at) => Path::Linear(at),
                    play::PositionPath::Stepped(steps) => Path::Stepped(steps),
                };
                self.move_entity(p.id, Some(path), Some((p.yaw, p.pitch)), p.on_ground, true)
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
                self.move_entity(
                    p.id,
                    Some(Path::Linear(position)),
                    Some((p.yaw, p.pitch)),
                    p.on_ground,
                    false,
                );
            }
            C::SetPassengers(p) => self.remote_set_passengers(p.vehicle, &p.passengers),
            // ClientPacketListener.handleEntityLinkPacket: who holds a mob's
            // lead (this player, another player or a mob), for the rope.
            C::SetEntityLink(p) => {
                let me = self.remote.as_ref().map(|r| r.entity_id);
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.source as u32) {
                    use horizon_client::mobs::leash::Holder;
                    m.extra.leash = match p.dest {
                        0 => None,
                        d if Some(d) == me => Some(Holder::Player),
                        d => Some(Holder::Mob(d as u32)),
                    };
                }
            }
            C::MoveVehicle(p) => self.remote_move_vehicle(p.position, p.yaw),
            // ClientPacketListener.handleRotateMob: lerpHeadTo(yHeadRot, 3).
            C::RotateHead(p) => {
                if let Some(r) = self.remote.as_mut() {
                    let yaw = horizon_protocol::buf::unpack_degrees(p.head_yaw);
                    r.head_lerps.insert(p.id, (yaw, 3));
                }
            }
            C::SetEntityData(p) => {
                // Entity.DATA_SHARED_FLAGS_ID, DATA_CUSTOM_NAME and
                // DATA_CUSTOM_NAME_VISIBLE: server NPCs and holograms rely on
                // these instead of saved NBT.
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    for (index, _, value) in &p.values {
                        match (index, value) {
                            (0, Value::Byte(flags)) => m.invisible = flags & 0x20 != 0,
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
                self.remote_mob_data(p.id, &p.values);
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32)
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
                // LivingEntity.onSyncedDataUpdated(DATA_LIVING_ENTITY_FLAGS):
                // another player starts using the item in a hand (eating, a
                // bow drawn, a shield up), or stops.
                let other = self
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.level_players.contains_key(&p.id) && r.entity_id != p.id);
                if other && let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    for (index, _, value) in &p.values {
                        if let (8, Value::Byte(flags)) = (index, value) {
                            m.extra.using_off_hand = flags & 2 != 0;
                            if flags & 1 == 0 {
                                m.extra.using_item = 0;
                            } else if m.extra.using_item == 0 {
                                m.extra.using_item = 1;
                            }
                        }
                    }
                }
                // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): another
                // player lies down in their bed (drawn lying in it), or gets up.
                if other {
                    for (index, _, value) in &p.values {
                        if let (14, Value::OptPos(bed)) = (index, value) {
                            let code = bed.map_or(0, |b| self.bed_facing(b));
                            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                                m.extra.ints[1] = code;
                            }
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
                            }
                            // Entity.DATA_SHARED_FLAGS_ID's sprinting bit: the
                            // server stops the sprint after a hit's extra
                            // knockback (the flag only, as assignValues sets
                            // it: the speed modifier and wasSprinting stay).
                            (0, Value::Byte(flags)) => {
                                if let Some(pl) = self.player.as_mut() {
                                    pl.sprinting = flags & 0x08 != 0;
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
                            (17, Value::Float(amount)) => {
                                if let Some(pl) = self.player.as_mut() {
                                    let _ = pl.internal_set_absorption(*amount);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                // Entity.DATA_SHARED_FLAGS_ID's crouching bit: another
                // player's crouch (their model and Entity.isDiscrete's name tag).
                if self
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.level_players.contains_key(&p.id))
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32)
                {
                    for (index, _, value) in &p.values {
                        if let (0, Value::Byte(flags)) = (index, value) {
                            m.extra.crouching = flags & 2 != 0;
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
                        o.value = (*v).max(0) as u32;
                    }
                }
                self.remote_object_data(p.id, &p.values);
                self.remote_projectile_data(p.id, &p.values);
            }
            // ClientPacketListener.handleSetEquipment: what a mob or another
            // player holds and wears (EquipmentSlot ordinals).
            C::SetEquipment(p) => {
                self.remote_object_equipment(p.id, &p.slots);
                let items: Vec<(u8, Option<Arc<str>>)> = p
                    .slots
                    .iter()
                    .map(|(slot, stack)| (*slot, stack.as_ref().map(|s| self.item_name(s.item))))
                    .collect();
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    for (slot, item) in items {
                        let e = &mut m.extra;
                        match slot {
                            0 => e.main_hand = item,
                            1 => e.off_hand = item,
                            2 => e.feet = item,
                            3 => e.legs = item,
                            4 => e.chest = item,
                            5 => e.head = item,
                            _ => {}
                        }
                    }
                }
            }
            C::TakeItemEntity(p) => {
                // An orb's sound comes from the server, at the orb.
                let orb = self.orbs.iter().any(|o| o.id == p.item as u32);
                if !orb
                    && self
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.entity_id == p.player)
                    && let Some(eye) = self.eye_position()
                {
                    let pitch = ((self.world.random.next_float() - self.world.random.next_float())
                        * 0.7
                        + 1.0)
                        * 2.0;
                    self.audio
                        .play("minecraft:entity.item.pickup", Some(eye), 0.2, pitch);
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
                }
                self.remove_entity(p.item);
            }
            // ClientPacketListener.handleSetEntityMotion: knockback on this player.
            C::SetEntityMotion(p) if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) => {
                if let Some(pl) = self.player.as_mut() {
                    pl.velocity = Vec3::new(p.velocity[0], p.velocity[1], p.velocity[2]);
                }
            }
            // The same packet for another entity: Entity.lerpMotion.
            C::SetEntityMotion(p) => self.remote_motion(p.id, p.velocity),
            // ClientPacketListener.handleDamageEvent: LivingEntity.handleDamageEvent
            // jolts the walk animation and restarts the hurt flash. Its hurt
            // sound comes as a sound from Horizon's host, which plays it itself.
            // This player's own hurt tilt and flash come from the server too
            // (sent to the player itself): its own moves never hurt it.
            C::DamageEvent(p) => {
                if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                    if let Some(pl) = self.player.as_mut() {
                        pl.health.animate_hurt();
                    }
                } else if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    m.limb_speed = 1.5;
                    m.hurt_time = 10;
                }
            }
            C::HurtAnimation(p) => {
                if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                    if let Some(pl) = self.player.as_mut() {
                        pl.health.animate_hurt();
                    }
                } else if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    m.hurt_time = 10;
                }
            }
            C::EntityEvent(p) => {
                // LocalPlayer.setPermissions also refreshes an open World Options screen.
                if (24..=28).contains(&p.event)
                    && self.remote.as_ref().is_some_and(|r| r.entity_id == p.id)
                {
                    self.ui.registries.permission = (p.event - 24) as u8;
                    self.remote_world_options_permission(p.event >= 26);
                }
                if p.event == 1
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32)
                    && m.species() == horizon_client::mobs::Species::Rabbit
                {
                    horizon_client::mobs::rabbit::hop_event(m);
                }
                // EntityEvent.DEATH: start the death animation.
                if p.event == 3
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32)
                {
                    m.health = 0.0;
                }
                if p.event == super::totem::TOTEM_EVENT {
                    self.remote_totem(p.id);
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
                } else if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    m.swing();
                }
            }
            // The player's own instances: its maximum health (the hearts the
            // HUD draws and the health it may hold), reach, speed and so on.
            // Then the received health again, which the old maximum clamped.
            C::UpdateAttributes(p) if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) => {
                if let Some(pl) = self.player.as_mut() {
                    horizon_client::lan::attributes::apply(&mut pl.attributes, &p.values);
                }
                self.apply_remote_health();
            }
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
                if p.health <= 0.0 && self.screen != Screen::Dead && self.player.is_some() {
                    self.screen = Screen::Dead;
                    self.capture(false);
                }
            }
            C::PlayerCombatKill(p) => {
                let lang = self
                    .remote
                    .as_ref()
                    .map(|r| r.lang.clone())
                    .unwrap_or_default();
                let message = plain(&p.message, &lang);
                let me = self.remote.as_ref().is_some_and(|r| r.entity_id == p.id);
                if let Some(r) = self.remote.as_mut() {
                    r.death_message = Some(message);
                }
                // ClientPacketListener.handlePlayerCombatKill: the death
                // screen, whatever health was last sent.
                if me && self.screen != Screen::Dead && self.player.is_some() {
                    self.screen = Screen::Dead;
                    self.capture(false);
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
            // ClientPacketListener.handleSetTime: the game time, then
            // ClientClockManager.handleUpdates: the clock catches up with the
            // game time and takes the server's state.
            C::SetTime(p) => {
                self.world.game_time = p.game_time;
                self.remote_clock_tick();
                let clock = {
                    let r = self.remote.as_ref();
                    let names =
                        r.and_then(|r| r.registries.registries.get("minecraft:world_clock"));
                    let wanted = r.map(|r| r.dimension.name.clone()).unwrap_or_default();
                    p.clocks
                        .iter()
                        .find(|c| {
                            names
                                .and_then(|n| n.get(c.clock as usize))
                                .is_some_and(|(n, _)| *n == wanted)
                        })
                        .or_else(|| p.clocks.first())
                        .cloned()
                };
                if let Some(c) = clock {
                    self.world.day_time = c.total_ticks;
                    if let Some(r) = self.remote.as_mut() {
                        r.clock = Some(c.clock);
                        r.clock_rate = c.rate;
                        r.clock_partial = c.partial_tick;
                    }
                }
            }
            C::GameEvent(p) => match p.event {
                // GUARDIAN_ELDER_EFFECT: the apparition, and the curse sound
                // unless the elder guardian is silent.
                10 => {
                    self.creatures.elder_apparition = super::mob_blocks::APPARITION_TICKS;
                    if p.param == 1.0
                        && let Some(at) = self.player.as_ref().map(|m| m.position)
                    {
                        self.audio.play(
                            "minecraft:entity.elder_guardian.curse",
                            Some([at.x, at.y, at.z]),
                            1.0,
                            1.0,
                        );
                    }
                }
                // WIN_GAME: the poem and credits, then PERFORM_RESPAWN.
                4 if self.screen != Screen::Win => {
                    self.open_win_screen(true, super::win::Finish::RemoteRespawn);
                }
                1 => {
                    self.world.weather.raining = true;
                    self.world.weather.rain_level = (0.0, 0.0);
                }
                2 => {
                    self.world.weather.raining = false;
                    self.world.weather.rain_level = (1.0, 1.0);
                }
                3 => {
                    let mode = p.param as i32;
                    self.set_mode_id(mode);
                    if let Some(r) = self.remote.as_mut() {
                        r.game_mode = mode;
                    }
                }
                7 => {
                    self.world.weather.rain_level = (p.param, p.param);
                    self.world.weather.raining = p.param > 0.0;
                }
                8 => {
                    self.world.weather.thunder_level = (p.param, p.param);
                    self.world.weather.thundering = p.param > 0.0;
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
            C::SetHeldSlot(p) => {
                self.selected = p.slot.clamp(0, 8) as usize;
                if let Some(r) = self.remote.as_mut() {
                    r.carried = Some(self.selected);
                }
            }
            C::ContainerSetContent(p) => {
                if let Some(r) = self.remote.as_mut() {
                    r.state_id = p.state;
                }
                for (slot, item) in p.items.iter().enumerate() {
                    let stack = self.to_stack(item);
                    self.set_menu_slot(p.container, slot as i32, stack);
                }
                self.carried = self.to_stack(&p.carried);
                self.remote_synced();
            }
            C::ContainerSetSlot(p) => {
                if let Some(r) = self.remote.as_mut() {
                    r.state_id = p.state;
                }
                let stack = self.to_stack(&p.item);
                self.set_menu_slot(p.container, i32::from(p.slot), stack);
                self.remote_synced();
            }
            C::SetCursorItem(p) => self.carried = self.to_stack(&p.item),
            C::SetPlayerInventory(p) => {
                let stack = self.to_stack(&p.item);
                match p.slot {
                    0..=35 => self.inventory[p.slot as usize] = stack,
                    36..=39 => self.armor[3 - (p.slot - 36) as usize] = stack,
                    // Inventory.SLOT_OFFHAND.
                    40 => self.offhand = stack,
                    _ => {}
                }
                self.remote_synced();
            }
            C::ContainerSetData(p) if self.open_menu_name(p.container) == "minecraft:lectern" => {
                self.remote_lectern_page(i32::from(p.value));
            }
            C::ContainerSetData(p) if self.remote_station_open(p.container) => {
                self.remote_station_data(usize::try_from(p.id).unwrap_or(usize::MAX), p.value);
            }
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
                        Ok(9) => c.triggered = v == 1,
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
            C::RecipeBookAdd(p) => self.remote_recipe_book_add(p),
            C::RecipeBookRemove(p) => self.remote_recipe_book_remove(p),
            C::RecipeBookSettings(p) => self.remote_recipe_book_settings(p),
            C::PlaceGhostRecipe(p) => self.remote_ghost_recipe(p),
            C::MerchantOffers(p) => self.remote_merchant_offers(p),
            C::ContainerClose(_) if self.remote_lectern_closed() => {}
            C::ContainerClose(_) => {
                if self.screen == Screen::Inventory {
                    self.remote_close_local();
                    self.screen = Screen::Playing;
                    self.capture(true);
                }
            }
            C::Unknown(id, data)
                if id == horizon_protocol::ids::play::clientbound::COMMAND_SUGGESTIONS =>
            {
                self.handle_command_suggestions(&data);
            }
            C::Commands(p) => {
                let tree = horizon_client::command::network::from_packet(
                    &p,
                    horizon_client::command::vanilla::dispatcher(),
                );
                if let Some(r) = self.remote.as_mut() {
                    r.commands = Some(tree);
                }
            }
            C::SystemChat(p) => {
                // The component as sent, translated in the client's own
                // language. ChatListener.handleOverlay for the action bar.
                let text = remote_text(&p.content);
                if p.overlay {
                    self.titles.set_overlay(text, false);
                } else {
                    self.chat_line(text);
                }
            }
            C::PlayerChat(p) => {
                use horizon_client::scoreboard::tag_to_json;
                let content = p
                    .unsigned_content
                    .as_ref()
                    .map_or_else(|| serde_json::Value::from(p.content.as_str()), tag_to_json);
                let line = self.format_chat(&p.chat_type, content);
                self.chat_line(line);
            }
            C::DisguisedChat(p) => {
                let content = horizon_client::scoreboard::tag_to_json(&p.message);
                let line = self.format_chat(&p.chat_type, content);
                self.chat_line(line);
            }
            C::PlayerInfoUpdate(p) => {
                if let Some(r) = self.remote.as_mut() {
                    // handlePlayerInfoUpdate: new entries first, then every
                    // action on every known entry.
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
                                        skins::unpack(&t.value, signature, &skins::TEXTURE_URIS);
                                    r.skin_textures.entry(e.uuid).or_insert(textures);
                                }
                            }
                        }
                    }
                    for e in p.entries {
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
                        r.skin_textures.remove(&u);
                        r.listed.remove(&u);
                    }
                }
            }
            C::TabList(p) => {
                // handleTabListCustomisation: empty text clears it.
                let text = |t: &Tag| {
                    let json = horizon_client::scoreboard::tag_to_json(t);
                    let text = horizon_client::scoreboard::comp_text(&json);
                    (!text.plain().is_empty()).then_some(text)
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
            C::SetChunkCacheRadius(p) => {
                self.streaming.radius = self.settings.render_distance.min(p.distance + 1).max(2);
                if let Some(r) = self.remote.as_mut() {
                    r.view_distance = p.distance;
                }
                // ClientChunkCache.updateViewRadius: a new storage of that size.
                self.drop_remote_chunks_out_of_range();
            }
            C::SetChunkCacheCenter(p) => {
                if let Some(r) = self.remote.as_mut() {
                    r.chunk_center = (p.x, p.z);
                }
                self.drop_remote_chunks_out_of_range();
            }
            C::BlockEntityData(p) => self.remote_block_entity_data(p.pos, p.kind, &p.tag),
            C::OpenSignEditor(p) => self.remote_open_sign_editor(p),
            C::OpenBook(p) => self.remote_open_book(p.hand),
            // handleSetSpawn: ClientLevel.setRespawnData, which spawn compasses point at.
            C::SetDefaultSpawnPosition(p) => {
                let [x, y, z] = p.pos.pos;
                self.spawn = (p.pos.dimension == "minecraft:overworld")
                    .then(|| Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5));
            }
            C::ServerData(p) => self.remote_server_data(p.icon.as_deref()),
            C::MapItemData(p) => self.remote_map_item_data(&p),
            // handleTickingState / handleTickingStep: the level's TickRateManager.
            C::TickingState(p) => {
                self.clock.set_rate(p.rate);
                self.clock.set_frozen(p.frozen);
            }
            C::TickingStep(p) => self.clock.set_frozen_ticks_to_run(p.steps),
            C::UpdateAdvancements(p) => {
                let icons = p
                    .added
                    .iter()
                    .map(|a| {
                        a.display
                            .as_ref()
                            .map_or_else(String::new, |d| self.item_name(d.icon.item).to_string())
                    })
                    .collect();
                self.remote_advancements(p, icons);
            }
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
            4 => {
                for player in &p.players {
                    board.remove_from_team(player);
                }
            }
            _ => {}
        }
    }
    /// The player list's view of the connection: the listed players (with
    /// their downloaded skins once ready), online mode, and which players
    /// are in the level with an upside-down name.
    pub(super) fn remote_tab_players(
        &mut self,
    ) -> Option<(Vec<horizon_client::tab_list::TabPlayer>, bool, Vec<u128>)> {
        self.update_remote_skins();
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
    /// `PlayerInfo.getSkin` for every player entity, including NPCs that are
    /// never in the tab list: resolve each profile's textures and give the
    /// level's player models their skin.
    fn update_remote_skins(&mut self) {
        let Some(r) = self.remote.as_ref() else {
            return;
        };
        let local = r.profile;
        let entries: Vec<(u128, horizon_client::skins::Textures)> = r
            .skin_textures
            .iter()
            .map(|(uuid, textures)| (*uuid, textures.clone()))
            .collect();
        let mut resolved: Vec<(u128, String)> = Vec::new();
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
                resolved.push((uuid, skin));
            }
        }
        resolved.retain(|(_, skin)| !skin.starts_with("skins/") || self.show_skin(skin));
        let Some((level, skins, own)) = self.remote.as_mut().map(|r| {
            r.skins = resolved.into_iter().collect();
            (
                r.level_players
                    .iter()
                    .map(|(id, uuid)| (*id as u32, *uuid))
                    .collect::<Vec<_>>(),
                r.skins.clone(),
                r.skins.get(&r.profile).cloned(),
            )
        }) else {
            return;
        };
        for (id, uuid) in level {
            let Some(skin) = skins.get(&uuid) else {
                continue;
            };
            if let Some(mob) = self
                .mobs
                .iter_mut()
                .find(|m| m.id == id && &*m.kind.id == "minecraft:player")
            {
                mob.skin = Some(skin.clone());
            }
        }
        if let Some(skin) = own
            && let Some(mob) = self.player_model.as_mut()
        {
            mob.skin = Some(skin);
        }
    }
    /// Puts a downloaded skin in an atlas skin slot: a free one, else the
    /// least recently shown.
    pub(super) fn store_skin(&mut self, location: String, rgba: Vec<u8>) {
        use horizon_client::hud::SKIN_SLOTS;
        let slot = if self.skin_slots.len() < SKIN_SLOTS {
            self.skin_slots.len()
        } else {
            let (old, slot) = self.skin_slots.remove(0);
            self.gui.atlas.sprites.remove(&old);
            slot
        };
        let Some(rect) = self
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
        self.gui.atlas.sprites.insert(location.clone(), rect);
        self.skin_slots.push((location, slot));
    }
    /// Whether a downloaded skin is in the atlas, marking it recently shown.
    fn show_skin(&mut self, location: &str) -> bool {
        let Some(i) = self.skin_slots.iter().position(|e| e.0 == location) else {
            return false;
        };
        let e = self.skin_slots.remove(i);
        self.skin_slots.push(e);
        true
    }
    /// ChatType decoration: `chat.translation_key` with sender/content parameters.
    /// ChatType.Bound.decorate: the chat type's translation, with the
    /// sender, the target and the message as its parameters, in its style.
    fn format_chat(
        &self,
        bound: &play::ChatTypeBound,
        content: serde_json::Value,
    ) -> horizon_client::text::Text {
        use horizon_client::scoreboard::{comp_text, tag_to_json};
        let Some(r) = self.remote.as_ref() else {
            return comp_text(&content);
        };
        let decoration = r
            .registries
            .data("minecraft:chat_type", bound.chat_type)
            .and_then(|t| t.get("chat"));
        let key = match decoration.and_then(|d| d.get("translation_key")) {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => "chat.type.text".into(),
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
        comp_text(&component)
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
        let bob_offset = self.world.random.next_float() * std::f32::consts::PI * 2.0;
        self.dropped.push(ItemEntity {
            health: 5,
            fire_ticks: 0,
            thrower: None,
            id: id as u32,
            stack,
            position: at,
            previous: at,
            velocity,
            on_ground: false,
            age: 0,
            pickup_delay: 0,
            bob_offset,
            ticks: 0,
        });
    }
    /// `Entity.lerpMotion` (and `recreateFromPacket`'s motion) for the
    /// entities whose own tick moves them on a client: dropped items and
    /// falling blocks.
    fn remote_motion(&mut self, id: i32, velocity: [f64; 3]) {
        let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
        if let Some(d) = self.dropped.iter_mut().find(|d| d.id == id as u32) {
            d.velocity = v;
        } else if let Some(f) = self.falling.iter_mut().find(|f| f.id == id as u32) {
            f.velocity = v;
        } else if let Some(p) = self
            .creatures
            .projectiles
            .iter_mut()
            .find(|p| p.id == id as u32)
        {
            p.velocity = v;
        } else if let Some(p) = self
            .remote
            .as_mut()
            .and_then(|r| r.pending_items.get_mut(&id))
        {
            p.1 = v;
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
    /// ClientPacketListener.handleMovePlayer.
    fn remote_teleport(&mut self, p: &play::PlayerPosition) -> Result<()> {
        use play::relative as rel;
        let current = self
            .player
            .as_ref()
            .map(|pl| pl.position)
            .unwrap_or(Vec3::ZERO);
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
        let pitch = (if p.relatives & rel::X_ROT != 0 {
            self.pitch + p.pitch
        } else {
            p.pitch
        })
        .clamp(-90.0, 90.0);
        let delta = horizon_client::player_sync::teleport_velocity(
            velocity,
            (self.yaw, self.pitch),
            (yaw, pitch),
            p,
        );
        match self.player.as_mut() {
            Some(player) => {
                player.position = position;
                player.velocity = delta;
            }
            None => {
                let mut player = PlayerMotion::new(self.motion, position)?;
                // Player.tick derives noPhysics from the current game mode.
                player.spectator = self.spectator();
                player.velocity = delta;
                self.player = Some(player);
                self.pad.player_replaced();
                self.sway = Default::default();
                self.eye = 1.62;
                self.previous_eye = 1.62;
                self.clock.reset();
                self.apply_remote_health();
                self.apply_remote_abilities();
            }
        }
        self.previous_position = position;
        self.yaw = yaw;
        self.pitch = pitch;
        if let Some(r) = self.remote.as_mut() {
            // handleMovePlayer does not change LocalPlayer.sendPosition's
            // baseline: the next tick must still report the corrected state.
            r.destroying = None;
        }
        self.mining = None;
        self.send(S::AcceptTeleportation(play::AcceptTeleportation {
            id: p.teleport,
            x: position.x,
            y: position.y,
            z: position.z,
            yaw,
            pitch,
        }));
        Ok(())
    }

    /// One client tick while connected (Minecraft.tick with a ClientLevel).
    pub(super) fn tick_remote(&mut self) -> Result<()> {
        self.tick_remote_level()?;
        // Client mods tick on any server, single player's own included.
        self.tick_mods();
        // Minecraft.tick sends this even when the player cannot tick yet,
        // including while waiting for chunks. Keep it outside early returns.
        if self.remote.as_ref().is_some_and(|r| r.joined) {
            self.send(S::ClientTickEnd(Empty));
        }
        Ok(())
    }

    fn tick_remote_level(&mut self) -> Result<()> {
        self.ticks += 1;
        self.track_experience_display();
        // Options.broadcastOptions: a changed render distance is sent again.
        let view = client_view_distance(self.settings.render_distance);
        if let Some(r) = self.remote.as_mut()
            && r.joined
            && r.sent_view != view
        {
            r.sent_view = view;
            self.send(S::ClientInformation(
                horizon_protocol::packets::configuration::ClientInformation::new(view),
            ));
        }
        // Minecraft.tick: TickRateManager.tick; ClientLevel.tickTime advances
        // the time only while the level runs normally.
        let running = self.clock.world_runs();
        self.boss_overlay.tick_darken(running);
        if running {
            self.world.game_time += 1;
            self.remote_clock_tick();
        }
        // Weather levels drift toward their targets between server updates.
        let w = &mut self.world.weather;
        w.rain_level.0 = w.rain_level.1;
        w.thunder_level.0 = w.thunder_level.1;
        if self.use_delay > 0 {
            self.use_delay -= 1;
        }
        // Minecraft.handleKeybinds precedes entity ticking and sendChanges.
        // Window callbacks only queue clicks, so an interaction cannot leak
        // between CLIENT_TICK_END and a later pong.
        self.remote_handle_input();
        // ClientLevel.tick: the border's size change.
        self.border.tick();
        self.player_border();
        self.tick_remote_entities(running);
        // ClientLevel.tickBlockEntities: chest lids, shulker boxes, bells.
        self.tick_block_entity_anims();
        self.tick_hand();
        self.tick_particles();
        self.tick_client_effects();
        // Player.tick's sleep counter (the darkening and the wake-up fade).
        self.sleep_timer = super::tick_sleep_timer(self.sleep_timer, self.sleeping.is_some());
        // Gui.tick: the titles, the action bar and the held item's name.
        self.tick_hud();
        let entity_collisions = self.player_entity_collisions();
        let pushed = self.remote_pushes();
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
        if self.remote.as_ref().is_some_and(|r| !r.loaded_sent) {
            // ClientPacketListener: the level around the player is ready.
            self.send(S::PlayerLoaded(Empty));
            if let Some(r) = self.remote.as_mut() {
                r.loaded_sent = true;
            }
            self.capture_loaded_world();
        }
        let stick = self.pad_move();
        if self.player.is_none() {
            return Ok(());
        }
        let playing = self.screen == Screen::Playing;
        let held = |k| playing && self.keys.contains(&k);
        let keys = Keys {
            forward: held(KeyCode::KeyW),
            backward: held(KeyCode::KeyS),
            left: held(KeyCode::KeyA),
            right: held(KeyCode::KeyD),
            jump: held(KeyCode::Space),
            shift: held(KeyCode::ShiftLeft),
            sprint: held(KeyCode::ControlLeft),
            analog: None,
        }
        .with_stick(stick.filter(|_| playing));
        let input = u8::from(keys.forward)
            | u8::from(keys.backward) << 1
            | u8::from(keys.left) << 2
            | u8::from(keys.right) << 3
            | u8::from(keys.jump) << 4
            | u8::from(keys.shift) << 5
            | u8::from(keys.sprint) << 6;
        // A passenger moves with its vehicle (Entity.rideTick), not by itself.
        let riding = self.screen != Screen::Dead && self.remote_ride(keys);
        // The server sends no passenger's position: they sit where their
        // vehicles are now.
        self.position_riders();
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        if !riding {
            self.previous_position = player.position;
        }
        let before = (
            player.ground.on_ground,
            [player.velocity.x, player.velocity.y, player.velocity.z],
            [player.position.x, player.position.y, player.position.z],
        );
        let was_flying = player.flying;
        self.sync_player_body();
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        let was_gliding = player.fall_flying;
        let dead = self.screen == Screen::Dead;
        // LivingEntity.isImmobile: a sleeper lies still in the bed.
        if let Some(head) = self.sleeping {
            player.position = Vec3::new(
                f64::from(head[0]) + 0.5,
                f64::from(head[1]) + 0.5625,
                f64::from(head[2]) + 0.5,
            );
            player.velocity = Vec3::ZERO;
        } else if !dead && !riding {
            player.client_side = true;
            player.velocity = player.velocity.add_vector(pushed);
            MotionWorld {
                entities: &entity_collisions,
                player,
                scene: &mut self.world,
                assets: &self.pack.assets,
                collision_binding: self.collision,
                resources: self.motion,
            }
            .controlled_simulation_tick(keys)?;
        }
        // LocalPlayer.onUpdateAbilities: flight turned on or off (a double
        // jump, a spectator, landing) is sent to the server.
        let flying = player.flying;
        // LocalPlayer.aiStep: a glide this client started is sent to the
        // server (ServerboundPlayerCommandPacket START_FALL_FLYING).
        let started_gliding = player.fall_flying && !was_gliding;
        let events = std::mem::take(&mut player.events);
        self.previous_eye = self.eye;
        self.eye += (player.dimensions.eye_height - self.eye) * 0.5;
        let crouching = player.pose == horizon_client::pose::Pose::Crouching;
        let feet = [player.position.x, player.position.y, player.position.z];
        for event in events {
            match event {
                horizon_client::motion::MotionEvent::Emission {
                    state, in_water, ..
                } if !crouching => {
                    let sounds = self.res.facts().get(state).sounds.clone();
                    // Player.playStepSound: muffled under the swim sound in water.
                    let (volume, pitch) = if in_water {
                        (sounds.volume * 0.05, sounds.pitch * 0.8)
                    } else {
                        (sounds.volume * 0.15, sounds.pitch)
                    };
                    self.audio.play(&sounds.step, Some(feet), volume, pitch);
                }
                horizon_client::motion::MotionEvent::Swim { volume, pitch } => {
                    self.audio
                        .play("minecraft:entity.player.swim", Some(feet), volume, pitch);
                }
                _ => {}
            }
        }
        // The server owns health and hunger.
        self.apply_remote_health();
        self.tick_avatar(before, false);
        // The player's own model (third person, the inventory's preview).
        self.tick_player_model();
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
        self.remote_send_state(input);
        Ok(())
    }

    /// ClientPacketListener.handlePlayerAbilities: the player flies, may fly
    /// and is invulnerable as the server says (bits 2, 4 and 1).
    fn apply_remote_abilities(&mut self) {
        let (Some(player), Some(a)) = (
            self.player.as_mut(),
            self.remote.as_ref().and_then(|r| r.abilities.as_ref()),
        ) else {
            return;
        };
        player.invulnerable = a.flags & 1 != 0;
        player.flying = a.flags & 2 != 0;
        player.may_fly = a.flags & 4 != 0;
        player.flying_speed = a.flying_speed;
    }
    /// The local player's share of every touching entity's
    /// `LivingEntity.pushEntities`, and of the boats' pushes: on a client
    /// `EntitySelector.pushableBy` lets entities push only the local player,
    /// so the server never sends these pushes. `doPush(player)` is
    /// `player.push(entity)`.
    fn remote_pushes(&self) -> Vec3 {
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
            || self.block_in_tag("minecraft:climbable", &self.block_name(feet))
        {
            return Vec3::ZERO;
        }
        let bounds = p.bounds();
        let teams = self.mob_teams();
        let mut push = Vec3::ZERO;
        for m in &self.mobs {
            let spectator = r
                .level_players
                .get(&(m.id as i32))
                .and_then(|uuid| r.tab_players.get(uuid))
                .is_some_and(|t| t.game_mode == SPECTATOR);
            // Bats never push; a spectator has no physics; passengers of the
            // same vehicle never push each other.
            if !m.alive()
                || spectator
                || m.species() == horizon_client::mobs::Species::Bat
                || self
                    .riding
                    .is_some_and(|v| v == m.id || m.extra.mount == Some(v))
                || !m.bounds().intersects(bounds)
                || !horizon_client::scoreboard::pushable_by(
                    teams.of(Who::Mob(m.id)),
                    teams.of(Who::Player),
                    true,
                )
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
        let player_team = teams.of(Who::Player);
        let area = Vec3::new(f64::from(0.2_f32), f64::from(-0.01_f32), f64::from(0.2_f32));
        for b in self.boats.iter().filter(|b| self.riding != Some(b.id)) {
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
    fn tick_remote_entities(&mut self, running: bool) {
        if running {
            self.tick_text_displays();
        }
        // EntityFluidInteraction.update for the entities this window only displays: the
        // server never sends the fluid contact, and the renderers read it (cod, salmon and
        // tropical fish lie on their side without `in_water`, #1591).
        // A frozen level ticks only the players, as below.
        let fluids: Vec<_> = {
            let w = self.mob_world_base(&[]);
            let players = self.remote.as_ref().map(|r| &r.level_players);
            self.mobs
                .iter()
                .map(|m| {
                    (running || players.is_some_and(|p| p.contains_key(&(m.id as i32))))
                        .then(|| w.fluids(m.bounds(), m.eye_y()))
                })
                .collect()
        };
        for (m, fluids) in self.mobs.iter_mut().zip(fluids) {
            if let Some(fluids) = fluids {
                m.fluids = fluids;
            }
        }
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut lerps = std::mem::take(&mut r.lerps);
        let stepped = &mut r.stepped;
        let head_lerps = &mut r.head_lerps;
        let players = &r.level_players;
        let mut using = Vec::new();
        for m in &mut self.mobs {
            let player = players.contains_key(&(m.id as i32));
            if !running && !player {
                continue;
            }
            m.previous = m.position;
            m.previous_yaw = m.yaw;
            m.previous_body_yaw = m.body_yaw;
            m.previous_head_yaw = m.head_yaw;
            m.previous_head_pitch = m.head_pitch;
            m.previous_limb_speed = m.limb_speed;
            m.age += 1;
            if m.species() == horizon_client::mobs::Species::Rabbit {
                horizon_client::mobs::rabbit::base_tick(m);
            }
            if m.species() == horizon_client::mobs::Species::Creeper {
                horizon_client::mobs::hostile::creeper_client_tick(m);
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
            // LivingEntity.aiStep: the head's own interpolation.
            if let Some((yaw, steps)) = head_lerps.get_mut(&(m.id as i32))
                && *steps > 0
            {
                m.head_yaw += wrap_degrees(*yaw - m.head_yaw) / *steps as f32;
                *steps -= 1;
            }
            // LivingEntity.updatingUsingItem for another player: the ticks the
            // item has been used before this one.
            if player && m.extra.using_item > 0 {
                let item = if m.extra.using_off_hand {
                    m.extra.off_hand.clone()
                } else {
                    m.extra.main_hand.clone()
                };
                if let Some(item) = item {
                    let eye = if m.extra.crouching { 1.27 } else { 1.62 };
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
                m.extra.using_item += 1;
            }
            // LivingEntity.tick's tickHeadTurn: a mob's BodyRotationControl,
            // a player's body following its walk and look.
            if player {
                remote_player_head_turn(m);
            } else if m.alive() {
                m.body_rotation_tick();
            }
            // WalkAnimationState.update from the horizontal distance moved.
            let (dx, dz) = (m.position.x - m.previous.x, m.position.z - m.previous.z);
            let target = (((dx * dx + dz * dz).sqrt() * 4.0) as f32).min(1.0);
            m.limb_speed += (target - m.limb_speed) * 0.4;
            m.limb_position += m.limb_speed;
            if m.species() == horizon_client::mobs::Species::Rabbit {
                horizon_client::mobs::rabbit::post_tick(m);
            }
        }
        for o in self.orbs.iter_mut().filter(|_| running) {
            o.previous = o.position;
            o.age += 1;
            if let Some(l) = lerps.get_mut(&(o.id as i32))
                && l.steps > 0
            {
                let n = f64::from(l.steps);
                o.position = Vec3::new(
                    o.position.x + (l.target[0] - o.position.x) / n,
                    o.position.y + (l.target[1] - o.position.y) / n,
                    o.position.z + (l.target[2] - o.position.z) / n,
                );
                l.steps -= 1;
            }
        }
        // ItemEntity.tick runs on a client too: gravity, collisions and
        // friction move the item between the server's updates (every 20
        // ticks), and only the server removes it.
        let (world, facts, collision) = (&self.world, self.res.facts(), self.collision);
        for d in self.dropped.iter_mut().filter(|_| running) {
            d.tick(world, facts, collision, false);
        }
        head_lerps.retain(|_, (_, steps)| *steps > 0);
        for (item, ticks, at, eye, pitch, yaw) in using {
            self.remote_player_eating(&item, ticks, at, eye, pitch, yaw);
        }
        if running {
            // Boats, minecarts, armor stands, projectiles and the rest
            // interpolate like mobs.
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
            self.tick_remote_objects(&mut step);
            self.tick_remote_projectiles(&mut step);
            // LightningBolt.tick on a client, and the sky flash fading.
            self.flash = self.flash.saturating_sub(1);
            self.tick_bolts();
        }
        lerps.retain(|_, l| l.steps > 0);
        if let Some(r) = self.remote.as_mut() {
            for (id, l) in lerps {
                r.lerps.entry(id).or_insert(l);
            }
        }
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
    /// LocalPlayer.sendPosition plus input, sprint and held-slot updates.
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
        let (yaw, pitch, selected) = (self.yaw, self.pitch, self.selected);
        let riding = self.riding.is_some();
        let vehicle = self.remote_vehicle_packets();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut out = Vec::new();
        if r.carried != Some(selected) {
            r.carried = Some(selected);
            out.push(S::SetCarriedItem(play::SetCarriedItem {
                slot: selected as i16,
            }));
        }
        if input != r.last_input {
            r.last_input = input;
            out.push(S::PlayerInput(play::PlayerInput { flags: input }));
        }
        if sprinting != r.was_sprinting {
            r.was_sprinting = sprinting;
            out.push(S::PlayerCommand(play::PlayerCommand {
                id: r.entity_id,
                action: if sprinting { 1 } else { 2 },
                data: 0,
            }));
        }
        if riding {
            // LocalPlayer.tick: a passenger sends its look, then the vehicle
            // it controls.
            out.push(S::MovePlayerRot(play::MovePlayerRot {
                yaw,
                pitch,
                flags: u8::from(on_ground) | u8::from(collision) << 1,
            }));
            out.extend(vehicle);
        } else if let Some(packet) =
            r.position_sender
                .update(pos, (yaw, pitch), on_ground, collision)
        {
            out.push(packet);
        }
        // Creative inventory edits are sent slot by slot (SetCreativeModeSlot).
        let mut creative_slots = Vec::new();
        if self.creative {
            for i in 0..36 {
                if self.inventory[i] != r.synced[i] {
                    r.synced[i] = self.inventory[i].clone();
                    creative_slots.push(i);
                }
            }
        }
        for packet in out {
            self.send(packet);
        }
        for i in creative_slots {
            let slot = if i < 9 { 36 + i } else { i } as i16;
            let item = self.to_network(&self.inventory[i]);
            self.send(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
                slot,
                item,
            }));
        }
    }
    /// Inventory as the server last described it.
    fn remote_synced(&mut self) {
        let inventory = self.inventory.clone();
        if let Some(r) = self.remote.as_mut() {
            r.synced = inventory;
        }
    }

    pub(super) fn stop_remote_mining(&mut self) {
        if let Some((pos, _, _)) = self.remote.as_mut().and_then(|r| r.destroying.take()) {
            self.send(S::PlayerAction(play::PlayerAction {
                action: 2,
                pos,
                direction: 0,
                sequence: 0,
            }));
        }
    }

    /// MultiPlayerGameMode.startDestroyBlock / continueDestroyBlock.
    fn remote_mining(&mut self, start: bool) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        if !start && r.destroy_delay > 0 {
            r.destroy_delay -= 1;
            return;
        }
        let Some(hit) = self.pick() else {
            self.mining = None;
            return;
        };
        let face = face_direction(hit.face);
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if self.res.facts().is_air(state) {
            return;
        }
        if self.creative {
            let seq = self.next_sequence();
            self.predict_break(hit.pos, state, seq);
            self.send(S::PlayerAction(play::PlayerAction {
                action: 0,
                pos: hit.pos,
                direction: face,
                sequence: seq,
            }));
            if let Some(r) = self.remote.as_mut() {
                r.destroy_delay = 5;
            }
            return;
        }
        let facts = self.res.facts().get(state).clone();
        // The server's own getDestroyProgress: under water, off the ground,
        // with haste, fatigue and efficiency, so the client stops digging
        // when the server is done too (#1254).
        let progress = if facts.hardness == 0.0 {
            1.0
        } else {
            self.destroy_progress(hit.pos)
        };
        let same = self
            .remote
            .as_ref()
            .and_then(|r| r.destroying)
            .is_some_and(|(p, _, _)| p == hit.pos);
        if start && same {
            return;
        }
        if !same {
            if let Some((old, _, _)) = self.remote.as_mut().and_then(|r| r.destroying.take()) {
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
            if progress >= 1.0 {
                // startDestroyBlock: instant survival breaks have no destroy delay.
                self.predict_break(hit.pos, state, seq);
                return;
            }
            if let Some(r) = self.remote.as_mut() {
                r.destroying = Some((hit.pos, face, 0.0));
            }
            self.mining = Some(Mining {
                pos: hit.pos,
                progress: 0.0,
                ticks: 0,
            });
            return;
        }
        let Some((pos, last_face, done)) = self.remote.as_ref().and_then(|r| r.destroying) else {
            return;
        };
        let done = done + progress;
        let ticks = self.mining.as_ref().map_or(0, |m| m.ticks) + 1;
        if ticks % 4 == 1 {
            let c = [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            ];
            self.audio.play(
                &facts.sounds.hit,
                Some(c),
                (facts.sounds.volume + 1.0) / 8.0,
                facts.sounds.pitch * 0.5,
            );
        }
        self.crack_particle(pos, hit.face, state);
        if done >= 1.0 {
            let seq = self.next_sequence();
            self.predict_break(pos, state, seq);
            self.send(S::PlayerAction(play::PlayerAction {
                action: 3,
                pos,
                direction: face,
                sequence: seq,
            }));
            if let Some(r) = self.remote.as_mut() {
                r.destroying = None;
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
                ticks,
            });
        }
    }
    /// MultiPlayerGameMode.destroyBlock: the predicted break.
    fn predict_break(&mut self, pos: [i32; 3], state: BlockStateId, sequence: i32) {
        // BlockStatePredictionHandler.retainKnownServerState.
        if let Some(r) = self.remote.as_mut() {
            r.predictions
                .entry(pos)
                .and_modify(|p| p.0 = sequence)
                .or_insert((sequence, state.0 as i32));
        }
        let s = self.res.facts().get(state).sounds.clone();
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        self.audio.play(
            &s.break_sound,
            Some(at),
            (s.volume + 1.0) / 2.0,
            s.pitch * 0.8,
        );
        self.destroy_particles(pos, state);
        // The server's block update follows (waterlogged blocks leave water).
        let air = self.world.air;
        if self.world.set(pos[0], pos[1], pos[2], air).is_ok() {
            self.world.block_entities.remove(&(pos[0], pos[1], pos[2]));
        }
        self.mining = None;
    }
    /// Minecraft.startUseItem: use one crosshair target, then the item itself.
    fn remote_use(&mut self) {
        // Minecraft.startUseItem returns before resetting rightClickDelay
        // while a survival block break is active.
        if self.remote.as_ref().is_some_and(|r| r.destroying.is_some()) {
            return;
        }
        self.use_delay = 4;
        // Resolve the crosshair once. A passing entity interaction keeps
        // that target and may only fall back to using the item itself.
        let entity = self.entity_targeted();
        horizon_client::interaction_input::use_on_target(&mut RemoteUseWorld(self), entity);
    }

    /// `Minecraft.startUseItem`'s entity turn for `hand`: Interact goes to
    /// the server, and whether the click ends there is what this client's
    /// copy of the entity answers (`MultiPlayerGameMode.interact`).
    fn remote_use_entity(&mut self, hand: Hand) -> bool {
        let sneaking = self.keys.contains(&KeyCode::ShiftLeft);
        // A boat, minecart, item frame, armor stand or painting nearer than
        // any mob: the server uses it (AbstractBoat.interact boards it).
        if let Some(hit) = self.remote_object_hit(false)
            && self.mob_hit().is_none_or(|(_, d)| hit.distance < d)
            && self.before_block(hit.distance)
        {
            // The main hand's turn takes it; there is no off hand turn.
            if hand == Hand::Off {
                return false;
            }
            self.send(S::Interact(play::Interact {
                id: hit.id,
                hand: 0,
                location: hit.location,
                secondary: sneaking,
            }));
            self.swing_arm();
            return true;
        }
        if let Some((i, distance)) = self.mob_hit()
            && self.before_block(distance)
        {
            let id = self.mobs[i].id as i32;
            self.send(S::Interact(play::Interact {
                id,
                hand: hand_id(hand),
                location: [0.0, 1.0, 0.0],
                secondary: sneaking,
            }));
            // Player.interactOn: another player passes, and so does a mob
            // the held item means nothing to: the item is used instead.
            let player = self
                .remote
                .as_ref()
                .is_some_and(|r| r.level_players.contains_key(&id));
            if !player && self.remote_mob_takes(i, hand) {
                self.with_hand(hand, Game::swing_arm);
                return true;
            }
        }
        false
    }

    /// MultiPlayerGameMode.useItemOn for `hand`: whether the click ends there
    /// (the block reacts, or a block item is placed).
    fn remote_use_block(&mut self, hand: Hand) -> bool {
        let held = self.with_hand(hand, |g| g.held().cloned());
        let held_block = held
            .as_ref()
            .and_then(|h| self.res.items().get(&h.item))
            .is_some_and(|i| i.block.is_some());
        if let Some(hit) = self.pick() {
            let (reacts, changes) = if hand == Hand::Main {
                self.remote_block_reacts(&hit, held.as_ref())
            } else {
                (false, DryUse::default())
            };
            let seq = self.next_sequence();
            let cursor = [
                (hit.point[0] - f64::from(hit.pos[0])) as f32,
                (hit.point[1] - f64::from(hit.pos[1])) as f32,
                (hit.point[2] - f64::from(hit.pos[2])) as f32,
            ];
            self.send(S::UseItemOn(play::UseItemOn {
                hand: hand_id(hand),
                pos: hit.pos,
                face: i32::from(face_direction(hit.face)),
                cursor,
                inside: hit.face == [0; 3],
                world_border: false,
                sequence: seq,
            }));
            let target = [
                hit.pos[0] + hit.face[0],
                hit.pos[1] + hit.face[1],
                hit.pos[2] + hit.face[2],
            ];
            let ticks = self.ticks;
            if let Some(r) = self.remote.as_mut() {
                r.use_target = Some(hit.pos);
                r.placing = Some((target, ticks));
            }
            if reacts {
                self.predict_use(&changes, seq);
            } else if let Some(stack) = held.as_ref()
                && self.predict_item_on_block(stack, &hit)
            {
                // Item.useOn succeeded here (InteractionResult.SUCCESS):
                // this hand swings and the click ends, so the other hand
                // (a shield, a torch) is never tried.
                self.with_hand(hand, Game::swing_arm);
                return true;
            }
            if held_block {
                let placed = if reacts {
                    None
                } else {
                    held.as_ref()
                        .and_then(|stack| self.remote_can_place(&hit, stack))
                };
                if let Some(placed) = &placed {
                    self.predict_place(placed, seq);
                    self.with_hand(hand, Game::consume_held);
                }
                if reacts || placed.is_some() {
                    self.with_hand(hand, Game::swing_arm);
                }
                // BlockItem.place returns FAIL when placement is rejected:
                // stop this click without trying the item or the other hand.
                return true;
            }
            return reacts;
        }
        false
    }

    /// `Item.useOn` on this client's level for the items whose effects are
    /// the clicker's own (`Level.levelEvent(player, ...)` and
    /// `playSound(player, ...)`: the server sends them to the others only):
    /// honeycomb waxing copper, and an axe, hoe or shovel transforming a
    /// block (stripping, scraping, removing wax, tilling, making a path).
    /// Whether the item's use succeeded, which ends the click.
    fn predict_item_on_block(&mut self, stack: &ItemStack, hit: &super::Hit) -> bool {
        let at = [
            f64::from(hit.pos[0]) + 0.5,
            f64::from(hit.pos[1]) + 0.5,
            f64::from(hit.pos[2]) + 0.5,
        ];
        if &*stack.item == "minecraft:honeycomb" {
            // HoneycombItem.useOn.
            let registry = self.pack.assets.registry;
            let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let waxes = registry
                .state(clicked)
                .and_then(|old| self.res.transformers().waxed(registry, old))
                .is_some();
            if waxes {
                self.level_event_at(3003, hit.pos, 0);
                self.audio
                    .play("minecraft:item.honeycomb.wax_on", Some(at), 1.0, 1.0);
            }
            waxes
        } else if let Some((_, sound, _, _, particle)) =
            self.block_transform(stack, hit.pos, hit.face)
        {
            self.transform_particle(&particle, hit.pos);
            self.audio.play(&sound, Some(at), 1.0, 1.0);
            true
        } else {
            false
        }
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
            self.audio.play(&name, Some(at), *volume, *pitch);
        }
    }

    /// Whether the block hit takes a main-hand right click with `held`
    /// (`BlockState.useItemOn`, then `useWithoutItem`): a door, a button, a
    /// container, and the blocks it changed. Vanilla's client runs it on its
    /// own level first; this runs it on a copy, and `predict_use` keeps the
    /// changes it predicts.
    fn remote_block_reacts(
        &mut self,
        hit: &super::Hit,
        held: Option<&ItemStack>,
    ) -> (bool, DryUse) {
        // Player.isSecondaryUseActive with something in either hand.
        let hands_full = self.stack_in(self.selected).is_some() || self.offhand.is_some();
        if self.keys.contains(&KeyCode::ShiftLeft) && hands_full {
            return (false, DryUse::default());
        }
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
        self.with_level(|b, level, _| {
            let mut level = DryLevel::new(level);
            let reacts = b
                .use_item_on(
                    &mut level,
                    &mut horizon_world::level::Ticks::default(),
                    horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
                    face,
                    horizon_core::geometry::Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
                    player,
                    held.map(|s| &*s.item),
                )
                .consumes_action();
            (reacts, (level.changed.into_iter().collect(), level.sounds))
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
    fn predict_blocks(&mut self, changes: &DryChanges, seq: i32) {
        for &(p, state) in changes {
            let pos = [p.x, p.y, p.z];
            let before = self.world.block(p.x, p.y, p.z);
            if let Some(r) = self.remote.as_mut() {
                r.predictions
                    .entry(pos)
                    .and_modify(|e| e.0 = seq)
                    .or_insert((seq, before.0 as i32));
            }
            self.set_block(pos, state.0 as i32);
        }
    }

    /// Run BlockItem.place on a disposable level: where it placed and the
    /// blocks it set. The real level supplies collision shapes and
    /// building-blocking entities; random draws, scheduled ticks and events
    /// stay in the preview.
    fn remote_can_place(&mut self, hit: &super::Hit, stack: &ItemStack) -> Option<DryPlace> {
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
            sneaking: self.keys.contains(&KeyCode::ShiftLeft),
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

    /// MultiPlayerGameMode.useItem for `hand`, if it holds something: whether
    /// the click ends there, as far as this client can tell (`item_has_use`).
    fn remote_use_item(&mut self, hand: Hand) -> bool {
        let Some(held) = self.with_hand(hand, |g| g.held().cloned()) else {
            return false;
        };
        let seq = self.next_sequence();
        self.send(S::UseItem(play::UseItem {
            hand: hand_id(hand),
            sequence: seq,
            yaw: self.yaw,
            pitch: self.pitch,
        }));
        let Some(info) = self.res.items().get(&held.item).cloned() else {
            return false;
        };
        // MultiPlayerGameMode.useItem runs the use here too: food starts
        // being eaten (Consumable.startConsuming), a book and quill opens.
        self.with_hand(hand, |g| g.start_eating(&info));
        if self.with_hand(hand, |g| g.remote_use_book(&held)) {
            return true;
        }
        self.with_hand(hand, |g| g.remote_start_held_use(&held));
        item_has_use(&info)
    }

    /// `Item.use` runs on the client too (MultiPlayerGameMode.useItem): an
    /// item held in use until the button lets go starts being used here, so
    /// that letting go sends RELEASE_USE_ITEM (Minecraft.handleKeybinds) and
    /// the server ends the use: a bow or a trident is let go, a crossbow's
    /// charge ends, a shield is lowered, a spyglass stops scoping. Their
    /// sounds, loaded arrows and thrown tridents come from the server.
    fn remote_start_held_use(&mut self, held: &ItemStack) {
        if self.using.is_some() {
            return;
        }
        let start = match &*held.item {
            // BowItem.use: with an arrow, or in creative.
            "minecraft:bow" => self.creative || self.projectile_slot("minecraft:bow").is_some(),
            // CrossbowItem.use: charging when not charged and with ammunition;
            // a charged one shoots on the server.
            "minecraft:crossbow" => {
                held.data().is_none_or(|d| d.charged.is_empty())
                    && (self.creative || self.projectile_slot("minecraft:crossbow").is_some())
            }
            // TridentItem.use: not about to break, Riptide only in water or rain.
            "minecraft:trident" => {
                self.use_trident(held);
                return;
            }
            // SpyglassItem.use (its sound is the server's).
            "minecraft:spyglass" => true,
            // ShieldItem.use: anything that blocks attacks, off cooldown.
            _ => {
                self.raise_shield(self.held_slot());
                return;
            }
        };
        if start {
            self.using = Some((self.held_slot(), 0));
        }
    }
    /// Whether `stack` stays in use until the button lets go (bows,
    /// crossbows, tridents, spyglasses, shields): its use only counts ticks here.
    fn held_until_released(&self, stack: &ItemStack) -> bool {
        matches!(
            &*stack.item,
            "minecraft:bow" | "minecraft:crossbow" | "minecraft:trident" | "minecraft:spyglass"
        ) || self.blocks_attacks(stack).is_some()
    }

    /// The rest of Minecraft.startUseItem once the main hand's target passed:
    /// the main hand's item, then the off hand's turn (its block, its item).
    /// The server does each packet's part with that packet's hand only.
    fn remote_use_hands(&mut self) {
        if self.remote_use_item(Hand::Main) {
            return;
        }
        // The off hand's turn: the entity again, or the block.
        let off_target = if self.entity_targeted() {
            self.remote_use_entity(Hand::Off)
        } else {
            self.remote_use_block(Hand::Off)
        };
        if off_target {
            return;
        }
        self.remote_use_item(Hand::Off);
    }
    /// `LivingEntity.updatingUsingItem` on a server client: an item held in
    /// use (`held_until_released`), and a
    /// consumable's eating animation and its crumbs. The server eats it and
    /// sends the stack, the hunger and the sounds.
    fn remote_tick_using(&mut self) {
        let Some((slot, ticks)) = self.using else {
            return;
        };
        let stack = self.stack_in(slot).cloned();
        // Another hotbar slot picked: LivingEntity.updatingUsingItem sees a
        // different stack in the hand. The off hand keeps its item.
        let in_hand = slot == self.selected || slot == super::shield::OFFHAND_SLOT;
        // A drawn bow, a charging crossbow, a raised shield...: held until
        // the button lets go.
        if stack.as_ref().is_some_and(|s| self.held_until_released(s)) {
            self.using = in_hand.then_some((slot, ticks + 1));
            return;
        }
        let consumable = stack
            .as_ref()
            .and_then(|s| self.res.items().get(&s.item))
            .and_then(|i| i.consumable.clone());
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
        // Consumable.shouldEmitParticlesAndSounds, as in tick_used_item.
        let duration = consumable.ticks().max(0) as u32;
        let remaining = duration - ticks.min(duration);
        if consumable.particles && consumable.should_emit(remaining as i32) {
            self.eat_particles(&stack, 5);
        }
        if ticks + 1 >= duration {
            self.using = None;
            if consumable.particles {
                self.eat_particles(&stack, 16);
            }
        } else {
            self.using = Some((slot, ticks + 1));
        }
    }
    /// MouseHandler updates KeyMapping; handleKeybinds sends packets on a tick.
    pub(super) fn remote_mouse(&mut self, button: MouseButton, pressed: bool) -> Result<()> {
        use horizon_client::interaction_input::Button;
        let input = match button {
            MouseButton::Left => {
                self.toggles
                    .button(2, pressed, false, self.settings.toggle_attack);
                self.left = self.toggles.is_down(2);
                Some(Button::Attack)
            }
            MouseButton::Right => {
                self.toggles
                    .button(3, pressed, false, self.settings.toggle_use);
                self.right = self.toggles.is_down(3);
                Some(Button::Use)
            }
            MouseButton::Middle => Some(Button::Pick),
            _ => None,
        };
        if pressed
            && let Some(button) = input
            && let Some(remote) = self.remote.as_mut()
        {
            remote.input.press(button);
        }
        Ok(())
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
        let ready = r.loaded_sent && self.screen == Screen::Playing && self.sleeping.is_none();
        if stop || !ready {
            self.stop_remote_mining();
        }
        if !ready {
            return;
        }
        // MultiPlayerGameMode.tick syncs the carried slot before keybinds.
        let carried_changed = self.remote.as_mut().is_some_and(|r| {
            if r.carried == Some(self.selected) {
                false
            } else {
                r.carried = Some(self.selected);
                true
            }
        });
        if carried_changed {
            self.send(S::SetCarriedItem(play::SetCarriedItem {
                slot: self.selected as i16,
            }));
        }
        let (attack, use_item) = (self.left, self.right);
        input.tick(&mut RemoteInputWorld(self), attack, use_item);
        if self.clicks.miss_time > 0 {
            self.clicks.miss_time -= 1;
        }
        if let Some(r) = self.remote.as_mut() {
            r.input = input;
        }
    }

    fn remote_start_attack(&mut self) -> bool {
        if self.clicks.miss_time > 0 {
            return false;
        }
        if self.spectator() {
            return true;
        }
        let mut instant = false;
        // A ghast's fireball or a wind charge nearer than any mob: the
        // server turns it (game/deflect.rs).
        if let Some((i, distance)) = self.redirectable_hit()
            && self.mob_hit().is_none_or(|(_, d)| distance <= d)
            && self.before_block(distance)
            && self.creatures.projectiles[i].id != 0
        {
            self.send(S::Attack(play::Attack {
                id: self.creatures.projectiles[i].id as i32,
            }));
            self.reset_attack_strength();
        } else if let Some(hit) = self.remote_object_hit(true)
            && self.mob_hit().is_none_or(|(_, d)| hit.distance < d)
            && self.before_block(hit.distance)
        {
            // VehicleEntity.hurtServer, ItemFrame and ArmorStand hurtServer,
            // a painting broken or an EndCrystal hurtServer: the server decides.
            self.send(S::Attack(play::Attack { id: hit.id }));
            self.reset_attack_strength();
        } else if let Some((i, distance)) = self.mob_hit()
            && self.before_block(distance)
        {
            self.send(S::Attack(play::Attack {
                id: self.mobs[i].id as i32,
            }));
            self.reset_attack_strength();
        } else if !self.entity_targeted()
            && let Some(hit) = self.pick()
        {
            self.remote_mining(true);
            instant = self
                .res
                .facts()
                .is_air(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]));
        } else {
            if !self.creative {
                self.clicks.miss_time = 10;
            }
            self.reset_attack_strength();
        }
        self.swing_arm();
        self.send(S::Punch(Empty));
        instant
    }

    fn remote_continue_attack(&mut self, down: bool) {
        if !down {
            self.clicks.miss_time = 0;
        }
        if self.clicks.miss_time > 0 || self.using.is_some() {
            return;
        }
        let entity = self.entity_targeted();
        if down
            && !entity
            && let Some(hit) = self.pick()
        {
            let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            if !self.res.facts().is_air(state) {
                self.remote_mining(false);
                self.swing_arm();
                self.send(S::Punch(Empty));
            } else {
                if let Some(r) = self.remote.as_mut() {
                    r.destroying = None;
                }
                self.mining = None;
            }
        } else {
            self.stop_remote_mining();
            self.mining = None;
        }
    }
    /// Keys with a multiplayer meaning; returns true when consumed.
    pub(super) fn remote_key(&mut self, key: KeyCode) -> Result<bool> {
        if self.screen != Screen::Playing {
            if (key == KeyCode::KeyE || key == KeyCode::Escape) && self.screen == Screen::Inventory
            {
                self.remote_close_container();
                self.screen = Screen::Playing;
                self.capture(true);
                return Ok(true);
            }
            return Ok(false);
        }
        match key {
            KeyCode::KeyT | KeyCode::Slash | KeyCode::Enter => {
                // ChatScreen, over the server's command tree.
                self.open_chat(if key == KeyCode::Slash { "/" } else { "" });
                Ok(true)
            }
            KeyCode::KeyQ => {
                if self.spectator() {
                    return Ok(true);
                }
                let all = self.keys.contains(&KeyCode::ControlLeft);
                // MultiPlayerGameMode.dropItem: predict removal before sending
                // the action, then swing only if a nonempty stack was removed.
                let dropped = horizon_client::items::take_held_drop(
                    &mut self.inventory[self.selected],
                    all,
                    false,
                );
                self.send(S::PlayerAction(play::PlayerAction {
                    action: if all { 4 } else { 5 },
                    pos: [0; 3],
                    direction: 0,
                    sequence: 0,
                }));
                if dropped.is_some() {
                    self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
                }
                Ok(true)
            }
            // MultiPlayerGameMode.isServerControlledInventory: riding a
            // horse or llama, the server opens its inventory
            // (ServerboundPlayerCommandPacket OPEN_INVENTORY).
            KeyCode::KeyE if self.riding_mount_with_menu() => {
                let id = self.remote.as_ref().map_or(0, |r| r.entity_id);
                self.send(S::PlayerCommand(play::PlayerCommand {
                    id,
                    action: 5,
                    data: 0,
                }));
                Ok(true)
            }
            KeyCode::KeyE => {
                // The inventory opens without telling the server.
                self.screen = if self.creative {
                    Screen::Creative
                } else {
                    Screen::Inventory
                };
                self.craft_size = 2;
                self.capture(false);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    /// ClientPacketListener.sendChat / sendCommand (unsigned).
    pub(super) fn send_chat(&mut self, message: &str) {
        let message = message.trim();
        if message.is_empty() {
            return;
        }
        if let Some(command) = message.strip_prefix('/') {
            self.send(S::ChatCommand(play::ChatCommand {
                command: command.to_owned(),
            }));
        } else {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64);
            self.send(S::Chat(play::Chat {
                message: message.chars().take(256).collect(),
                timestamp,
                salt: 0,
                signature: None,
                last_seen: play::LastSeenUpdate {
                    offset: 0,
                    acknowledged: [0; 3],
                    checksum: 0,
                },
            }));
        }
    }
    /// Minecraft.handleKeybinds, key.swapOffhand: the server swaps the hands
    /// (ServerboundPlayerActionPacket SWAP_ITEM_WITH_OFFHAND at BlockPos.ZERO, facing down) and
    /// sends both slots back; nothing is predicted.
    pub(super) fn remote_swap_hands(&mut self) {
        self.send(S::PlayerAction(play::PlayerAction {
            action: 7,
            pos: [0; 3],
            direction: 0,
            sequence: 0,
        }));
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
        let batch = HudBatch::new(size);
        let Some(internal) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else {
            return;
        };
        let input = if self.has_shift_down() {
            Input::QuickMove
        } else {
            Input::Pickup
        };
        self.remote_menu_ops(&[(Some(internal), i32::from(right), input)]);
    }
    /// The server's open menu: its window id and menu type (0, "inventory"
    /// when none is open).
    fn remote_menu(&self) -> (i32, String) {
        match self.remote.as_ref().and_then(|r| r.menu.clone()) {
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
        // The engine's slot indices are the menu's own, as on the network.
        self.menu_session(|m, env, _| {
            let mut predicted = Vec::with_capacity(clicks.len());
            for &(slot, button, input) in &clicks {
                let before = menu_items(m);
                if m.click(env, i32::from(slot), button, input).is_err() {
                    // The server throws on it too: nothing is predicted.
                    return Some(false);
                }
                let changed = changed_slots(&before, m);
                predicted.push((slot, button, input, changed, m.carried.clone()));
            }
            sent = predicted;
            Some(true)
        });
        self.send_clicks(container, sent);
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
        let state = self.remote.as_ref().map_or(0, |r| r.state_id);
        let hashed = |g: &Self, s: &Option<ItemStack>| g.to_network(s).map(|s| Hashed::of(&s));
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
    /// A `ContainerClick` on game slot `internal` of the server's open menu
    /// (`input` is the ContainerInput ordinal). False when the menu has no
    /// such slot.
    pub(super) fn remote_slot_click(&mut self, internal: usize, button: i8, input: i32) -> bool {
        let (container, menu) = self.remote_menu();
        let slot = Self::menu_slot_index(&menu, internal);
        let Some(slot) = slot else { return false };
        let carried = self.to_network(&self.carried).map(|s| Hashed::of(&s));
        let state = self.remote.as_ref().map_or(0, |r| r.state_id);
        self.send(S::ContainerClick(play::ContainerClick {
            container,
            state,
            slot: slot as i16,
            button,
            input,
            changed: Vec::new(),
            carried,
        }));
        true
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
        let carried = self.to_network(&self.carried).map(|s| Hashed::of(&s));
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
    pub(super) fn remote_close_container(&mut self) {
        let id = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref().map(|m| m.0))
            .unwrap_or(0);
        self.send(S::ContainerClose(play::ContainerClose { container: id }));
        self.remote_close_local();
    }
    fn remote_close_local(&mut self) {
        if let Some(r) = self.remote.as_mut() {
            r.menu = None;
            r.menu_title = None;
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
    fn remote_open_merchant(&mut self, container: i32, title: String) {
        if let Some(r) = self.remote.as_mut() {
            r.menu = Some((container, "minecraft:merchant".to_owned()));
        }
        let session = horizon_client::merchant::Session::new(Vec::new(), true);
        self.open_remote_merchant(session, (!title.is_empty()).then_some(title));
    }
    /// ClientboundMerchantOffersPacket.
    fn remote_merchant_offers(&mut self, p: play::MerchantOffers) {
        if self.open_menu_name(p.container) != "minecraft:merchant" {
            return;
        }
        let cost = |g: &Self, c: &play::NetCost| horizon_client::merchant::ItemCost {
            stack: ItemStack::new(&g.item_name(c.item), c.count.max(0) as u32),
        };
        let offers: Vec<_> = p
            .offers
            .iter()
            .filter_map(|o| {
                let mut offer = horizon_client::merchant::MerchantOffer {
                    base_cost_a: cost(self, &o.cost_a),
                    cost_b: o.cost_b.as_ref().map(|b| cost(self, b)),
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
            .as_ref()
            .and_then(|r| r.use_target)
            .map(|t| (t[0], t[1], t[2]));
        if menu == "minecraft:lectern" {
            // LecternScreen: the book and page come in the menu's content and data.
            if let Some(r) = self.remote.as_mut() {
                r.menu = Some((p.container, menu));
            }
            self.remote_open_lectern(target.unwrap_or_default());
            return;
        }
        if let Some(kind) = Self::remote_station_kind(&menu) {
            let title = plain(
                &p.title,
                &self
                    .remote
                    .as_ref()
                    .map(|r| r.lang.clone())
                    .unwrap_or_default(),
            );
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
            self.remote_open_merchant(p.container, title);
            return;
        }
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
            _ => {
                // Unsupported menu: tell the server we closed it.
                self.send(S::ContainerClose(play::ContainerClose {
                    container: p.container,
                }));
                self.chat_line(horizon_client::text::Text::literal(format!(
                    "(Horizon cannot show {menu} screens yet)"
                )));
                return;
            }
        };
        let title = plain(
            &p.title,
            &self
                .remote
                .as_ref()
                .map(|r| r.lang.clone())
                .unwrap_or_default(),
        );
        if let Some(r) = self.remote.as_mut() {
            r.menu = Some((p.container, menu.clone()));
            r.menu_title = Some(title);
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
        }
        self.screen = Screen::Inventory;
        self.capture(false);
    }
    /// `ClientPacketListener.handleMountScreenOpen`: a horse's or llama's
    /// inventory; for another entity the window is closed again.
    fn remote_open_mount(&mut self, p: play::MountScreenOpen) {
        let columns = usize::try_from(p.columns).unwrap_or(0).min(5);
        if !self.open_remote_mount_menu(p.entity as u32, columns) {
            self.send(S::ContainerClose(play::ContainerClose {
                container: p.container,
            }));
            return;
        }
        if let Some(r) = self.remote.as_mut() {
            r.menu = Some((p.container, format!("{MOUNT_MENU}{columns}")));
            r.menu_title = None;
        }
    }
    /// The type of the menu a server opened (`MenuType`'s id).
    pub(super) fn remote_menu_type(&self) -> Option<&str> {
        Some(self.remote.as_ref()?.menu.as_ref()?.1.as_str())
    }
    /// The title the server gave the menu it opened.
    pub(super) fn remote_menu_title(&self) -> Option<String> {
        self.remote.as_ref()?.menu_title.clone()
    }
    /// A server's chest-like menu (`MenuType.GENERIC_9xN`): its rows and title.
    pub(super) fn remote_chest_menu(&self) -> Option<(usize, String)> {
        let r = self.remote.as_ref()?;
        let rows = r
            .menu
            .as_ref()?
            .1
            .strip_prefix("minecraft:generic_9x")?
            .parse()
            .ok()?;
        Some((rows, r.menu_title.clone().unwrap_or_default()))
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

    /// Connecting / loading screens; true when it covered the frame.
    pub(super) fn remote_overlay(&self, batch: &mut HudBatch) -> bool {
        let Some(r) = &self.remote else { return false };
        if self.player.is_some() {
            return false;
        }
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        batch.fill(atlas, 0.0, 0.0, w, h, rgba(20, 20, 20, 255));
        let shown = horizon_client::net::shown_address(&r.session.address);
        for (i, line) in [r.status.as_str(), shown.as_str()].iter().enumerate() {
            let tw = HudBatch::text_width(atlas, line);
            batch.text(
                atlas,
                line,
                (w - tw) / 2.0,
                h / 2.0 - 12.0 + i as f32 * 12.0,
                if i == 0 {
                    u32::MAX
                } else {
                    rgba(160, 160, 160, 255)
                },
                true,
            );
        }
        let _ = r.joined;
        true
    }
    /// The server's death message; chat uses the shared ChatComponent.
    pub(super) fn remote_hud(&self, batch: &mut HudBatch) {
        let Some(r) = &self.remote else { return };
        let atlas = &self.gui.atlas;
        let w = batch.width;
        if self.screen == Screen::Dead
            && let Some(message) = &r.death_message
        {
            let tw = HudBatch::text_width(atlas, message);
            batch.text(atlas, message, (w - tw) / 2.0, 85.0, u32::MAX, true);
        }
    }
    /// Status for `horizon-ctl status`.
    /// The other players' entities in the level and their names (from the
    /// player list).
    pub(super) fn remote_player_names(&self) -> Vec<(u32, String)> {
        let Some(r) = self.remote.as_ref() else {
            return Vec::new();
        };
        r.level_players
            .iter()
            .filter(|(_, uuid)| **uuid != r.profile)
            .filter_map(|(id, uuid)| Some((*id as u32, r.players.get(uuid)?.clone())))
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
        let partial = f64::from(r.clock_partial) + delta as f64 * f64::from(r.clock_rate);
        let full = partial.floor();
        r.clock_partial = (partial - full) as f32;
        self.world.day_time += full as i64;
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
                    let armor = [&e.head, &e.chest, &e.legs, &e.feet].map(|s| s.as_deref());
                    serde_json::json!({
                        "id": m.id,
                        "swinging": m.ai.swinging,
                        "using": m.extra.using_item,
                        "armor": armor,
                        "main_hand": e.main_hand.as_deref(),
                        "off_hand": e.off_hand.as_deref(),
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
            "text_display_vertices": self.text_display_vertices,
            "text_displays": self.text_displays.len(),
            "health": r.health.0,
            "food": r.health.1,
            "decode_errors": r.decode_errors,
            "chat": r.chat.iter().rev().take(10).rev().map(|(t, _)| t.clone()).collect::<Vec<_>>(),
        }))
    }
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

fn menu_items(m: &horizon_client::menu::Menu) -> Vec<Option<ItemStack>> {
    (0..m.slot_count()).map(|i| m.slot(i).cloned()).collect()
}

/// The slots whose stack is no longer `before`'s (`ItemStack.matches`).
fn changed_slots(
    before: &[Option<ItemStack>],
    m: &horizon_client::menu::Menu,
) -> Vec<(i16, Option<ItemStack>)> {
    (0..m.slot_count())
        .filter(|&i| m.slot(i) != before.get(i).and_then(Option::as_ref))
        .map(|i| (i as i16, m.slot(i).cloned()))
        .collect()
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

/// The client's world for a guess at what a click does: reads go to the
/// world, writes stay here and are dropped.
struct DryLevel<'w> {
    base: &'w dyn horizon_world::level::LevelAccess,
    changed: HashMap<horizon_core::position::BlockPos, BlockStateId>,
    random: horizon_core::random::LegacyRandom,
    /// The sounds the use plays (a door's creak).
    sounds: Vec<DrySound>,
}

type DrySound = (
    horizon_core::position::BlockPos,
    std::sync::Arc<str>,
    f32,
    f32,
);
/// The blocks a use on the dry level set.
type DryChanges = Vec<(horizon_core::position::BlockPos, BlockStateId)>;
/// What a use on the dry level changed, and the sounds it played.
type DryUse = (DryChanges, Vec<DrySound>);
/// Where `BlockItem.place` on the dry level placed, and the blocks it set.
type DryPlace = (horizon_core::position::BlockPos, DryChanges);

/// The blocks a guest's hand flips that its use predicts (and hears at once): a wooden or
/// copper door, trapdoor or fence gate, a lever or a button. An iron door or trapdoor has
/// no hand use.
pub(super) fn hand_operated(name: &str) -> bool {
    let name = name.trim_start_matches("minecraft:");
    ((name.ends_with("_door") || name.ends_with("_trapdoor")) && !name.starts_with("iron_"))
        || name.ends_with("_fence_gate")
        || name.ends_with("_button")
        || name == "lever"
}

impl<'w> DryLevel<'w> {
    fn new(base: &'w dyn horizon_world::level::LevelAccess) -> Self {
        Self {
            base,
            changed: HashMap::new(),
            random: horizon_core::random::LegacyRandom::new(0),
            sounds: Vec::new(),
        }
    }
}

impl horizon_world::level::LevelAccess for DryLevel<'_> {
    fn emit(&mut self, event: horizon_world::level::LevelEvent) {
        if let horizon_world::level::LevelEvent::Sound {
            pos,
            sound,
            volume,
            pitch,
        } = event
        {
            self.sounds.push((pos, sound, volume, pitch));
        }
    }
    fn block_state(&self, pos: horizon_core::position::BlockPos) -> BlockStateId {
        self.changed
            .get(&pos)
            .copied()
            .unwrap_or_else(|| self.base.block_state(pos))
    }
    fn set_raw(
        &mut self,
        pos: horizon_core::position::BlockPos,
        state: BlockStateId,
    ) -> Option<BlockStateId> {
        let previous = self.block_state(pos);
        self.changed.insert(pos, state);
        Some(previous)
    }
    fn is_loaded(&self, pos: horizon_core::position::BlockPos) -> bool {
        self.base.is_loaded(pos)
    }
    fn min_y(&self) -> i32 {
        self.base.min_y()
    }
    fn max_y(&self) -> i32 {
        self.base.max_y()
    }
    fn block_light(&self, pos: horizon_core::position::BlockPos) -> i32 {
        self.base.block_light(pos)
    }
    fn sky_light(&self, pos: horizon_core::position::BlockPos) -> i32 {
        self.base.sky_light(pos)
    }
    fn sky_darken(&self) -> i32 {
        self.base.sky_darken()
    }
    fn is_unobstructed(&self, pos: horizon_core::position::BlockPos, state: BlockStateId) -> bool {
        self.base.is_unobstructed(pos, state)
    }
    fn random(&mut self) -> &mut horizon_core::random::LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        self.base.game_time()
    }
}

/// `InteractionHand` on the wire.
fn hand_id(hand: Hand) -> i32 {
    match hand {
        Hand::Main => 0,
        Hand::Off => 1,
    }
}

/// Whether `Item.use` usually acts on this item (eats, drinks, equips, raises,
/// throws, shoots, fills or places), which ends `Minecraft.startUseItem` at
/// its hand. Tools and materials pass, and the off hand gets its turn. The
/// server decides; this is the client's guess of whether to go on.
fn item_has_use(info: &horizon_client::items::ItemInfo) -> bool {
    let id = info.id.trim_start_matches("minecraft:");
    info.consumable.is_some()
        || info.equip_slot.is_some()
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
                | "fishing_rod"
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
                | "carrot_on_a_stick"
                | "warped_fungus_on_a_stick"
        )
}

/// The view distance a client asks for: its render distance, which Horizon
/// lets go past vanilla's 32 (a vanilla server still caps it at its own).
fn client_view_distance(render_distance: i32) -> i8 {
    render_distance.clamp(2, i32::from(i8::MAX)) as i8
}

struct RemoteInputWorld<'g, 'a>(&'g mut Game<'a>);

struct RemoteUseWorld<'g, 'a>(&'g mut Game<'a>);

impl horizon_client::interaction_input::TargetUseWorld for RemoteUseWorld<'_, '_> {
    fn use_entity(&mut self) -> bool {
        self.0.remote_use_entity(Hand::Main)
    }
    fn use_block(&mut self) -> bool {
        self.0.remote_use_block(Hand::Main)
    }
    fn use_item(&mut self) {
        self.0.remote_use_hands();
    }
}

impl horizon_client::interaction_input::InteractionWorld for RemoteInputWorld<'_, '_> {
    fn using_item(&self) -> bool {
        self.0.using.is_some()
    }
    fn release_using(&mut self) {
        self.0.send(S::PlayerAction(play::PlayerAction {
            action: 6,
            pos: [0; 3],
            direction: 0,
            sequence: 0,
        }));
        self.0.using = None;
    }
    fn start_attack(&mut self) -> bool {
        self.0.remote_start_attack()
    }
    fn start_use(&mut self) {
        self.0.remote_use();
    }
    fn pick(&mut self) {
        if let Some(hit) = self.0.pick() {
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

impl Remote {
    /// Players in the server's player list, this one included.
    pub(super) fn player_count(&self) -> usize {
        self.players.len()
    }
    /// The local player's profile UUID.
    pub(super) fn profile(&self) -> u128 {
        self.profile
    }
    /// The entity id of a player in the level, by profile UUID.
    pub(super) fn player_entity(&self, uuid: u128) -> Option<u32> {
        self.level_players
            .iter()
            .find(|(_, u)| **u == uuid)
            .map(|(id, _)| *id as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::Game;

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
            obstructed: bool,
            loaded: bool,
            ceiling: i32,
        }
        impl LevelAccess for Level {
            fn block_state(&self, pos: BlockPos) -> BlockStateId {
                if pos == BlockPos::new(0, 0, 0) {
                    self.stone
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
            obstructed: false,
            loaded: true,
            ceiling: 320,
        };
        for item in [
            "minecraft:stone",
            "minecraft:oak_slab",
            "minecraft:oak_door",
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
            for (obstructed, loaded, ceiling) in [
                (false, true, 320),
                (true, true, 320),
                (false, false, 320),
                (false, true, 1),
            ] {
                base.obstructed = obstructed;
                base.loaded = loaded;
                base.ceiling = ceiling;
                let valid = !obstructed && loaded && ceiling > 1;
                let mut preview = DryLevel::new(&base);
                let outcome = behaviours.place_block(&mut preview, &mut Ticks::default(), &ctx);
                assert_eq!(
                    matches!(outcome, PlaceOutcome::Placed { .. }),
                    valid,
                    "{item}, obstructed={obstructed}, loaded={loaded}, ceiling={ceiling}"
                );
                assert_eq!(preview.changed.is_empty(), !valid);
                assert_eq!(base.block_state(BlockPos::new(0, 1, 0)), base.air);
            }
        }
    }

    #[test]
    fn every_inventory_menu_slot_is_a_game_slot_and_45_is_the_off_hand() {
        // InventoryMenu: result, 2x2 grid, armour, 27 + 9 inventory slots, then the off hand.
        assert_eq!(Game::menu_slot("inventory", 45), Some(504));
        let mut seen = std::collections::HashSet::new();
        for slot in 0..46 {
            let internal = Game::menu_slot("inventory", slot);
            assert!(internal.is_some_and(|i| seen.insert(i)), "slot {slot}");
        }
        assert_eq!(Game::menu_slot("inventory", 46), None);
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
}
