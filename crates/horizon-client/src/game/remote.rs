//! Multiplayer: the game driven by a server connection instead of the local
//! simulation. Mirrors vanilla's ClientPacketListener (chunks, entities,
//! inventory, health, time, weather, chat, respawn and dimension changes),
//! LocalPlayer.sendPosition (movement packets) and MultiPlayerGameMode
//! (block breaking with sequence numbers, use-item, attacks, container
//! clicks). The player still moves with the local physics, as the vanilla
//! client predicts its own movement; the server stays authoritative for
//! everything else.
use super::{Dim, Game, Mining, Screen};
use horizon_client::{
    Result,
    chunk::{BIOME_STRATEGY, BLOCK_STRATEGY, Chunk, Light, MIN_Y, SECTIONS, Section},
    containers::{BlockEntity, Furnace, FurnaceKind},
    entities::{Experience, ItemEntity},
    hud::{HudBatch, rgba},
    items::ItemStack,
    mobs::{Mob, MobType},
    motion::{MotionWorld, PlayerMotion},
    net::{Event, Session},
    save::SaveDir,
    sprint::Keys,
    streaming::{ChunkGenerator, Proto, Streaming},
    world::World,
};
use horizon_core::{geometry::Vec3, paletted::PalettedContainer, random::RandomSource};
use horizon_protocol::{
    entity_data::{ITEM_ENTITY_ITEM, Value},
    item::{Hashed, Stack},
    packets::{
        Empty,
        play::{self, Clientbound as C, Serverbound as S},
    },
    registry::{Builtin, DimensionType, Dynamic, ceil_log2},
    text::{Language, format_java, plain},
};
use horizon_world::{nbt::Tag, registry::BlockStateId};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{event::MouseButton, keyboard::KeyCode};

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
struct Lerp {
    target: [f64; 3],
    steps: u32,
}

