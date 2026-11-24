//! Hub operations on what guests perceive and do beyond the basics: sounds,
//! level events, animations and block containers with their clicks.
use super::{
    hub::Hub,
    menu::{self, Menu, NativeMenuEvent, Rules, SlotRef, Slots, View},
};
use crate::{
    containers::BlockEntity, first_person::SwingAnimation, items::ItemStack, world::World,
};
use horizon_core::nbt::Tag;
use horizon_core::random::RandomSource;
use horizon_protocol::{
    item::Hashed,
    packets::{
        configuration::CustomPayload,
        play::{self, Clientbound as C},
    },
};
use std::collections::HashMap;

/// Encode a captured level sound and its broadcast radius without a live hub.
/// Entity-bound events use the entity packet; ordinary level sounds retain
/// their fixed-point coordinates. The caller supplies the synced registry id.
pub fn captured_sound_packet(
    sound: &crate::audio::CapturedSound,
    sound_id: Option<i32>,
    seed: i64,
) -> (C, f64) {
    let holder = match sound_id {
        Some(id) => (id, None),
        None => (-1, Some((sound.event.clone(), None))),
    };
    let packet = match sound.entity {
        Some(id) => C::SoundEntity(play::SoundEntity {
            sound: holder,
            source: sound.source as i32,
            id: id as i32,
            volume: sound.volume,
            pitch: sound.pitch,
            seed,
        }),
        None => C::Sound(play::Sound {
            sound: holder,
            source: sound.source as i32,
            x: (sound.position[0] * 8.0) as i32,
            y: (sound.position[1] * 8.0) as i32,
            z: (sound.position[2] * 8.0) as i32,
            volume: sound.volume,
            pitch: sound.pitch,
            seed,
        }),
    };
    (packet, captured_sound_range(sound))
}

pub(super) fn captured_sound_range(sound: &crate::audio::CapturedSound) -> f64 {
    // SoundEvent.getRange multiplies as floats before the broadcast widens it.
    f64::from(if sound.volume > 1.0 {
        16.0 * sound.volume
    } else {
        16.0
    })
}

/// A container click read from the world and not yet run (`Hub::prepare_click`).
pub struct PreparedClick {
    i: usize,
    in_menu: bool,
    source: Option<Slots>,
    shulker: bool,
    drag: Option<(u8, Vec<usize>)>,
    hidden: Vec<usize>,
    items: Option<Vec<Option<ItemStack>>>,
    container_limits: Vec<Option<u32>>,
    /// The sounds the click played at the player (`BundleItem.playInsertSound` and the others,
    /// `Player.playSound`), in order; `run_click` fills them, `finish_click` sends them.
    sounds: Vec<&'static str>,
}

impl PreparedClick {
    /// The container's own slots, in the menu's order: as `prepare_click` read them, then as
    /// `run_click` left them. `None` for a click in the player's own inventory.
    pub fn container(&self) -> Option<&[Option<ItemStack>]> {
        self.items.as_deref()
    }
}

impl Hub {
    pub fn send_to(&self, conn: u32, packet: &C) {
        if let Some(g) = self.guest(conn) {
            if let C::RemoveEntities(p) = packet {
                self.send_ordered_removal(g, &p.ids);
            } else {
                g.send(packet);
            }
        }
    }

    /// `ChunkMap.sendToTrackingPlayers`: send to every placed guest that
    /// tracks the entity `id`, and wrapped to the clients viewing the level that do.
    pub fn send_to_tracking(&self, id: i32, packet: &C) {
        let body = packet.encode();
        for g in self
            .guests
            .iter()
            .filter(|g| self.here(g) && g.tracker.is_tracking(id))
        {
            g.sender.send(body.clone());
        }
        self.send_view_trackers(id, packet);
    }

    /// `ServerChunkCache.sendToTrackingPlayersAndSelf`: send to every placed
    /// guest that tracks the entity `id`, and to the guest whose entity it is.
    pub fn send_to_tracking_and_self(&self, id: i32, packet: &C) {
        let body = packet.encode();
        for g in self
            .guests
            .iter()
            .filter(|g| self.here(g) && (g.entity_id == id || g.tracker.is_tracking(id)))
        {
            g.sender.send(body.clone());
        }
        self.send_view_trackers(id, packet);
    }

    /// Send to every placed guest within `range` blocks of `at`
    /// (`PlayerList.broadcast`, which leaves out a player exactly at `range`).
    pub fn broadcast_near(&self, at: [f64; 3], range: f64, packet: &C, except: Option<u32>) {
        let body = packet.encode();
        for g in self
            .guests
            .iter()
            .filter(|g| self.here(g) && Some(g.conn) != except)
        {
            if within_range(at, g.position, range) {
                g.sender.send(body.clone());
            }
        }
    }

    /// ClientboundSoundPacket for a sound event name (an inline event when
    /// the registry does not know it).
    pub fn sound_packet(
        &self,
        name: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        source: i32,
        seed: i64,
    ) -> C {
        let sound = match self.sync.id("minecraft:sound_event", name) {
            Some(id) => (id, None),
            None => (-1, Some((name.to_owned(), None))),
        };
        C::Sound(play::Sound {
            sound,
            source,
            x: (at[0] * 8.0) as i32,
            y: (at[1] * 8.0) as i32,
            z: (at[2] * 8.0) as i32,
            volume,
            pitch,
            seed,
        })
    }
    /// Draw the next seed from the server sound source.
    pub fn next_sound_seed(&self) -> i64 {
        next_sound_seed(&mut *self.sound_seeds.lock().unwrap())
    }

