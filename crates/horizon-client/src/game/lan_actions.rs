//! What guests do in a hosted world. The host's gameplay code is written for
//! the one local player, so a guest's action runs through it with the guest's
//! state swapped in for the duration of the call (position, inventory, game
//! mode, experience, mining and item use): digging, placing, using items and
//! blocks, attacking and the commands all behave exactly as they do for the
//! host, and every world change they cause reaches all players through the
//! block log, the level events and the entity tracker.
use super::{Game, Hit, Mining, Screen, lan::LanHost};
use horizon_client::{
    command::vanilla,
    containers::BlockEntity,
    entities::Experience,
    items::ItemStack,
    lan::{guest::SPECTATOR, menu::Rules},
    motion::PlayerMotion,
};
use horizon_core::geometry::Vec3;
use horizon_protocol::{
    packets::play::{self, Clientbound as C, Serverbound as S},
    text::literal,
};
use winit::keyboard::KeyCode;

/// Per-guest state of the gameplay code that lives outside `Guest`.
#[derive(Default)]
pub(super) struct GuestPlay {
    mining: Option<Mining>,
    destroy_delay: u32,
    using: Option<(usize, u32)>,
    use_delay: u32,
    attack_ticks: u32,
    swing: horizon_client::first_person::SwingState,
    /// The block being dug and the face the client aimed at.
    digging: Option<([i32; 3], [i32; 3])>,
    last_progress: i32,
    /// Hand movements the client announced and the host animated.
    swung: bool,
    /// The trading screen the guest has open.
    pub(super) merchant: Option<super::lan_trading::GuestMerchant>,
}

struct ItemRules<'a>(&'a horizon_client::items::Items);
impl Rules for ItemRules<'_> {
    fn max_stack(&self, item: &str) -> u32 {
        self.0.get(item).map_or(64, |i| i.max_stack)
    }
    fn armor_index(&self, item: &str) -> Option<usize> {
        match self.0.get(item)?.equip_slot.as_deref()? {
            "head" => Some(0),
            "chest" => Some(1),
            "legs" => Some(2),
            "feet" => Some(3),
            _ => None,
        }
    }
}

fn direction(face: i32) -> [i32; 3] {
    match face {
        0 => [0, -1, 0],
        1 => [0, 1, 0],
        2 => [0, 0, -1],
        3 => [0, 0, 1],
        4 => [-1, 0, 0],
        _ => [1, 0, 0],
    }
}

