//! The play state of a hosted world (PlayerList, ServerGamePacketListenerImpl,
//! ChunkMap, PlayerChunkSender): admits the players the connection threads
//! hand over, sends them the level (chunks, time, weather, entities, player
//! list), keeps their views up to date as blocks and entities change, and
//! turns what they send into events for the gameplay code. It runs on the
//! game thread inside the tick, only when the world is open to the network.
use super::{
    Lan, NetEvent,
    chunks::ChunkEncoder,
    data::SyncData,
    entity::EntityState,
    guest::{Guest, stack_to_network},
};
use crate::{items::ItemStack, world::World};
use horizon_protocol::ids::play::clientbound as ids;
use horizon_protocol::{
    entity_data::{Value, serializer},
    packets::play::{self, Clientbound as C, Serverbound as S},
    text::{literal, translatable},
};
use horizon_world::nbt::{JavaString, Tag};
use std::sync::{Arc, atomic::Ordering};

/// GameEvent types (ClientboundGameEventPacket).
pub mod game_event {
    pub const START_RAINING: u8 = 1;
    pub const STOP_RAINING: u8 = 2;
    pub const CHANGE_GAME_MODE: u8 = 3;
    pub const RAIN_LEVEL: u8 = 7;
    pub const THUNDER_LEVEL: u8 = 8;
    pub const LEVEL_CHUNKS_LOAD_START: u8 = 13;
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
    pub seed: i64,
    pub max_players: usize,
    /// A dedicated server: operators from `ops`, the dedicated-only commands.
    pub dedicated: bool,
}

/// The host player as the other players see them.
#[derive(Clone, Debug)]
pub struct HostState {
    pub uuid: u128,
    pub name: String,
    pub entity_id: i32,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub on_ground: bool,
    pub game_mode: i32,
    pub sneaking: bool,
    pub sprinting: bool,
    pub equipment: [Option<ItemStack>; 6],
    pub health: f32,
}

/// Everything the hub needs from the game for one tick.
pub struct TickCtx<'a> {
    pub world: &'a mut World,
    /// The player playing on the host (none on a dedicated server).
    pub host: Option<&'a HostState>,
    pub entities: &'a [EntityState],
    pub next_entity: &'a mut u32,
    /// Where new players appear.
    pub spawn: [f64; 3],
    pub difficulty: i32,
    pub difficulty_locked: bool,
    /// The overworld clock's speed (0 when the day does not advance).
    pub clock_rate: f32,
    /// TickRateManager: the tick rate and whether the game is frozen.
    pub tick_rate: f32,
    pub tick_frozen: bool,
}

/// What the gameplay code has to do for a guest.
pub enum HostEvent {
    /// A guest entered the world; load their saved state.
    Joined(u32),
    /// A guest left (state to save, entities to drop).
    Left(Box<Guest>),
    Packet(u32, Box<S>),
    /// A chat line to show on the host (already broadcast to the guests).
    Chat(String),
}

pub struct Hub {
    pub lan: Lan,
    pub config: HubConfig,
    pub encoder: ChunkEncoder,
    pub guests: Vec<Guest>,
    pub(super) sync: Arc<SyncData>,
    pub ticks: u64,
    last_clock: i64,
    last_rate: f32,
    ticking: (f32, bool),
    weather: (bool, f32, f32),
    /// Operators and their permission levels (dedicated servers).
    pub ops: std::collections::HashMap<u128, u8>,
    chat_type: i32,
    dimension_type: i32,
    clock_ids: Vec<i32>,
    item_ids: std::collections::HashMap<Arc<str>, i32>,
    entity_types: std::collections::HashMap<String, i32>,
}

fn string(s: &str) -> Tag {
    Tag::String(JavaString::from(s))
}
pub fn yellow_translation(key: &str, args: &[&str]) -> Tag {
    let mut t = translatable(key, args);
    if let Tag::Compound(map) = &mut t {
        map.insert(JavaString::from("color"), string("yellow"));
    }
    t
}

/// Player.getDisplayName for a player in no team: the name, suggesting
/// `/tell NAME `, showing the player on hover and inserting the name.
pub fn display_name(uuid: u128, name: &str) -> Tag {
    let words = [
        (uuid >> 96) as i32,
        (uuid >> 64) as i32,
        (uuid >> 32) as i32,
        uuid as i32,
    ];
    Tag::compound([
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
    ])
}

/// A yellow translation of player names (`multiplayer.player.joined`).
fn yellow_names(key: &str, names: Vec<Tag>) -> Tag {
    Tag::compound([
        ("translate", string(key)),
        ("with", Tag::List(names)),
        ("color", string("yellow")),
    ])
}

/// ChunkTrackingView.contains: the chunks a player at (cx, cz) with view
/// distance `r` is sent (isWithinDistance with the neighbours' buffer).
fn tracked(cx: i32, cz: i32, r: i32, x: i32, z: i32) -> bool {
    let dx = i64::from(((x - cx).abs() - 2).max(0));
    let dz = i64::from(((z - cz).abs() - 2).max(0));
    dx * dx + dz * dz < i64::from(r) * i64::from(r)
}

impl Hub {
    pub fn new(lan: Lan, sync: Arc<SyncData>, encoder: ChunkEncoder, config: HubConfig) -> Self {
        let chat_type = sync
            .id("minecraft:chat_type", "minecraft:chat")
            .unwrap_or(0);
        let dimension_type = sync
            .id("minecraft:dimension_type", "minecraft:overworld")
            .unwrap_or(0);
        let clock_ids = (0..sync.dynamic.len("minecraft:world_clock") as i32).collect();
        Self {
            lan,
            config,
            encoder,
            guests: Vec::new(),
            sync,
            ticks: 0,
            last_clock: i64::MIN,
            last_rate: 0.0,
            ticking: (20.0, false),
            ops: Default::default(),
            weather: (false, 0.0, 0.0),
            chat_type,
            dimension_type,
            clock_ids,
            item_ids: Default::default(),
            entity_types: Default::default(),
        }
    }

