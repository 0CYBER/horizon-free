//! Hosting the open world on the local network ("Open to LAN"): starts the
//! server threads and the play-state hub, feeds it the world's entities each
//! tick and turns what the guests do into gameplay. A world that is not open
//! costs one `Option` check per tick.
use super::{Game, chat::PLAYER_NAME};
use horizon_client::{
    Result,
    items::ItemStack,
    lan::{
        self, Lan, MAX_PLAYERS, NetConfig,
        chunks::ChunkEncoder,
        data::SyncData,
        entity::EntityState,
        hub::{HostEvent, HostState, Hub, HubConfig, TickCtx},
        playerdata,
    },
    mobs::Mob,
    text::Text,
};
use horizon_core::geometry::Vec3;
use horizon_protocol::{
    crypto::offline_uuid,
    entity_data::{Value, serializer},
};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

/// The registries and tags are the same for every world: load them once.
static SYNC: OnceLock<std::result::Result<Arc<SyncData>, String>> = OnceLock::new();

/// The settings of the open world (WorldOptionsScreen's multiplayer section).
#[derive(Clone, Debug)]
pub struct LanSettings {
    pub port: u16,
    pub guest_commands: bool,
    pub force_game_mode: bool,
}

pub(super) struct LanHost {
    pub hub: Hub,
    pub settings: LanSettings,
    pub host_entity: i32,
    pub host_uuid: u128,
    /// The guests' models in the host's world.
    pub models: HashMap<u32, Mob>,
    host_mode: i32,
    pub play: HashMap<u32, super::lan_actions::GuestPlay>,
    /// Blocks broken this tick (their sound is a level event for the guests).
    pub broken: Vec<[f64; 3]>,
    /// Hurt and death state of the mobs guests see.
    mob_state: HashMap<u32, (i32, bool)>,
}

/// A stable UUID for an entity that has none.
fn entity_uuid(id: u32) -> u128 {
    0x4856_5a4e_0000_0000_0000_0000_0000_0000u128 | u128::from(id)
}

/// The SynchedEntityData index of the baby flag: AgeableMob.DATA_BABY_ID and
/// Zombie.DATA_BABY_ID are both the first entry after Mob's (16); Piglin's
/// comes after AbstractPiglin's zombification flag (17).
fn baby_index(kind: &str) -> Option<u8> {
    match kind {
        "minecraft:piglin" => Some(17),
        "minecraft:cow"
        | "minecraft:mooshroom"
        | "minecraft:pig"
        | "minecraft:sheep"
        | "minecraft:chicken"
        | "minecraft:horse"
        | "minecraft:donkey"
        | "minecraft:mule"
        | "minecraft:skeleton_horse"
        | "minecraft:zombie_horse"
        | "minecraft:llama"
        | "minecraft:trader_llama"
        | "minecraft:cat"
        | "minecraft:wolf"
        | "minecraft:fox"
        | "minecraft:rabbit"
        | "minecraft:panda"
        | "minecraft:polar_bear"
        | "minecraft:bee"
        | "minecraft:frog"
        | "minecraft:sniffer"
        | "minecraft:armadillo"
        | "minecraft:villager"
        | "minecraft:zombie"
        | "minecraft:husk"
        | "minecraft:drowned"
        | "minecraft:zombie_villager"
        | "minecraft:zombified_piglin"
        | "minecraft:hoglin"
        | "minecraft:goat"
        | "minecraft:ocelot"
        | "minecraft:turtle"
        | "minecraft:strider"
        | "minecraft:camel"
        | "minecraft:axolotl"
        | "minecraft:dolphin"
        | "minecraft:squid"
        | "minecraft:glow_squid" => Some(16),
        _ => None,
    }
}