impl Game<'_> {
    /// Run `f` as the guest `conn`: the host's player state is swapped for theirs.
    pub(super) fn as_guest<R>(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        let i = host.hub.guests.iter().position(|g| g.conn == conn)?;
        if host.hub.guests[i].motion.is_none() {
            let p = Vec3::new(
                host.hub.guests[i].position[0],
                host.hub.guests[i].position[1],
                host.hub.guests[i].position[2],
            );
            host.hub.guests[i].motion = PlayerMotion::new(self.motion, p).ok().map(Box::new);
        }
        let mut play = host.play.remove(&conn).unwrap_or_default();
        let g = &mut host.hub.guests[i];
        // The guest's body.
        let mut motion = g.motion.take()?;
        motion.position = Vec3::new(g.position[0], g.position[1], g.position[2]);
        motion.ground.on_ground = g.on_ground;
        motion.yaw = g.yaw;
        motion.pitch = g.pitch;
        motion.sprinting = g.sprinting;
        motion.may_fly = g.creative() || g.game_mode == SPECTATOR;
        motion.invulnerable = motion.may_fly;
        let _ = motion.set_health(g.health, false);
        motion.health.food.food = g.food;
        motion.health.food.saturation = g.saturation;
        let mut player = Some(*motion);
        std::mem::swap(&mut self.player, &mut player);
        let eye = (g.eye_height() as f32, self.eye);
        self.eye = eye.0;
        let (yaw, pitch) = (self.yaw, self.pitch);
        self.yaw = g.yaw;
        self.pitch = g.pitch;
        std::mem::swap(&mut self.inventory, &mut g.inventory);
        std::mem::swap(&mut self.armor, &mut g.armor);
        std::mem::swap(&mut self.offhand, &mut g.offhand);
        std::mem::swap(&mut self.carried, &mut g.carried);
        let selected = std::mem::replace(&mut self.selected, g.selected);
        let (mode, creative) = (self.mode, self.creative);
        self.set_mode_raw(g.game_mode);
        let xp = std::mem::replace(
            &mut self.experience,
            Experience {
                level: g.xp_level,
                progress: g.xp_progress,
                total: g.xp_total,
            },
        );
        let sneak = self.keys.contains(&KeyCode::ShiftLeft);
        if g.sneaking {
            self.keys.insert(KeyCode::ShiftLeft);
        } else {
            self.keys.remove(&KeyCode::ShiftLeft);
        }
        let screen = std::mem::replace(&mut self.screen, Screen::Playing);
        let container = self.container.take();
        let mining = std::mem::replace(&mut self.mining, play.mining.take());
        let destroy_delay = std::mem::replace(&mut self.destroy_delay, play.destroy_delay);
        let using = std::mem::replace(&mut self.using, play.using.take());
        let use_delay = std::mem::replace(&mut self.use_delay, play.use_delay);
        let attack_ticks = std::mem::replace(&mut self.attack_ticks, play.attack_ticks);
        let swing = std::mem::replace(&mut self.swing, play.swing);
        let (left, right) = (self.left, self.right);
        self.left = play.digging.is_some();
        self.right = play.using.is_some() || self.using.is_some();
        let riding = self.riding.take();
        let host_station = self.station.take();
        self.lan_acting = Some(conn);

        let result = f(self);
        self.lan_acting = None;

        // What the action opened: handed to the guest instead of the host.
        let opened = self.container.take();
        let opened_station = std::mem::replace(&mut self.station, host_station);
        self.riding = riding;
        self.left = left;
        self.right = right;
        play.mining = std::mem::replace(&mut self.mining, mining);
        play.destroy_delay = std::mem::replace(&mut self.destroy_delay, destroy_delay);
        play.using = std::mem::replace(&mut self.using, using);
        play.use_delay = std::mem::replace(&mut self.use_delay, use_delay);
        play.attack_ticks = std::mem::replace(&mut self.attack_ticks, attack_ticks);
        play.swing = std::mem::replace(&mut self.swing, swing);
        self.container = container;
        self.screen = screen;
        if sneak {
            self.keys.insert(KeyCode::ShiftLeft);
        } else {
            self.keys.remove(&KeyCode::ShiftLeft);
        }
        let g = &mut host.hub.guests[i];
        let new_xp = std::mem::replace(&mut self.experience, xp);
        g.xp_level = new_xp.level;
        g.xp_progress = new_xp.progress;
        g.xp_total = new_xp.total;
        let new_mode = self.mode_id();
        self.mode = mode;
        self.creative = creative;
        self.selected = std::mem::replace(&mut g.selected, selected);
        // `g.selected` now holds the host's value: swap the inventories back.
        std::mem::swap(&mut self.selected, &mut g.selected);
        std::mem::swap(&mut self.inventory, &mut g.inventory);
        std::mem::swap(&mut self.armor, &mut g.armor);
        std::mem::swap(&mut self.offhand, &mut g.offhand);
        std::mem::swap(&mut self.carried, &mut g.carried);
        self.yaw = yaw;
        self.pitch = pitch;
        self.eye = eye.1;
        std::mem::swap(&mut self.player, &mut player);
        if let Some(mut m) = player {
            g.health = m.health.health();
            g.food = m.health.food.food;
            g.saturation = m.health.food.saturation;
            // A teleport (commands) moves the guest.
            let at = [m.position.x, m.position.y, m.position.z];
            m.sprinting = false;
            let moved: f64 = (0..3).map(|k| (at[k] - g.position[k]).powi(2)).sum();
            g.motion = Some(Box::new(m));
            if moved > 0.0001 {
                host.hub.teleport(i, Some(at));
            }
        }
        let g = &mut host.hub.guests[i];
        if new_mode != g.game_mode {
            let conn = g.conn;
            host.hub.set_game_mode(conn, new_mode);
        }
        host.play.insert(conn, play);
        if let Some(pos) = opened {
            self.lan_open_container(host, conn, pos);
        }
        if let Some(station) = opened_station {
            self.lan_station_opened(host, conn, station);
        }
        Some(result)
    }

    /// The game mode fields without `set_mode`'s side effects on the host.
    fn set_mode_raw(&mut self, id: i32) {
        use horizon_client::command::args::GameMode;
        self.mode = match id {
            1 => GameMode::Creative,
            2 => GameMode::Adventure,
            3 => GameMode::Spectator,
            _ => GameMode::Survival,
        };
        self.creative = self.mode == GameMode::Creative;
    }

    /// Hand a container the guest opened over (ServerPlayer.openMenu).
    fn lan_open_container(&mut self, host: &mut LanHost, conn: u32, pos: (i32, i32, i32)) {
        let Some(entity) = self.world.block_entities.get(&pos) else {
            return;
        };
        let (menu, title) = match entity {
            BlockEntity::Chest(items) => match items.len() {
                27 => ("minecraft:generic_9x3", "container.chest"),
                9 => ("minecraft:generic_3x3", "container.dispenser"),
                5 => ("minecraft:hopper", "container.hopper"),
                _ => return,
            },
            _ => return,
        };
        let name = &self
            .res
            .facts
            .get(self.world.block(pos.0, pos.1, pos.2))
            .name;
        let title = match &**name {
            "minecraft:barrel" => "container.barrel",
            "minecraft:dropper" => "container.dropper",
            n if n.ends_with("shulker_box") => "container.shulkerBox",
            _ => title,
        };
        let items = entity.slots().to_vec();
        host.hub.open_container(
            conn,
            menu,
            horizon_protocol::text::translatable(title, &[]),
            pos,
            items,
        );
    }

    /// A packet from a guest that the hub does not handle itself.
    pub(super) fn lan_guest_packet(&mut self, host: &mut LanHost, conn: u32, packet: S) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        if host.hub.guests[gi].dead && !matches!(packet, S::ClientCommand(_)) {
            return;
        }
        let spectator = host.hub.guests[gi].spectator();
        match packet {
            S::PlayerAction(a) => match a.action {
                0 => {
                    host.hub.ack(conn, a.sequence);
                    if spectator {
                        return;
                    }
                    let face = direction(i32::from(a.direction));
                    let play = host.play.entry(conn).or_default();
                    play.digging = Some((a.pos, face));
                    play.last_progress = -1;
                }
                1 => {}
                2 | 3 => {
                    host.hub.ack(conn, a.sequence);
                    let play = host.play.entry(conn).or_default();
                    play.digging = None;
                    play.mining = None;
                    if a.action == 2 {
                        let state = self.world.block(a.pos[0], a.pos[1], a.pos[2]).0 as i32;
                        host.hub.send_to(
                            conn,
                            &C::BlockUpdate(play::BlockUpdate { pos: a.pos, state }),
                        );
                        host.hub.broadcast_near(
                            [
                                f64::from(a.pos[0]),
                                f64::from(a.pos[1]),
                                f64::from(a.pos[2]),
                            ],
                            64.0,
                            &C::BlockDestruction(play::BlockDestruction {
                                id: host.hub.guests[gi].entity_id,
                                pos: a.pos,
                                progress: 255,
                            }),
                            Some(conn),
                        );
                    }
                }
                4 | 5 => {
                    if !spectator {
                        self.as_guest(host, conn, |g| g.drop_held(a.action == 4));
                    }
                }
                6 => {
                    self.as_guest(host, conn, |g| {
                        if let Some((slot, ticks)) = g.using.take()
                            && g.stack_in(slot)
                                .is_some_and(|s| &*s.item == "minecraft:bow")
                        {
                            g.release_bow(ticks);
                        }
                    });
                }
                7 => {
                    let g = &mut host.hub.guests[gi];
                    let slot = g.selected;
                    std::mem::swap(&mut g.inventory[slot], &mut g.offhand);
                }
                _ => {}
            },
            S::SetCarriedItem(p) => {
                if (0..9).contains(&p.slot) {
                    host.hub.guests[gi].selected = p.slot as usize;
                    if let Some(play) = host.play.get_mut(&conn) {
                        play.using = None;
                    }
                }
            }
            S::UseItemOn(p) => {
                host.hub.ack(conn, p.sequence);
                if spectator {
                    return;
                }
                let face = direction(p.face);
                let hit = Hit {
                    pos: p.pos,
                    face,
                    point: [
                        f64::from(p.pos[0]) + f64::from(p.cursor[0]),
                        f64::from(p.pos[1]) + f64::from(p.cursor[1]),
                        f64::from(p.pos[2]) + f64::from(p.cursor[2]),
                    ],
                };
                self.as_guest(host, conn, |g| {
                    g.forced_hit = Some(hit);
                    let _ = g.use_item();
                    g.forced_hit = None;
                });
                // The client predicted a result: correct it with what really is.
                for pos in [
                    p.pos,
                    [p.pos[0] + face[0], p.pos[1] + face[1], p.pos[2] + face[2]],
                ] {
                    let state = self.world.block(pos[0], pos[1], pos[2]).0 as i32;
                    host.hub
                        .send_to(conn, &C::BlockUpdate(play::BlockUpdate { pos, state }));
                }
            }
            S::UseItem(p) => {
                host.hub.ack(conn, p.sequence);
                if spectator {
                    return;
                }
                self.as_guest(host, conn, |g| {
                    let _ = g.use_item();
                });
            }
            S::Attack(p) => self.guest_attack(host, conn, p.id),
            S::Interact(p) => {
                if let Some((position, eye)) = self.look_at_entity(host, gi, p.id) {
                    let _ = (position, eye);
                    self.as_guest(host, conn, |g| {
                        let _ = g.use_item();
                    });
                }
            }
            S::SetCreativeModeSlot(p) => {
                let g = &mut host.hub.guests[gi];
                if !g.creative() || !(0..46).contains(&p.slot) {
                    return;
                }
                let item = self.item_from_network(host, &p.item);
                host.hub.guests[gi].set_menu_slot(p.slot as usize, item);
            }
            S::ContainerClick(p) if self.lan_merchant_click(host, conn, &p) => {}
            S::SelectTrade(p) => self.lan_select_trade(host, conn, p.item),
            S::ContainerClose(p) if self.lan_merchant_closed(host, conn, Some(p.container)) => {
                let g = &mut host.hub.guests[gi];
                if let Some(c) = g.carried.take() {
                    self.drop_guest_items(host, gi, vec![c]);
                }
            }
            S::ContainerClick(p) => {
                let rules = ItemRules(&self.res.items);
                let drops = host.hub.container_click(conn, p, &mut self.world, &rules);
                self.drop_guest_items(host, gi, drops);
            }
            S::ContainerClose(p) => {
                host.hub.container_closed(conn, p.container);
                let g = &mut host.hub.guests[gi];
                if let Some(c) = g.carried.take() {
                    self.drop_guest_items(host, gi, vec![c]);
                }
            }
            S::ChatCommand(c) => self.guest_command(host, conn, &c.command),
            S::ChangeGameMode(p) if host.hub.guests[gi].permission >= 2 => {
                if let Some(mode) = usize::try_from(p.mode)
                    .ok()
                    .and_then(|index| horizon_client::command::args::GameMode::ALL.get(index))
                {
                    let lines = self.as_guest(host, conn, |g| {
                        g.chat_capture = Some(Vec::new());
                        g.change_game_mode(*mode);
                        g.chat_capture
                            .take()
                            .unwrap_or_default()
                            .iter()
                            .map(|t| horizon_client::lan::data::component_nbt(&t.component()))
                            .collect::<Vec<_>>()
                    });
                    for content in lines.unwrap_or_default() {
                        host.hub.send_to(
                            conn,
                            &C::SystemChat(play::SystemChat {
                                content,
                                overlay: false,
                            }),
                        );
                    }
                }
            }
            S::Unknown(8, payload) => {
                // A signed command: the text comes first.
                if let Ok(text) = horizon_protocol::buf::Reader::new(&payload).string(32767) {
                    self.guest_command(host, conn, &text);
                }
            }
            S::Unknown(id, payload)
                if id == horizon_protocol::ids::play::serverbound::COMMAND_SUGGESTION =>
            {
                self.guest_suggestions(host, conn, &payload);
            }
            S::ClientCommand(c) if c.action == 0 => self.guest_respawn(host, conn),
            S::PickItemFromBlock(p) => {
                let state = self.world.block(p.pos[0], p.pos[1], p.pos[2]);
                let item = self.res.facts.get(state).item.clone();
                let g = &mut host.hub.guests[gi];
                if g.creative() && &*item != "minecraft:air" {
                    let slot = g.selected;
                    g.inventory[slot] = Some(ItemStack::new(&item, 1));
                }
            }
            _ => {}
        }
    }

    fn item_from_network(
        &self,
        host: &LanHost,
        stack: &Option<horizon_protocol::item::Stack>,
    ) -> Option<ItemStack> {
        use horizon_protocol::item::{Component, component};
        let s = stack.as_ref()?;
        let name = host
            .hub
            .item_name(s.item)
            .unwrap_or_else(|| "minecraft:air".into());
        let mut out = ItemStack::new(&name, s.count.max(1) as u32);
        out.damage = s.damage().max(0) as u32;
        if let Some(Component::Enchantments(list)) = s.component(component::ENCHANTMENTS) {
            for (id, level) in list {
                if let Some(e) = host.hub.enchantment_name(*id) {
                    out.enchant(&e, *level as u32);
                }
            }
        }
        Some(out)
    }

    pub(super) fn drop_guest_items(&mut self, host: &LanHost, gi: usize, stacks: Vec<ItemStack>) {
        let g = &host.hub.guests[gi];
        let eye = [
            g.position[0],
            g.position[1] + g.eye_height() - 0.3,
            g.position[2],
        ];
        let (yaw, pitch) = (
            f64::from(g.yaw).to_radians(),
            f64::from(g.pitch).to_radians(),
        );
        for stack in stacks {
            let v = [
                -yaw.sin() * pitch.cos() * 0.3,
                -pitch.sin() * 0.3 + 0.1,
                yaw.cos() * pitch.cos() * 0.3,
            ];
            self.spawn_item(stack, eye, v, 40);
        }
    }

    /// Aim the swapped-in player at an entity; returns its centre and the eye.
    fn look_at_entity(
        &mut self,
        host: &mut LanHost,
        gi: usize,
        id: i32,
    ) -> Option<([f64; 3], [f64; 3])> {
        let m = self.mobs.iter().find(|m| m.id as i32 == id)?;
        let centre = [
            m.position.x,
            m.position.y + m.kind.height * 0.5,
            m.position.z,
        ];
        let g = &mut host.hub.guests[gi];
        let eye = [g.position[0], g.position[1] + g.eye_height(), g.position[2]];
        let d = [centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]];
        let horizontal = (d[0] * d[0] + d[2] * d[2]).sqrt();
        g.yaw = (-d[0].atan2(d[2])).to_degrees() as f32;
        g.pitch = (-d[1].atan2(horizontal)).to_degrees() as f32;
        Some((centre, eye))
    }

    fn guest_attack(&mut self, host: &mut LanHost, conn: u32, id: i32) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        if host.hub.guests[gi].spectator() {
            return;
        }
        let (old_yaw, old_pitch) = (host.hub.guests[gi].yaw, host.hub.guests[gi].pitch);
        if self.look_at_entity(host, gi, id).is_none() {
            return;
        }
        self.as_guest(host, conn, |g| {
            g.attack();
        });
        host.hub.guests[gi].yaw = old_yaw;
        host.hub.guests[gi].pitch = old_pitch;
    }

    /// Commands.performCommand for a guest with command access.
    fn guest_command(&mut self, host: &mut LanHost, conn: u32, command: &str) {
        // ListPlayersCommand needs no permission and lists every player.
        let words: Vec<&str> = command.trim_start_matches('/').split(' ').collect();
        if words[0] == "list" && (words.len() == 1 || words[1..] == ["uuids"]) {
            let me = (self.options.dedicated.is_none() && self.player.is_some())
                .then_some((host.host_uuid, super::chat::PLAYER_NAME));
            let content = host.hub.list_reply(me, words.len() == 2);
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content,
                    overlay: false,
                }),
            );
            return;
        }
        if !host.settings.guest_commands || !self.cheats {
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content: horizon_protocol::text::translatable("command.unknown.command", &[]),
                    overlay: false,
                }),
            );
            return;
        }
        let command = command.trim_start_matches('/').to_owned();
        let lines = self.as_guest(host, conn, |g| {
            g.chat_capture = Some(Vec::new());
            g.run_command(&command);
            g.chat_capture
                .take()
                .unwrap_or_default()
                .iter()
                .map(|t| horizon_client::lan::data::component_nbt(&t.component()))
                .collect::<Vec<_>>()
        });
        for content in lines.unwrap_or_default() {
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content,
                    overlay: false,
                }),
            );
        }
    }

    /// ServerGamePacketListenerImpl.handleCustomCommandSuggestions: the
    /// completions of the command typed so far (after an optional slash), for
    /// the nodes the tree marks `ask_server`.
    fn guest_suggestions(&mut self, host: &mut LanHost, conn: u32, payload: &[u8]) {
        let mut r = horizon_protocol::buf::Reader::new(payload);
        let (Ok(id), Ok(text)) = (r.varint(), r.string(32500)) else {
            return;
        };
        let Some(permission) = host.hub.guest(conn).map(|g| g.permission) else {
            return;
        };
        let skip = usize::from(text.starts_with('/'));
        let command = &text[skip..];
        let d = vanilla::game_dispatcher();
        let level = std::mem::replace(&mut self.ui.registries.permission, permission);
        let parse = d.parse(command, &self.ui.registries);
        let ((start, end), suggestions) = d.suggestions(&parse, command.len(), &self.ui.registries);
        self.ui.registries.permission = level;
        let utf16 = |byte: usize| command[..byte.min(command.len())].encode_utf16().count() as i32;
        let mut w = horizon_protocol::buf::Writer::new();
        w.varint(id);
        w.varint(utf16(start) + skip as i32);
        w.varint(utf16(end) - utf16(start));
        w.varint(suggestions.len() as i32);
        for s in &suggestions {
            w.string(&s.text);
            w.bool(s.tooltip.is_some());
            if let Some(tip) = &s.tooltip {
                w.nbt(&literal(tip));
            }
        }
        host.hub.send_to(
            conn,
            &C::Unknown(
                horizon_protocol::ids::play::clientbound::COMMAND_SUGGESTIONS,
                w.data,
            ),
        );
    }

    fn guest_respawn(&mut self, host: &mut LanHost, conn: u32) {
        let spawn = self.spawn.map_or([0.5, 80.0, 0.5], |s| [s.x, s.y, s.z]);
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        if !host.hub.guests[i].dead {
            return;
        }
        let g = &mut host.hub.guests[i];
        g.dead = false;
        g.health = 20.0;
        g.food = 20;
        g.saturation = 5.0;
        g.sent_health = None;
        g.motion = None;
        host.hub.respawn(i, spawn);
    }

    /// Per tick, for every guest: dig, use items, pick up drops, die.
    pub(super) fn tick_guests(&mut self, host: &mut LanHost) {
        self.lan_merchant_tick(host);
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| g.placed && g.loaded)
            .map(|g| g.conn)
            .collect();
        for conn in conns {
            let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
                continue;
            };
            if host.hub.guests[gi].dead {
                continue;
            }
            let play = host.play.entry(conn).or_default();
            let digging = play.digging;
            let using = play.using.is_some();
            let delayed = play.destroy_delay > 0 || play.use_delay > 0 || play.attack_ticks < 100;
            if digging.is_some() || using || delayed {
                self.as_guest(host, conn, |g| {
                    g.attack_ticks = g.attack_ticks.saturating_add(1);
                    if let Some((pos, face)) = digging {
                        g.forced_hit = Some(Hit {
                            pos,
                            face,
                            point: [
                                f64::from(pos[0]) + 0.5,
                                f64::from(pos[1]) + 0.5,
                                f64::from(pos[2]) + 0.5,
                            ],
                        });
                    }
                    let _ = g.tick_mining();
                    g.forced_hit = None;
                    g.right = g.using.is_some();
                    g.tick_using();
                    if g.use_delay > 0 {
                        g.use_delay -= 1;
                    }
                });
            }
            self.guest_dig_feedback(host, conn);
            self.guest_pickups(host, gi);
            let g = &mut host.hub.guests[gi];
            if g.health <= 0.0 && !g.dead {
                g.dead = true;
                let message = literal(&format!("{} died", g.name));
                host.hub.send_to(
                    conn,
                    &C::PlayerCombatKill(play::PlayerCombatKill {
                        id: host.hub.guests[gi].entity_id,
                        message,
                    }),
                );
            }
        }
    }

    /// The crack animation other players see.
    fn guest_dig_feedback(&mut self, host: &mut LanHost, conn: u32) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let play = host.play.entry(conn).or_default();
        let Some((pos, _)) = play.digging else {
            return;
        };
        let progress = play
            .mining
            .as_ref()
            .map_or(-1, |m| (m.progress.clamp(0.0, 1.0) * 10.0) as i32 - 1);
        let swinging = play.swing.swinging();
        if progress != play.last_progress && progress >= 0 {
            play.last_progress = progress;
            host.hub.broadcast_near(
                [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])],
                64.0,
                &C::BlockDestruction(play::BlockDestruction {
                    id: host.hub.guests[gi].entity_id,
                    pos,
                    progress: progress as u8,
                }),
                Some(conn),
            );
        }
        let _ = swinging;
    }

    /// Dropped items touching a guest go into their inventory.
    fn guest_pickups(&mut self, host: &mut LanHost, gi: usize) {
        let (conn, mode, position, id) = {
            let g = &host.hub.guests[gi];
            (g.conn, g.game_mode, g.position, g.entity_id)
        };
        if mode == SPECTATOR || self.dropped.is_empty() {
            return;
        }
        let near: Vec<usize> = self
            .dropped
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                d.pickup_delay == 0
                    && (d.position.x - position[0]).abs() < 1.5
                    && (d.position.z - position[2]).abs() < 1.5
                    && d.position.y > position[1] - 0.6
                    && d.position.y < position[1] + 2.4
            })
            .map(|(i, _)| i)
            .collect();
        let mut taken = Vec::new();
        for i in near.into_iter().rev() {
            let (stack, item_id) = (self.dropped[i].stack.clone(), self.dropped[i].id);
            let before = stack.count;
            let rest = self.as_guest(host, conn, |g| g.give(stack)).flatten();
            match rest {
                None => {
                    let item = self.dropped.remove(i);
                    self.guest_pickup(host, id, item);
                    taken.push((item_id, before));
                }
                Some(rest) if rest.count < before => {
                    self.guest_pickup(host, id, self.dropped[i].clone());
                    self.dropped[i].stack = rest;
                    taken.push((item_id, before - self.dropped[i].stack.count));
                }
                Some(_) => {}
            }
        }
        for (item, count) in taken {
            host.hub.take_item(item as i32, id, count as i32, position);
            host.hub
                .sound("minecraft:entity.item.pickup", position, 0.2, 1.4, 7, None);
        }
    }

    /// The host's own `ItemPickupParticle` toward the guest's model.
    fn guest_pickup(
        &mut self,
        host: &LanHost,
        id: i32,
        item: horizon_client::entities::ItemEntity,
    ) {
        use super::pickups::{Collector, Taken, guest_target};
        if let Some(target) = guest_target(host, id as u32) {
            self.add_pickup_toward(
                Taken::Item(Box::new(item)),
                Collector::Guest(id as u32),
                target,
            );
        }
    }

    /// Positional sounds and level events of the tick, for the guests.
    pub(super) fn lan_broadcast_effects(&mut self, host: &mut LanHost) {
        let sounds = self.audio.capture.take().unwrap_or_default();
        self.audio.capture = Some(Vec::new());
        let blocks: Vec<[f64; 3]> = std::mem::take(&mut host.broken);
        for (name, at, volume, pitch) in sounds {
            // Block breaks are heard through the level event.
            if blocks
                .iter()
                .any(|b| (0..3).all(|k| (b[k] + 0.5 - at[k]).abs() < 1e-6))
            {
                continue;
            }
            let source = horizon_client::settings::sound_source(&name) as i32;
            host.hub.sound(&name, at, volume, pitch, source, None);
        }
    }

    /// Guests' arm swings for everyone else.
    pub(super) fn lan_swings(&mut self, host: &mut LanHost) {
        let conns: Vec<u32> = host.play.keys().copied().collect();
        for conn in conns {
            let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn) else {
                continue;
            };
            let (id, at) = (g.entity_id, g.position);
            let play = host.play.get_mut(&conn).expect("play");
            let swinging = play.swing.swinging() && play.swing.ticks == 0;
            if swinging && !play.swung {
                host.hub.swing(id, 0, at, Some(conn));
            }
            play.swung = swinging;
        }
    }
}