    /// The entity overload of `ServerLevel.playSeededSound`. The emitter stays
    /// an entity id on the wire; clients which do not know it drop the sound.
    pub fn captured_sound(&self, sound: &crate::audio::CapturedSound, except: Option<u32>) {
        let except = except.or_else(|| {
            sound.except_entity.and_then(|id| {
                self.guests
                    .iter()
                    .find(|g| g.entity_id as u32 == id)
                    .map(|g| g.conn)
            })
        });
        let seed = self.next_sound_seed();
        let (packet, range) = captured_sound_packet(
            sound,
            self.sync.id("minecraft:sound_event", &sound.event),
            seed,
        );
        self.broadcast_near(sound.position, range, &packet, except);
        // Wrapped for the level's viewers too: one bound to an entity for those tracking it.
        let emitter = sound.entity.map(|id| id as i32);
        self.send_viewed_sound(
            &self.config.dimension,
            sound.position,
            range,
            emitter,
            || packet.encode(),
        );
    }

    /// `Level.playSound` at `at` for the players within range in the level in place, and wrapped
    /// for the clients that view it there (`send_viewed_sound`).
    pub fn sound(
        &self,
        name: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        source: i32,
        except: Option<u32>,
    ) {
        // Level.playSound: soundSeedGenerator.nextLong per call, not the tick.
        let seed = self.next_sound_seed();
        let packet = self.sound_packet(name, at, volume, pitch, source, seed);
        let range = f64::from(volume.max(1.0)) * 16.0;
        self.broadcast_near(at, range, &packet, except);
        self.send_viewed_sound(&self.config.dimension, at, range, None, || packet.encode());
    }