    pub fn guest(&self, conn: u32) -> Option<&Guest> {
        self.guests.iter().find(|g| g.conn == conn)
    }
    pub fn guest_mut(&mut self, conn: u32) -> Option<&mut Guest> {
        self.guests.iter_mut().find(|g| g.conn == conn)
    }

    /// The network id of a name in a built-in or synchronized registry.
    pub fn registry_id(&self, registry: &str, name: &str) -> Option<i32> {
        self.sync.id(registry, name)
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
        let id = s.as_ref().map(|s| self.item_id(&s.item))?;
        let sync = self.sync.clone();
        stack_to_network(|_| id, |name| sync.id("minecraft:enchantment", name), s)
    }
    fn entity_type(&mut self, kind: &str) -> Option<i32> {
        if let Some(id) = self.entity_types.get(kind) {
            return Some(*id);
        }
        let id = self.sync.builtin.get("minecraft:entity_type")?.id(kind)?;
        self.entity_types.insert(kind.to_owned(), id);
        Some(id)
    }

    pub fn broadcast(&self, packet: &C) {
        let body = packet.encode();
        for g in self.guests.iter().filter(|g| g.placed) {
            g.sender.send(body.clone());
        }
    }
    pub fn broadcast_except(&self, conn: u32, packet: &C) {
        let body = packet.encode();
        for g in self.guests.iter().filter(|g| g.placed && g.conn != conn) {
            g.sender.send(body.clone());
        }
    }
    /// PlayerList.broadcastSystemMessage.
    pub fn system_message(&self, content: Tag) {
        self.broadcast(&C::SystemChat(play::SystemChat {
            content,
            overlay: false,
        }));
    }
    /// A chat message from the host (a disguised chat: the host has no
    /// connection to sign with).
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
    /// LAN world first), as display names or names with ids, joined by
    /// gray commas (ComponentUtils.formatList).
    pub fn list_reply(&self, host: Option<(u128, &str)>, uuids: bool) -> Tag {
        let players: Vec<(u128, &str)> = host
            .into_iter()
            .chain(
                self.guests
                    .iter()
                    .filter(|g| g.placed)
                    .map(|g| (g.uuid, g.name.as_str())),
            )
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
                display_name(uuid, name)
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
    /// PlayerList.broadcastChatMessage of a guest's unsigned message
    /// (PlayerChatMessage.unsigned): a ClientboundPlayerChatPacket to every
    /// player, each connection numbering its messages.
    pub fn player_chat(&mut self, from: usize, text: &str) {
        let (uuid, name) = (self.guests[from].uuid, self.guests[from].name.clone());
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let bound = play::ChatTypeBound {
            chat_type: self.chat_type,
            name: display_name(uuid, &name),
            target: None,
        };
        for g in self.guests.iter_mut().filter(|g| g.placed) {
            let packet = C::PlayerChat(play::PlayerChat {
                global_index: g.chat_index,
                sender: uuid,
                index: 0,
                signature: None,
                content: text.to_owned(),
                timestamp,
                salt: 0,
                last_seen: Vec::new(),
                unsigned_content: None,
                filter: (0, Vec::new()),
                chat_type: bound.clone(),
            });
            g.chat_index += 1;
            g.send(&packet);
        }
    }

    /// Disconnect a guest with a reason (ServerGamePacketListenerImpl.disconnect).
    pub fn kick(&mut self, conn: u32, reason: Tag) {
        if let Some(g) = self.guest(conn) {
            let _ = g.sender.tx.send(super::Out::Disconnect(reason));
        }
    }

    fn player_info(&self, uuid: u128, name: &str, mode: i32, latency: i32) -> play::PlayerInfo {
        play::PlayerInfo {
            uuid,
            name: Some(name.to_owned()),
            properties: Vec::new(),
            chat_session: None,
            game_mode: Some(mode),
            listed: Some(true),
            latency: Some(latency),
            display_name: Some(None),
            list_order: Some(0),
            show_hat: Some(true),
        }
    }
    /// ClientboundPlayerInfoUpdatePacket.createPlayerInitializing: every action.
    const INFO_ACTIONS: u8 = 0xFF;

    // ---- joining -------------------------------------------------------

    fn spawn_info(&self, game_mode: i32) -> play::SpawnInfo {
        play::SpawnInfo {
            dimension_type: self.dimension_type,
            dimension: self.config.dimension.clone(),
            seed: horizon_core::biome::obfuscate_seed(self.config.seed),
            game_type: game_mode,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        }
    }

    fn admit(&mut self, j: super::Joined, ctx: &mut TickCtx) -> u32 {
        // The game's ids are taken by incrementing first: the current value
        // is the last id in use (the host's own, right after opening to LAN).
        *ctx.next_entity += 1;
        let entity_id = *ctx.next_entity as i32;
        let conn = j.conn;
        let mut guest = Guest::new(j, entity_id, ctx.spawn, self.config.view_distance);
        guest.last_action_tick = self.ticks;
        guest.game_mode = self.config.default_game_mode;
        guest.permission = self.default_permission(guest.uuid);
        guest.joined_tick = self.ticks;
        guest.position = ctx.spawn;
        guest.previous_position = ctx.spawn;
        self.guests.push(guest);
        conn
    }

    /// The permission level a player gets: an integrated server gives every
    /// guest the game masters' level when commands are allowed
    /// (IntegratedServer.setGuestCommandAccess); a dedicated server its
    /// operators' level (`ops`).
    fn default_permission(&self, uuid: u128) -> u8 {
        if self.config.dedicated {
            self.ops.get(&uuid).copied().unwrap_or(0)
        } else if self.config.guest_commands {
            2
        } else {
            0
        }
    }

    /// The levels of a server, in MinecraftServer.levelKeys order.
    fn level_keys() -> Vec<String> {
        ["overworld", "the_end", "the_nether"]
            .map(|l| format!("minecraft:{l}"))
            .to_vec()
    }

    /// PlayerList.placeNewPlayer for a guest whose saved state is loaded.
    pub fn place_new_player(&mut self, conn: u32, ctx: &mut TickCtx) {
        let Some(i) = self.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let (name, uuid, entity_id, mode) = {
            let g = &self.guests[i];
            (g.name.clone(), g.uuid, g.entity_id, g.game_mode)
        };
        let g = &self.guests[i];
        g.send(&C::Login(play::Login {
            player_id: entity_id,
            hardcore: self.config.hardcore,
            levels: Self::level_keys(),
            max_players: self.config.max_players as i32,
            chunk_radius: self.config.view_distance,
            simulation_distance: self.config.simulation_distance,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: self.spawn_info(mode),
            online_mode: false,
            enforces_secure_chat: false,
        }));
        g.send(&C::ChangeDifficulty(play::ChangeDifficulty {
            difficulty: ctx.difficulty,
            locked: ctx.difficulty_locked,
        }));
        g.send(&C::PlayerAbilities(play::PlayerAbilities {
            flags: g.ability_flags(),
            flying_speed: 0.05,
            walking_speed: 0.1,
        }));
        g.send(&C::SetHeldSlot(play::SetHeldSlot {
            slot: g.selected as i32,
        }));
        // PlayerList.placeNewPlayer: ClientboundUpdateRecipesPacket.
        g.send(&raw(ids::UPDATE_RECIPES, self.sync.recipes.clone()));
        // PlayerList.sendPlayerPermissionLevel, then the recipe book.
        self.send_permission_level(i);
        let g = &self.guests[i];
        g.send(&raw(
            ids::RECIPE_BOOK_SETTINGS,
            vec![0; 8], // four books, each closed and unfiltered
        ));
        g.send(&raw(ids::RECIPE_BOOK_ADD, vec![0, 1])); // no entries, replace
        // The others hear of the newcomer before it joins the list.
        self.broadcast_except(
            conn,
            &C::SystemChat(play::SystemChat {
                content: yellow_names(
                    "multiplayer.player.joined",
                    vec![display_name(self.guests[i].uuid, &name)],
                ),
                overlay: false,
            }),
        );
        self.guests[i].placed = true;
        self.teleport(i, None);
        let g = &self.guests[i];
        g.send(&C::ServerData(play::ServerData {
            motd: literal(&self.lan.shared.motd()),
            icon: self.lan.shared.config.favicon.clone(),
        }));
        // The player list: everyone already here, then the newcomer to all.
        let mut entries = Vec::new();
        if let Some(host) = ctx.host {
            entries.push(self.player_info(host.uuid, &host.name, host.game_mode, 0));
        }
        for o in self.guests.iter().filter(|o| o.conn != conn && o.placed) {
            entries.push(self.player_info(
                o.uuid,
                &o.name,
                o.game_mode,
                o.latency.load(Ordering::Relaxed),
            ));
        }
        self.guests[i].send(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: Self::INFO_ACTIONS,
            entries,
        }));
        let newcomer = play::PlayerInfoUpdate {
            actions: Self::INFO_ACTIONS,
            entries: vec![self.player_info(uuid, &name, mode, 0)],
        };
        self.broadcast(&C::PlayerInfoUpdate(newcomer));
        self.send_level_info(i, ctx);
        // ServerLevel.addNewPlayer: the chunk tracking starts at the player.
        let g = &mut self.guests[i];
        let (cx, cz) = g.chunk();
        g.send(&C::SetChunkCacheCenter(play::SetChunkCacheCenter {
            x: cx,
            z: cz,
        }));
        g.center_sent = Some((cx, cz));
        g.placed_at = self.ticks;
        // ServerPlayer.sendPostEffects and initInventoryMenu.
        self.guests[i].send(&raw(ids::POST_EFFECTS, vec![0]));
        self.send_inventory(i);
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
        for i in 0..self.guests.len() {
            self.guests[i].permission = self.default_permission(self.guests[i].uuid);
            if self.guests[i].placed {
                self.send_permission_level(i);
            }
        }
    }

    /// Commands.sendCommands: the command tree for this player.
    fn send_commands(&self, i: usize) {
        let g = &self.guests[i];
        let packet = match &self.sync.commands {
            Some(tree) => tree.packet(g.permission, self.config.dedicated, &self.sync.builtin),
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
        };
        g.send(&C::Commands(packet));
    }

    /// ServerGamePacketListenerImpl.teleport: move the client and wait for its confirmation.
    pub fn teleport(&mut self, i: usize, position: Option<[f64; 3]>) {
        let g = &mut self.guests[i];
        if let Some(p) = position {
            g.position = p;
            g.previous_position = p;
        }
        g.teleport_counter = (g.teleport_counter + 1) & 0x7FFF_FFFF;
        let id = g.teleport_counter;
        g.awaiting_teleport = Some((id, g.position));
        g.send(&C::PlayerPosition(play::PlayerPosition {
            teleport: id,
            position: g.position,
            delta: [0.0; 3],
            yaw: g.yaw,
            pitch: g.pitch,
            relatives: 0,
        }));
    }

    /// PlayerList.sendLevelInfo.
    fn send_level_info(&mut self, i: usize, ctx: &TickCtx) {
        let spawn = ctx.spawn;
        let g = &self.guests[i];
        g.send(&C::InitializeBorder(play::InitializeBorder {
            center_x: 0.0,
            center_z: 0.0,
            old_size: 59_999_968.0,
            new_size: 59_999_968.0,
            lerp_time: 0,
            absolute_max_size: 29_999_984,
            warning_blocks: 5,
            // WorldBorder.DEFAULT_WARNING_TIME, in ticks.
            warning_time: 300,
        }));
        g.send(&self.time_packet(ctx.world, ctx.clock_rate));
        g.send(&C::SetDefaultSpawnPosition(play::SetDefaultSpawnPosition {
            pos: play::GlobalPos {
                dimension: self.config.dimension.clone(),
                pos: [
                    spawn[0].floor() as i32,
                    spawn[1].floor() as i32,
                    spawn[2].floor() as i32,
                ],
            },
            yaw: 0.0,
            pitch: 0.0,
        }));
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
        g.send(&C::GameEvent(play::GameEvent {
            event: game_event::LEVEL_CHUNKS_LOAD_START,
            param: 0.0,
        }));
        // TickRateManager.updateJoiningPlayer.
        g.send(&ticking_state(ctx.tick_rate, ctx.tick_frozen));
        g.send(&raw(ids::TICKING_STEP, vec![0]));
    }

    /// ServerClockManager.createFullSyncPacket: every clock. Horizon keeps the
    /// overworld clock (the day time); the End's clock runs with the game
    /// time. Clocks do not advance without the advance_time rule.
    fn time_packet(&self, world: &World, rate: f32) -> C {
        let overworld = self
            .sync
            .id("minecraft:world_clock", "minecraft:overworld")
            .unwrap_or(0);
        C::SetTime(play::SetTime {
            game_time: world.game_time,
            clocks: self
                .clock_ids
                .iter()
                .map(|&id| {
                    if id == overworld {
                        play::ClockState {
                            clock: id,
                            total_ticks: world.day_time,
                            partial_tick: 0.0,
                            rate,
                        }
                    } else {
                        play::ClockState {
                            clock: id,
                            total_ticks: world.game_time,
                            partial_tick: 0.0,
                            rate: if rate > 0.0 { 1.0 } else { 0.0 },
                        }
                    }
                })
                .collect(),
        })
    }

    /// ServerPlayer.initInventoryMenu / sendAllDataToRemote for the inventory menu.
    pub fn send_inventory(&mut self, i: usize) {
        let items: Vec<Option<ItemStack>> = (0..46).map(|s| self.guests[i].menu_slot(s)).collect();
        let net: Vec<_> = items.iter().map(|s| self.stack(s)).collect();
        let carried = self.guests[i].carried.clone();
        let carried = self.stack(&carried);
        let g = &mut self.guests[i];
        g.menu_state = (g.menu_state + 1) & 0x7FFF;
        g.send(&C::ContainerSetContent(play::ContainerSetContent {
            container: 0,
            state: g.menu_state,
            items: net,
            carried,
        }));
        g.menu_sent = items;
        g.sent_selected = g.selected;
    }

    // ---- per tick ------------------------------------------------------

    pub fn tick(&mut self, ctx: &mut TickCtx) -> Vec<HostEvent> {
        self.ticks += 1;
        let mut events = Vec::new();
        for event in self.lan.poll() {
            match event {
                NetEvent::Joined(j) => {
                    let conn = self.admit(*j, ctx);
                    events.push(HostEvent::Joined(conn));
                }
                NetEvent::Left(conn, reason) => {
                    if let Some(i) = self.guests.iter().position(|g| g.conn == conn) {
                        let g = self.remove_guest(i, &reason);
                        events.push(HostEvent::Left(Box::new(g)));
                    }
                }
                NetEvent::Packet(conn, packet) => {
                    if let Some(g) = self.guest_mut(conn) {
                        g.inbox.push_back(*packet);
                    }
                }
                NetEvent::Duplicate(uuid) => {
                    let conns: Vec<u32> = self
                        .guests
                        .iter()
                        .filter(|g| g.uuid == uuid)
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
        for i in 0..self.guests.len() {
            if self.guests[i].placed {
                self.process_inbox(i, ctx, &mut events);
            }
        }
        if !self.guests.iter().any(|g| g.placed) {
            if self.guests.is_empty() {
                let _ = ctx.world.block_log.take();
            }
            return events;
        }
        if ctx.world.block_log.is_none() {
            ctx.world.block_log = Some(Vec::new());
        }
        self.sync_world(ctx);
        for i in 0..self.guests.len() {
            if self.guests[i].placed {
                self.update_chunks(i, ctx);
            }
        }
        self.broadcast_block_changes(ctx.world);
        self.sync_containers(ctx.world);
        self.track_entities(ctx);
        for i in 0..self.guests.len() {
            if self.guests[i].placed {
                self.sync_player_state(i);
            }
        }
        let latencies: Vec<_> = self
            .guests
            .iter()
            .map(|g| (g.uuid, g.latency.load(Ordering::Relaxed)))
            .collect();
        if self.ticks.is_multiple_of(100) && !latencies.is_empty() {
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
        events
    }

    fn remove_guest(&mut self, i: usize, reason: &str) -> Guest {
        let mut g = self.guests.remove(i);
        g.left_reason = reason.to_owned();
        if std::env::var_os("HORIZON_NET_DEBUG").is_some() {
            log::info!("[lan] {} left: {reason}", g.name);
        }
        if !g.placed {
            return g;
        }
        self.broadcast(&C::PlayerInfoRemove(play::PlayerInfoRemove {
            uuids: vec![g.uuid],
        }));
        self.broadcast(&C::RemoveEntities(play::RemoveEntities {
            ids: vec![g.entity_id],
        }));
        self.system_message(yellow_names(
            "multiplayer.player.left",
            vec![display_name(g.uuid, &g.name)],
        ));
        g
    }

    /// Time and weather (ServerLevel.tickTime / advanceWeatherCycle broadcasts).
    fn sync_world(&mut self, ctx: &TickCtx) {
        let world = &*ctx.world;
        // MinecraftServer.tickServer: the game time every 20 ticks
        // (forceGameTimeSynchronization, no clocks).
        if self.ticks.is_multiple_of(20) {
            self.broadcast(&C::SetTime(play::SetTime {
                game_time: world.game_time,
                clocks: Vec::new(),
            }));
        }
        // ServerClockManager.modifyClock: a day that jumped (`time set`, `time
        // add`) or a new rate updates the overworld clock.
        let (day, rate) = (world.day_time, ctx.clock_rate);
        let step = i64::from(rate.ceil() as i32).max(1);
        if self.last_clock != i64::MIN
            && ((day - self.last_clock).abs() > step || rate != self.last_rate)
        {
            let overworld = self
                .sync
                .id("minecraft:world_clock", "minecraft:overworld")
                .unwrap_or(0);
            self.broadcast(&C::SetTime(play::SetTime {
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
        if (ctx.tick_rate, ctx.tick_frozen) != self.ticking {
            self.ticking = (ctx.tick_rate, ctx.tick_frozen);
            self.broadcast(&ticking_state(ctx.tick_rate, ctx.tick_frozen));
        }
        let w = &world.weather;
        let (raining, rain, thunder) = self.weather;
        if w.raining != raining {
            self.broadcast(&C::GameEvent(play::GameEvent {
                event: if w.raining {
                    game_event::START_RAINING
                } else {
                    game_event::STOP_RAINING
                },
                param: 0.0,
            }));
        }
        if (w.rain_level.1 - rain).abs() > f32::EPSILON {
            self.broadcast(&C::GameEvent(play::GameEvent {
                event: game_event::RAIN_LEVEL,
                param: w.rain_level.1,
            }));
        }
        if (w.thunder_level.1 - thunder).abs() > f32::EPSILON {
            self.broadcast(&C::GameEvent(play::GameEvent {
                event: game_event::THUNDER_LEVEL,
                param: w.thunder_level.1,
            }));
        }
        self.weather = (w.raining, w.rain_level.1, w.thunder_level.1);
    }

    // ---- chunks --------------------------------------------------------

    fn update_chunks(&mut self, i: usize, ctx: &TickCtx) {
        let (cx, cz) = self.guests[i].chunk();
        let r = self.guests[i].view_distance;
        let g = &mut self.guests[i];
        if g.center != Some((cx, cz)) {
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
                .filter(|&(x, z)| !tracked(cx, cz, r, x, z))
                .collect();
            for (x, z) in forget {
                g.sent_chunks.remove(&(x, z));
                g.send(&C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [x, z] }));
            }
            g.chunk_queue.retain(|&(x, z)| tracked(cx, cz, r, x, z));
            for dz in -r - 2..=r + 2 {
                for dx in -r - 2..=r + 2 {
                    let key = (cx + dx, cz + dz);
                    if tracked(cx, cz, r, key.0, key.1)
                        && !g.sent_chunks.contains(&key)
                        && !g.chunk_queue.contains(&key)
                    {
                        g.chunk_queue.push(key);
                    }
                }
            }
            g.chunk_queue
                .sort_by_key(|&(x, z)| i64::from(x - cx).pow(2) + i64::from(z - cz).pow(2));
        }
        // Send ready chunks, closest first, in batches.
        if g.chunk_queue.is_empty() || g.unacknowledged >= 10 || self.ticks <= g.placed_at + 1 {
            return;
        }
        g.chunk_credit = (g.chunk_credit + g.chunks_per_tick).min(g.chunks_per_tick.max(1.0) * 2.0);
        let budget = g.chunk_credit.floor() as usize;
        if budget == 0 {
            return;
        }
        let mut packets = Vec::new();
        let mut taken = Vec::new();
        for (idx, key) in g.chunk_queue.iter().enumerate() {
            if packets.len() >= budget {
                break;
            }
            let Some(chunk) = ctx.world.chunks.get(key) else {
                continue;
            };
            if !chunk.lit {
                continue;
            }
            packets.push(C::LevelChunkWithLight(
                self.encoder.encode_in(chunk, ctx.world),
            ));
            taken.push(idx);
        }
        if packets.is_empty() {
            return;
        }
        g.chunk_credit -= packets.len() as f32;
        g.send(&C::ChunkBatchStart(horizon_protocol::packets::Empty));
        for p in &packets {
            g.send(p);
        }
        g.send(&C::ChunkBatchFinished(play::ChunkBatchFinished {
            size: packets.len() as i32,
        }));
        g.unacknowledged += 1;
        for idx in taken.into_iter().rev() {
            let key = g.chunk_queue.remove(idx);
            g.sent_chunks.insert(key);
        }
    }

    /// Resend a chunk to everyone who has it (a block entity changed, relight).
    pub fn resend_chunk(&mut self, world: &World, x: i32, z: i32) {
        let Some(chunk) = world.chunks.get(&(x, z)) else {
            return;
        };
        let packet = C::LevelChunkWithLight(self.encoder.encode_in(chunk, world));
        let body = packet.encode();
        for g in self
            .guests
            .iter()
            .filter(|g| g.sent_chunks.contains(&(x, z)))
        {
            g.sender.send(body.clone());
        }
    }

    fn broadcast_block_changes(&mut self, world: &mut World) {
        let Some(log) = world.block_log.as_mut() else {
            return;
        };
        if log.is_empty() {
            return;
        }
        let mut changes = std::mem::take(log);
        changes.sort_unstable();
        changes.dedup();
        // Group by section.
        let mut sections: std::collections::BTreeMap<(i32, i32, i32), Vec<(i32, i32, i32)>> =
            Default::default();
        for p in changes {
            sections
                .entry((p.0 >> 4, p.1 >> 4, p.2 >> 4))
                .or_default()
                .push(p);
        }
        for ((sx, sy, sz), positions) in sections {
            let viewers: Vec<usize> = (0..self.guests.len())
                .filter(|&i| {
                    self.guests[i].placed && self.guests[i].sent_chunks.contains(&(sx, sz))
                })
                .collect();
            if viewers.is_empty() {
                continue;
            }
            let packet = if positions.len() == 1 {
                let (x, y, z) = positions[0];
                C::BlockUpdate(play::BlockUpdate {
                    pos: [x, y, z],
                    state: i32::from(world.block(x, y, z).0 as u16),
                })
            } else {
                C::SectionBlocksUpdate(play::SectionBlocksUpdate {
                    section: [sx, sy, sz],
                    changes: positions
                        .iter()
                        .map(|&(x, y, z)| {
                            let state = i64::from(world.block(x, y, z).0);
                            state << 12
                                | i64::from(x & 15) << 8
                                | i64::from(z & 15) << 4
                                | i64::from(y & 15)
                        })
                        .collect(),
                })
            };
            let body = packet.encode();
            for &i in &viewers {
                self.guests[i].sender.send(body.clone());
            }
            // Block entities with data the client shows.
            for &(x, y, z) in &positions {
                let Some(entity) = world.block_entities.get(&(x, y, z)) else {
                    continue;
                };
                let Some(tag) = super::chunks::entity_tag(entity) else {
                    continue;
                };
                let state = world.block(x, y, z).0 as u16;
                let Some(kind) = self.encoder.entity_type(state) else {
                    continue;
                };
                let body = C::BlockEntityData(play::BlockEntityData {
                    pos: [x, y, z],
                    kind,
                    tag,
                })
                .encode();
                for &i in &viewers {
                    self.guests[i].sender.send(body.clone());
                }
            }
        }
    }

    // ---- entities ------------------------------------------------------

    fn player_state(
        &self,
        id: i32,
        uuid: u128,
        position: [f64; 3],
        rot: (f32, f32, f32),
        on_ground: bool,
        flags: (bool, bool, bool, bool),
        skin: u8,
        health: f32,
        equipment: Vec<(u8, Option<horizon_protocol::item::Stack>)>,
    ) -> EntityState {
        let (sneaking, sprinting, gliding, sleeping) = flags;
        let mut shared = 0i8;
        if sneaking {
            shared |= 0x02;
        }
        if sprinting {
            shared |= 0x08;
        }
        if gliding {
            shared |= -128;
        }
        let pose = if sleeping {
            2
        } else if gliding {
            1
        } else if sneaking {
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
        if (health - 1.0).abs() > f32::EPSILON && health != 20.0 {
            meta.push((9, serializer::FLOAT, Value::Float(health)));
        }
        if skin != 0 {
            meta.push((16, serializer::BYTE, Value::Byte(skin as i8)));
        }
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
            equipment,
        }
    }

    fn equipment_of(
        &mut self,
        main: &Option<ItemStack>,
        off: &Option<ItemStack>,
        armor: [&Option<ItemStack>; 4],
    ) -> Vec<(u8, Option<horizon_protocol::item::Stack>)> {
        // Armor is stored head..feet; the wire slots are feet(2) legs(3) chest(4) head(5).
        let mut out = Vec::new();
        let slots = [
            (0u8, main),
            (1, off),
            (5, armor[0]),
            (4, armor[1]),
            (3, armor[2]),
            (2, armor[3]),
        ];
        for (slot, item) in slots {
            if item.is_some() {
                out.push((slot, self.stack(item)));
            }
        }
        out
    }

    /// All entities as one client sees them: the world's, the host, the other guests.
    fn track_entities(&mut self, ctx: &mut TickCtx) {
        let mut all: Vec<EntityState> =
            Vec::with_capacity(ctx.entities.len() + self.guests.len() + 1);
        if let Some(h) = ctx.host {
            let equipment = {
                let e = &h.equipment;
                self.equipment_of(&e[0], &e[1], [&e[5], &e[4], &e[3], &e[2]])
            };
            all.push(self.player_state(
                h.entity_id,
                h.uuid,
                h.position,
                (h.yaw, h.pitch, h.head_yaw),
                h.on_ground,
                (h.sneaking, h.sprinting, false, false),
                0x7F,
                h.health,
                equipment,
            ));
        }
        let placed: Vec<usize> = (0..self.guests.len())
            .filter(|&i| self.guests[i].placed)
            .collect();
        let guest_states: Vec<_> = placed
            .into_iter()
            .map(|i| {
                let (main, off, armor) = {
                    let g = &self.guests[i];
                    (
                        g.inventory.get(g.selected).cloned().flatten(),
                        g.offhand.clone(),
                        g.armor.clone(),
                    )
                };
                let equipment =
                    self.equipment_of(&main, &off, [&armor[0], &armor[1], &armor[2], &armor[3]]);
                let g = &self.guests[i];
                self.player_state(
                    g.entity_id,
                    g.uuid,
                    g.position,
                    (g.yaw, g.pitch, g.head_yaw),
                    g.on_ground,
                    (g.sneaking, g.sprinting, g.fall_flying, g.sleeping),
                    g.info.model_customisation,
                    g.health,
                    equipment,
                )
            })
            .collect();
        all.extend(guest_states);
        all.extend(ctx.entities.iter().cloned());
        let by_id: std::collections::HashMap<i32, usize> =
            all.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
        for gi in 0..self.guests.len() {
            if !self.guests[gi].placed {
                continue;
            }
            let view = self.guests[gi].view_distance;
            let me = self.guests[gi].entity_id;
            let origin = self.guests[gi].position;
            // TrackedEntity.updatePlayer: within the type's range (capped by the
            // view distance) horizontally, in a chunk the client has.
            let visible = |g: &Guest, e: &EntityState| {
                let range = super::entity::visible_range(&e.kind, view);
                let (dx, dz) = (origin[0] - e.position[0], origin[2] - e.position[2]);
                let chunk = (
                    (e.position[0].floor() as i32) >> 4,
                    (e.position[2].floor() as i32) >> 4,
                );
                dx * dx + dz * dz <= range * range && g.sent_chunks.contains(&chunk)
            };
            // Forget what disappeared or left range.
            let mut removed = Vec::new();
            for id in self.guests[gi].tracker.ids() {
                match by_id.get(&id) {
                    Some(&ei) if visible(&self.guests[gi], &all[ei]) => {}
                    _ => removed.push(id),
                }
            }
            if !removed.is_empty() {
                for id in &removed {
                    self.guests[gi].tracker.forget(*id);
                }
                self.guests[gi].send(&C::RemoveEntities(play::RemoveEntities { ids: removed }));
            }
            for e in &all {
                if e.id == me {
                    continue;
                }
                if self.guests[gi].tracker.is_tracking(e.id) {
                    let packets = self.guests[gi].tracker.update(e);
                    self.guests[gi].send_all(&packets);
                } else if visible(&self.guests[gi], e) {
                    let Some(kind) = self.entity_type(&e.kind) else {
                        continue;
                    };
                    // Players need their list entry first; guests got it at join.
                    let packets = self.guests[gi].tracker.spawn(e, kind);
                    self.guests[gi].send_all(&packets);
                }
            }
        }
    }

    // ---- per-player state ------------------------------------------------

    /// ServerPlayer.doTick: health, food, experience and the inventory menu.
    fn sync_player_state(&mut self, i: usize) {
        // ServerEntity.sendDirtyEntityData on the player's first tick: the
        // health its creation set is still dirty, and goes to the player too.
        if self.guests[i].sent_health.is_none() {
            let g = &self.guests[i];
            g.send(&C::SetEntityData(play::SetEntityData {
                id: g.entity_id,
                values: vec![(9, serializer::FLOAT, Value::Float(g.health))],
            }));
            // ServerPlayer.tick's CriteriaTriggers.TICK, then flushDirty.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64);
            let first = self.sync.advancements.first_tick(&self.sync, now);
            if let Some(add) = first.recipe_book_add {
                g.send(&raw(ids::RECIPE_BOOK_ADD, add));
            }
            g.send(&raw(ids::UPDATE_ADVANCEMENTS, first.update_advancements));
        }
        let health = {
            let g = &self.guests[i];
            (g.health, g.food, g.saturation)
        };
        if self.guests[i].sent_health != Some(health) {
            let g = &mut self.guests[i];
            g.send(&C::SetHealth(play::SetHealth {
                health: health.0,
                food: health.1,
                saturation: health.2,
            }));
            g.sent_health = Some(health);
        }
        let xp = {
            let g = &self.guests[i];
            (g.xp_progress, g.xp_level, g.xp_total)
        };
        if self.guests[i].sent_xp != Some(xp) {
            let g = &mut self.guests[i];
            g.send(&C::SetExperience(play::SetExperience {
                progress: xp.0,
                level: xp.1,
                total: xp.2,
            }));
            g.sent_xp = Some(xp);
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
            let g = &mut self.guests[i];
            g.menu_state = (g.menu_state + 1) & 0x7FFF;
            g.send(&C::ContainerSetSlot(play::ContainerSetSlot {
                container: 0,
                state: g.menu_state,
                slot: slot as i16,
                item: net,
            }));
            g.menu_sent[slot] = now;
        }
    }

    // ---- incoming ------------------------------------------------------

    /// The squared movement past 1.0E-5F (handlePlayerKnownMovement).
    fn moved_far(&self, i: usize, to: [f64; 3]) -> bool {
        let from = self.guests[i].position;
        let d: f64 = (0..3).map(|k| (to[k] - from[k]).powi(2)).sum();
        d > f64::from(1.0E-5_f32)
    }

    fn process_inbox(&mut self, i: usize, ctx: &mut TickCtx, events: &mut Vec<HostEvent>) {
        let mut budget = 400;
        while budget > 0 {
            let Some(packet) = self.guests[i].inbox.pop_front() else {
                break;
            };
            budget -= 1;
            let conn = self.guests[i].conn;
            // ServerPlayer.resetLastActionTime: every handler of a player's
            // own action resets it, movement only when it moved
            // (handlePlayerKnownMovement), PlayerInput once loaded.
            let acted = match &packet {
                S::AcceptTeleportation(_)
                | S::MovePlayerStatus(_)
                | S::MovePlayerRot(_)
                | S::PlayerLoaded(_)
                | S::ChunkBatchReceived(_)
                | S::ClientInformation(_)
                | S::ClientTickEnd(_)
                | S::KeepAlive(_) => false,
                S::MovePlayerPos(p) => self.moved_far(i, [p.x, p.y, p.z]),
                S::MovePlayerPosRot(p) => self.moved_far(i, [p.x, p.y, p.z]),
                S::PlayerInput(_) => self.guests[i].loaded,
                _ => true,
            };
            if acted {
                self.guests[i].last_action_tick = self.ticks;
            }
            match packet {
                S::AcceptTeleportation(p) => {
                    let g = &mut self.guests[i];
                    if g.awaiting_teleport.is_some_and(|(id, _)| id == p.id) {
                        g.awaiting_teleport = None;
                    }
                }
                S::MovePlayerPos(p) => {
                    self.move_player(i, Some([p.x, p.y, p.z]), None, p.flags);
                }
                S::MovePlayerPosRot(p) => {
                    self.move_player(i, Some([p.x, p.y, p.z]), Some((p.yaw, p.pitch)), p.flags);
                }
                S::MovePlayerRot(p) => {
                    self.move_player(i, None, Some((p.yaw, p.pitch)), p.flags);
                }
                S::MovePlayerStatus(p) => {
                    self.guests[i].on_ground = p.flags & 1 != 0;
                }
                S::PlayerLoaded(_) => self.guests[i].loaded = true,
                S::ChunkBatchReceived(p) => {
                    let g = &mut self.guests[i];
                    g.unacknowledged = g.unacknowledged.saturating_sub(1);
                    g.chunks_per_tick = if p.chunks_per_tick.is_nan() {
                        0.01
                    } else {
                        p.chunks_per_tick.clamp(0.01, 64.0)
                    };
                }
                S::ClientInformation(info) => {
                    let g = &mut self.guests[i];
                    g.view_distance =
                        i32::from(info.view_distance).clamp(2, self.config.view_distance);
                    g.info = info;
                    g.center = None;
                }
                S::PlayerInput(p) => {
                    self.guests[i].sneaking = p.flags & 32 != 0;
                    self.guests[i].sprinting = p.flags & 64 != 0 && p.flags & 1 != 0;
                }
                S::PlayerCommand(p) => match p.action {
                    1 => self.guests[i].sprinting = true,
                    2 => self.guests[i].sprinting = false,
                    6 => self.guests[i].fall_flying = true,
                    0 => self.guests[i].sleeping = false,
                    _ => {}
                },
                S::ClientTickEnd(_) => {}
                S::Chat(c) => {
                    let text: String = c.message.chars().filter(|c| !c.is_control()).collect();
                    if !text.is_empty() {
                        let name = self.guests[i].name.clone();
                        self.player_chat(i, &text);
                        events.push(HostEvent::Chat(format!("<{name}> {text}")));
                    }
                }
                other => events.push(HostEvent::Packet(conn, Box::new(other))),
            }
        }
        let _ = ctx;
    }

    /// ServerGamePacketListenerImpl.handleMovePlayer.
    fn move_player(&mut self, i: usize, pos: Option<[f64; 3]>, rot: Option<(f32, f32)>, flags: u8) {
        let g = &mut self.guests[i];
        if g.awaiting_teleport.is_some() || !g.loaded && g.ticks == 0 && pos.is_none() {
            return;
        }
        if let Some(p) = pos {
            if p.iter().any(|v| !v.is_finite() || v.abs() > 3.0e7) {
                return;
            }
            // A jump of more than a hundred blocks is a rejected move.
            let d2: f64 = (0..3).map(|k| (p[k] - g.position[k]).powi(2)).sum();
            if d2 > 100.0 * 100.0 {
                let at = g.position;
                self.teleport(i, Some(at));
                return;
            }
            g.previous_position = g.position;
            g.position = p;
        }
        if let Some((yaw, pitch)) = rot
            && yaw.is_finite()
            && pitch.is_finite()
        {
            g.yaw = yaw % 360.0;
            g.pitch = pitch.clamp(-90.0, 90.0);
            g.head_yaw = g.yaw;
        }
        g.on_ground = flags & 1 != 0;
        g.ticks = g.ticks.max(1);
    }

    /// Acknowledge a block interaction (ClientboundBlockChangedAckPacket).
    pub fn ack(&self, conn: u32, sequence: i32) {
        if sequence >= 0
            && let Some(g) = self.guest(conn)
        {
            g.send(&C::BlockChangedAck(play::BlockChangedAck { sequence }));
        }
    }

    /// Set a guest's game mode and tell everyone (ServerPlayer.setGameMode).
    pub fn set_game_mode(&mut self, conn: u32, mode: i32) {
        let Some(g) = self.guest_mut(conn) else {
            return;
        };
        g.game_mode = mode;
        let uuid = g.uuid;
        g.send(&C::GameEvent(play::GameEvent {
            event: game_event::CHANGE_GAME_MODE,
            param: mode as f32,
        }));
        g.send(&C::PlayerAbilities(play::PlayerAbilities {
            flags: g.ability_flags(),
            flying_speed: 0.05,
            walking_speed: 0.1,
        }));
        self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 4,
            entries: vec![play::PlayerInfo {
                uuid,
                game_mode: Some(mode),
                ..Default::default()
            }],
        }));
    }

    /// The host's game mode changed (PlayerInfo UPDATE_GAME_MODE for everyone).
    pub fn set_host_game_mode(&self, uuid: u128, mode: i32) {
        self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 4,
            entries: vec![play::PlayerInfo {
                uuid,
                game_mode: Some(mode),
                ..Default::default()
            }],
        }));
    }

    pub fn disconnect_all(&mut self) {
        let reason = translatable("multiplayer.disconnect.server_shutdown", &[]);
        for g in &self.guests {
            let _ = g.sender.tx.send(super::Out::Disconnect(reason.clone()));
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

    /// ServerPlayer.respawn: back to `at` with a fresh start.
    pub fn respawn(&mut self, i: usize, at: [f64; 3]) {
        let mode = self.guests[i].game_mode;
        let info = self.spawn_info(mode);
        self.close_container_for(i);
        let g = &mut self.guests[i];
        g.position = at;
        g.previous_position = at;
        g.send(&C::Respawn(play::Respawn {
            spawn: info,
            keep: 0,
        }));
        g.center = None;
        g.sent_chunks.clear();
        g.chunk_queue.clear();
        g.tracker.clear();
        self.teleport(i, None);
        self.send_inventory(i);
    }
}

/// A packet Horizon sends as raw bytes (no fields worth a type).
fn raw(id: i32, payload: Vec<u8>) -> C {
    C::Unknown(id, payload)
}

/// ClientboundTickingStatePacket: the tick rate and whether the game is frozen.
fn ticking_state(rate: f32, frozen: bool) -> C {
    let mut w = horizon_protocol::buf::Writer::new();
    w.f32(rate);
    w.bool(frozen);
    raw(ids::TICKING_STATE, w.data)
}
