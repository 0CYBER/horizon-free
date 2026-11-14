//! Handles to supported menus: shared reads, server storage and client clicks.
use super::{
    Game,
    lan_crafting::{load_player, machine_menu},
};
use horizon_client::{
    containers::BlockEntity,
    items::ItemStack,
    lan::{
        guest::Guest,
        menu::{SlotBacking, Slots, VEHICLE_KEY_Y},
    },
    menu::{Input, Kind, Menu},
};
use mod_api::__private::game::*;
use mod_api::{
    DragKind, EntityRef, MenuClick, MenuRef, MenuSlotBacking, MouseButton, PlayerRef, SwapWith,
};

impl Game<'_> {
    /// Submit clicks on the local player's live menu through normal prediction
    /// and `ContainerClick`. Slots use the same menu order as `slot`; `None`
    /// is outside (-999). All clicks are validated before any prediction or send.
    /// A drag expands to one complete quick-craft gesture.
    /// Refuses stale menus, invalid slots/hotbar indices, spectators, non-client hosts,
    /// the creative inventory screen and empty batches without changing items.
    /// True means submitted, not accepted by the server: its corrections win.
    pub(super) fn menu_click(&mut self, menu: MenuRef, clicks: &[MenuClick]) -> bool {
        if self.is_logical_server() || self.screen == super::Screen::Creative || self.spectator() {
            return false;
        }
        let Some(name) = self.native_client_menu_name(menu) else {
            return false;
        };
        let Some(ops) = client_click_ops(name, clicks, self.instabuild()) else {
            return false;
        };
        self.remote_menu_ops(&ops)
    }

    /// Close the local player's open container screen as Escape does
    /// (`AbstractContainerScreen.onClose`, `LocalPlayer.closeContainer`): the
    /// server is told (`ContainerClose`) and the screen goes. Refuses stale
    /// menus, non-client hosts and a menu whose screen is not open, such as
    /// the inventory menu while playing.
    pub(super) fn menu_close(&mut self, menu: MenuRef) -> bool {
        if self.is_logical_server()
            || self.screen != super::Screen::Inventory
            || self.native_client_menu_name(menu).is_none()
        {
            return false;
        }
        self.close_container();
        self.set_screen(super::Screen::Playing);
        self.capture(true);
        true
    }

    /// `ServerPlayer.closeContainer` of a guest's menu: every menu they have open closes through
    /// the path a disconnect and a `stillValid` failure use, then their client is told. Refuses
    /// clients, stale menus, a player swapped in for a turn, and the inventory menu (window 0),
    /// which is never closed.
    pub(super) fn menu_close_container(&mut self, menu: MenuRef) -> bool {
        if !self.is_logical_server() || menu.fields().window == 0 {
            return false;
        }
        let Some(conn) = self.native_menu_guest(menu).map(|guest| guest.conn) else {
            return false;
        };
        self.guest_close_menus_and_tell(conn);
        true
    }

    /// Press button `button` of the local player's open menu as a click on
    /// one of its buttons does (`MultiPlayerGameMode.handleInventoryButtonClick`,
    /// `ContainerButtonClick`): the server's menu decides what it does
    /// (`clickMenuButton`, and `menu_button` for a mod's menu). For a client
    /// mod's own input on a menu's screen where `screen_click` doesn't come,
    /// such as a key or the wheel; no sound, that is the screen's. Refuses
    /// stale menus, non-client hosts and the inventory menu, which no server
    /// opened. True means sent, not that the menu took it.
    pub(super) fn menu_click_button(&mut self, menu: MenuRef, button: i32) -> bool {
        !self.is_logical_server()
            && self.native_client_menu_name(menu).is_some()
            && self.remote_button_click(button)
    }

    pub(super) fn menu_slot_boxes(&self, menu: MenuRef) -> Option<Vec<Option<[f32; 4]>>> {
        if self.screen != super::Screen::Inventory || !self.screen_hooks_open() {
            return None;
        }
        let name = self.native_client_menu_name(menu)?;
        let renderer = self.renderer.as_ref()?;
        let batch = super::HudBatch::new(
            [renderer.config.width, renderer.config.height],
            self.settings.gui_scale_inputs(),
        );
        self.inventory_menu_slot_boxes(name, &batch)
    }

    fn inventory_menu_slot_boxes(
        &self,
        name: &str,
        batch: &super::HudBatch,
    ) -> Option<Vec<Option<[f32; 4]>>> {
        Self::inventory_screen_slot(name, 0)?;
        if self.book_covers_menu(batch.width) {
            return None;
        }
        let visible = self.slots();
        let (ox, oy) = self.inventory_origin(batch);
        Some(
            (0..)
                .map_while(|slot| Self::inventory_screen_slot(name, slot))
                .map(|slot| {
                    visible.contains(&slot).then(|| {
                        let (x, y) = self.slot_position(slot);
                        [ox + x, oy + y, 16.0, 16.0]
                    })
                })
                .collect(),
        )
    }

    fn native_client_menu_name(&self, menu: MenuRef) -> Option<&str> {
        let remote = self.remote.as_ref().filter(|remote| remote.logged_in)?;
        let epoch = self.native_lifecycle.epoch()?;
        (menu.fields().player.fields().entity.epoch() == epoch
            && menu.fields().player.fields().entity.id()
                == u32::try_from(remote.entity_id).ok()?
            && menu.fields().player.fields().uuid == remote.profile
            && menu.fields().generation == remote.menu.generation
            && menu.fields().window == remote.menu.as_ref().map_or(0, |entry| entry.0))
        .then_some(())?;
        Some(
            remote
                .menu
                .as_ref()
                .map_or("inventory", |entry| entry.1.as_str()),
        )
    }

    /// The local player in a logged-in client world.
    pub(super) fn local_player(&self) -> Option<PlayerRef> {
        let remote = self.remote.as_ref().filter(|remote| remote.logged_in)?;
        self.player(remote.profile)
    }

    fn native_menu_guest(&self, menu: MenuRef) -> Option<&Guest> {
        let super::native_server::PlayerRead::Guest(guest) =
            self.player_handle_read(menu.fields().player)?
        else {
            return None;
        };
        let host = self.lan.as_ref()?;
        (host.hub.holder_guest(guest.uuid, self.dim.id())?.conn == guest.conn).then_some(())?;
        (guest.menu_generation == menu.fields().generation).then_some(())?;
        if let Some(open) = &guest.menu {
            (open.window == menu.fields().window && open.hidden.is_empty()).then_some(guest)
        } else {
            let play = host.play.get(&guest.conn);
            let menus = host.menus.get(&guest.conn);
            if menu.fields().window != 0
                || menus.is_some_and(|m| m.crafting.table.is_some() || m.station.is_some())
                || play.is_some_and(|p| {
                    p.merchant.is_some() || p.lectern.is_some() || p.mount_window.is_some()
                })
            {
                return None;
            }
            Some(guest)
        }
    }

    /// A player's current supported menu. Refuses crafting tables, merchants,
    /// workstations, lecterns, mounts and menus with hidden slots; also refuses
    /// clients, stale players and players whose state is swapped out.
    pub(super) fn player_container_menu(&self, player: PlayerRef) -> Option<MenuRef> {
        if self.remote.is_some() {
            let remote = self.remote.as_ref()?;
            (self.local_player()? == player).then_some(())?;
            let name = remote
                .menu
                .as_ref()
                .map_or("inventory", |entry| entry.1.as_str());
            Self::inventory_screen_slot(name, 0)?;
            return Some(mod_api::__private::game::menu_ref(
                player,
                remote.menu.as_ref().map_or(0, |entry| entry.0),
                remote.menu.generation,
            ));
        }
        let super::native_server::PlayerRead::Guest(guest) = self.player_handle_read(player)?
        else {
            return None;
        };
        let menu = mod_api::__private::game::menu_ref(
            player,
            guest.menu.as_ref().map_or(0, |m| m.window),
            guest.menu_generation,
        );
        self.native_menu_guest(menu)?;
        Some(menu)
    }

    /// Container id (0 for the player's inventory menu).
    pub(super) fn menu_container_id(&self, menu: MenuRef) -> Option<i32> {
        self.native_menu_guest(menu)?;
        Some(menu.fields().window)
    }

    /// Registry menu type; the inventory menu has no MenuType in vanilla.
    /// The outer None means an unavailable menu.
    pub(super) fn menu_kind(&self, menu: MenuRef) -> Option<Option<String>> {
        let guest = self.native_menu_guest(menu)?;
        match &guest.menu {
            Some(m) => Some(Some(
                self.lan
                    .as_ref()?
                    .hub
                    .registry_name("minecraft:menu", m.kind)?
                    .to_owned(),
            )),
            None => Some(None),
        }
    }

    /// Number of slots in AbstractContainerMenu.slots.
    pub(super) fn menu_slot_count(&self, menu: MenuRef) -> Option<usize> {
        if self.remote.is_some() {
            let name = self.native_client_menu_name(menu)?;
            return Some(
                (0..)
                    .map_while(|slot| Self::inventory_screen_slot(name, slot))
                    .count(),
            );
        }
        let guest = self.native_menu_guest(menu)?;
        Some(guest.menu.as_ref().map_or(46, |m| m.size + 36))
    }

    /// The storage menu slot `slot` shows (`Slot.container` and `getContainerSlot`): the
    /// player's inventory by `ENTITY_ITEMS` index (`InventoryMenu`'s armour slots 5..9 go head
    /// to feet), a block entity's or an entity's slot, the ender chest's, the crafting grid's
    /// or a mod's chest menu's.
    pub(super) fn menu_slot_backing(&self, menu: MenuRef, slot: usize) -> Option<MenuSlotBacking> {
        let guest = self.native_menu_guest(menu)?;
        let Some(open) = &guest.menu else {
            return match slot {
                0 => Some(MenuSlotBacking::CraftResult),
                1..=4 => Some(MenuSlotBacking::CraftGrid(slot - 1)),
                5..=8 => Some(MenuSlotBacking::Player(39 - (slot - 5))),
                9..=35 => Some(MenuSlotBacking::Player(slot)),
                36..=44 => Some(MenuSlotBacking::Player(slot - 36)),
                45 => Some(MenuSlotBacking::Player(40)),
                _ => None,
            };
        };
        // A block menu whose block entities no longer show its slots (a chest's, over a
        // furnace set in its place) backs none of them, the player's included, until the close
        // pass ends it (`Container.stillValid`).
        let level = self.level(&guest.dimension)?;
        let world = self.native_level_world(level)?;
        if open
            .slots
            .block_slots(world)
            .is_some_and(|n| n != open.size)
        {
            return None;
        }
        if slot >= open.size {
            return match slot - open.size {
                n @ 0..=26 => Some(MenuSlotBacking::Player(n + 9)),
                n @ 27..=35 => Some(MenuSlotBacking::Player(n - 27)),
                _ => None,
            };
        }
        Some(match open.slots.backing(world, slot)? {
            SlotBacking::Block(pos, n) if pos.1 == VEHICLE_KEY_Y => MenuSlotBacking::Entity {
                entity: EntityRef::new(level.epoch(), pos.2 as u32),
                slot: n,
            },
            SlotBacking::Block(pos, n) => MenuSlotBacking::BlockEntity {
                level,
                pos: [pos.0, pos.1, pos.2],
                slot: n,
            },
            SlotBacking::Ender(n) => MenuSlotBacking::Ender(n),
            // Only a slot a mod backed (`player_back_mod_menu_slots`) reads as a place.
            SlotBacking::Native(n) => match self.native_menu_place(guest.conn, open.window, n) {
                Some(place) => MenuSlotBacking::Place(place),
                None => MenuSlotBacking::Native(n),
            },
        })
    }

    /// Full stack at a menu index. Outer None means unavailable or out of
    /// range; Some(None) means an empty slot.
    pub(super) fn menu_slot(&self, menu: MenuRef, slot: usize) -> Option<Option<ItemStack>> {
        if self.remote.is_some() {
            let name = self.native_client_menu_name(menu)?;
            let index = Self::inventory_screen_slot(name, i32::try_from(slot).ok()?)?;
            return Some(self.slot_view(index));
        }
        let backing = self.menu_slot_backing(menu, slot)?;
        let guest = self.native_menu_guest(menu)?;
        Some(match backing {
            MenuSlotBacking::BlockEntity { .. } | MenuSlotBacking::Entity { .. } => {
                let world = self.native_level_world(self.level(&guest.dimension)?)?;
                guest.menu.as_ref()?.slots.get(world, guest, slot)
            }
            MenuSlotBacking::Player(n @ 0..36) => guest.inventory.get(n)?.clone(),
            MenuSlotBacking::Player(n @ 36..40) => guest.armor[39 - n].clone(),
            MenuSlotBacking::Player(_) => guest.offhand.clone(),
            MenuSlotBacking::CraftGrid(n) => guest.craft_grid.get(n)?.clone(),
            MenuSlotBacking::CraftResult => guest.craft_result.clone(),
            MenuSlotBacking::Ender(n) => guest.ender_items.get(n)?.clone(),
            MenuSlotBacking::Native(n) => guest.native_menu.get(n)?.clone(),
            // The place's mirror, refreshed after each click and each tick.
            MenuSlotBacking::Place(_) => guest.native_menu.get(slot)?.clone(),
            _ => unreachable!("MenuSlotBacking: unhandled variant"),
        })
    }

    /// How many of `stack` menu slot `slot` holds (`Slot.mayPlace`, then `Slot.getMaxStackSize`):
    /// a furnace's fuel slot one bucket, an armour slot one piece that fits it, a crafting result
    /// none.
    pub(super) fn menu_slot_holds(
        &self,
        menu: MenuRef,
        slot: usize,
        stack: &ItemStack,
    ) -> Option<u32> {
        let rules = self.native_menu_rules(menu, slot)?;
        Some(rules.slot_holds(&self.menu_env(), slot, stack))
    }

    /// Whether menu slot `slot` gives up what it holds (`Slot.mayPickup`): bound armour stays
    /// on outside creative.
    pub(super) fn menu_slot_gives(&self, menu: MenuRef, slot: usize) -> Option<bool> {
        let rules = self.native_menu_rules(menu, slot)?;
        Some(rules.slot_gives(&self.menu_env(), slot))
    }

    /// The menu engine's model of `menu`, holding slot `slot`, for its slots' rules: the
    /// inventory menu, a furnace's, brewing stand's, crafter's or content mod's menu over its
    /// block entity, a shulker box's, or a plain container of the menu's size. None for a slot
    /// with no backing (`menu_slot_backing`), and for one the model hasn't.
    fn native_menu_rules(&self, menu: MenuRef, slot: usize) -> Option<Box<Menu>> {
        self.menu_slot_backing(menu, slot)?;
        let guest = self.native_menu_guest(menu)?;
        let (kind, disabled, menu_id) = match &guest.menu {
            None => (Kind::Inventory, None, None),
            Some(open) => {
                let world = self.native_level_world(self.level(&guest.dimension)?)?;
                let name = self
                    .lan
                    .as_ref()?
                    .hub
                    .registry_name("minecraft:menu", open.kind);
                let entity = match open.slots {
                    Slots::Block(pos) => world.block_entities.get(&pos),
                    _ => None,
                };
                let kind = match entity.and_then(|e| machine_menu(e, open.mod_type)) {
                    Some((kind, _)) => kind,
                    None if name == Some("minecraft:shulker_box") => Kind::Shulker,
                    None if open.size == 5 => Kind::Hopper,
                    None if open.size == 9 => Kind::Dispenser,
                    None => Kind::Chest(open.size / 9),
                };
                // `CrafterSlot.mayPlace`: a disabled slot takes nothing.
                let disabled = match entity {
                    Some(BlockEntity::Crafter(c)) => Some(c.disabled),
                    _ => None,
                };
                (kind, disabled, name)
            }
        };
        let mut rules = Box::new(Menu::new(kind));
        rules.set_registry_id(menu_id);
        if let Some(open) = guest.menu.as_ref() {
            let world = self.native_level_world(self.level(&guest.dimension)?)?;
            let limits = open.slots.stack_limits(world, open.size);
            rules.set_container_limits(limits);
        }
        (slot < rules.slot_count()).then_some(())?;
        load_player(&mut rules, guest);
        for (k, off) in disabled.iter().flatten().enumerate() {
            rules.data[k] = i32::from(*off);
        }
        Some(rules)
    }

    /// The game's own storage menu slot `slot` shows, for a write: the ender chest's, the
    /// crafting grid's or a mod's chest menu's slot, kept on the guest. None for any other.
    pub(super) fn native_menu_storage(
        &mut self,
        menu: MenuRef,
        slot: usize,
    ) -> Option<&mut Option<ItemStack>> {
        let backing = self.menu_slot_backing(menu, slot)?;
        let guests = &mut self.lan.as_mut()?.hub.guests;
        let index = guests.iter().position(|g| {
            g.placed
                && g.uuid == menu.fields().player.uuid()
                && g.entity_id as u32 == menu.fields().player.entity().id()
        })?;
        match backing {
            MenuSlotBacking::Ender(n) => guests.ender_items_mut(index)?.get_mut(n),
            MenuSlotBacking::CraftGrid(n) => guests.craft_grid_mut(index)?.get_mut(n),
            MenuSlotBacking::Native(n) => guests.native_menu_mut(index)?.get_mut(n),
            _ => None,
        }
    }

    /// Carried (cursor) stack; outer None means unavailable.
    pub(super) fn menu_carried(&self, menu: MenuRef) -> Option<Option<ItemStack>> {
        if self.remote.is_some() {
            self.native_client_menu_name(menu)?;
            return Some(self.carried.clone());
        }
        Some(self.native_menu_guest(menu)?.carried.clone())
    }
}
fn client_click_ops(
    menu: &str,
    clicks: &[MenuClick],
    creative: bool,
) -> Option<Vec<(Option<usize>, i32, Input)>> {
    if clicks.is_empty() {
        return None;
    }
    let mapped_slot =
        |slot| Game::inventory_screen_slot(menu, i32::from(i16::try_from(slot).ok()?));
    let mut ops = Vec::new();
    for click in clicks {
        let op = match click {
            MenuClick::Pickup { slot, button } => (
                match slot {
                    Some(slot) => Some(mapped_slot(*slot)?),
                    None => None,
                },
                match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                },
                Input::Pickup,
            ),
            MenuClick::QuickMove { slot } => (Some(mapped_slot(*slot)?), 0, Input::QuickMove),
            MenuClick::Swap { slot, with } => (
                Some(mapped_slot(*slot)?),
                match with {
                    SwapWith::Hotbar(index) if *index <= 8 => i32::from(*index),
                    SwapWith::Offhand => 40,
                    _ => return None,
                },
                Input::Swap,
            ),
            MenuClick::Clone { slot } if creative => (Some(mapped_slot(*slot)?), 2, Input::Clone),
            MenuClick::Throw { slot, whole_stack } => (
                Some(mapped_slot(*slot)?),
                i32::from(*whole_stack),
                Input::Throw,
            ),
            MenuClick::Drag { kind, slots } => {
                let mode = match kind {
                    DragKind::Split => 0,
                    DragKind::One => 4,
                    DragKind::Clone if creative => 8,
                    _ => return None,
                };
                ops.push((None, mode, Input::QuickCraft));
                for slot in slots {
                    ops.push((Some(mapped_slot(*slot)?), mode + 1, Input::QuickCraft));
                }
                ops.push((None, mode + 2, Input::QuickCraft));
                continue;
            }
            MenuClick::PickupAll { slot } => (Some(mapped_slot(*slot)?), 0, Input::PickupAll),
            _ => return None,
        };
        ops.push(op);
    }
    Some(ops)
}