    /// ClientboundLevelEventPacket (block break particles and sounds ...); wrapped, to the
    /// clients that view the level and hold its chunk.
    pub fn level_event(&self, kind: i32, pos: [i32; 3], data: i32, except: Option<u32>) {
        let at = [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])];
        let packet = C::LevelEvent(play::LevelEvent {
            kind,
            pos,
            data,
            global: false,
        });
        self.broadcast_near(at, 64.0, &packet, except);
        self.send_viewers_of((pos[0] >> 4, pos[2] >> 4), &packet);
    }

    /// `ServerLevel.globalLevelEvent` for an event in level `level`: the packet
    /// with its global flag for every player on the server
    /// (`PlayerList.getPlayers`), each at a position of their own
    /// (`global_level_event_position`). The level is the event's, not the one in
    /// place: an event queued during one level's pass may be sent during another's.
    pub fn global_level_event(&self, level: &str, kind: i32, pos: [i32; 3], data: i32) {
        for g in self.online_players() {
            g.send(&C::LevelEvent(play::LevelEvent {
                kind,
                pos: global_level_event_position(pos, g.position, g.placed_in(level)),
                data,
                global: true,
            }));
        }
    }

    /// ClientboundBlockEventPacket (`PlayerList.broadcast` within 64 blocks); wrapped, to the
    /// clients that view the level and hold its chunk.
    pub fn block_event(&self, world: &World, pos: [i32; 3], b0: i32, b1: i32, state: u32) {
        let block = self
            .encoder
            .block_ids
            .get(state as usize)
            .copied()
            .unwrap_or(0);
        let at = [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])];
        let packet = C::BlockEvent(play::BlockEvent {
            pos,
            b0: b0 as u8,
            b1: b1 as u8,
            block,
        });
        let key = (pos[0] >> 4, pos[2] >> 4);
        if !self.has_terrain_views() {
            self.broadcast_near(at, 64.0, &packet, None);
            self.send_viewers_of(key, &packet);
            return;
        }
        let body = packet.encode();
        for g in self
            .guests
            .iter()
            .filter(|g| self.terrain_state(g.conn, world, (pos[0], pos[1], pos[2]), state) == state)
        {
            if self.here(g) && within_range(at, g.position, 64.0) {
                g.sender.send(body.clone());
            } else if let Some(view) = self.view_here(g).filter(|v| v.sent.contains(&key)) {
                let mut runs = super::viewed_levels::send::Runs::new(view.level);
                runs.push(&body);
                g.sender.send_many(runs.finish());
            }
        }
    }

    /// `ServerLevel.broadcastEntityEvent`: send to the players tracking the
    /// entity and the entity itself (`sendToTrackingPlayersAndSelf`).
    pub fn entity_event(&self, id: i32, event: i8) {
        self.send_to_tracking_and_self(id, &C::EntityEvent(play::EntityEvent { id, event }));
    }

    /// `ServerPlayer.crit` (`magicCrit` when `magic`): send the victim's
    /// animation to the players tracking the attacker and the attacker
    /// themselves (`sendToTrackingPlayersAndSelf`).
    pub fn critical_hit(&self, attacker: i32, target: i32, magic: bool) {
        self.send_to_tracking_and_self(
            attacker,
            &C::Animate(play::Animate {
                id: target,
                action: if magic { 2 } else { 1 },
            }),
        );
    }

    /// `ServerLevel.broadcastDamageEvent`: the clients around show the hurt
    /// entity's flash and hear its hurt sound.
    pub fn damage_event(
        &self,
        id: i32,
        kind: &str,
        cause: Option<i32>,
        direct: Option<i32>,
        at: [f64; 3],
    ) {
        let Some(source_type) = self.sync.id("minecraft:damage_type", kind) else {
            return;
        };
        self.broadcast_near(
            at,
            128.0,
            &C::DamageEvent(play::DamageEvent {
                id,
                source_type,
                cause,
                direct,
                position: None,
            }),
            None,
        );
    }

    /// `ClientboundSwingAnimationPacket`: the hand and the animation the swing
    /// started with (its base duration; a client applies the swinger's effects).
    pub fn swing(
        &self,
        id: i32,
        off_hand: bool,
        animation: SwingAnimation,
        at: [f64; 3],
        except: Option<u32>,
    ) {
        self.broadcast_near(
            at,
            128.0,
            &C::SwingAnimation(play::SwingAnimation {
                id,
                hand: i32::from(off_hand),
                kind: i32::from(animation.kind),
                duration: animation.duration,
            }),
            except,
        );
    }

    pub fn take_item(&self, item: i32, collector: i32, count: i32, at: [f64; 3]) {
        self.broadcast_near(
            at,
            64.0,
            &C::TakeItemEntity(play::TakeItemEntity {
                item,
                player: collector,
                amount: count,
            }),
            None,
        );
    }

    /// Open a block container for a guest (ServerPlayer.openMenu). `extra`,
    /// for a mod menu type, goes first in a `horizon:menu_extra` payload with
    /// the window and `pos` (`openMenu` with an extra-data writer).
    #[allow(clippy::too_many_arguments)]
    pub fn open_container(
        &mut self,
        conn: u32,
        menu_kind: &str,
        title: Tag,
        pos: (i32, i32, i32),
        slots: Slots,
        items: Vec<Option<ItemStack>>,
        extra: Option<Vec<u8>>,
    ) {
        self.open_menu_window(conn, menu_kind, title, Some(pos), slots, items, extra);
    }

    /// Open menu type `menu_kind` for a guest, of a block at `pos` or of none:
    /// its window id, None for an unknown type or guest. A [`Slots::Native`]
    /// menu's items are the guest's `native_menu`, as `open_native_menu` keeps.
    #[allow(clippy::too_many_arguments)]
    pub fn open_menu_window(
        &mut self,
        conn: u32,
        menu_kind: &str,
        title: Tag,
        pos: Option<(i32, i32, i32)>,
        slots: Slots,
        items: Vec<Option<ItemStack>>,
        extra: Option<Vec<u8>>,
    ) -> Option<i32> {
        let kind = self
            .sync
            .builtin
            .get("minecraft:menu")
            .and_then(|m| m.id(menu_kind))?;
        let i = self.guests.iter().position(|g| g.conn == conn)?;
        self.close_container_at(i);
        let window = self.guests.update(i, |g| {
            let window = g.next_menu_window();
            if slots == Slots::Native {
                g.native_menu = items.clone();
            }
            if let Some(data) = extra {
                let message = horizon_protocol::menu_extra::Message {
                    window,
                    pos: pos.map(|p| [p.0, p.1, p.2]),
                    data,
                };
                // A client that answered no revision query (an older Horizon build)
                // reads the legacy layout, which has no way to say "no block".
                let sent = if g.sender.menu_extra_rev >= 1 {
                    horizon_protocol::menu_extra::encode(&message)
                } else {
                    horizon_protocol::menu_extra::encode_legacy(&message)
                };
                match sent {
                    Ok(data) => g.send(&C::CustomPayload(CustomPayload {
                        channel: horizon_protocol::menu_extra::CHANNEL.into(),
                        data,
                    })),
                    Err(e) => log::warn!("Horizon: {menu_kind}'s extra data not sent: {e}"),
                }
            }
            g.send(&C::OpenScreen(play::OpenScreen {
                container: window,
                menu: kind,
                title,
            }));
            let mod_type = crate::containers::mod_menu_type(menu_kind);
            g.menu = Some(Menu {
                window,
                kind,
                registry_id: menu_kind.into(),
                pos,
                slots,
                size: items.len(),
                state: 0,
                sent: Vec::new(),
                drag: None,
                sent_data: Vec::new(),
                hidden: Vec::new(),
                mod_type,
                craft: None,
                links: Vec::new(),
                asks_still_valid: super::menu::asks_still_valid(mod_type, menu_kind),
            });
            window
        });
        self.send_menu_content(i, items);
        Some(window)
    }

    /// The open menu of `conn` has no `hidden` container slots: a mod's screen
    /// does not show them, so a click or a shift-click never reaches them.
    pub fn hide_menu_slots(&mut self, conn: u32, hidden: Vec<usize>) {
        let _ = self.update_guest(conn, |g| {
            if let Some(m) = g.menu.as_mut() {
                m.hidden = hidden;
            }
        });
    }

    /// Opens a native mod's menu of `rows` rows (1 to 6, a chest's
    /// `generic_9xN`) showing `items` (padded to the rows' slots), over the
    /// guest's inventory: no click moves anything, each is a
    /// [`NativeMenuEvent`]. The window's id, or None for an unknown guest.
    pub fn open_native_menu(
        &mut self,
        conn: u32,
        rows: u8,
        title: Tag,
        mut items: Vec<Option<ItemStack>>,
    ) -> Option<i32> {
        let rows = rows.clamp(1, 6);
        let kind = self
            .sync
            .builtin
            .get("minecraft:menu")
            .and_then(|m| m.id(&format!("minecraft:generic_9x{rows}")))?;
        let i = self.guests.iter().position(|g| g.conn == conn)?;
        self.close_container_at(i);
        let size = usize::from(rows) * 9;
        items.resize(size, None);
        let window = self.guests.update(i, |g| {
            g.native_menu = items.clone();
            let window = g.next_menu_window();
            g.send(&C::OpenScreen(play::OpenScreen {
                container: window,
                menu: kind,
                title,
            }));
            g.menu = Some(Menu {
                window,
                kind,
                registry_id: format!("minecraft:generic_9x{rows}").into_boxed_str(),
                pos: None,
                slots: Slots::Native,
                size,
                state: 0,
                sent: Vec::new(),
                drag: None,
                sent_data: Vec::new(),
                hidden: Vec::new(),
                mod_type: None,
                craft: None,
                links: Vec::new(),
                asks_still_valid: false,
            });
            window
        });
        self.send_menu_content(i, items);
        Some(window)
    }

    /// Replaces what the guest's open native menu `window` shows with `items` (padded to its
    /// slots, extra ones dropped) and sends them at once, as
    /// `AbstractContainerMenu.sendAllDataToRemote`: the screen stays open. False when that window
    /// isn't the guest's open native menu.
    pub fn set_native_menu_items(
        &mut self,
        conn: u32,
        window: i32,
        mut items: Vec<Option<ItemStack>>,
    ) -> bool {
        let Some(i) = self.guests.iter().position(|g| g.conn == conn) else {
            return false;
        };
        let Some(size) = self.guests[i]
            .menu
            .as_ref()
            .filter(|m| m.slots == Slots::Native && m.window == window)
            .map(|m| m.size)
        else {
            return false;
        };
        items.resize(size, None);
        self.guests.update(i, |g| g.native_menu = items.clone());
        self.send_menu_content(i, items);
        true
    }

    /// Open a trading window for a guest (`Merchant.openTradingScreen`):
    /// its id, or None when the menu type is unknown.
    pub fn open_merchant_window(&mut self, conn: u32, title: Tag) -> Option<i32> {
        self.open_window(conn, "minecraft:merchant", title)
    }

    /// ServerPlayer.openMenu for a menu the game runs (merchant, crafting
    /// table): the open container closes, a new window id opens this menu.
    pub fn open_window(&mut self, conn: u32, menu: &str, title: Tag) -> Option<i32> {
        let kind = self
            .sync
            .builtin
            .get("minecraft:menu")
            .and_then(|m| m.id(menu))?;
        let i = self.guests.iter().position(|g| g.conn == conn)?;
        self.close_container_at(i);
        Some(self.guests.update(i, |g| {
            let window = g.next_menu_window();
            g.send(&C::OpenScreen(play::OpenScreen {
                container: window,
                menu: kind,
                title,
            }));
            window
        }))
    }

    /// ServerPlayer.openHorseInventory: the open container closes, a new
    /// window id opens mount `entity`'s inventory with its chest's columns
    /// (ClientboundMountScreenOpenPacket).
    pub fn open_mount_window(&mut self, conn: u32, columns: i32, entity: i32) -> Option<i32> {
        let i = self.guests.iter().position(|g| g.conn == conn)?;
        self.close_container_at(i);
        Some(self.guests.update(i, |g| {
            let window = g.next_menu_window();
            g.send(&C::MountScreenOpen(play::MountScreenOpen {
                container: window,
                columns,
                entity,
            }));
            window
        }))
    }

    fn send_menu_content(&mut self, i: usize, mut items: Vec<Option<ItemStack>>) {
        for j in 0..36 {
            items.push(self.guests[i].inventory[player_slot(j)].clone());
        }
        let net: Vec<_> = items.iter().map(|s| self.stack(s)).collect();
        let carried = self.guests[i].carried.clone();
        let carried = self.stack(&carried);
        self.guests.update(i, |g| {
            let Some(menu) = g.menu.as_mut() else {
                return;
            };
            menu.state = (menu.state + 1) & 0x7FFF;
            let packet = C::ContainerSetContent(play::ContainerSetContent {
                container: menu.window,
                state: menu.state,
                items: net,
                carried,
            });
            menu.sent = items;
            g.send(&packet);
            g.inventory_told();
        });
    }

    /// `AbstractContainerMenu.sendAllDataToRemote` for the guest's open
    /// block container `window`: every slot and the carried stack again.
    pub fn resend_menu(&mut self, conn: u32, window: i32, world: &World) {
        let Some(i) = self.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let g = &self.guests[i];
        let Some(items) = g
            .menu
            .as_ref()
            .filter(|m| window != 0 && m.window == window)
            .and_then(|m| m.slots.read(world, g))
        else {
            return;
        };
        self.send_menu_content(i, items);
    }

    fn close_container_at(&mut self, i: usize) {
        let closed = self.guests.update(i, |g| {
            let menu = g.menu.take()?;
            g.send(&C::ContainerClose(play::ContainerClose {
                container: menu.window,
            }));
            if menu.slots == Slots::Native {
                g.native_menu.clear();
                Some((g.uuid, menu.window))
            } else {
                None
            }
        });
        if let Some((uuid, window)) = closed {
            self.native_menu_events
                .push(NativeMenuEvent::Closed { uuid, window });
        }
    }

    pub(super) fn close_container_for(&mut self, i: usize) {
        self.close_container_at(i);
    }

    pub fn close_container(&mut self, conn: u32) {
        if let Some(i) = self.guests.iter().position(|g| g.conn == conn) {
            self.close_container_at(i);
        }
    }

    /// The client closed its window.
    pub fn container_closed(&mut self, conn: u32, window: i32) {
        let closed = self
            .update_guest(conn, |g| {
                if !g.menu.as_ref().is_some_and(|m| m.window == window) {
                    return None;
                }
                let native = g.menu.take().is_some_and(|m| m.slots == Slots::Native);
                if native {
                    g.native_menu.clear();
                    Some(g.uuid)
                } else {
                    None
                }
            })
            .flatten();
        if let Some(uuid) = closed {
            self.native_menu_events
                .push(NativeMenuEvent::Closed { uuid, window });
        }
    }

    fn hashed_matches(&mut self, claimed: &Option<Hashed>, mine: &Option<ItemStack>) -> bool {
        match (claimed, mine) {
            (None, None) => true,
            (Some(h), Some(s)) => h.item == self.item_id(&s.item) && h.count == s.count as i32,
            _ => false,
        }
    }

    /// The slot's stack as the menu holds it: borrowed where its storage can
    /// lend it, so a tick's comparison copies nothing.
    fn view_slot_ref<'a>(&'a self, i: usize, world: &'a World, slot: usize) -> SlotRef<'a> {
        let g = &self.guests[i];
        match &g.menu {
            Some(m) if slot < m.size => m.slots.get_ref(world, g, slot),
            Some(m) => SlotRef::Borrowed(
                g.inventory
                    .get(player_slot(slot - m.size))
                    .and_then(|s| s.as_ref()),
            ),
            None => SlotRef::Borrowed(g.menu_slot_ref(slot)),
        }
    }

    fn view_slot(&self, i: usize, world: &World, slot: usize) -> Option<ItemStack> {
        self.view_slot_ref(i, world, slot).into_stack()
    }

    /// ServerGamePacketListenerImpl.handleContainerClick. Returns the stacks
    /// thrown out of the menu. One call for the three steps the game runs apart when it owns
    /// the click (`prepare_click`, `run_click`, `finish_click`).
    pub fn container_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
        world: &mut World,
        rules: &dyn Rules,
    ) -> Vec<ItemStack> {
        let Some(mut prepared) = self.prepare_click(conn, p, world) else {
            return Vec::new();
        };
        let outcome = self.run_click(&mut prepared, p, rules);
        self.finish_click(prepared, p, world, outcome)
    }

    /// The first step of a container click: what the click may do, checked against the world,
    /// and the container's slots read from it. `None` is a click that changes nothing (the
    /// guest is gone, the window is not theirs, or the menu is no longer valid), already answered
    /// where vanilla answers it. Only this step and `finish_click` touch the world.
    pub fn prepare_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
        world: &World,
    ) -> Option<PreparedClick> {
        let i = self.guests.iter().position(|g| g.conn == conn)?;
        let in_menu = p.container != 0;
        let window_ok = !in_menu
            || self.guests[i]
                .menu
                .as_ref()
                .is_some_and(|m| m.window == p.container);
        if !window_ok {
            return None;
        }
        if self.guests[i].spectator() {
            // A spectator's click changes nothing, and sendAllDataToRemote
            // takes back what its client predicted.
            self.resend_menu(conn, p.container, world);
            return None;
        }
        // A native mod's menu: the click is the mod's to answer, and the
        // whole menu and the cursor go back as they were.
        if in_menu && self.guests[i].menu.as_ref().map(|m| m.slots) == Some(Slots::Native) {
            self.native_menu_events.push(NativeMenuEvent::Click {
                uuid: self.guests[i].uuid,
                window: p.container,
                slot: i32::from(p.slot),
                button: i32::from(p.button),
                mode: p.input,
            });
            self.resend_menu(conn, p.container, world);
            return None;
        }
        // A menu that is no longer valid takes no clicks.
        if let Some(m) = self.guests[i].menu.as_ref().filter(|_| in_menu)
            && !m.still_valid(&self.loop_context, world, &self.guests[i])
        {
            return None;
        }
        // AbstractContainerMenu.isValidSlotIndex: -999 or below the menu's slot
        // count. A slot past the menu would reach the inventory beyond its 36 slots.
        let slots = match self.guests[i].menu.as_ref().filter(|_| in_menu) {
            Some(m) => m.size + 36,
            None => 46,
        };
        if i32::from(p.slot) >= slots as i32 && p.slot != -999 {
            return None;
        }
        let source = self.guests[i].menu.as_ref().map(|m| m.slots);
        let shulker_box = self
            .sync
            .builtin
            .get("minecraft:menu")
            .and_then(|m| m.id("minecraft:shulker_box"));
        let shulker = in_menu
            && shulker_box.is_some()
            && self.guests[i].menu.as_ref().map(|m| m.kind) == shulker_box;
        let (drag, hidden, items, container_limits) = self.guests.update(i, |guest| {
            let drag = if in_menu {
                guest.menu.as_mut().and_then(|m| m.drag.take())
            } else {
                guest.drag.take()
            };
            let hidden = guest
                .menu
                .as_ref()
                .filter(|_| in_menu)
                .map_or_else(Vec::new, |m| m.hidden.clone());
            let items = source.and_then(|s| s.read(world, guest));
            if in_menu && items.is_none() {
                return None;
            }
            let container_limits = source
                .map(|source| source.stack_limits(world, items.as_ref().map_or(0, Vec::len)))
                .unwrap_or_default();
            Some((drag, hidden, items, container_limits))
        })?;
        Some(PreparedClick {
            i,
            in_menu,
            source,
            shulker,
            drag,
            hidden,
            items,
            container_limits,
            sounds: Vec::new(),
        })
    }

    /// The second step: the click itself, on the guest and the container's slots the first step
    /// read. It takes no world and no game, so the caller may lend the menu code anything that
    /// does not alias the hub. Returns the stacks thrown out and the slots that changed.
    pub fn run_click(
        &mut self,
        prepared: &mut PreparedClick,
        p: &play::ContainerClick,
        rules: &dyn Rules,
    ) -> (Vec<ItemStack>, Vec<usize>) {
        let loop_context = &self.loop_context;
        self.guests.update(prepared.i, |guest| {
            let mut view = View {
                guest,
                container: prepared.items.as_deref_mut(),
                shulker: prepared.shulker,
                container_limits: std::mem::take(&mut prepared.container_limits),
                hidden: &prepared.hidden,
                sounds: Vec::new(),
                loop_context,
                slot_snapshot: Default::default(),
            };
            let outcome = menu::click(
                &mut view,
                &mut prepared.drag,
                i32::from(p.slot),
                i32::from(p.button),
                p.input,
                rules,
            );
            prepared.sounds = std::mem::take(&mut view.sounds);
            outcome
        })
    }

    /// The last step: the container's slots back into the world, the unfinished drag back on the
    /// menu, and the client told what it got wrong. Returns the stacks the click threw out.
    pub fn finish_click(
        &mut self,
        prepared: PreparedClick,
        p: &play::ContainerClick,
        world: &mut World,
        (drops, changed): (Vec<ItemStack>, Vec<usize>),
    ) -> Vec<ItemStack> {
        let PreparedClick {
            i,
            in_menu,
            source,
            drag,
            items,
            sounds,
            ..
        } = prepared;
        self.guests.update(i, |guest| {
            if let (Some(source), Some(items)) = (source, items) {
                source.write(world, guest, items);
            }
            if in_menu {
                if let Some(m) = guest.menu.as_mut() {
                    m.drag = drag;
                }
            } else {
                guest.drag = drag;
            }
        });
        // BundleItem's sounds are `Player.playSound`: at the player, in the
        // players' source, for everyone near but the clicker, whose client
        // played its own prediction. Sent during the click, before its answer.
        let (conn, at, players) = (
            self.guests[i].conn,
            self.guests[i].position,
            player_source(),
        );
        for name in sounds {
            let (volume, pitch) =
                crate::bundle::sound_volume_pitch(name, || world.random.next_float());
            self.sound(name, at, volume, pitch, players, Some(conn));
        }
        self.answer_click(i, p, world, &changed);
        drops
    }

    /// `AbstractContainerMenu.setSelectedBundleItemIndex` on the guest's open
    /// menu: a container's own slots, or the inventory's. A bundle's selection
    /// lives on the server's stack until the click that takes from it.
    pub fn select_bundle_item(&mut self, conn: u32, slot: usize, index: i32, world: &mut World) {
        let _ = self.update_guest(conn, |g| {
            let open = g.menu.as_ref().map(|m| (m.size, m.slots));
            let mut items = match open {
                Some((size, source)) if slot < size => source.read(world, g),
                _ => None,
            };
            let stack = match open {
                Some((size, _)) if slot < size => items
                    .as_mut()
                    .and_then(|items| items.get_mut(slot))
                    .and_then(Option::as_mut),
                // setSelectedBundleItemIndex: a slot of the menu only.
                Some((size, _)) if slot - size < 36 => g
                    .inventory
                    .get_mut(player_slot(slot - size))
                    .and_then(Option::as_mut),
                Some(_) => None,
                None => match slot {
                    9..=35 => g.inventory.get_mut(slot).and_then(Option::as_mut),
                    36..=44 => g.inventory.get_mut(slot - 36).and_then(Option::as_mut),
                    45 => g.offhand.as_mut(),
                    _ => None,
                },
            };
            if let Some(stack) = stack {
                crate::bundle::toggle_selected(stack, index);
            }
            if let (Some((_, source)), Some(items)) = (open, items) {
                source.write(world, g, items);
            }
        });
    }

    /// A click on the inventory menu (window 0) the game applied itself
    /// (the crafting grid): the client is corrected like after any click.
    pub fn inventory_clicked(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
        world: &World,
        changed: &[usize],
    ) {
        if let Some(i) = self.guests.iter().position(|g| g.conn == conn) {
            self.answer_click(i, p, world, changed);
        }
    }

    /// What the client predicted for click `p` is what the server has, for a
    /// menu of `total` slots read through `view`.
    pub fn click_agreed(
        &mut self,
        i: usize,
        p: &play::ContainerClick,
        total: usize,
        view: &dyn Fn(usize) -> Option<ItemStack>,
        changed: &[usize],
    ) -> bool {
        let carried = self.guests[i].carried.clone();
        if !self.hashed_matches(&p.carried, &carried) {
            return false;
        }
        let claimed: HashMap<usize, &Option<Hashed>> =
            p.changed.iter().map(|(s, h)| (*s as usize, h)).collect();
        (0..total).all(|slot| match claimed.get(&slot) {
            Some(c) => self.hashed_matches(c, &view(slot)),
            None => !changed.contains(&slot),
        })
    }

    /// After a click: tell the client about every slot it got wrong, or that
    /// it did not claim, and the carried stack.
    fn answer_click(
        &mut self,
        i: usize,
        p: &play::ContainerClick,
        world: &World,
        changed: &[usize],
    ) {
        let in_menu = p.container != 0;
        // handleContainerClick: a click made without the latest state gets the
        // whole menu (`broadcastFullState`). Answering it slot by slot can leave
        // the client with a correction of a click it has already gone past.
        let g = &self.guests[i];
        let open = g
            .menu
            .as_ref()
            .filter(|_| in_menu)
            .map(|m| (m.size, m.state));
        let stale = match open {
            Some((_, state)) => p.state != state,
            None => !in_menu && p.state != g.menu_state,
        };
        if stale {
            match open {
                Some((size, _)) => {
                    let items = (0..size).map(|s| self.view_slot(i, world, s)).collect();
                    self.send_menu_content(i, items);
                }
                None => self.send_inventory(i),
            }
            return;
        }
        let claimed: HashMap<usize, Option<Hashed>> = p
            .changed
            .iter()
            .map(|(s, h)| (*s as usize, h.clone()))
            .collect();
        let total = if in_menu {
            self.guests[i].menu.as_ref().map_or(0, |m| m.size) + 36
        } else {
            46
        };
        let mut resend: Vec<usize> = Vec::new();
        for slot in 0..total {
            let mine = self.view_slot(i, world, slot);
            let agreed = match claimed.get(&slot) {
                Some(c) => self.hashed_matches(c, &mine),
                None => !changed.contains(&slot),
            };
            if !agreed {
                resend.push(slot);
            }
        }
        for slot in resend {
            let mine = self.view_slot(i, world, slot);
            let net = self.stack(&mine);
            self.guests.update(i, |g| {
                let state = if in_menu {
                    g.menu.as_mut().map_or(0, |m| {
                        m.state = (m.state + 1) & 0x7FFF;
                        m.state
                    })
                } else {
                    g.menu_state = (g.menu_state + 1) & 0x7FFF;
                    g.menu_state
                };
                g.send(&C::ContainerSetSlot(play::ContainerSetSlot {
                    container: p.container,
                    state,
                    slot: slot as i16,
                    item: net,
                }));
            });
        }
        let carried = self.guests[i].carried.clone();
        if !self.hashed_matches(&p.carried, &carried) {
            let net = self.stack(&carried);
            self.guests[i].send(&C::SetCursorItem(play::SetCursorItem { item: net }));
        }
        // What the client now believes equals the server's state.
        let items: Vec<_> = (0..total).map(|s| self.view_slot(i, world, s)).collect();
        self.guests.update(i, |g| {
            if in_menu {
                // Nor is the inventory menu's sync to send what this click settled:
                // ServerPlayer.tick broadcasts only the open menu.
                let own = open.map_or(0, |(size, _)| size);
                let settled = claimed.keys().copied().chain(changed.iter().copied());
                for slot in settled.filter(|&s| s >= own && s < own + 36) {
                    g.menu_sent[slot - own + 9] = items[slot].clone();
                }
                if let Some(m) = g.menu.as_mut() {
                    m.sent = items;
                }
            } else {
                g.menu_sent = items;
            }
        });
    }

    /// AbstractContainerMenu.broadcastChanges' data slots: a furnace's
    /// AbstractFurnaceBlockEntity.dataAccess (lit time left, lit total, cooking
    /// timer, cooking total), each sent when it changes.
    fn sync_container_data(&mut self, i: usize, world: &World) {
        self.guests.update(i, |g| {
            let Some(m) = g.menu.as_mut() else {
                return;
            };
            let Slots::Block(pos) = m.slots else {
                return;
            };
            // AbstractFurnaceBlockEntity.dataAccess, BrewingStandBlockEntity.dataAccess,
            // CrafterBlockEntity.containerData.
            let data = match world.block_entities.get(&pos) {
                Some(BlockEntity::Furnace(f)) => vec![
                    f.lit_remaining,
                    f.lit_total,
                    f.cooking_timer,
                    f.cooking_total,
                ],
                Some(BlockEntity::Brewing(b)) => {
                    vec![b.brew_time, b.fuel, b.total_brew_time, b.total_fuel]
                }
                // CrafterBlockEntity.containerData: the disabled slots, then triggered.
                Some(BlockEntity::Crafter(c)) => c
                    .disabled
                    .iter()
                    .chain([&c.is_triggered()])
                    .map(|&d| i32::from(d))
                    .collect(),
                // A mod's menu: the data slots its tick set. A mod menu type's
                // `ContainerData` has as many as it declares, all sent on open
                // (`sendInitialData`), 0 where the tick set none.
                Some(BlockEntity::Mod(e)) if m.mod_type.is_some() || !e.data.is_empty() => {
                    let mut data = e.data.clone();
                    if let Some(t) = m.mod_type {
                        data.resize(t.data_slots, 0);
                    }
                    data
                }
                _ => return,
            }
            .into_iter()
            .map(|v| v as i16)
            .collect::<Vec<i16>>();
            m.sent_data.resize(data.len(), None);
            let window = m.window;
            let changed: Vec<(usize, i16)> = data
                .iter()
                .enumerate()
                .filter(|&(k, v)| m.sent_data[k] != Some(*v))
                .map(|(k, &v)| (k, v))
                .collect();
            for &(k, value) in &changed {
                m.sent_data[k] = Some(value);
            }
            for (k, value) in changed {
                g.send(&C::ContainerSetData(play::ContainerSetData {
                    container: window,
                    id: k as i16,
                    value,
                }));
            }
        });
    }

    /// Slot changes of open containers (AbstractContainerMenu.broadcastChanges).
    pub fn sync_containers(&mut self, world: &World) {
        for i in 0..self.guests.len() {
            let Some(menu) = self.guests[i]
                .menu
                .as_ref()
                .filter(|_| self.here(&self.guests[i]))
            else {
                continue;
            };
            let (size, window, slots) = (menu.size, menu.window, menu.slots);
            if !slots.valid(world) {
                self.close_container_at(i);
                continue;
            }
            for slot in 0..size + 36 {
                let now = self.view_slot_ref(i, world, slot);
                let sent = self.guests[i].menu.as_ref().and_then(|m| m.sent.get(slot));
                if now.matches_sent(sent) {
                    continue;
                }
                let now = now.into_stack();
                let net = self.stack(&now);
                self.guests.update(i, |g| {
                    if let Some(m) = g.menu.as_mut() {
                        m.state = (m.state + 1) & 0x7FFF;
                        let state = m.state;
                        if m.sent.len() <= slot {
                            m.sent.resize(slot + 1, None);
                        }
                        m.sent[slot] = now;
                        g.send(&C::ContainerSetSlot(play::ContainerSetSlot {
                            container: window,
                            state,
                            slot: slot as i16,
                            item: net,
                        }));
                    }
                });
            }
            // Then the data slots, as broadcastChanges does.
            self.sync_container_data(i, world);
        }
    }
}