pub(super) struct Remote {
    pub session: Session,
    builtin: Arc<Builtin>,
    lang: Arc<Language>,
    registries: Arc<Dynamic>,
    joined: bool,
    status: String,
    entity_id: i32,
    dimension: DimensionType,
    biome_map: Vec<u16>,
    biome_bits: u8,
    view_distance: i32,
    sequence: i32,
    last_pos: [f64; 3],
    last_rot: (f32, f32),
    last_on_ground: bool,
    last_collision: bool,
    reminder: u32,
    last_input: u8,
    was_sprinting: bool,
    carried: Option<usize>,
    loaded_sent: bool,
    /// Open container: (window id, menu name).
    menu: Option<(i32, String)>,
    state_id: i32,
    use_target: Option<[i32; 3]>,
    entities: HashMap<i32, Tracked>,
    lerps: HashMap<i32, Lerp>,
    /// Item entities waiting for their item data.
    pending_items: HashMap<i32, [f64; 3]>,
    health: (f32, i32, f32),
    game_mode: i32,
    pub chat: Vec<(String, Instant)>,
    pub chat_input: Option<String>,
    action_bar: Option<(String, Instant)>,
    players: HashMap<u128, String>,
    /// `ClientPacketListener.playerInfoMap` as player-list entries, and
    /// which of them are listed.
    tab_players: HashMap<u128, horizon_client::tab_list::TabPlayer>,
    /// Each listed profile's unpacked `textures` property.
    skin_textures: HashMap<u128, horizon_client::skins::Textures>,
    listed: std::collections::HashSet<u128>,
    /// `ClientboundLoginPacket.onlineMode`: the player list shows heads.
    online_mode: bool,
    /// Player entities in the level: entity id to profile UUID.
    level_players: HashMap<i32, u128>,
    /// The local player's profile UUID (always in the level).
    profile: u128,
    destroying: Option<([i32; 3], u8, f32)>,
    destroy_delay: u32,
    placing: Option<([i32; 3], u64)>,
    death_message: Option<String>,
    player_type: Option<Arc<MobType>>,
    /// Inventory as last synchronised, for creative-mode slot updates.
    synced: Vec<Option<ItemStack>>,
    decode_errors: u32,
    clock: Option<i32>,
    clock_rate: f32,
}

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
    /// Leave the title for a server (ConnectScreen): an empty world fed by the connection.
    pub(super) fn connect_server(&mut self, address: &str, name: &str) -> Result<()> {
        self.perf.world_opening();
        let root = self.options.resources.clone();
        let builtin = Builtin::find(&root)
            .and_then(|p| Builtin::load(&p).ok())
            .ok_or("registries.json not found next to the resources; run the data generator reports (horizon-prepare)")?;
        let lang = Arc::new(horizon_client::net::load_language(&root));
        let view = self.settings.render_distance.clamp(2, 32) as i8;
        horizon_client::crash::set_detail("Server", horizon_client::net::shown_address(address));
        // A signed-in account always plays under its own name.
        let auth = horizon_client::auth::current();
        let name = auth.as_ref().map_or(name, |a| a.name.as_str());
        let session = Session::connect(address, name, auth.clone(), view, lang.clone());
        self.extras_connecting(address, name);
        let player_type = super::perspective::player_type(&root);
        let mut streaming = Streaming::new(
            Arc::new(NoTerrain),
            self.res.context.clone(),
            self.res.threads,
        );
        streaming.remote = true;
        streaming.radius = self.settings.render_distance;
        self.streaming = streaming;
        self.world = World::new(self.res.facts.air, 0);
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
        self.set_mode_id(0);
        self.experience = Experience {
            level: 0,
            progress: 0.0,
            total: 0,
        };
        self.screen = Screen::Playing;
        self.title = None;
        self.dim = Dim::Overworld;
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections();
        }
        self.apply_settings();
        self.remote = Some(Remote {
            session,
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
            last_pos: [0.0; 3],
            last_rot: (0.0, 0.0),
            last_on_ground: false,
            last_collision: false,
            reminder: 0,
            last_input: 0,
            was_sprinting: false,
            carried: None,
            loaded_sent: false,
            menu: None,
            state_id: 0,
            use_target: None,
            entities: HashMap::new(),
            lerps: HashMap::new(),
            pending_items: HashMap::new(),
            health: (20.0, 20, 5.0),
            game_mode: 0,
            chat: Vec::new(),
            chat_input: None,
            action_bar: None,
            players: HashMap::new(),
            tab_players: HashMap::new(),
            skin_textures: HashMap::new(),
            listed: Default::default(),
            online_mode: false,
            level_players: HashMap::new(),
            profile: 0,
            destroying: None,
            destroy_delay: 0,
            placing: None,
            death_message: None,
            player_type,
            synced: vec![None; 36],
            decode_errors: 0,
            clock: None,
            clock_rate: 1.0,
        });
        Ok(())
    }

    /// Disconnect and return to the multiplayer screen (DisconnectedScreen).
    pub(super) fn leave_server(&mut self, reason: Option<String>) -> Result<()> {
        self.finish_perf();
        if reason.is_none() {
            // Leaving on purpose: nothing for auto-reconnect to retry.
            self.extras_left();
        }
        if let Some(mut r) = self.remote.take() {
            r.session.disconnect();
        }
        horizon_client::crash::clear_detail("Server");
        self.debug_charts.reset();
        self.tab_list.reset();
        self.score.board = Default::default();
        self.streaming = Streaming::new(Arc::new(NoTerrain), self.res.context.clone(), 1);
        self.streaming.paused = true;
        self.world = World::new(self.res.facts.air, 0);
        self.player = None;
        self.mobs.clear();
        self.dropped.clear();
        self.particles.clear();
        self.pickups.clear();
        self.score.board = Default::default();
        self.tab_list = Default::default();
        self.border = Default::default();
        self.screen = Screen::Playing;
        self.container = None;
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections();
        }
        let saves = std::env::current_dir()?.join("saves");
        let mut title = horizon_client::title::Title::new(saves.clone(), &self.options.resources);
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
    fn item_id(&self, name: &str) -> i32 {
        let r = self.remote.as_ref().expect("remote");
        r.builtin
            .get("minecraft:item")
            .and_then(|i| i.id(name))
            .unwrap_or(0)
    }
    fn to_stack(&self, s: &Option<Stack>) -> Option<ItemStack> {
        let s = s.as_ref()?;
        let mut out = ItemStack {
            item: self.item_name(s.item),
            count: s.count.max(0) as u32,
            damage: s.damage().max(0) as u32,
            extra: None,
        };
        use horizon_protocol::item::{Component, component};
        if let Some(Component::Int(rgb)) = s.component(component::DYED_COLOR) {
            out.data_mut().dyed_color = Some(*rgb);
        }
        if let Some(Component::Int(id)) = s.component(component::MAP_ID) {
            out.data_mut().map_id = Some(*id);
        }
        Some(out)
    }
    fn to_network(&self, s: &Option<ItemStack>) -> Option<Stack> {
        let s = s.as_ref()?;
        let mut stack = Stack::new(self.item_id(&s.item), s.count as i32);
        if s.damage > 0 {
            stack.added.push((
                horizon_protocol::item::component::DAMAGE,
                horizon_protocol::item::Component::Int(s.damage as i32),
            ));
        }
        Some(stack)
    }
    fn chat_line(&mut self, text: String) {
        log::info!("[chat] {text}");
        if let Some(r) = self.remote.as_mut() {
            r.chat.push((text, Instant::now()));
            if r.chat.len() > 100 {
                r.chat.remove(0);
            }
        }
    }

    /// Process everything the network thread received (once per frame, like
    /// the client's packet processor).
    pub(super) fn poll_remote(&mut self) -> Result<()> {
        let Some(r) = &self.remote else { return Ok(()) };
        let events = r.session.poll();
        for event in events {
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
                        r.status = format!("Joined as {name}; loading terrain...");
                    }
                }
                Event::Reconfigured(registries) => self.set_registries(registries),
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
                    let retry = self.reconnect_after(&reason, from_server);
                    self.leave_server(Some(reason))?;
                    self.offer_reconnect(retry);
                    return Ok(());
                }
                Event::Packet(packet) => self.handle_packet(*packet)?,
            }
            if self.remote.is_none() {
                break;
            }
        }
        Ok(())
    }
    fn set_registries(&mut self, registries: Arc<Dynamic>) {
        let biomes: Vec<u16> = registries
            .registries
            .get("minecraft:worldgen/biome")
            .map(|list| {
                list.iter()
                    .map(|(name, _)| self.res.biomes.id(name))
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
        self.set_mode_id(spawn.game_type);
        self.dim = if spawn.dimension == "minecraft:the_nether" {
            Dim::Nether
        } else {
            Dim::Overworld
        };
        if !same {
            let mut streaming = Streaming::new(
                Arc::new(NoTerrain),
                self.res.context.clone(),
                self.res.threads,
            );
            streaming.remote = true;
            streaming.radius = self.streaming.radius;
            self.streaming = streaming;
            let (time, day, weather) = (
                self.world.game_time,
                self.world.day_time,
                self.world.weather,
            );
            self.world = World::new(self.res.facts.air, spawn.seed);
            self.world.game_time = time;
            self.world.day_time = day;
            self.world.weather = weather;
            self.mobs.clear();
            self.dropped.clear();
            self.particles.clear();
            self.pickups.clear();
            if let Some(r) = self.remote.as_mut() {
                r.entities.clear();
                r.lerps.clear();
                r.pending_items.clear();
            }
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
            let strategy = BLOCK_STRATEGY;
            let palette = s.states.palette.iter().map(|&p| p as u32).collect();
            let words = s.states.data.iter().map(|&w| w as u64).collect();
            let blocks = PalettedContainer::from_wire(strategy, s.states.bits, palette, words)
                .map_err(|e| e.to_string())?;
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
        Ok(chunk)
    }
    /// Keep the renderable block entities (chests, furnaces) in step with blocks.
    fn sync_block_entity(&mut self, pos: (i32, i32, i32), state: BlockStateId) {
        let name = self.res.facts.get(state).name.clone();
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
            && !self.res.facts.is_air(state)
        {
            let s = self.res.facts.get(state).sounds.clone();
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
    fn spawn_entity(&mut self, id: i32, kind: &str, position: [f64; 3], yaw: f32, head: f32) {
        let at = Vec3::new(position[0], position[1], position[2]);
        let mob_type = if kind == "minecraft:player" {
            self.remote.as_ref().and_then(|r| r.player_type.clone())
        } else {
            self.res.mob_types.get(kind).cloned()
        };
        if let Some(t) = mob_type {
            let mut mob = Mob::new(id as u32, t, at, head);
            mob.body_yaw = yaw;
            mob.previous_body_yaw = yaw;
            mob.persistent = true;
            self.mobs.retain(|m| m.id != id as u32);
            self.mobs.push(mob);
        }
        if let Some(r) = self.remote.as_mut() {
            if kind == "minecraft:item" {
                r.pending_items.insert(id, position);
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
    fn move_entity(
        &mut self,
        id: i32,
        position: [f64; 3],
        rotation: Option<(f32, f32)>,
        on_ground: bool,
    ) {
        if let Some(r) = self.remote.as_mut() {
            if let Some(t) = r.entities.get_mut(&id) {
                t.position = position;
            }
            r.lerps.insert(
                id,
                Lerp {
                    target: position,
                    steps: 3,
                },
            );
            if let Some(p) = r.pending_items.get_mut(&id) {
                *p = position;
            }
        }
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id as u32) {
            m.on_ground = on_ground;
            if let Some((yaw, pitch)) = rotation {
                m.body_yaw = yaw;
                m.head_pitch = pitch;
            }
        }
    }
    fn remove_entity(&mut self, id: i32) {
        self.mobs.retain(|m| m.id != id as u32);
        self.dropped.retain(|d| d.id != id as u32);
        if let Some(r) = self.remote.as_mut() {
            r.entities.remove(&id);
            r.lerps.remove(&id);
            r.pending_items.remove(&id);
        }
    }
    /// Map a menu slot to Horizon's inventory screen slot ids.
    fn menu_slot(menu: &str, slot: i32) -> Option<usize> {
        let s = usize::try_from(slot).ok()?;
        let rows = |n: usize| -> Option<usize> {
            match s {
                _ if s < n * 9 => Some(300 + s),
                _ if s < n * 9 + 27 => Some(9 + s - n * 9),
                _ if s < n * 9 + 36 => Some(s - n * 9 - 27),
                _ => None,
            }
        };
        match menu {
            "inventory" => match s {
                0 => Some(200),
                1..=4 => Some(100 + s - 1),
                5..=8 => Some(500 + s - 5),
                9..=35 => Some(s),
                36..=44 => Some(s - 36),
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
            _ => None,
        }
    }
    fn open_menu_name(&self, container: i32) -> String {
        match self.remote.as_ref().and_then(|r| r.menu.as_ref()) {
            Some((id, name)) if *id == container && container != 0 => name.clone(),
            _ => "inventory".into(),
        }
    }
    fn set_menu_slot(&mut self, container: i32, slot: i32, item: Option<ItemStack>) {
        let menu = self.open_menu_name(container);
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
    }

    fn handle_packet(&mut self, packet: C) -> Result<()> {
        match packet {
            // PingDebugMonitor.onPongReceived.
            C::PongResponse(p) => {
                let delay = self.debug_millis() - p.time;
                self.debug_charts.ping.log(delay);
            }
            C::Login(p) => {
                if let Some(r) = self.remote.as_mut() {
                    r.online_mode = p.online_mode;
                    r.entity_id = p.player_id;
                    r.view_distance = p.chunk_radius;
                }
                self.streaming.radius =
                    self.settings.render_distance.min(p.chunk_radius + 1).max(2);
                self.enter_remote_dimension(&p.spawn, false);
                self.send(S::ClientInformation(
                    horizon_protocol::packets::configuration::ClientInformation::new(
                        self.settings.render_distance.clamp(2, 32) as i8,
                    ),
                ));
            }
            C::Respawn(p) => {
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
                    }
                }
                Err(e) => log::warn!("Horizon: chunk {} {}: {e}", p.x, p.z),
            },
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
            C::BlockUpdate(p) => self.set_block(p.pos, p.state),
            C::SectionBlocksUpdate(p) => {
                let [sx, sy, sz] = p.section;
                for change in p.changes {
                    let local = (change & 0xFFF) as i32;
                    let pos = [
                        sx * 16 + (local >> 8 & 15),
                        sy * 16 + (local & 15),
                        sz * 16 + (local >> 4 & 15),
                    ];
                    self.set_block(pos, (change >> 12) as i32);
                }
            }
            C::BlockChangedAck(_) => {}
            C::LevelEvent(p) if p.kind == 2015 => {
                self.dragon_egg_particles(p.pos, p.data);
            }
            C::LevelEvent(p) if p.kind == 2001 => {
                // LevelEvent.PARTICLES_DESTROY_BLOCK: someone else broke a block.
                let state = BlockStateId(p.data.max(0) as u32);
                let s = self.res.facts.get(state).sounds.clone();
                let at = [
                    f64::from(p.pos[0]) + 0.5,
                    f64::from(p.pos[1]) + 0.5,
                    f64::from(p.pos[2]) + 0.5,
                ];
                self.audio.play(
                    &s.break_sound,
                    Some(at),
                    (s.volume + 1.0) / 2.0,
                    s.pitch * 0.8,
                );
                self.destroy_particles(p.pos, state);
            }
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
                let yaw = horizon_protocol::buf::unpack_degrees(p.yaw);
                let head = horizon_protocol::buf::unpack_degrees(p.head_yaw);
                if let Some(r) = self.remote.as_mut() {
                    if kind == "minecraft:player" {
                        r.level_players.insert(p.id, p.uuid);
                    } else {
                        r.level_players.remove(&p.id);
                    }
                }
                self.spawn_entity(p.id, &kind, p.position, yaw, head);
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
                let position = m.delta.as_ref().map_or(base, |d| d.end(base));
                let rotation = m.rotation.map(|(y, x)| {
                    (
                        horizon_protocol::buf::unpack_degrees(y),
                        horizon_protocol::buf::unpack_degrees(x),
                    )
                });
                self.move_entity(m.id, position, rotation, m.on_ground);
            }
            C::EntityPositionSync(p) => {
                self.move_entity(p.id, p.path.end(), Some((p.yaw, p.pitch)), p.on_ground)
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
                self.move_entity(p.id, position, Some((p.yaw, p.pitch)), p.on_ground);
            }
            C::RotateHead(p) => {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    m.yaw = horizon_protocol::buf::unpack_degrees(p.head_yaw);
                }
            }
            C::SetEntityData(p) => {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32)
                    && m.species() == horizon_client::mobs::Species::Rabbit
                {
                    for (index, _, value) in &p.values {
                        match (index, value) {
                            (15, Value::Byte(flags)) => m.no_ai = flags & 1 != 0,
                            (16, Value::Bool(baby)) => m.extra.age = if *baby { -1 } else { 0 },
                            _ => {}
                        }
                    }
                }
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
                for (index, _, value) in &p.values {
                    if *index == ITEM_ENTITY_ITEM
                        && let Value::Item(stack) = value
                    {
                        self.remote_item_data(p.id, stack);
                    }
                }
            }
            C::TakeItemEntity(p) => {
                if self
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
                if let Some(item) = self.dropped.iter().find(|d| d.id == p.item as u32).cloned() {
                    let collector = if self
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.entity_id == p.player)
                    {
                        super::pickups::Collector::Player
                    } else {
                        super::pickups::Collector::Mob(p.player as u32)
                    };
                    self.add_pickup(super::pickups::Taken::Item(Box::new(item)), collector);
                }
                self.remove_entity(p.item);
            }
            C::HurtAnimation(p) => {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    m.hurt_time = 10;
                }
            }
            C::EntityEvent(p) => {
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
            }
            C::Animate(p) if p.action == 0 => {}
            C::SetHealth(p) => {
                if let Some(r) = self.remote.as_mut() {
                    r.health = (p.health, p.food, p.saturation);
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
                if let Some(r) = self.remote.as_mut() {
                    r.death_message = Some(message);
                }
            }
            C::SetExperience(p) => {
                self.experience = Experience {
                    level: p.level,
                    progress: p.progress,
                    total: p.total,
                }
            }
            C::SetTime(p) => {
                self.world.game_time = p.game_time;
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
                    }
                }
            }
            C::GameEvent(p) => match p.event {
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
            C::PlayerAbilities(p) => self.creative = p.flags & 8 != 0,
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
                    _ => {}
                }
                self.remote_synced();
            }
            C::ContainerSetData(p) => {
                if let Some(key) = self.container
                    && let Some(BlockEntity::Furnace(f)) = self.world.block_entities.get_mut(&key)
                {
                    let v = i32::from(p.value);
                    match p.id {
                        0 => f.lit_remaining = v,
                        1 => f.lit_total = v,
                        2 => f.cooking_timer = v,
                        3 => f.cooking_total = v,
                        _ => {}
                    }
                }
            }
            C::OpenScreen(p) => self.remote_open_screen(p),
            C::MerchantOffers(p) => self.remote_merchant_offers(p),
            C::ContainerClose(_) => {
                if self.screen == Screen::Inventory {
                    self.remote_close_local();
                    self.screen = Screen::Playing;
                    self.capture(true);
                }
            }
            C::SystemChat(p) => {
                let text = plain(
                    &p.content,
                    &self
                        .remote
                        .as_ref()
                        .map(|r| r.lang.clone())
                        .unwrap_or_default(),
                );
                if p.overlay {
                    if let Some(r) = self.remote.as_mut() {
                        r.action_bar = Some((text, Instant::now()));
                    }
                } else {
                    self.chat_line(text);
                }
            }
            C::PlayerChat(p) => {
                let lang = self
                    .remote
                    .as_ref()
                    .map(|r| r.lang.clone())
                    .unwrap_or_default();
                let content = p
                    .unsigned_content
                    .as_ref()
                    .map_or_else(|| p.content.clone(), |t| plain(t, &lang));
                let line = self.format_chat(&p.chat_type, &content);
                self.chat_line(line);
            }
            C::DisguisedChat(p) => {
                let lang = self
                    .remote
                    .as_ref()
                    .map(|r| r.lang.clone())
                    .unwrap_or_default();
                let content = plain(&p.message, &lang);
                let line = self.format_chat(&p.chat_type, &content);
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
            }
            C::SetActionBarText(p) | C::SetTitleText(p) => {
                let lang = self
                    .remote
                    .as_ref()
                    .map(|r| r.lang.clone())
                    .unwrap_or_default();
                let text = plain(&p.text, &lang);
                if let Some(r) = self.remote.as_mut() {
                    r.action_bar = Some((text, Instant::now()));
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
        let r = self.remote.as_ref()?;
        let mut players: Vec<_> = r
            .listed
            .iter()
            .filter_map(|u| r.tab_players.get(u).cloned())
            .collect();
        for p in &mut players {
            let Some(textures) = r.skin_textures.get(&p.uuid) else {
                continue;
            };
            // PlayerInfo.createSkinLookup: only the local player's own skin
            // may be unsigned.
            let downloads = &mut self.skin_downloads;
            let skin = horizon_client::skins::lookup(textures, p.uuid != r.profile, |kind, url| {
                downloads.state(kind, url)
            });
            if let Some(skin) = skin {
                p.skin = skin;
            }
        }
        for (location, rgba) in self.skin_downloads.take_ready() {
            self.store_skin(location, rgba);
        }
        for p in &mut players {
            if p.skin.starts_with("skins/") && !self.show_skin(&p.skin) {
                p.skin = horizon_client::tab_list::default_skin(p.uuid).to_owned();
            }
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
    /// Puts a downloaded skin in an atlas skin slot: a free one, else the
    /// least recently shown.
    fn store_skin(&mut self, location: String, rgba: Vec<u8>) {
        use horizon_client::hud::SKIN_SLOTS;
        let slot = if self.skin_slots.len() < SKIN_SLOTS {
            self.skin_slots.len()
        } else {
            let (old, slot) = self.skin_slots.remove(0);
            self.res.atlas.sprites.remove(&old);
            slot
        };
        let Some(rect) = self
            .res
            .atlas
            .sprites
            .get(&format!("horizon/skin{slot}"))
            .copied()
        else {
            return;
        };
        let (w, h) = (self.res.atlas.width as f32, self.res.atlas.height as f32);
        self.map_patches.push(horizon_client::font::Patch {
            x: (rect.u0 * w).round() as u32,
            y: (rect.v0 * h).round() as u32,
            width: 64,
            height: 64,
            rgba,
        });
        self.res.atlas.sprites.insert(location.clone(), rect);
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
    fn format_chat(&self, bound: &play::ChatTypeBound, content: &str) -> String {
        let Some(r) = self.remote.as_ref() else {
            return content.to_owned();
        };
        let sender = plain(&bound.name, &r.lang);
        let decoration = r
            .registries
            .data("minecraft:chat_type", bound.chat_type)
            .and_then(|t| t.get("chat"));
        let key = match decoration.and_then(|d| d.get("translation_key")) {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => "chat.type.text".into(),
        };
        let params: Vec<String> = match decoration.and_then(|d| d.get("parameters")) {
            Some(Tag::List(list)) => list
                .iter()
                .map(|p| match p {
                    Tag::String(s) => match s.to_string_lossy().as_str() {
                        "sender" => sender.clone(),
                        "target" => bound
                            .target
                            .as_ref()
                            .map(|t| plain(t, &r.lang))
                            .unwrap_or_default(),
                        _ => content.to_owned(),
                    },
                    _ => String::new(),
                })
                .collect(),
            _ => vec![sender, content.to_owned()],
        };
        let pattern = r
            .lang
            .get(&key)
            .cloned()
            .unwrap_or_else(|| "<%s> %s".into());
        format_java(&pattern, &params)
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
        let Some(p) = position else { return };
        let at = Vec3::new(p[0], p[1], p[2]);
        let bob_offset = self.world.random.next_float() * std::f32::consts::PI * 2.0;
        self.dropped.push(ItemEntity {
            health: 5,
            fire_ticks: 0,
            id: id as u32,
            stack,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            on_ground: false,
            age: 0,
            pickup_delay: 0,
            bob_offset,
        });
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
        let delta = Vec3::new(
            pick(rel::DELTA_X, velocity.x, p.delta[0]),
            pick(rel::DELTA_Y, velocity.y, p.delta[1]),
            pick(rel::DELTA_Z, velocity.z, p.delta[2]),
        );
        match self.player.as_mut() {
            Some(player) => {
                player.position = position;
                player.velocity = delta;
            }
            None => {
                let mut player = PlayerMotion::new(self.motion, position)?;
                player.velocity = delta;
                self.player = Some(player);
                self.pad.player_replaced();
                self.sway = Default::default();
                self.eye = 1.62;
                self.previous_eye = 1.62;
                self.clock.reset();
                self.apply_remote_health();
            }
        }
        self.previous_position = position;
        self.yaw = yaw;
        self.pitch = pitch;
        if let Some(r) = self.remote.as_mut() {
            r.last_pos = [position.x, position.y, position.z];
            r.last_rot = (yaw, pitch);
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
        self.ticks += 1;
        self.world.game_time += 1;
        let rate = self.remote.as_ref().map_or(1.0, |r| r.clock_rate);
        if rate > 0.0 {
            self.world.day_time += rate.round() as i64;
        }
        // Weather levels drift toward their targets between server updates.
        let w = &mut self.world.weather;
        w.rain_level.0 = w.rain_level.1;
        w.thunder_level.0 = w.thunder_level.1;
        if self.use_delay > 0 {
            self.use_delay -= 1;
        }
        // ClientLevel.tick: the border's size change.
        self.border.tick();
        self.tick_remote_entities();
        self.tick_hand();
        self.tick_particles();
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        let (cx, cz) = (
            (player.position.x.floor() as i32) >> 4,
            (player.position.z.floor() as i32) >> 4,
        );
        if !self.streaming.is_lit(cx, cz) {
            return Ok(());
        }
        if self.remote.as_ref().is_some_and(|r| !r.loaded_sent) {
            // ClientPacketListener: the level around the player is ready.
            self.send(S::PlayerLoaded(Empty));
            if let Some(r) = self.remote.as_mut() {
                r.loaded_sent = true;
            }
        }
        let stick = self.pad_move();
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        let chatting = self.remote.as_ref().is_some_and(|r| r.chat_input.is_some());
        let playing = self.screen == Screen::Playing && !chatting;
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
        self.previous_position = player.position;
        let before = (
            player.ground.on_ground,
            [player.velocity.x, player.velocity.y, player.velocity.z],
            [player.position.x, player.position.y, player.position.z],
        );
        player.yaw = self.yaw;
        let dead = self.screen == Screen::Dead;
        if !dead {
            MotionWorld {
                player,
                scene: &mut self.world,
                assets: &self.res.assets,
                collision_binding: self.collision,
                resources: self.motion,
            }
            .controlled_simulation_tick(keys)?;
        }
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
                    let sounds = self.res.facts.get(state).sounds.clone();
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
        // The server owns health and hunger: undo local food/fall changes.
        self.apply_remote_health();
        self.tick_avatar(before, false);
        if !dead {
            self.remote_mining();
            if self.right && self.use_delay == 0 && playing {
                self.remote_use();
                self.use_delay = 4;
            }
        }
        self.remote_send_state(input);
        self.send(S::ClientTickEnd(Empty));
        Ok(())
    }
    fn tick_remote_entities(&mut self) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut lerps = std::mem::take(&mut r.lerps);
        for m in &mut self.mobs {
            m.previous = m.position;
            m.previous_yaw = m.yaw;
            m.previous_body_yaw = m.body_yaw;
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
            if let Some(l) = lerps.get_mut(&(m.id as i32))
                && l.steps > 0
            {
                let n = f64::from(l.steps);
                m.position = Vec3::new(
                    m.position.x + (l.target[0] - m.position.x) / n,
                    m.position.y + (l.target[1] - m.position.y) / n,
                    m.position.z + (l.target[2] - m.position.z) / n,
                );
                l.steps -= 1;
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
        for d in &mut self.dropped {
            d.previous = d.position;
            d.age += 1;
            if let Some(l) = lerps.get_mut(&(d.id as i32))
                && l.steps > 0
            {
                let n = f64::from(l.steps);
                d.position = Vec3::new(
                    d.position.x + (l.target[0] - d.position.x) / n,
                    d.position.y + (l.target[1] - d.position.y) / n,
                    d.position.z + (l.target[2] - d.position.z) / n,
                );
                l.steps -= 1;
            }
        }
        lerps.retain(|_, l| l.steps > 0);
        if let Some(r) = self.remote.as_mut() {
            for (id, l) in lerps {
                r.lerps.entry(id).or_insert(l);
            }
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
        r.reminder += 1;
        let d = [
            pos[0] - r.last_pos[0],
            pos[1] - r.last_pos[1],
            pos[2] - r.last_pos[2],
        ];
        let moved = d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 2.0e-4 * 2.0e-4 || r.reminder >= 20;
        let rotated = yaw != r.last_rot.0 || pitch != r.last_rot.1;
        let flags = u8::from(on_ground) | u8::from(collision) << 1;
        let [x, y, z] = pos;
        if moved && rotated {
            out.push(S::MovePlayerPosRot(play::MovePlayerPosRot {
                x,
                y,
                z,
                yaw,
                pitch,
                flags,
            }));
        } else if moved {
            out.push(S::MovePlayerPos(play::MovePlayerPos { x, y, z, flags }));
        } else if rotated {
            out.push(S::MovePlayerRot(play::MovePlayerRot { yaw, pitch, flags }));
        } else if r.last_on_ground != on_ground || r.last_collision != collision {
            out.push(S::MovePlayerStatus(play::MovePlayerStatus { flags }));
        }
        if moved {
            r.last_pos = pos;
            r.reminder = 0;
        }
        if rotated {
            r.last_rot = (yaw, pitch);
        }
        r.last_on_ground = on_ground;
        r.last_collision = collision;
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

    /// MultiPlayerGameMode.startDestroyBlock / continueDestroyBlock.
    fn remote_mining(&mut self) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        if r.destroy_delay > 0 {
            r.destroy_delay -= 1;
        }
        let chatting = r.chat_input.is_some();
        if !self.left || self.screen != Screen::Playing || chatting {
            if let Some((pos, _, _)) = r.destroying.take() {
                self.send(S::PlayerAction(play::PlayerAction {
                    action: 2,
                    pos,
                    direction: 0,
                    sequence: 0,
                }));
            }
            self.mining = None;
            return;
        }
        let Some(hit) = self.pick() else {
            self.mining = None;
            return;
        };
        let face = face_direction(hit.face);
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if self.res.facts.is_air(state) {
            return;
        }
        if self.remote.as_ref().is_some_and(|r| r.destroy_delay > 0) {
            return;
        }
        self.swing_arm();
        self.send(S::Punch(Empty));
        if self.creative {
            let seq = self.next_sequence();
            self.predict_break(hit.pos, state);
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
        let facts = self.res.facts.get(state).clone();
        let held = self.held().cloned();
        let mut speed = self.res.items.mining_speed(held.as_ref(), &facts.name);
        if self.player.as_ref().is_some_and(|p| !p.ground.on_ground) {
            speed /= 5.0;
        }
        let harvest =
            !facts.requires_tool || self.res.items.correct_for_drops(held.as_ref(), &facts.name);
        let progress = if facts.hardness < 0.0 {
            0.0
        } else if facts.hardness == 0.0 {
            1.0
        } else {
            speed / facts.hardness / if harvest { 30.0 } else { 100.0 }
        };
        let same = self
            .remote
            .as_ref()
            .and_then(|r| r.destroying)
            .is_some_and(|(p, _, _)| p == hit.pos);
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
                self.predict_break(hit.pos, state);
                if let Some(r) = self.remote.as_mut() {
                    r.destroy_delay = 5;
                }
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
            self.predict_break(pos, state);
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
    fn predict_break(&mut self, pos: [i32; 3], state: BlockStateId) {
        let s = self.res.facts.get(state).sounds.clone();
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
    /// Minecraft.startUseItem: use on the targeted block, then the item itself.
    fn remote_use(&mut self) {
        let held_block = self
            .held()
            .and_then(|h| self.res.items.get(&h.item))
            .is_some_and(|i| i.block.is_some());
        let sneaking = self.keys.contains(&KeyCode::ShiftLeft);
        // Entities first (interact), like the hit result order.
        if let Some((i, _)) = self.mob_hit() {
            let id = self.mobs[i].id as i32;
            self.send(S::Interact(play::Interact {
                id,
                hand: 0,
                location: [0.0, 1.0, 0.0],
                secondary: sneaking,
            }));
            self.swing_arm();
            return;
        }
        if let Some(hit) = self.pick() {
            let seq = self.next_sequence();
            let cursor = [
                (hit.point[0] - f64::from(hit.pos[0])) as f32,
                (hit.point[1] - f64::from(hit.pos[1])) as f32,
                (hit.point[2] - f64::from(hit.pos[2])) as f32,
            ];
            self.send(S::UseItemOn(play::UseItemOn {
                hand: 0,
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
            if held_block {
                self.swing_arm();
                return;
            }
        }
        if self.held().is_some() {
            let seq = self.next_sequence();
            self.send(S::UseItem(play::UseItem {
                hand: 0,
                sequence: seq,
                yaw: self.yaw,
                pitch: self.pitch,
            }));
        }
    }
    /// Left click: attack the targeted entity (MultiPlayerGameMode.attack).
    pub(super) fn remote_mouse(&mut self, button: MouseButton, pressed: bool) -> Result<()> {
        if self.remote.as_ref().is_some_and(|r| r.chat_input.is_some()) {
            return Ok(());
        }
        match button {
            MouseButton::Left => {
                if pressed && let Some((i, _)) = self.mob_hit() {
                    let id = self.mobs[i].id as i32;
                    self.send(S::Attack(play::Attack { id }));
                    self.send(S::Punch(Empty));
                    self.swing_arm();
                    return Ok(());
                }
                if pressed && self.pick().is_none() {
                    self.send(S::Punch(Empty));
                    self.swing_arm();
                }
                self.left = pressed;
            }
            MouseButton::Right => {
                self.right = pressed;
                if pressed {
                    self.remote_use();
                    self.use_delay = 4;
                } else {
                    self.send(S::PlayerAction(play::PlayerAction {
                        action: 6,
                        pos: [0; 3],
                        direction: 0,
                        sequence: 0,
                    }));
                }
            }
            MouseButton::Middle if pressed => {
                if let Some(hit) = self.pick() {
                    self.send(S::PickItemFromBlock(play::PickItemFromBlock {
                        pos: hit.pos,
                        include_data: false,
                    }));
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// Keys with a multiplayer meaning; returns true when consumed.
    pub(super) fn remote_key(&mut self, key: KeyCode) -> Result<bool> {
        if self.screen != Screen::Playing {
            if key == KeyCode::KeyE && self.screen == Screen::Inventory
                || key == KeyCode::Escape && self.screen == Screen::Inventory
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
                let text = if key == KeyCode::Slash { "/" } else { "" };
                if let Some(r) = self.remote.as_mut() {
                    r.chat_input = Some(text.to_owned());
                }
                self.keys.clear();
                self.left = false;
                self.right = false;
                self.capture(false);
                Ok(true)
            }
            KeyCode::KeyQ => {
                let all = self.keys.contains(&KeyCode::ControlLeft);
                self.send(S::PlayerAction(play::PlayerAction {
                    action: if all { 4 } else { 5 },
                    pos: [0; 3],
                    direction: 0,
                    sequence: 0,
                }));
                // Inventory.removeFromSelected (prediction).
                let slot = &mut self.inventory[self.selected];
                if let Some(s) = slot {
                    if all || s.count <= 1 {
                        *slot = None;
                    } else {
                        s.count -= 1;
                    }
                }
                self.swing_arm();
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
    pub(super) fn remote_chat_open(&self) -> bool {
        self.remote.as_ref().is_some_and(|r| r.chat_input.is_some())
    }
    /// ChatScreen: typing, Enter to send, Escape to cancel.
    pub(super) fn remote_chat_key(&mut self, event: &winit::event::KeyEvent) {
        use winit::keyboard::Key;
        if event.state != winit::event::ElementState::Pressed {
            return;
        }
        let shortcut = self.edit_shortcut(event);
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let Some(input) = r.chat_input.as_mut() else {
            return;
        };
        // ChatScreen's input: an EditBox of 256.
        const INPUT: horizon_client::clipboard::FieldId = ("remote.chat", 0);
        if let Some(shortcut) = shortcut {
            Self::append_shortcut(
                &mut self.text_selection,
                &mut self.clipboard,
                INPUT,
                input,
                shortcut,
                256,
            );
            return;
        }
        // Logical keys: the text, not the scancode, decides (layouts, virtual keyboards).
        let named = match &event.logical_key {
            Key::Named(n) => Some(*n),
            _ => None,
        };
        self.remote_chat_input(named, event.text.as_deref());
    }
    /// A named key or typed text in the chat input (the keyboard, or a
    /// controller's on-screen keyboard).
    pub(super) fn remote_chat_input(
        &mut self,
        named: Option<winit::keyboard::NamedKey>,
        text: Option<&str>,
    ) {
        use winit::keyboard::NamedKey;
        const INPUT: horizon_client::clipboard::FieldId = ("remote.chat", 0);
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let Some(input) = r.chat_input.as_mut() else {
            return;
        };
        match named {
            Some(NamedKey::Escape) => {
                r.chat_input = None;
                self.capture(true);
            }
            Some(NamedKey::Backspace) => {
                if !self.text_selection.take(INPUT, input) {
                    input.pop();
                }
            }
            Some(NamedKey::Enter) => {
                let message = r.chat_input.take().unwrap_or_default();
                self.send_chat(&message);
                self.capture(true);
            }
            _ => {
                if let Some(text) = text {
                    self.text_selection.take(INPUT, input);
                    for c in text.chars().filter(|c| !c.is_control()) {
                        if input.chars().count() < 256 {
                            input.push(c);
                        }
                    }
                }
            }
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
        let (container, menu) = match self.remote.as_ref().and_then(|r| r.menu.clone()) {
            Some((id, name)) => (id, name),
            None => (0, "inventory".to_owned()),
        };
        let slot = (0..64).find(|&s| Self::menu_slot(&menu, s) == Some(internal));
        let Some(slot) = slot else { return };
        let shift = self.has_shift_down();
        let carried = self.to_network(&self.carried).map(|s| Hashed::of(&s));
        let state = self.remote.as_ref().map_or(0, |r| r.state_id);
        self.send(S::ContainerClick(play::ContainerClick {
            container,
            state,
            slot: slot as i16,
            button: i8::from(right),
            input: if shift { 1 } else { 0 },
            changed: Vec::new(),
            carried,
        }));
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
        }
        if self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == super::stations::Kind::Merchant)
        {
            self.station = None;
        }
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
                self.chat_line(format!("(Horizon cannot show {menu} screens yet)"));
                return;
            }
        };
        if let Some(r) = self.remote.as_mut() {
            r.menu = Some((p.container, menu.clone()));
        }
        self.craft_size = if menu == "minecraft:crafting" { 3 } else { 2 };
        self.container = None;
        if let Some(entity) = entity {
            let key = target.unwrap_or((i32::MIN, 0, 0));
            self.world.block_entities.insert(key, entity);
            self.container = Some(key);
        }
        self.screen = Screen::Inventory;
        self.capture(false);
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
        let atlas = &self.res.atlas;
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
    /// Chat (ChatComponent), the chat input line and the action bar.
    pub(super) fn remote_hud(&self, batch: &mut HudBatch) {
        let Some(r) = &self.remote else { return };
        let atlas = &self.res.atlas;
        let (w, h) = (batch.width, batch.height);
        let open = r.chat_input.is_some();
        let now = Instant::now();
        let mut y = h - 48.0;
        // Container screens cover the chat (vanilla draws them over the HUD).
        let shown = if matches!(self.screen, Screen::Playing | Screen::Dead) {
            if open { 20 } else { 10 }
        } else {
            0
        };
        for (text, at) in r.chat.iter().rev().take(shown) {
            let age = now.duration_since(*at);
            // Lines fade out after ten seconds unless the chat is open.
            let alpha = if open {
                1.0
            } else {
                (1.0 - (age.as_secs_f32() - 9.0).max(0.0)).clamp(0.0, 1.0)
            };
            if alpha <= 0.0 {
                break;
            }
            let a = (alpha * 255.0) as u8;
            batch.fill(
                atlas,
                2.0,
                y - 1.0,
                320.0,
                9.0,
                rgba(0, 0, 0, (alpha * 128.0) as u8),
            );
            batch.text(atlas, text, 4.0, y, rgba(255, 255, 255, a), true);
            y -= 9.0;
        }
        if let Some(input) = &r.chat_input {
            batch.fill(atlas, 2.0, h - 14.0, w - 4.0, 12.0, rgba(0, 0, 0, 128));
            let blink = (self.started.elapsed().as_millis() / 300).is_multiple_of(2);
            let shown = if blink {
                format!("{input}_")
            } else {
                input.clone()
            };
            batch.text(atlas, &shown, 4.0, h - 12.0, rgba(224, 224, 224, 255), true);
        }
        if let Some((text, at)) = &r.action_bar
            && now.duration_since(*at) < Duration::from_secs(3)
        {
            let tw = HudBatch::text_width(atlas, text);
            batch.text(atlas, text, (w - tw) / 2.0, h - 68.0, u32::MAX, true);
        }
        if self.screen == Screen::Dead
            && let Some(message) = &r.death_message
        {
            let tw = HudBatch::text_width(atlas, message);
            batch.text(atlas, message, (w - tw) / 2.0, 85.0, u32::MAX, true);
        }
    }
    /// Status for `horizon-ctl status`.
    pub(super) fn remote_status(&self) -> Option<serde_json::Value> {
        let r = self.remote.as_ref()?;
        Some(serde_json::json!({
            "address": horizon_client::net::shown_address(&r.session.address),
            "name": r.session.name,
            "joined": r.joined,
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
            "player_positions": r
                .entities
                .values()
                .filter(|t| t.kind == "minecraft:player")
                .map(|t| t.position)
                .collect::<Vec<_>>(),
            "health": r.health.0,
            "food": r.health.1,
            "decode_errors": r.decode_errors,
            "chat": r.chat.iter().rev().take(10).rev().map(|(t, _)| t.clone()).collect::<Vec<_>>(),
        }))
    }
}