#[cfg(test)]
mod tests {
    use super::super::api_values::TryIntoApi;
    use super::super::{lan::LanSettings, native_entity_refs::tests::with_game};
    use super::*;
    use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    use horizon_protocol::packets::configuration::ClientInformation;
    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize},
    };

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn inventory_boxes_follow_menu_indices_and_the_input_layout() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.screen = super::super::Screen::Inventory;
            let batch = super::super::HudBatch::new([800, 600], game.settings.gui_scale_inputs());
            let boxes = game.inventory_menu_slot_boxes("inventory", &batch).unwrap();
            assert_eq!(boxes.len(), 46);
            for (index, expected_slot) in [(0, 200), (9, 9), (36, 0), (45, 504)] {
                let [x, y, w, h] = boxes[index].unwrap();
                assert_eq!(
                    game.slot_at(&batch, x + w / 2.0, y + h / 2.0),
                    Some(expected_slot)
                );
            }
            assert!(
                game.inventory_menu_slot_boxes("unsupported", &batch)
                    .is_none()
            );
        });
    }

    #[test]
    fn client_click_actions_encode_vanilla_ops() {
        let actions = [
            MenuClick::Pickup {
                slot: Some(36),
                button: MouseButton::Left,
            },
            MenuClick::Pickup {
                slot: None,
                button: MouseButton::Right,
            },
            MenuClick::QuickMove { slot: 9 },
            MenuClick::Swap {
                slot: 9,
                with: SwapWith::Hotbar(8),
            },
            MenuClick::Swap {
                slot: 9,
                with: SwapWith::Offhand,
            },
            MenuClick::Clone { slot: 9 },
            MenuClick::Throw {
                slot: 9,
                whole_stack: false,
            },
            MenuClick::Throw {
                slot: 9,
                whole_stack: true,
            },
            MenuClick::PickupAll { slot: 9 },
        ];
        assert_eq!(
            client_click_ops("inventory", &actions, true),
            Some(vec![
                (Some(0), 0, Input::Pickup),
                (None, 1, Input::Pickup),
                (Some(9), 0, Input::QuickMove),
                (Some(9), 8, Input::Swap),
                (Some(9), 40, Input::Swap),
                (Some(9), 2, Input::Clone),
                (Some(9), 0, Input::Throw),
                (Some(9), 1, Input::Throw),
                (Some(9), 0, Input::PickupAll),
            ])
        );
        for (kind, mode, creative) in [
            (DragKind::Split, 0, false),
            (DragKind::One, 4, false),
            (DragKind::Clone, 8, true),
        ] {
            assert_eq!(
                client_click_ops(
                    "inventory",
                    &[
                        MenuClick::Drag {
                            kind,
                            slots: vec![36, 9]
                        },
                        MenuClick::QuickMove { slot: 9 },
                    ],
                    creative
                ),
                Some(vec![
                    (None, mode, Input::QuickCraft),
                    (Some(0), mode + 1, Input::QuickCraft),
                    (Some(9), mode + 1, Input::QuickCraft),
                    (None, mode + 2, Input::QuickCraft),
                    (Some(9), 0, Input::QuickMove),
                ])
            );
        }
    }

    #[test]
    fn client_click_actions_refuse_invalid_batches_before_prediction() {
        assert!(client_click_ops("inventory", &[], false).is_none());
        for invalid in [
            MenuClick::QuickMove { slot: 46 },
            MenuClick::QuickMove { slot: usize::MAX },
            MenuClick::Swap {
                slot: 9,
                with: SwapWith::Hotbar(9),
            },
            MenuClick::Swap {
                slot: 9,
                with: SwapWith::Hotbar(u8::MAX),
            },
            MenuClick::Clone { slot: 9 },
            MenuClick::Drag {
                kind: DragKind::Clone,
                slots: vec![9],
            },
            MenuClick::Drag {
                kind: DragKind::Split,
                slots: vec![9, usize::MAX],
            },
        ] {
            assert!(
                client_click_ops(
                    "inventory",
                    &[MenuClick::QuickMove { slot: 9 }, invalid,],
                    false
                )
                .is_none()
            );
        }
        assert!(
            client_click_ops("unsupported", &[MenuClick::QuickMove { slot: 0 }], true).is_none()
        );
    }

    fn guest() -> Guest {
        let (tx, _) = std::sync::mpsc::channel();
        let mut g = Guest::new(
            Joined {
                conn: 1,
                uuid: 123,
                name: "guest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            1,
            [0.0; 3],
            10,
        );
        g.placed = true;
        g
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_server_closes_a_guests_menu_and_sets_their_selected_slot() {
        with_game(|game| {
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 4,
                simulation_distance: 4,
                dimension: "minecraft:overworld".into(),
                allow_flight: true,
                seed: 0,
                max_players: 10,
                integrated: true,
                levels: game.level_shapes,
            };
            let mut net = NetConfig::lan(0, "test".into(), 10);
            net.listen = false;
            game.lan_host(
                LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                net,
                config,
            )
            .unwrap();
            game.lan.as_mut().unwrap().hub.guests.push(guest());
            let player = game.player(123).unwrap();

            // The selected slot: a slot past the hotbar is refused, a real one is held.
            assert_eq!(player.selected_slot(game), Some(0));
            assert!(!player.set_selected_slot(game, 9));
            assert!(player.set_selected_slot(game, 4));
            assert!(
                player.set_selected_slot(game, 4),
                "the same slot is no error"
            );
            assert_eq!(player.selected_slot(game), Some(4));
            assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].selected, 4);

            // A menu: closing it returns the carried stack and leaves no menu behind.
            let stone = ItemStack::new("minecraft:stone", 4);
            let host = game.lan.as_mut().unwrap();
            {
                let value = Some(stone.clone());
                host.hub.guests.update(0, |guest| guest.carried = value);
            }
            host.hub
                .open_native_menu(
                    1,
                    1,
                    horizon_core::nbt::Tag::String("test".into()),
                    vec![Some(stone.clone())],
                )
                .unwrap();
            let menu = game.player_container_menu(player).unwrap();
            assert_ne!(menu.container_id(game), Some(0));
            assert!(menu.close_container(game));
            let guest = &game.lan.as_ref().unwrap().hub.guests[0];
            assert!(guest.menu.is_none(), "the menu is gone");
            assert!(guest.carried.is_none(), "the cursor stack went back");
            assert_eq!(
                guest
                    .inventory
                    .iter()
                    .flatten()
                    .map(|s| s.count)
                    .sum::<u32>(),
                4,
                "to the inventory, none lost or doubled"
            );
            assert!(!menu.close_container(game), "a stale menu is refused");
            // The player's own inventory menu is never closed.
            let inventory = game.player_container_menu(player).unwrap();
            assert_eq!(inventory.container_id(game), Some(0));
            assert!(!inventory.close_container(game));
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_stack_is_thrown_from_the_eyes_as_player_drop_does() {
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;

        with_game(|game| {
            // A body to throw from: `with_game` makes none.
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            let stone = ItemStack::new("minecraft:stone", 3);
            let before = game.dropped.len();
            let id = game.drop_stack_from_eyes(stone.clone(), true).unwrap();
            assert_eq!(game.dropped.len(), before + 1);
            let thrown = game.dropped.last().unwrap();
            assert_eq!((thrown.id, &thrown.stack), (id, &stone));
            assert!(thrown.thrower.is_some(), "the drop key names the thrower");
            game.drop_stack_from_eyes(stone.clone(), false).unwrap();
            assert!(game.dropped.last().unwrap().thrower.is_none());
            assert_eq!(
                game.drop_stack_from_eyes(ItemStack::new("minecraft:stone", 0), true),
                None,
                "an empty stack is no drop"
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn menus_read_storage_and_invalidate_on_replacement_wrap_and_reconnect() {
        with_game(|game| {
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 4,
                simulation_distance: 4,
                dimension: "minecraft:overworld".into(),
                allow_flight: true,
                seed: 0,
                max_players: 10,
                integrated: true,
                levels: game.level_shapes,
            };
            let mut net = NetConfig::lan(0, "test".into(), 10);
            net.listen = false;
            game.lan_host(
                LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                net,
                config,
            )
            .unwrap();
            game.lan.as_mut().unwrap().hub.guests.push(guest());
            let player = game.player(123).unwrap();
            let inventory = game.player_container_menu(player).unwrap();
            assert_eq!(inventory.container_id(game), Some(0));
            assert_eq!(inventory.kind(game), Some(None));
            assert_eq!(inventory.slot_count(game), Some(46));
            assert_eq!(inventory.slot(game, 0), Some(None));
            assert_eq!(
                game.menu_slot_backing(inventory, 36),
                Some(MenuSlotBacking::Player(0))
            );
            assert_eq!(
                game.menu_slot_backing(inventory, 45),
                Some(MenuSlotBacking::Player(40))
            );
            assert_eq!(inventory.slot(game, 46), None);
            let stone = ItemStack::new("minecraft:stone", 4);
            let host = game.lan.as_mut().unwrap();
            {
                let value = Some(stone.clone());
                host.hub.guests.update(0, |g| g.inventory[9] = value);
            }
            {
                let value = Some(stone.clone());
                host.hub.guests.update(0, |guest| guest.carried = value);
            }
            let title = horizon_core::nbt::Tag::String("test".into());
            let window = host
                .hub
                .open_native_menu(1, 1, title.clone(), vec![Some(stone.clone())])
                .unwrap();
            assert_eq!(inventory.slot_count(game), None);
            let menu = game.player_container_menu(player).unwrap();
            assert_eq!(menu.container_id(game), Some(window));
            assert_eq!(menu.kind(game), Some(Some("minecraft:generic_9x1".into())));
            assert_eq!(menu.slot_count(game), Some(45));
            assert_eq!(
                menu.slot(game, 0),
                Some(Some(stone.try_into_api().unwrap()))
            );
            assert_eq!(menu.slot(game, 1), Some(None));
            assert_eq!(
                menu.slot(game, 9),
                Some(Some(stone.try_into_api().unwrap()))
            );
            assert_eq!(
                menu.carried(game),
                Some(Some(stone.try_into_api().unwrap()))
            );
            assert_eq!(
                game.menu_slot_backing(menu, 0),
                Some(MenuSlotBacking::Native(0))
            );
            assert_eq!(
                game.menu_slot_backing(menu, 9),
                Some(MenuSlotBacking::Player(9))
            );
            assert_eq!(
                game.menu_slot_backing(menu, 36),
                Some(MenuSlotBacking::Player(0))
            );
            assert_eq!(game.menu_slot_backing(menu, 45), None);
            let host = game.lan.as_mut().unwrap();
            for _ in 0..100 {
                host.hub
                    .open_native_menu(1, 1, title.clone(), vec![])
                    .unwrap();
            }
            assert_eq!(host.hub.guests[0].menu.as_ref().unwrap().window, window);
            assert_eq!(menu.slot_count(game), None);
            let newer = game.player_container_menu(player).unwrap();
            let host = game.lan.as_mut().unwrap();
            host.hub.close_container(1);
            assert_eq!(newer.slot_count(game), None);
            let inventory = game.player_container_menu(player).unwrap();
            let host = game.lan.as_mut().unwrap();
            *host.menus.entry(1).or_default().crafting.table = Some((
                7,
                0,
                Box::new(horizon_client::menu::Menu::new(
                    horizon_client::menu::Kind::Crafting,
                )),
                [0; 3],
            ));
            assert_eq!(game.player_container_menu(player), None);
            assert_eq!(inventory.slot_count(game), None);
            let host = game.lan.as_mut().unwrap();
            host.play.clear();
            host.menus.clear();
            host.hub.guests.replace(0, guest());
            assert_eq!(inventory.slot_count(game), None);

            use horizon_client::{containers::BlockEntity, lan::menu::Slots};
            let a = (1, 2, 3);
            let b = (2, 2, 3);
            let stone = ItemStack::new("minecraft:stone", 3);
            game.world
                .block_entities
                .insert(a, BlockEntity::Chest(vec![None; 2]));
            game.world
                .block_entities
                .insert(b, BlockEntity::Chest(vec![Some(stone.clone()); 3]));
            game.lan.as_mut().unwrap().hub.open_container(
                1,
                "minecraft:generic_9x1",
                title.clone(),
                a,
                Slots::Double(a, b),
                vec![None; 5],
                None,
            );
            let menu = game.player_container_menu(player).unwrap();
            assert_eq!(
                game.menu_slot_backing(menu, 2),
                Some(MenuSlotBacking::BlockEntity {
                    level: game.level("minecraft:overworld").unwrap(),
                    pos: [b.0, b.1, b.2],
                    slot: 0
                })
            );
            assert_eq!(
                menu.slot(game, 2),
                Some(Some(stone.try_into_api().unwrap()))
            );
            let vehicle = (i32::MAX - 15, VEHICLE_KEY_Y, 42);
            game.world
                .block_entities
                .insert(vehicle, BlockEntity::Chest(vec![Some(stone.clone())]));
            game.lan.as_mut().unwrap().hub.open_container(
                1,
                "minecraft:generic_9x1",
                title,
                vehicle,
                Slots::Block(vehicle),
                vec![None],
                None,
            );
            let menu = game.player_container_menu(player).unwrap();
            assert_eq!(
                game.menu_slot_backing(menu, 0),
                Some(MenuSlotBacking::Entity {
                    entity: EntityRef::new(game.level("minecraft:overworld").unwrap().epoch(), 42),
                    slot: 0
                })
            );
            assert_eq!(
                menu.slot(game, 0),
                Some(Some(stone.try_into_api().unwrap()))
            );

            // Identical coordinates in different dimensions must not alias.
            let gold = ItemStack::new("minecraft:gold_ingot", 2);
            let emerald = ItemStack::new("minecraft:emerald", 3);
            game.world
                .block_entities
                .insert(a, BlockEntity::Chest(vec![Some(gold.clone())]));
            let title = horizon_core::nbt::Tag::String("dimension".into());
            game.lan.as_mut().unwrap().hub.open_container(
                1,
                "minecraft:generic_9x1",
                title.clone(),
                a,
                Slots::Block(a),
                vec![Some(gold.clone())],
                None,
            );
            let overworld_menu = game.player_container_menu(player).unwrap();
            let overworld = game.level("minecraft:overworld").unwrap();
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            game.world
                .block_entities
                .insert(a, BlockEntity::Chest(vec![Some(emerald.clone())]));
            let mut nether_guest = guest();
            nether_guest.conn = 2;
            nether_guest.uuid = 456;
            nether_guest.entity_id = 2;
            nether_guest.dimension = "minecraft:the_nether".into();
            game.lan.as_mut().unwrap().hub.guests.push(nether_guest);
            game.lan.as_mut().unwrap().hub.open_container(
                2,
                "minecraft:generic_9x1",
                title,
                a,
                Slots::Block(a),
                vec![Some(emerald.clone())],
                None,
            );
            let other_player = game.player(456).unwrap();
            let nether_menu = game.player_container_menu(other_player).unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            assert_eq!(
                overworld_menu.slot(game, 0),
                Some(Some(gold.try_into_api().unwrap()))
            );
            assert_eq!(
                game.menu_slot_backing(overworld_menu, 0),
                Some(MenuSlotBacking::BlockEntity {
                    level: overworld,
                    pos: [a.0, a.1, a.2],
                    slot: 0
                })
            );
            assert!(
                game.enter_command_level(super::super::Dim::Overworld)
                    .is_some()
            );
            assert_eq!(
                nether_menu.slot(game, 0),
                Some(Some(emerald.try_into_api().unwrap()))
            );
            assert_eq!(
                game.menu_slot_backing(nether_menu, 0),
                Some(MenuSlotBacking::BlockEntity {
                    level: nether,
                    pos: [a.0, a.1, a.2],
                    slot: 0
                })
            );
        });
    }
}
