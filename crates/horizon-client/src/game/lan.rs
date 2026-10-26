//! Hosting the open world on the local network ("Open to LAN"): starts the
//! server threads and the play-state hub, feeds it the world's entities each
//! tick and turns what the guests do into gameplay. A world that is not open
//! costs one `Option` check per tick.
use super::{Game, shield::OFFHAND_SLOT};
use horizon_client::{
    Result,
    items::ItemStack,
    lan::{
        self, Lan, MAX_PLAYERS, NetConfig,
        chunks::ChunkEncoder,
        data::SyncData,
        entity::{EntityState, States},
        hub::{HostEvent, HostState, Hub, HubConfig, TickCtx},
        playerdata,
    },
    mobs::{Mob, leash::Holder},
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
#[derive(Clone, Copy, Debug)]
pub struct LanSettings {
    pub port: u16,
    pub guest_commands: bool,
    pub force_game_mode: bool,
}

pub(super) struct LanHost {
    pub hub: Hub,
    pub settings: LanSettings,
    pub host_entity: i32,
    /// The host's swings the guests were told of (`Game::swings_started`).
    pub host_swings: u32,
    /// The host's crack the guests were told of: the block and its stage.
    pub host_crack: Option<([i32; 3], i32)>,
    pub host_uuid: u128,
    /// The host's profile name (the account's, or the offline name).
    pub host_name: String,
    /// The guests' models in the host's world.
    pub models: HashMap<u32, Mob>,
    host_mode: i32,
    pub play: HashMap<u32, super::lan_actions::GuestPlay>,
    /// Blocks broken this tick (their sound is a level event for the guests).
    pub broken: Vec<[f64; 3]>,
    /// Hurt time, death and swing time (while swinging) of the mobs guests see.
    mob_state: HashMap<super::Dim, HashMap<u32, (i32, bool, Option<i32>)>>,
    /// The guest each mob sees as its player, when not the host (game/lan_mobs.rs).
    pub mob_players: HashMap<u32, u32>,
    /// An integrated server's owner (game/integrated.rs).
    pub owner: Option<Owner>,
    /// The players' locator bars, one waypoint manager per level by
    /// dimension id (game/lan_waypoints.rs), and the guests who are dead,
    /// whose respawn replaces their waypoint.
    pub waypoints: std::collections::HashMap<String, lan::waypoints::Waypoints>,
    pub waypoint_dead: std::collections::HashSet<u128>,
    /// The world border as the guests were last told of it (lan/border.rs).
    pub border_sent: Option<lan::border::Sent>,
    /// Guests' commands waiting for the hub to be back in `self.lan`, where
    /// a command can reach every player (game/lan_actions.rs).
    pub commands: Vec<super::lan_actions::GuestCommand>,
    /// The entities the guests track, described again every tick
    /// (`lan_entities`) over last tick's states.
    states: lan::entity::States,
}

/// The player who owns an integrated server's world: a guest whose state is
/// the world's own player (`players/data/<singleplayer_uuid>.dat`), as in
/// single player.
pub(super) struct Owner {
    pub uuid: u128,
    /// The world's player as loaded, until the owner joins.
    pub saved: Option<Box<horizon_client::save::Level>>,
}

/// A stable UUID for an entity that has none.
pub(super) fn entity_uuid(id: u32) -> u128 {
    0x4856_5a4e_0000_0000_0000_0000_0000_0000u128 | u128::from(id)
}

/// The SynchedEntityData index of the baby flag: AgeableMob.DATA_BABY_ID and
/// Zombie.DATA_BABY_ID are both the first entry after Mob's (16); Piglin's
/// comes after AbstractPiglin's zombification flag (17).
pub(super) fn baby_index(kind: &str) -> Option<u8> {
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
/// `defaults` gets the defaults of the entries that can go back to theirs.
fn mob_meta(
    m: &Mob,
    hub: &Hub,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
) {
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
    // LivingEntity.DATA_LIVING_ENTITY_FLAGS: using an item 1, in the off
    // hand 2 (a skeleton draws its bow, a witch raises her potion).
    let using = i8::from(m.extra.using_item > 0) | i8::from(m.extra.using_off_hand) << 1;
    if using != 0 {
        meta.push((8, serializer::BYTE, Value::Byte(using)));
    }
    defaults.push((8, serializer::BYTE, Value::Byte(0)));
    // LivingEntity.DATA_HEALTH_ID: defined as 1, then set to the max health.
    let health = m.health.max(0.0);
    if health != 1.0 {
        meta.push((9, serializer::FLOAT, Value::Float(health)));
    }
    defaults.push((9, serializer::FLOAT, Value::Float(1.0)));
    // Mob.DATA_MOB_FLAGS_ID: no AI 1, left-handed 2, aggressive 4 (what
    // draws a vindicator's axe, a skeleton's drawn bow, a piglin's raised
    // weapon).
    let flags =
        i8::from(m.no_ai) | i8::from(m.extra.left_handed) << 1 | i8::from(m.ai.aggressive) << 2;
    if flags != 0 {
        meta.push((15, serializer::BYTE, Value::Byte(flags)));
    }
    defaults.push((15, serializer::BYTE, Value::Byte(0)));
    // AgeableMob.DATA_BABY_ID (and Zombie's, Piglin's).
    if let Some(index) = baby_index(kind) {
        if m.is_baby() {
            meta.push((index, serializer::BOOLEAN, Value::Bool(true)));
        }
        defaults.push((index, serializer::BOOLEAN, Value::Bool(false)));
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
            defaults.push((18, serializer::BYTE, Value::Byte(0)));
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
            // Wolf.DATA_COLLAR_COLOR and Cat's, red by default.
            let collar = if kind == "minecraft:cat" { 23 } else { 21 };
            let red = i32::from(horizon_client::mobs::tame::RED);
            if i32::from(e.color & 15) != red {
                meta.push((collar, serializer::INT, Value::Int(i32::from(e.color & 15))));
            }
            defaults.push((collar, serializer::INT, Value::Int(red)));
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
        // Pillager.IS_CHARGING_CROSSBOW: the charge pose and the held
        // item's loading animation.
        "minecraft:pillager" => {
            if e.flag {
                meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((17, serializer::BOOLEAN, Value::Bool(false)));
        }
        // SpellcasterIllager.DATA_SPELL_CASTING_ID: the casting pose.
        "minecraft:evoker" | "minecraft:illusioner" => {
            if e.variant2 != 0 {
                meta.push((17, serializer::BYTE, Value::Byte(e.variant2 as i8)));
            }
            defaults.push((17, serializer::BYTE, Value::Byte(0)));
        }
        // Enderman.DATA_CARRY_STATE.
        "minecraft:enderman" => {
            if let Some(state) = e.carried {
                meta.push((
                    16,
                    serializer::OPTIONAL_BLOCK_STATE,
                    Value::Id(state.0 as i32),
                ));
            }
        }
        _ => {}
    }
}

/// The equipment a mob shows (ClientboundSetEquipmentPacket slots: main hand,
/// off hand, feet, legs, chest, head, body, saddle).
fn mob_equipment(
    m: &Mob,
    hub: &mut Hub,
    out: &mut Vec<(u8, Option<horizon_protocol::item::Stack>)>,
) {
    let e = &m.extra;
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
        if let Some(server) = &self.integrated
            && words.get(1) == Some(&"status")
        {
            // The hub is the server thread's.
            let status = server
                .call(|g| g.lan_status())
                .ok_or("the integrated server stopped")?;
            return Ok(Some(status));
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
            Some("status") => Ok(Some(self.lan_status())),
            _ => Err("lan open|close|status".into()),
        }
    }

    /// `horizon-ctl lan status`.
    fn lan_status(&self) -> serde_json::Value {
        use serde_json::json;
        match &self.lan {
            Some(h) if !self.serving() || self.published_port().is_some() => json!({
                "open": true,
                "host": h.host_name,
                "port": h.settings.port,
                // The server's own world: its finished chunks and how many it
                // has generated or loaded so far.
                "world": {
                    "chunks": self.world.chunks.len(),
                    "generated": self.streaming.generated_total,
                },
                "guests": h.hub.guests.iter().map(|g| json!({
                    "name": g.name,
                    "position": g.position,
                    "placed": g.placed,
                    "loaded": g.loaded,
                    "game_mode": g.game_mode,
                    // ItemCooldowns: the ticks the shield an axe disabled stays down.
                    "shield_cooldown": h.play.get(&g.conn).map_or(0, |p| p.shield_cooldown),
                    // The item the server has this guest using: its slot and ticks.
                    "using": h.play.get(&g.conn).and_then(|p| p.using())
                        .map(|(slot, ticks)| json!({ "slot": slot, "ticks": ticks })),
                    "sneaking": g.sneaking,
                    // The server's Elytra glide (shared flag 7).
                    "fall_flying": g.fall_flying,
                    "sleeping": g.bed,
                    "health": g.health,
                    // The server's body: its air supply and golden hearts.
                    "air": g.motion.as_ref().map(|m| m.air),
                    "absorption": g.motion.as_ref().map(|m| m.health.absorption()),
                    // FoodData: food level, saturation and exhaustion.
                    "food": [
                        f64::from(g.food),
                        f64::from(g.saturation),
                        g.motion.as_ref().map_or(0.0, |m| f64::from(m.health.food.exhaustion)),
                    ],
                    "chunks": g.sent_chunks.len(),
                    "tracked": g.tracker.len(),
                    // Whether the host draws this guest lying in a bed.
                    "model_lying": h.models.get(&g.conn).map(|m| m.extra.ints[1] != 0),
                    // The armour the host draws on this guest, head to feet.
                    "model_armor": h.models.get(&g.conn).map(|m| {
                        [&m.extra.head, &m.extra.chest, &m.extra.legs, &m.extra.feet]
                            .map(|s| s.as_deref())
                    }),
                    // The host draws this guest's arm swinging.
                    "model_swinging": h.models.get(&g.conn).map(|m| m.ai.swinging),
                    // For how many ticks the host draws this guest using an item.
                    "model_using": h.models.get(&g.conn).map(|m| m.extra.using_item),
                })).collect::<Vec<_>>(),
            }),
            _ => json!({ "open": false }),
        }
    }

    pub(super) fn lan_open(&mut self, settings: LanSettings) -> Result<()> {
        let port = if let Some(server) = &self.integrated {
            // ShareToLanScreen: IntegratedServer.publishServer.
            server
                .call(move |g| g.publish(settings))
                .ok_or("the integrated server stopped")??
        } else if self.serving() {
            return self.publish(settings).map(drop).map_err(Into::into);
        } else {
            self.lan_start(settings)?;
            self.lan.as_ref().map_or(0, |l| l.settings.port)
        };
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
        // IntegratedServer.publishServer: the MOTD names the host's profile.
        let name = self.options.player_name();
        let motd = format!("{name} - {}", self.level.name);
        let host_uuid = self.level.player_uuid.map_or_else(
            || offline_uuid(&name),
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
            integrated: false,
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
            .biomes()
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
                .facts()
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
                .get_or_init(|| Arc::new(horizon_client::save::SaveTables::new(&self.pack.assets)))
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
        self.world.light_log = Some(Default::default());
        self.lan = Some(LanHost {
            hub,
            settings: LanSettings { port, ..settings },
            host_entity,
            host_swings: self.swings_started,
            host_crack: None,
            host_uuid,
            host_name: self.options.player_name(),
            models: HashMap::new(),
            host_mode: self.mode_id(),
            play: HashMap::new(),
            broken: Vec::new(),
            mob_state: HashMap::new(),
            mob_players: HashMap::new(),
            owner: None,
            waypoints: Default::default(),
            waypoint_dead: Default::default(),
            border_sent: None,
            commands: Vec::new(),
            states: Default::default(),
        });
        self.audio.capture = Some(Vec::new());
        Ok(())
    }

    /// IntegratedServer.unpublishServer: the guests are sent away.
    pub(super) fn lan_close(&mut self, announce: bool) {
        if self.integrated.is_some() || self.serving() {
            let closed = match &self.integrated {
                Some(server) => server.call(|g| g.unpublish()).unwrap_or(false),
                None => self.unpublish(),
            };
            if closed {
                self.friends_lan_closed();
            }
            if closed && announce && self.integrated.is_some() {
                self.system_message(Text::translate(
                    "menu.multiplayerOptions.publish.stopped",
                    &[],
                ));
            }
            return;
        }
        self.friends_lan_closed();
        let Some(mut host) = self.lan.take() else {
            return;
        };
        for g in &host.hub.guests {
            self.save_guest(&host, g);
        }
        host.hub.disconnect_all();
        // Give the writer threads a moment to flush the disconnect packets.
        std::thread::sleep(std::time::Duration::from_millis(100));
        self.world.block_log = None;
        self.world.light_log = None;
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

    /// Save a guest when they leave or the world closes: an integrated
    /// server's owner with the world (`save_game`), anyone else in their file.
    pub(super) fn save_guest(&mut self, host: &LanHost, guest: &horizon_client::lan::guest::Guest) {
        if host.owner.as_ref().is_some_and(|o| o.uuid == guest.uuid) {
            if guest.placed {
                playerdata::store(guest, &mut self.level);
            }
        } else {
            playerdata::save(guest, &self.world_root);
        }
        self.save_guest_records(host, guest);
    }
    /// The file name of a player's advancements and statistics: an
    /// integrated server's owner keeps single player's.
    fn records_uuid(&self, host: &LanHost, uuid: u128) -> Option<[i32; 4]> {
        if host.owner.as_ref().is_some_and(|o| o.uuid == uuid) {
            self.level.player_uuid
        } else {
            Some(playerdata::uuid_ints(uuid))
        }
    }
    /// `PlayerAdvancements.save` and `ServerStatsCounter.save` of a network player.
    pub(super) fn save_guest_records(
        &self,
        host: &LanHost,
        guest: &horizon_client::lan::guest::Guest,
    ) {
        if !guest.placed || self.adv.tree.list.is_empty() {
            return;
        }
        let Some(uuid) = self.records_uuid(host, guest.uuid) else {
            return;
        };
        let path = super::stats::player_json(&self.world_root, "advancements", uuid);
        let json = guest
            .advancements
            .to_json(&self.adv.tree, horizon_world::DATA_VERSION);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&json) {
            let _ = std::fs::write(path, text);
        }
        if let Some(play) = host.play.get(&guest.conn) {
            play.stats
                .write(&super::stats::player_json(&self.world_root, "stats", uuid));
        }
    }
    /// `PlayerAdvancements.load` and `ServerStatsCounter.parseLocal` of a
    /// network player who joined; every statistic goes out on the first
    /// request (`PlayerList.placeNewPlayer`'s `markAllDirty`).
    fn load_guest_records(&mut self, host: &mut LanHost, conn: u32) {
        let Some(uuid) = host
            .hub
            .guest(conn)
            .and_then(|g| self.records_uuid(host, g.uuid))
        else {
            return;
        };
        let tree = self.advancement_tree();
        let path = super::stats::player_json(&self.world_root, "advancements", uuid);
        let saved = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        if let Some(g) = host.hub.guest_mut(conn)
            && let Some(json) = saved
        {
            let mut progress = horizon_client::advancements::Progress::from_json(&json);
            // PlayerAdvancements.applyFrom: advancements this server lacks are dropped.
            progress
                .criteria
                .retain(|id, _| tree.by_id.contains_key(id));
            g.advancements = progress;
            g.advancements_loaded = true;
        }
        let mut stats =
            super::stats::Stats::read(&super::stats::player_json(&self.world_root, "stats", uuid));
        stats.mark_all_dirty();
        host.play.entry(conn).or_default().stats = stats;
    }
    /// The integrated server's owner into the world's player, for `Level::write`.
    pub(super) fn store_owner(&mut self) {
        let Some(host) = &self.lan else {
            return;
        };
        let Some(uuid) = host.owner.as_ref().map(|o| o.uuid) else {
            return;
        };
        if let Some(g) = host.hub.guests.iter().find(|g| g.uuid == uuid && g.placed) {
            playerdata::store(g, &mut self.level);
            self.save_guest_records(host, g);
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
        if let Some(server) = &self.integrated {
            server.call(move |g| g.lan_apply_settings(guest_commands, force));
            return;
        }
        // An integrated server gives guests commands only where the world
        // allows them (IntegratedServer.getCustomPermissionLevel).
        let effective = guest_commands && (!self.serving() || self.allow_commands());
        if let Some(host) = self.lan.as_mut() {
            host.settings.guest_commands = guest_commands;
            host.settings.force_game_mode = force;
            host.hub.config.guest_commands = effective;
            host.hub.config.force_game_mode = force;
            host.hub.resend_commands();
        }
    }

    pub(super) fn lan_port(&self) -> Option<u16> {
        if let Some(server) = &self.integrated {
            return server.published();
        }
        if self.serving() {
            return self.published_port();
        }
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
            name: host.host_name.clone(),
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
            attributes: horizon_client::lan::attributes::syncable(&p.attributes),
            // Bit 4: Riptide's spin (LivingEntity.isAutoSpinAttack).
            living_flags: living_flags(self.using)
                | if p.auto_spin_attack_ticks > 0 { 4 } else { 0 },
            bed: self.sleeping,
        })
    }

    /// The entities guests track: mobs, items and the rest (game/lan_entities.rs).
    /// `mob_players` is the guest each mob sees as its player (game/lan_mobs.rs).
    fn lan_entities(
        &self,
        host_entity: Option<i32>,
        mob_players: &HashMap<u32, u32>,
        hub: &mut Hub,
        out: &mut States,
    ) {
        out.clear();
        for m in self.mobs.iter().filter(|m| !m.removed) {
            // The lead's holder: the player the mob sees (a guest, else the
            // host) or another mob. A fence knot isn't an entity here.
            let leash = m.extra.leash.and_then(|h| match h {
                Holder::Player => mob_players
                    .get(&m.id)
                    .and_then(|conn| hub.guest(*conn))
                    .map(|g| g.entity_id)
                    .or(host_entity),
                Holder::Mob(id) => Some(id as i32),
                Holder::Knot(_) => None,
            });
            let e = out.add(m.id as i32, entity_uuid(m.id), m.kind.id.as_str());
            e.position = [m.position.x, m.position.y, m.position.z];
            (e.yaw, e.pitch, e.head_yaw) = (m.yaw, m.head_pitch, m.head_yaw);
            e.on_ground = m.on_ground;
            e.velocity = [m.velocity.x, m.velocity.y, m.velocity.z];
            e.needs_sync = m.needs_sync;
            mob_meta(m, hub, &mut e.meta, &mut e.defaults);
            mob_equipment(m, hub, &mut e.equipment);
            self.mob_attributes(m, &mut e.attributes);
            e.leash = leash;
        }
        for d in &self.dropped {
            let stack = hub.stack(&Some(d.stack.clone()));
            let e = out.add(d.id as i32, entity_uuid(d.id), "minecraft:item");
            e.position = [d.position.x, d.position.y, d.position.z];
            e.on_ground = d.on_ground;
            e.velocity = [d.velocity.x, d.velocity.y, d.velocity.z];
            e.meta.push((8, serializer::ITEM_STACK, Value::Item(stack)));
        }
        for o in &self.orbs {
            let e = out.add(o.id as i32, entity_uuid(o.id), "minecraft:experience_orb");
            e.position = [o.position.x, o.position.y, o.position.z];
            e.on_ground = o.on_ground;
            e.velocity = [o.velocity.x, o.velocity.y, o.velocity.z];
            // ExperienceOrb.DATA_VALUE.
            e.meta
                .push((8, serializer::INT, Value::Int(o.value as i32)));
        }
        self.lan_object_states(host_entity, hub, out);
    }

    /// Run the hub for this tick and apply what the guests did. While the
    /// single player game is `paused` only the connections run
    /// (`IntegratedServer.tickPaused`): `ServerGamePacketListenerImpl.tick`
    /// skips `tickPlayer`, so no player's air, hunger or damage moves on.
    pub(super) fn tick_lan(&mut self, paused: bool) {
        let Some(mut host) = self.lan.take() else {
            return;
        };
        // What the tick sends a guest goes to its connection at the end.
        let holding = lan::Holding::start();
        // The host's own commands' (and command blocks') players moving
        // to another level, and their broadcastToAdmins lines.
        if !self.level_teleports.is_empty() {
            self.run_level_teleports(&mut host);
        }
        if !self.admin_feedback.is_empty() {
            self.send_admin_feedback(&mut host, None);
        }
        let state = self.lan_host_state(&host);
        // An animal whose rider left the game (or the level) stops taking
        // its input.
        for m in &mut self.mobs {
            if m.rider.is_some()
                && self.riding != Some(m.id)
                && !host.hub.guests.iter().any(|g| g.vehicle == Some(m.id))
            {
                m.rider = None;
            }
        }
        // A dedicated server runs the hub without a host player.
        if state.is_some() || host.hub.config.dedicated {
            let has_guests = !host.hub.guests.is_empty();
            let mut states = std::mem::take(&mut host.states);
            if has_guests {
                let host_entity = state.as_ref().map(|s| s.entity_id);
                self.lan_assign_ids();
                let guests: Vec<(u32, i32)> = host
                    .hub
                    .guests
                    .iter()
                    .filter_map(|g| Some((g.vehicle?, g.entity_id)))
                    .collect();
                self.lan_riding(host_entity, &guests, &mut host.hub.riding);
                self.lan_entities(host_entity, &host.mob_players, &mut host.hub, &mut states);
                self.lan_hook_states(host_entity, &host.play, &host.hub, &mut states);
            } else {
                states.clear();
            }
            // ServerEntity.sendChanges clears needsSync once it has looked.
            for m in &mut self.mobs {
                m.needs_sync = false;
            }
            let entities = states.as_slice();
            let spawn = self.spawn.map_or([0.5, 80.0, 0.5], |s| [s.x, s.y, s.z]);
            let events = {
                let clock_rate = self.network_clock_rate();
                let mut ctx = TickCtx {
                    world: &mut self.world,
                    host: state.as_ref(),
                    entities,
                    next_entity: &mut self.next_entity,
                    spawn,
                    difficulty: i32::from(self.ui.difficulty),
                    difficulty_locked: self.level.difficulty_locked,
                    clock_rate,
                    tick_rate: self.clock.rate(),
                    tick_frozen: self.clock.frozen(),
                    tick_steps: self.clock.step_state(),
                    collision: Some(self.collision),
                    border: &self.border,
                };
                host.hub.movement_checks = (
                    self.ui.rules.bool("player_movement_check"),
                    self.ui.rules.bool("elytra_movement_check"),
                );
                // A server with several levels runs a pass for each
                // (`tick_server`): the network is read once, before the first.
                let pass = self.level_pass;
                let mut events = if pass.first {
                    host.hub.poll(&mut ctx)
                } else {
                    Vec::new()
                };
                events.extend(host.hub.tick_level(&mut ctx));
                if pass.last {
                    host.hub.end_tick();
                }
                events
            };
            if let Some(state) = &state
                && host.host_mode != state.game_mode
            {
                host.host_mode = state.game_mode;
                host.hub.set_host_game_mode(state.uuid, state.game_mode);
            }
            for event in events {
                match event {
                    event @ (HostEvent::Chat { .. } | HostEvent::Warn(_) | HostEvent::Error(_)) => {
                        self.lan_log_event(event);
                    }
                    HostEvent::Joined(conn) => {
                        self.lan_guest_joined(&mut host, conn, state.as_ref(), entities, spawn);
                        if self.options.dedicated.is_some()
                            && let Some(g) = host.hub.guest(conn)
                        {
                            // PlayerList.placeNewPlayer, then the broadcast join message.
                            super::dedicated::player_joined(g.uuid, &g.name);
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
                    HostEvent::Left(mut guest) => {
                        // The container they had open loses an opener.
                        if let Some(pos) = guest.menu.as_ref().and_then(|m| m.pos)
                            && !guest.spectator()
                        {
                            // In the level they were in.
                            let back = self.dim;
                            let theirs = super::Dim::from_id(&guest.dimension);
                            if self.enter_level(Some(&mut host), theirs).is_ok() {
                                self.container_openers_changed(pos, false);
                            }
                            if let Err(e) = self.enter_level(Some(&mut host), back) {
                                log::error!("could not reopen {}: {e}", back.id());
                            }
                        }
                        self.lan_crafting_left(&mut host, &mut guest);
                        self.save_guest(&host, &guest);
                        host.models.remove(&guest.conn);
                        host.play.remove(&guest.conn);
                        self.guest_combat.remove(&guest.conn);
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
                    HostEvent::Packet(conn, packet) => {
                        self.lan_guest_packet(&mut host, conn, *packet);
                        if !host.commands.is_empty() {
                            self.lan = Some(host);
                            self.run_guest_commands();
                            host = self.lan.take().expect("the hub is back after the commands");
                        }
                    }
                }
            }
            host.states = states;
        }
        self.lan_flush_effects(&mut host);
        if !paused {
            self.tick_guests(&mut host);
        }
        self.lan_living_flags(&mut host);
        self.tick_server_travel(&mut host);
        self.lan_swings(&mut host);
        self.lan_update_streaming(&host);
        self.lan_update_models(&mut host);
        self.lan_update_waypoints(&mut host, state.as_ref());
        self.lan_sync_scoreboard(&mut host);
        self.lan_sync_border(&mut host);
        drop(holding);
        lan::flush_all(host.hub.guests.iter().map(|g| &g.sender));
        self.lan = Some(host);
        // A mace's wind burst from a guest's attack.
        let _ = self.flush_wind_bursts();
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
        let in_place = host.hub.config.dimension.clone();
        let owner = host.owner.as_mut();
        let Some(g) = host.hub.guest_mut(conn) else {
            return;
        };
        let saved = owner
            .filter(|o| o.uuid == g.uuid)
            .and_then(|o| o.saved.take());
        let known = match saved {
            Some(level) => {
                playerdata::apply(g, &level);
                level.position.is_some()
            }
            None => playerdata::load(g, &root),
        };
        if !known || force {
            g.game_mode = default_mode;
        }
        if !known {
            g.position = spawn;
            g.previous_position = spawn;
            g.health = 20.0;
            g.food = 20;
            g.dimension = super::Dim::Overworld.id().to_owned();
        }
        // A server places each player in their own level
        // (`PlayerList.placeNewPlayer`); the in-frame game has one.
        let back = self.dim;
        let level = super::Dim::from_id(&g.dimension);
        let elsewhere = self.options.dedicated.is_some() && level != back;
        if !elsewhere {
            g.dimension = in_place;
        }
        self.load_guest_records(host, conn);
        let mut theirs = States::default();
        let entities = if elsewhere {
            if let Err(e) = self.enter_level(Some(host), level) {
                log::error!("could not open {}: {e}", level.id());
                return;
            }
            // A dedicated server has no host player.
            self.lan_entities(None, &host.mob_players, &mut host.hub, &mut theirs);
            self.lan_hook_states(None, &host.play, &host.hub, &mut theirs);
            theirs.as_slice()
        } else {
            entities
        };
        let clock_rate = self.network_clock_rate();
        let mut ctx = TickCtx {
            world: &mut self.world,
            host: state,
            entities,
            next_entity: &mut self.next_entity,
            spawn,
            difficulty: i32::from(self.ui.difficulty),
            difficulty_locked: self.level.difficulty_locked,
            clock_rate,
            tick_rate: self.clock.rate(),
            tick_frozen: self.clock.frozen(),
            tick_steps: self.clock.step_state(),
            collision: Some(self.collision),
            border: &self.border,
        };
        host.hub.place_new_player(conn, &mut ctx);
        if elsewhere && let Err(e) = self.enter_level(Some(host), back) {
            log::error!("could not reopen {}: {e}", back.id());
        }
    }

    /// Level events the game queued, for the guests (`process_level_events`).
    pub(super) fn lan_record_events(&mut self, events: &[horizon_world::level::LevelEvent]) {
        if self.audio.capture.is_none() {
            return;
        }
        for e in events {
            if let horizon_world::level::LevelEvent::LevelEvent { id, pos, data } = e {
                // The acting guest does not predict the egg's teleport
                // particles, bone meal's growth, or anvil result sounds
                // (these levelEvent calls name no excluded player).
                let except = if matches!(*id, 2015 | 1505 | 1029 | 1030) {
                    None
                } else {
                    self.lan_acting
                };
                self.lan_events
                    .push((*id, [pos.x, pos.y, pos.z], *data, except));
            }
        }
    }

    /// `Level.blockEvent` for the guests (`ClientboundBlockEventPacket`).
    pub(super) fn lan_block_event(&mut self, pos: [i32; 3], b0: i32, b1: i32) {
        if self.audio.capture.is_some() {
            let state = self.world.block(pos[0], pos[1], pos[2]).0;
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
        for (kind, pos, data) in std::mem::take(&mut self.lan_global_events) {
            host.hub.global_level_event(kind, pos, data);
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
            let old = host
                .mob_state
                .get(&self.dim)
                .and_then(|s| s.get(&m.id))
                .copied();
            let at = [m.position.x, m.position.y, m.position.z];
            // LivingEntity.hurtServer: a hurt that restarted the hurt time.
            if m.hurt_time > old.map_or(0, |o| o.0)
                && !dead
                && let Some((source, _)) = m.last_damage
            {
                use horizon_client::mobs::{DamageKind as K, Who};
                let player = source.attacker == Some(Who::Player);
                let kind = super::creatures::damage_type(source.kind, player);
                // The player a mob sees is the guest acting, else the host.
                let cause = source.attacker.map(|who| match who {
                    Who::Player => host
                        .mob_players
                        .get(&m.id)
                        .and_then(|conn| host.hub.guest(*conn))
                        .map_or(host.host_entity, |g| g.entity_id),
                    Who::Mob(id) => id as i32,
                });
                // A projectile is the direct entity of its hit: not tracked here.
                let melee = matches!(
                    source.kind,
                    K::PlayerAttack | K::MaceSmash | K::MobAttack | K::Sting | K::SonicBoom
                );
                let direct = cause.filter(|_| melee);
                host.hub.damage_event(m.id as i32, kind, cause, direct, at);
            }
            if dead && !old.is_some_and(|o| o.1) {
                host.hub.entity_event(m.id as i32, 3, at);
            }
            // LivingEntity.swing on a server (a melee hit, a thrown potion):
            // ClientboundSwingAnimationPacket for the players around. A swing
            // started since the last flush, or restarted (its time went back).
            let swing = m.ai.swinging.then_some(m.ai.swing_time);
            if let Some(time) = swing
                && !dead
                && old.and_then(|o| o.2).is_none_or(|before| time < before)
            {
                host.hub.swing(m.id as i32, 0, at, None);
            }
            seen.insert(m.id, (m.hurt_time, dead, swing));
        }
        host.mob_state.insert(self.dim, seen);
    }

    /// Keep the chunks around every guest loaded.
    fn lan_update_streaming(&mut self, host: &LanHost) {
        // DistanceManager.PlayerTicketTracker: every chunk within the server's
        // view distance (Chebyshev) of a player holds a PLAYER_LOADING ticket
        // at the entity-ticking level (31), whatever the player asked for;
        // levels rise by one a chunk, so FULL (33) reaches two chunks past
        // it. An extra area finishes its chunks out to its radius + 1. That
        // covers what the player tracks (`ChunkTrackingView`, within its view
        // distance + 1) and those chunks' lit neighbours.
        let radius = host.hub.config.view_distance + 1;
        self.streaming.extra = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g))
            .map(|g| (g.chunk(), radius))
            .collect();
    }

    /// The guests' models, for the host to see them.
    fn lan_update_models(&mut self, host: &mut LanHost) {
        let Some(kind) = self.res.player_type().clone() else {
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
            [m.extra.head, m.extra.chest, m.extra.legs, m.extra.feet] = g
                .armor
                .each_ref()
                .map(|s| s.as_ref().map(|s| s.item.clone()));
            m.extra.crouching = g.sneaking;
            // Lying in its bed (LivingEntity.getBedOrientation).
            m.extra.ints[1] = g.bed.map_or(0, |b| self.bed_facing(b));
            // LivingEntity.updateSwingTime: the arm swing lan_swings started.
            m.anim.attack_anim_o = m.anim.attack_anim;
            horizon_client::mobs::ai::tick_swing(m);
            // LivingEntity.onSyncedDataUpdated(DATA_LIVING_ENTITY_FLAGS): the
            // item in use and its hand (the arm poses of a bow, a shield...).
            let using = host.play.get(&g.conn).and_then(|p| p.using());
            m.extra.using_item = using.map_or(0, |(_, ticks)| ticks as i32 + 1);
            m.extra.using_off_hand = using.is_some_and(|(slot, _)| slot == OFFHAND_SLOT);
        }
    }

    /// The bed at `head` as a sleeping model lies in it (`ints[1]`,
    /// `villager::sleep_pose`): its `facing`.
    pub(super) fn bed_facing(&self, head: [i32; 3]) -> i32 {
        let state = self.world.block(head[0], head[1], head[2]);
        let facing = self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("facing"))
            .map(String::as_str);
        horizon_client::mobs::villager::bed_facing_code(facing)
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
        let atlas = &self.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        for (conn, m) in &host.models {
            let entity_start = out.len();
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
                self.res.mob_data(),
                &host.hub.guest(*conn).map_or_else(
                    || self.worn_armor_layers(m),
                    |g| self.stack_armor_layers(&g.armor),
                ),
                out,
            );
            self.held_item_geometry(m, camera, alpha, packed, true, out);
            if !m.glowing {
                self.cull_entity_vertices(out, entity_start);
            }
        }
    }

    /// A chat line the host typed, for the guests.
    pub(super) fn lan_host_chat(&mut self, message: &str) {
        if let Some(host) = &self.lan {
            host.hub.chat(&host.host_name, message);
        }
    }
}

impl Game<'_> {
    /// A hub's chat and log lines: a dedicated server logs them
    /// (MinecraftServer.logChatMessage, `[Not Secure]` for unsigned or
    /// expired messages); a LAN host shows the chat.
    pub(super) fn lan_log_event(&mut self, event: HostEvent) {
        let dedicated = self.options.dedicated.is_some();
        match event {
            HostEvent::Chat { line, secure } if dedicated => {
                let tag = if secure { "" } else { "[Not Secure] " };
                super::dedicated::log("INFO", &format!("{tag}{line}"));
            }
            HostEvent::Chat { line, .. } => self.system_message(Text::literal(line)),
            HostEvent::Warn(line) if dedicated => super::dedicated::log("WARN", &line),
            HostEvent::Warn(line) => log::warn!("{line}"),
            HostEvent::Error(line) if dedicated => super::dedicated::log("ERROR", &line),
            HostEvent::Error(line) => log::error!("{line}"),
            _ => {}
        }
    }
}

/// `LivingEntity.DATA_LIVING_ENTITY_FLAGS` for an item in use: 1, and 2 when
/// it is the off hand's.
pub(super) fn living_flags(using: Option<(usize, u32)>) -> i8 {
    using.map_or(0, |(slot, _)| if slot == OFFHAND_SLOT { 3 } else { 1 })
}