/// Inventory index of the n-th player slot of a container menu (27 main, then the hotbar).
fn player_slot(n: usize) -> usize {
    if n < 27 { 9 + n } else { n - 27 }
}

/// `Level.soundSeedGenerator.nextLong` for `ClientboundSoundPacket.seed`.
fn next_sound_seed(g: &mut impl RandomSource) -> i64 {
    g.next_long()
}

/// `PlayerList.broadcast`'s distance test: the summed squared axis distances
/// against the squared radius, and a player exactly at the radius is out.
pub fn within_range(at: [f64; 3], pos: [f64; 3], range: f64) -> bool {
    let d: f64 = (0..3).map(|k| (pos[k] - at[k]).powi(2)).sum();
    d < range * range
}

/// `SoundSource.PLAYERS`, by its index.
fn player_source() -> i32 {
    horizon_client_core::sound_source::SOUND_SOURCES
        .iter()
        .position(|s| *s == "player")
        .unwrap_or(0) as i32
}

/// The position `ServerLevel.globalLevelEvent` sends a player whose feet are
/// at `player` for an event at `pos` (its lambda): `pos` for a player in the
/// event's level (`same_level`) whose squared distance to the block's centre is
/// below `Mth.square(32)`, the block 32 blocks from a further one towards that
/// centre, and the block a player in another level stands in. The client
/// places the sound from it (`LevelEventHandler.globalLevelEvent`).
fn global_level_event_position(pos: [i32; 3], player: [f64; 3], same_level: bool) -> [i32; 3] {
    use horizon_core::{geometry::Vec3, position::BlockPos};
    let feet = Vec3::new(player[0], player[1], player[2]);
    let at = if same_level {
        let center = Vec3::new(
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        );
        // Entity.distanceToSqr against the int square widened (`i2d`, `dcmpg`, `ifge`).
        if feet.distance_squared(center) < f64::from(32 * 32) {
            center
        } else {
            feet.add_vector(center.subtract(feet).normalize().scale(32.0))
        }
    } else {
        feet
    };
    let block = BlockPos::containing(at);
    [block.x, block.y, block.z]
}

#[cfg(test)]
mod tests {
    use super::{next_sound_seed, within_range};
    use horizon_core::random::LegacyRandom;

    #[test]
    fn two_sounds_in_one_tick_get_different_seeds() {
        let mut g = LegacyRandom::new(0);
        let a = next_sound_seed(&mut g);
        let b = next_sound_seed(&mut g);
        assert_ne!(a, b);
        assert_ne!(a, 0);
        assert_ne!(b, 0);
    }

    #[test]
    fn the_tick_is_not_the_sound_seed() {
        let ticks = 12i64;
        let mut g = LegacyRandom::new(0);
        assert_ne!(next_sound_seed(&mut g), ticks);
    }

    /// `PlayerList.broadcast` compares the squared distances with `ifge`, so a
    /// player exactly on the radius gets no packet while one a hair inside does.
    #[test]
    fn a_guest_exactly_on_the_radius_hears_nothing() {
        let at = [10.0, 64.0, -3.0];
        let at_radius = [at[0] + 16.0, at[1], at[2]];
        assert!(!within_range(at, at_radius, 16.0));
        assert!(within_range(at, [at[0] + 15.99, at[1], at[2]], 16.0));
        assert!(within_range(at, at, 16.0));
    }
}