/// A mob's synched data as ServerEntity sends it (only the values that differ
/// from the defaults the client constructs), with vanilla's entry indices:
/// Entity 0..=7, LivingEntity 8..=14, Mob 15, then the subclasses.
fn mob_meta(m: &Mob, hub: &Hub) -> Vec<(u8, i32, Value)> {
    let mut meta = Vec::new();
    let kind = m.kind.id.as_str();
    if m.fire_ticks > 0 {
        meta.push((0, serializer::BYTE, Value::Byte(1)));
    }
    if let Some(name) = &m.custom_name {
        meta.push((
            2,
            serializer::OPTIONAL_COMPONENT,
            Value::OptText(Some(horizon_protocol::text::literal(name))),
        ));
        if m.name_visible {
            meta.push((3, serializer::BOOLEAN, Value::Bool(true)));
        }
    }
    if m.health < m.kind.max_health {
        meta.push((9, serializer::FLOAT, Value::Float(m.health.max(0.0))));
    }
    if kind == "minecraft:rabbit" && m.no_ai {
        meta.push((15, serializer::BYTE, Value::Byte(1)));
    }
    if m.is_baby()
        && let Some(index) = baby_index(kind)
    {
        meta.push((index, serializer::BOOLEAN, Value::Bool(true)));
    }
    let e = &m.extra;
    let id = |registry: &str, name: &str| hub.registry_id(registry, &format!("minecraft:{name}"));
    match kind {
        // Sheep.DATA_WOOL_ID: the colour in the low nibble, 16 when sheared.
        "minecraft:sheep" => {
            let value = (e.color & 15) | if e.sheared { 16 } else { 0 };
            if value != 0 {
                meta.push((18, serializer::BYTE, Value::Byte(value as i8)));
            }
        }
        "minecraft:cat" | "minecraft:wolf" => {
            // TamableAnimal.DATA_FLAGS_ID: sitting 1, tame 4.
            let flags = i8::from(e.sitting) | if e.tame { 4 } else { 0 };
            if flags != 0 {
                meta.push((18, serializer::BYTE, Value::Byte(flags)));
            }
            let (index, registry, name) = if kind == "minecraft:cat" {
                let v = horizon_client::mobs::tame::CAT_VARIANTS[(e.variant as usize).min(10)];
                (20, "minecraft:cat_variant", v)
            } else {
                let v = horizon_client::mobs::tame::WOLF_VARIANTS[(e.variant as usize).min(8)].0;
                (23, "minecraft:wolf_variant", v)
            };
            let serializer = if index == 20 {
                serializer::CAT_VARIANT
            } else {
                serializer::WOLF_VARIANT
            };
            if let Some(v) = id(registry, name) {
                meta.push((index, serializer, Value::Id(v)));
            }
        }
        "minecraft:horse"
        | "minecraft:donkey"
        | "minecraft:mule"
        | "minecraft:llama"
        | "minecraft:trader_llama"
        | "minecraft:skeleton_horse"
        | "minecraft:zombie_horse"
        | "minecraft:camel" => {
            // AbstractHorse.DATA_ID_FLAGS: tame 2.
            if e.tame {
                meta.push((18, serializer::BYTE, Value::Byte(2)));
            }
            if kind == "minecraft:horse" {
                let v = i32::from(e.variant) | i32::from(e.variant2) << 8;
                if v != 0 {
                    meta.push((19, serializer::INT, Value::Int(v)));
                }
            } else if kind.ends_with("llama") {
                if e.chested {
                    meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
                }
                meta.push((
                    20,
                    serializer::INT,
                    Value::Int(i32::from(e.variant2.max(1))),
                ));
                if e.variant != 0 {
                    meta.push((21, serializer::INT, Value::Int(i32::from(e.variant))));
                }
            } else if (kind == "minecraft:donkey" || kind == "minecraft:mule") && e.chested {
                meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
            }
        }
        // Villager.DATA_VILLAGER_DATA and ZombieVillager's (after Zombie's three).
        "minecraft:villager" | "minecraft:zombie_villager" => {
            use horizon_client::mobs::render::{PROFESSIONS, VILLAGER_TYPES};
            let villager_type = VILLAGER_TYPES[e.variant as usize % VILLAGER_TYPES.len()];
            let profession = PROFESSIONS[e.variant2 as usize % PROFESSIONS.len()];
            if let (Some(t), Some(p)) = (
                id("minecraft:villager_type", villager_type),
                id("minecraft:villager_profession", profession),
            ) {
                let index = if kind == "minecraft:villager" { 19 } else { 20 };
                let level = i32::from(e.variant3.max(1));
                meta.push((
                    index,
                    serializer::VILLAGER_DATA,
                    Value::Ids(vec![t, p, level]),
                ));
            }
        }
        // AbstractCubeMob.ID_SIZE.
        "minecraft:slime" | "minecraft:magma_cube" => {
            if e.size > 1 {
                meta.push((18, serializer::INT, Value::Int(e.size)));
            }
        }
        // Creeper.DATA_SWELL_DIR, DATA_IS_POWERED and DATA_IS_IGNITED: the
        // guest swells the creeper from them. The direction always goes in,
        // as only changed entries are sent and it drops back to -1.
        "minecraft:creeper" => {
            meta.push((16, serializer::INT, Value::Int(e.counter)));
            if e.charged {
                meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
            }
            if e.flag {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
        }
        // Enderman.DATA_CARRY_STATE.
        "minecraft:enderman" => {
            if let Some(state) = e.carried {
                meta.push((
                    16,
                    serializer::OPTIONAL_BLOCK_STATE,
                    Value::Id(i32::from(state.0 as u16)),
                ));
            }
        }
        _ => {}
    }
    meta
}

/// The equipment a mob shows (ClientboundSetEquipmentPacket slots: main hand,
/// off hand, feet, legs, chest, head, body, saddle).
fn mob_equipment(m: &Mob, hub: &mut Hub) -> Vec<(u8, Option<horizon_protocol::item::Stack>)> {
    let e = &m.extra;
    let mut out = Vec::new();
    let slots = [
        (0u8, &e.main_hand),
        (1, &e.off_hand),
        (2, &e.feet),
        (3, &e.legs),
        (4, &e.chest),
        (5, &e.head),
    ];
    for (slot, item) in slots {
        if let Some(item) = item {
            let stack = ItemStack {
                item: item.clone(),
                count: 1,
                damage: 0,
                extra: None,
            };
            out.push((slot, hub.stack(&Some(stack))));
        }
    }
    if e.saddled {
        let stack = e.saddle_stack.clone().unwrap_or(ItemStack {
            item: Arc::from("minecraft:saddle"),
            count: 1,
            damage: 0,
            extra: None,
        });
        out.push((7, hub.stack(&Some(stack))));
    }
    out
}

pub const HELP: &[&str] = &[
    "lan open [PORT] [commands] [force]  open the world to the network (PORT 0 picks one)",
    "lan close                      stop hosting",
    "lan status                     the hosting state and the guests",
];

impl Game<'_> {
    /// `horizon-ctl lan ...`.
    pub(super) fn lan_control(&mut self, words: &[&str]) -> Result<Option<serde_json::Value>> {
        use serde_json::json;
        if words[0] != "lan" {
            return Ok(None);
        }
        match words.get(1).copied() {
            Some("open") => {
                if self.title.is_some() {
                    return Err("open a world first".into());
                }
                let port = words.get(2).map_or(Ok(0), |p| p.parse::<u16>())?;
                self.lan_open(LanSettings {
                    port: if port == 0 {
                        lan::available_port()
                    } else {
                        port
                    },
                    guest_commands: words.contains(&"commands"),
                    force_game_mode: words.contains(&"force"),
                })?;
                Ok(Some(json!({ "ok": true, "port": self.lan_port() })))
            }
            Some("close") => {
                self.lan_close(true);
                Ok(Some(json!({ "ok": true })))
            }
            Some("status") => Ok(Some(match &self.lan {
                Some(h) => json!({
                    "open": true,
                    "port": h.settings.port,
                    "guests": h.hub.guests.iter().map(|g| json!({
                        "name": g.name,
                        "position": g.position,
                        "placed": g.placed,
                        "loaded": g.loaded,
                        "game_mode": g.game_mode,
                        "chunks": g.sent_chunks.len(),
                        "tracked": g.tracker.len(),
                    })).collect::<Vec<_>>(),
                }),
                None => json!({ "open": false }),
            })),
            _ => Err("lan open|close|status".into()),
        }
    }

    pub(super) fn lan_open(&mut self, settings: LanSettings) -> Result<()> {
        self.lan_start(settings)?;
        let port = self.lan.as_ref().map_or(0, |l| l.settings.port);
        self.system_message(Text::translate(
            "menu.multiplayerOptions.publish.started.lan",
            &[Text::literal(port.to_string())],
        ));
        Ok(())
    }
    /// IntegratedServer.publishServer without the screen's announcement.
    pub(super) fn lan_start(&mut self, settings: LanSettings) -> Result<()> {
        if self.lan.is_some() || self.remote.is_some() || self.title.is_some() {
            return Err("already open".into());
        }
        let motd = format!("{PLAYER_NAME} - {}", self.level.name);
        let host_uuid = self.level.player_uuid.map_or_else(
            || offline_uuid(PLAYER_NAME),
            |u| {
                u128::from(u[0] as u32) << 96
                    | u128::from(u[1] as u32) << 64
                    | u128::from(u[2] as u32) << 32
                    | u128::from(u[3] as u32)
            },
        );
        let config = HubConfig {
            default_game_mode: self.level.mode(),
            guest_commands: settings.guest_commands,
            force_game_mode: settings.force_game_mode,
            hardcore: self.level.hardcore,
            view_distance: self.settings.render_distance.clamp(2, 32),
            simulation_distance: self.settings.render_distance.clamp(2, 32),
            dimension: self.dim.id().to_owned(),
            seed: self.level.seed,
            max_players: MAX_PLAYERS,
            dedicated: false,
        };
        let net = NetConfig::lan(settings.port, motd, MAX_PLAYERS);
        self.lan_host(settings, net, config, Some(host_uuid))
    }
    /// Start the network side and the play-state hub: for the LAN (with the
    /// host's own profile reserved) or for a dedicated server (no host).
    pub(super) fn lan_host(
        &mut self,
        settings: LanSettings,
        net: NetConfig,
        config: HubConfig,
        host: Option<u128>,
    ) -> Result<()> {
        let root = self.options.resources.clone();
        let sync = SYNC
            .get_or_init(|| SyncData::load(&root).map(Arc::new))
            .clone()?;
        let biome_names: Vec<String> = self
            .res
            .biomes
            .list
            .iter()
            .map(|b| b.name.clone())
            .collect();
        let registry: Vec<String> = sync
            .dynamic
            .registries
            .get("minecraft:worldgen/biome")
            .map(|v| v.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default();
        let mut encoder = ChunkEncoder::new(
            self.res
                .facts
                .states
                .iter()
                .map(|s| (s.name.clone(), s.fluid_level > 0 || s.waterlogged)),
            &sync.builtin,
            &biome_names,
            &registry,
        );
        encoder.tables = Some(
            self.save
                .tables
                .get_or_init(|| Arc::new(horizon_client::save::SaveTables::new(&self.res.assets)))
                .clone(),
        );
        let lan = Lan::start_with(sync.clone(), net)?;
        if let Some(uuid) = host {
            lan.reserve(uuid);
        }
        let host_uuid = host.unwrap_or(0);
        self.next_entity += 1;
        let host_entity = self.next_entity as i32;
        let hub = Hub::new(lan, sync, encoder, config);
        let port = hub.lan.port;
        self.world.block_log = Some(Vec::new());
        self.lan = Some(LanHost {
            hub,
            settings: LanSettings { port, ..settings },
            host_entity,
            host_uuid,
            models: HashMap::new(),
            host_mode: self.mode_id(),
            play: HashMap::new(),
            broken: Vec::new(),
            mob_state: HashMap::new(),
        });
        self.audio.capture = Some(Vec::new());
        Ok(())
    }

    /// IntegratedServer.unpublishServer: the guests are sent away.
    pub(super) fn lan_close(&mut self, announce: bool) {
        self.friends_lan_closed();
        let Some(mut host) = self.lan.take() else {
            return;
        };
        for g in &host.hub.guests {
            playerdata::save(g, &self.world_root);
        }
        host.hub.disconnect_all();
        // Give the writer threads a moment to flush the disconnect packets.
        std::thread::sleep(std::time::Duration::from_millis(100));
        self.world.block_log = None;
        self.audio.capture = None;
        self.lan_events.clear();
        self.streaming.extra.clear();
        if announce {
            self.system_message(Text::translate(
                "menu.multiplayerOptions.publish.stopped",
                &[],
            ));
        }
    }

    /// The world's default game mode (`defaultgamemode`).
    pub(super) fn set_default_game_mode(&mut self, mode: i32) {
        self.level.game_mode = mode;
        self.level.creative = mode == 1;
        if let Some(host) = self.lan.as_mut() {
            host.hub.config.default_game_mode = mode;
        }
    }

    /// New command access and game mode forcing settings of the open world.
    pub(super) fn lan_apply_settings(&mut self, guest_commands: bool, force: bool) {
        if let Some(host) = self.lan.as_mut() {
            host.settings.guest_commands = guest_commands;
            host.settings.force_game_mode = force;
            host.hub.config.guest_commands = guest_commands;
            host.hub.config.force_game_mode = force;
            host.hub.resend_commands();
        }
    }

    pub(super) fn lan_port(&self) -> Option<u16> {
        self.lan.as_ref().map(|h| h.settings.port)
    }

    /// The host as the guests see them.
    fn lan_host_state(&self, host: &LanHost) -> Option<HostState> {
        let p = self.player.as_ref()?;
        let mut equipment: [Option<ItemStack>; 6] = Default::default();
        equipment[0] = self.inventory.get(self.selected).cloned().flatten();
        equipment[1] = self.offhand.clone();
        // Feet, legs, chest, head are slots 2..=5; armor is stored head first.
        equipment[2] = self.armor[3].clone();
        equipment[3] = self.armor[2].clone();
        equipment[4] = self.armor[1].clone();
        equipment[5] = self.armor[0].clone();
        Some(HostState {
            uuid: host.host_uuid,
            name: PLAYER_NAME.to_owned(),
            entity_id: host.host_entity,
            position: [p.position.x, p.position.y, p.position.z],
            yaw: self.yaw,
            pitch: self.pitch,
            head_yaw: self.yaw,
            on_ground: p.ground.on_ground,
            game_mode: self.mode_id(),
            sneaking: p.pose == horizon_client::pose::Pose::Crouching,
            sprinting: p.sprinting,
            equipment,
            health: p.health.health(),
        })
    }

    /// Mobs and items as the entities guests track.
    fn lan_entities(&self, hub: &mut Hub) -> Vec<EntityState> {
        let mut out = Vec::with_capacity(self.mobs.len() + self.dropped.len());
        for m in self.mobs.iter().filter(|m| !m.removed) {
            let kind = m.kind.id.as_str();
            let meta = mob_meta(m, hub);
            let equipment = mob_equipment(m, hub);
            let (yaw, pitch) = (m.yaw, m.head_pitch);
            out.push(EntityState {
                id: m.id as i32,
                uuid: entity_uuid(m.id),
                kind: kind.to_owned(),
                position: [m.position.x, m.position.y, m.position.z],
                yaw,
                pitch,
                head_yaw: m.head_yaw,
                on_ground: m.on_ground,
                velocity: [m.velocity.x, m.velocity.y, m.velocity.z],
                data: 0,
                meta,
                equipment,
            });
        }
        for d in &self.dropped {
            let stack = hub.stack(&Some(d.stack.clone()));
            out.push(EntityState {
                id: d.id as i32,
                uuid: entity_uuid(d.id),
                kind: "minecraft:item".to_owned(),
                position: [d.position.x, d.position.y, d.position.z],
                yaw: 0.0,
                pitch: 0.0,
                head_yaw: 0.0,
                on_ground: d.on_ground,
                velocity: [d.velocity.x, d.velocity.y, d.velocity.z],
                data: 0,
                meta: vec![(8, serializer::ITEM_STACK, Value::Item(stack))],
                equipment: Vec::new(),
            });
        }
        out
    }

    /// Run the hub for this tick and apply what the guests did.
    pub(super) fn tick_lan(&mut self) {
        let Some(mut host) = self.lan.take() else {
            return;
        };
        let state = self.lan_host_state(&host);
        // A dedicated server runs the hub without a host player.
        if state.is_some() || host.hub.config.dedicated {
            let has_guests = !host.hub.guests.is_empty();
            let entities = if has_guests {
                self.lan_entities(&mut host.hub)
            } else {
                Vec::new()
            };
            let spawn = self.spawn.map_or([0.5, 80.0, 0.5], |s| [s.x, s.y, s.z]);
            let events = {
                let mut ctx = TickCtx {
                    world: &mut self.world,
                    host: state.as_ref(),
                    entities: &entities,
                    next_entity: &mut self.next_entity,
                    spawn,
                    difficulty: i32::from(self.ui.difficulty),
                    difficulty_locked: false,
                    clock_rate: if self.ui.rules.bool("advance_time") {
                        self.ui.clock.rate
                    } else {
                        0.0
                    },
                    tick_rate: self.clock.rate(),
                    tick_frozen: self.clock.frozen(),
                };
                host.hub.tick(&mut ctx)
            };
            if let Some(state) = &state
                && host.host_mode != state.game_mode
            {
                host.host_mode = state.game_mode;
                host.hub.set_host_game_mode(state.uuid, state.game_mode);
            }
            for event in events {
                match event {
                    HostEvent::Joined(conn) => {
                        self.lan_guest_joined(&mut host, conn, state.as_ref(), &entities, spawn);
                        if self.options.dedicated.is_some()
                            && let Some(g) = host.hub.guest(conn)
                        {
                            // PlayerList.placeNewPlayer, then the broadcast join message.
                            let [x, y, z] = g.position;
                            super::dedicated::log(
                                "INFO",
                                &format!(
                                    "{}[/{}] logged in with entity id {} at ({}, {}, {})",
                                    g.name,
                                    g.address,
                                    g.entity_id,
                                    super::dedicated::java_double(x),
                                    super::dedicated::java_double(y),
                                    super::dedicated::java_double(z)
                                ),
                            );
                            super::dedicated::log(
                                "INFO",
                                &format!("System chat: {} joined the game", g.name),
                            );
                        }
                    }
                    HostEvent::Left(guest) => {
                        playerdata::save(&guest, &self.world_root);
                        host.models.remove(&guest.conn);
                        if self.options.dedicated.is_some() && guest.placed {
                            super::dedicated::log(
                                "INFO",
                                &format!("{} lost connection: {}", guest.name, guest.left_reason),
                            );
                            super::dedicated::log(
                                "INFO",
                                &format!("System chat: {} left the game", guest.name),
                            );
                        }
                    }
                    // MinecraftServer.logChatMessage of an unsigned message.
                    HostEvent::Chat(line) if self.options.dedicated.is_some() => {
                        super::dedicated::log("INFO", &format!("[Not Secure] {line}"));
                    }
                    HostEvent::Chat(line) => self.system_message(Text::literal(line)),
                    HostEvent::Packet(conn, packet) => {
                        self.lan_guest_packet(&mut host, conn, *packet);
                    }
                }
            }
        }
        self.lan_flush_effects(&mut host);
        self.tick_guests(&mut host);
        self.lan_swings(&mut host);
        self.lan_update_streaming(&host);
        self.lan_update_models(&mut host);
        self.lan = Some(host);
    }

    fn lan_guest_joined(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        state: Option<&HostState>,
        entities: &[EntityState],
        spawn: [f64; 3],
    ) {
        let default_mode = host.hub.config.default_game_mode;
        let force = host.hub.config.force_game_mode;
        let root = self.world_root.clone();
        let Some(g) = host.hub.guest_mut(conn) else {
            return;
        };
        let known = playerdata::load(g, &root);
        if !known || force {
            g.game_mode = default_mode;
        }
        if !known {
            g.position = spawn;
            g.previous_position = spawn;
            g.health = 20.0;
            g.food = 20;
        }
        let mut ctx = TickCtx {
            world: &mut self.world,
            host: state,
            entities,
            next_entity: &mut self.next_entity,
            spawn,
            difficulty: i32::from(self.ui.difficulty),
            difficulty_locked: false,
            clock_rate: self.ui.clock.rate,
            tick_rate: self.clock.rate(),
            tick_frozen: self.clock.frozen(),
        };
        host.hub.place_new_player(conn, &mut ctx);
    }

    /// Level events the game queued, for the guests (`process_level_events`).
    pub(super) fn lan_record_events(&mut self, events: &[horizon_world::level::LevelEvent]) {
        if self.audio.capture.is_none() {
            return;
        }
        for e in events {
            if let horizon_world::level::LevelEvent::LevelEvent { id, pos, data } = e {
                // The acting guest does not predict the egg's teleport particles.
                let except = if *id == 2015 { None } else { self.lan_acting };
                self.lan_events
                    .push((*id, [pos.x, pos.y, pos.z], *data, except));
            }
        }
    }

    /// `Level.blockEvent` for the guests (`ClientboundBlockEventPacket`).
    pub(super) fn lan_block_event(&mut self, pos: [i32; 3], b0: i32, b1: i32) {
        if self.audio.capture.is_some() {
            let state = self.world.block(pos[0], pos[1], pos[2]).0 as u16;
            self.lan_block_events.push((pos, b0, b1, state));
        }
    }
    /// `Level.broadcastEntityEvent` for the guests.
    pub(super) fn lan_entity_event(&mut self, id: i32, event: i8, at: [f64; 3]) {
        if self.audio.capture.is_some() {
            self.lan_entity_events.push((id, event, at));
        }
    }
    /// Send the tick's level events, sounds and mob animations.
    fn lan_flush_effects(&mut self, host: &mut LanHost) {
        for (kind, pos, data, acting) in std::mem::take(&mut self.lan_events) {
            if kind == 2001 {
                host.broken
                    .push([f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])]);
            }
            host.hub.level_event(kind, pos, data, acting);
        }
        for (pos, b0, b1, state) in std::mem::take(&mut self.lan_block_events) {
            host.hub.block_event(pos, b0, b1, state);
        }
        for (id, event, at) in std::mem::take(&mut self.lan_entity_events) {
            host.hub.entity_event(id, event, at);
        }
        self.lan_broadcast_effects(host);
        if host.hub.guests.is_empty() {
            host.mob_state.clear();
            return;
        }
        // Hurt and death animations of mobs.
        let mut seen = HashMap::new();
        for m in &self.mobs {
            let dead = !m.alive();
            let old = host.mob_state.get(&m.id).copied();
            let at = [m.position.x, m.position.y, m.position.z];
            if m.hurt_time > old.map_or(0, |o| o.0) && !dead {
                host.hub.entity_event(m.id as i32, 2, at);
            }
            if dead && !old.is_some_and(|o| o.1) {
                host.hub.entity_event(m.id as i32, 3, at);
            }
            seen.insert(m.id, (m.hurt_time, dead));
        }
        host.mob_state = seen;
    }

    /// Keep the chunks around every guest loaded.
    fn lan_update_streaming(&mut self, host: &LanHost) {
        self.streaming.extra = host
            .hub
            .guests
            .iter()
            .filter(|g| g.placed)
            // The tracked square's corners and their lit neighbours lie
            // within (view distance + 2) * √2 of the player's chunk.
            .map(|g| (g.chunk(), (g.view_distance + 2) * 3 / 2 + 1))
            .collect();
    }

    /// The guests' models, for the host to see them.
    fn lan_update_models(&mut self, host: &mut LanHost) {
        let Some(kind) = self.res.player_type.clone() else {
            return;
        };
        host.models
            .retain(|conn, _| host.hub.guest(*conn).is_some_and(|g| g.placed));
        for g in host.hub.guests.iter().filter(|g| g.placed) {
            let at = Vec3::new(
                g.position[0],
                g.position[1] - if g.sneaking { 0.125 } else { 0.0 },
                g.position[2],
            );
            let m = host
                .models
                .entry(g.conn)
                .or_insert_with(|| Mob::new(g.entity_id as u32, kind.clone(), at, g.yaw));
            let (dx, dy, dz) = (
                at.x - m.position.x,
                at.y - m.position.y,
                at.z - m.position.z,
            );
            m.previous = m.position;
            m.position = at;
            if dx * dx + dy * dy + dz * dz > 100.0 {
                m.previous = at;
            }
            m.previous_yaw = m.yaw;
            m.previous_body_yaw = m.body_yaw;
            m.previous_head_yaw = m.head_yaw;
            m.previous_head_pitch = m.head_pitch;
            m.yaw = g.yaw;
            m.head_yaw = g.head_yaw;
            m.head_pitch = g.pitch;
            let mut target = m.body_yaw;
            if dx * dx + dz * dz > 0.002_500_000_2 {
                target = (dz.atan2(dx) as f32).to_degrees() - 90.0;
            }
            let wrap = |a: f32| (a + 180.0).rem_euclid(360.0) - 180.0;
            m.body_yaw += wrap(target - m.body_yaw) * 0.3;
            let diff = wrap(g.yaw - m.body_yaw);
            if diff.abs() > 50.0 {
                m.body_yaw += diff - diff.signum() * 50.0;
            }
            let dist = (dx * dx + dz * dz).sqrt() as f32;
            m.previous_limb_speed = m.limb_speed;
            m.limb_speed += ((dist * 4.0).min(1.0) - m.limb_speed) * 0.4;
            m.limb_position += m.limb_speed;
            m.age = m.age.wrapping_add(1);
            m.extra.main_hand = g
                .inventory
                .get(g.selected)
                .and_then(|s| s.as_ref())
                .map(|s| s.item.clone());
            m.extra.off_hand = g.offhand.as_ref().map(|s| s.item.clone());
            m.extra.crouching = g.sneaking;
        }
    }

    /// The guests' models for the entity pass.
    pub(super) fn guest_model_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<horizon_client::mesher::Vertex>,
    ) {
        let Some(host) = &self.lan else {
            return;
        };
        let atlas = &self.res.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        for m in host.models.values() {
            let light = self.world.light(
                m.position.x.floor() as i32,
                (m.position.y + 1.62).floor() as i32,
                m.position.z.floor() as i32,
            );
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            horizon_client::mobs::render::render_with(
                m,
                alpha as f32,
                camera,
                &lookup,
                packed,
                &self.res.mob_data,
                &self.player_armor_layers(),
                out,
            );
        }
    }

    /// A chat line the host typed, for the guests.
    pub(super) fn lan_host_chat(&mut self, message: &str) {
        if let Some(host) = &self.lan {
            host.hub.chat(PLAYER_NAME, message);
        }
    }
}
