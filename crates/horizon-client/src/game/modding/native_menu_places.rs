//! Mod menu slots backed by storage that is not a block entity (arismoko/horizon#3385):
//! NeoForge 26.3's `StackCopySlot` and `ResourceHandlerSlot`. A mod opens its menu with
//! `player_open_mod_menu` (#3384) and backs its container slots with
//! `player_back_mod_menu_slots`. The guest's `native_menu` then mirrors those places, so the
//! client sees ordinary synced slots, and a pickup or shift-click on the menu goes through the
//! places' own handlers in one transaction. A menu no mod backed never reaches this module's
//! work: `GuestMenus::slot_places` stays `None` and every path below returns first.
use super::Game;
use super::api_values::TryIntoEngine;
use super::native_menus::native_menu_inventory_slot;
use mod_api::__private::game::*;
use mod_api::{Hand, HostTransaction, ItemPlace, MenuRef, MenuSlotPlace, ServerHost};

/// The places backing one guest's open mod menu.
pub(in crate::game) struct MenuSlotPlaces {
    /// The menu's window id: the places end with that window.
    window: i32,
    /// What container slot `n` shows; slots past these stay the mods' own.
    places: Vec<MenuSlotPlace>,
    /// The hotbar slot holding a main-hand `HeldItem` place's stack, which takes no click.
    locked: Option<usize>,
}

impl Game<'_> {
    /// `player_back_mod_menu_slots`: backs `window`'s first container slots with `places`. An
    /// empty list takes the backing away.
    pub(super) fn player_back_mod_menu_slots(
        &mut self,
        player: mod_api::PlayerRef,
        window: i32,
        places: Vec<MenuSlotPlace>,
    ) -> bool {
        let uuid = player.uuid();
        let Some(selected) = self.online_player_record(player).map(|g| g.selected) else {
            return false;
        };
        let (Some(conn), Some(menu)) = (self.native_conn(uuid), self.menu(uuid, window)) else {
            return false;
        };
        let size = self
            .lan
            .as_ref()
            .and_then(|lan| Some(lan.hub.guest(conn)?.menu.as_ref()?.size));
        if size.is_none_or(|size| places.len() > size) {
            return false;
        }
        let host: &mut dyn ServerHost = self;
        if !places.iter().all(|place| place.resolves(host, player)) {
            return false;
        }
        let locked = places
            .iter()
            .any(|place| {
                matches!(
                    place,
                    MenuSlotPlace::HeldItem {
                        hand: Hand::Main,
                        ..
                    }
                )
            })
            .then_some(selected);
        let Some(lan) = self.lan.as_mut() else {
            return false;
        };
        let backed = !places.is_empty();
        lan.menus.entry(conn).or_default().slot_places = backed.then_some(MenuSlotPlaces {
            window,
            places,
            locked,
        });
        if backed {
            self.refresh_menu_places(conn, uuid, menu);
        }
        true
    }

    /// What container slot `n` of `conn`'s menu `window` shows, when a mod backed it.
    pub(super) fn native_menu_place(
        &self,
        conn: u32,
        window: i32,
        n: usize,
    ) -> Option<MenuSlotPlace> {
        let backed = self.lan.as_ref()?.menus.get(&conn)?.slot_places.as_ref()?;
        (backed.window == window).then_some(())?;
        backed.places.get(n).copied()
    }

    /// The backing of `conn`'s menu: its window, how many slots and the locked hotbar slot.
    fn menu_places_of(&self, conn: u32) -> Option<(i32, usize, Option<usize>)> {
        let backed = self.lan.as_ref()?.menus.get(&conn)?.slot_places.as_ref()?;
        Some((backed.window, backed.places.len(), backed.locked))
    }

    fn forget_menu_places(&mut self, conn: u32) {
        if let Some(menus) = self.lan.as_mut().and_then(|lan| lan.menus.get_mut(&conn)) {
            menus.slot_places = None;
        }
    }

    /// Copies each place's stack into the guest's `native_menu`, which the next container sync
    /// sends; closes the menu, as a gone block entity's, once a place stops resolving or the
    /// locked hotbar slot is no longer the selected one. False when it closed or was not open.
    fn refresh_menu_places(&mut self, conn: u32, uuid: u128, menu: MenuRef) -> bool {
        let Some((window, count, locked)) = self.menu_places_of(conn) else {
            return false;
        };
        let player = menu.fields().player;
        let selected = self.online_player_record(player).map(|g| g.selected);
        let mut open = locked.is_none_or(|slot| selected == Some(slot));
        for n in 0..count {
            let Some(place) = self.native_menu_place(conn, window, n) else {
                break;
            };
            let host: &mut dyn ServerHost = self;
            if !open || !place.resolves(host, player) {
                open = false;
                break;
            }
            let stack = ItemPlace::MenuSlot { menu, slot: n }
                .stack(host)
                .and_then(|stack| stack.try_into_engine(self.items()).ok());
            let Some(lan) = self.lan.as_mut() else {
                return false;
            };
            let guests = &mut lan.hub.guests;
            let Some(index) = guests.iter().position(|g| g.conn == conn) else {
                return false;
            };
            if let Some(shown) = guests.native_menu_mut(index).and_then(|m| m.get_mut(n))
                && *shown != stack
            {
                *shown = stack;
            }
        }
        if !open {
            self.forget_menu_places(conn);
            self.close_menu(uuid);
        }
        open
    }

    /// Each tick: the backed menus' places, mirrored, and their menus closed once a place is
    /// gone. No guest has any where no mod backed a menu, and this returns at once.
    pub(in crate::game) fn tick_menu_slot_places(&mut self) {
        let Some(lan) = self.lan.as_ref() else {
            return;
        };
        if lan.menus.values().all(|m| m.slot_places.is_none()) {
            return;
        }
        let backed: Vec<(u32, i32)> = lan
            .menus
            .iter()
            .filter_map(|(conn, m)| Some((*conn, m.slot_places.as_ref()?.window)))
            .collect();
        for (conn, window) in backed {
            let uuid = self
                .lan
                .as_ref()
                .and_then(|lan| Some(lan.hub.guest(conn)?.uuid));
            match uuid.and_then(|uuid| Some((uuid, self.menu(uuid, window)?))) {
                Some((uuid, menu)) => {
                    self.refresh_menu_places(conn, uuid, menu);
                }
                // The menu closed or another replaced it: its places end with it.
                None => self.forget_menu_places(conn),
            }
        }
    }

    /// A click on a backed menu (`AbstractContainerMenu.clicked`'s `PICKUP` and `QUICK_MOVE`),
    /// done through the places' handlers in one transaction; true when it was answered here,
    /// false to leave it to the mods (`native_menu_clicked`): a click in a menu no mod backed,
    /// on a container slot past the places, outside the menu, or of another kind.
    pub(super) fn place_menu_click(
        &mut self,
        uuid: u128,
        window: i32,
        slot: i32,
        button: i32,
        mode: i32,
    ) -> bool {
        let Some(conn) = self.native_conn(uuid) else {
            return false;
        };
        let Some((backed_window, count, locked)) = self.menu_places_of(conn) else {
            return false;
        };
        let Some(size) = self
            .lan
            .as_ref()
            .and_then(|lan| Some(lan.hub.guest(conn)?.menu.as_ref()?.size))
        else {
            return false;
        };
        let Ok(at) = usize::try_from(slot) else {
            return false;
        };
        let pickup = mode == 0 && matches!(button, 0 | 1);
        if backed_window != window
            || !(pickup || mode == 1)
            || (count..size).contains(&at)
            || at >= size + 36
        {
            return false;
        }
        let Some(menu) = self.menu(uuid, window) else {
            return false;
        };
        let inventory = native_menu_inventory_slot(size, slot);
        // The held backpack's own slot takes no click, so it can't go into itself.
        if inventory.is_none() || inventory != locked {
            let host: &mut dyn ServerHost = self;
            if pickup {
                click_pickup(host, menu, at, button == 1);
            } else {
                click_quick_move(host, menu, at, size, count);
            }
        }
        if self.refresh_menu_places(conn, uuid, menu)
            && let Some(lan) = self.lan.as_mut()
        {
            lan.hub.resend_menu(conn, window, &self.world);
        }
        true
    }
}

/// `PICKUP` on menu slot `at`: an empty cursor takes all (or half, `right`) of the slot; a
/// full one puts all (or one) in, or swaps with a slot of another item. All or nothing.
fn click_pickup(host: &mut dyn ServerHost, menu: MenuRef, at: usize, right: bool) {
    let cursor = menu.cursor();
    let at = ItemPlace::MenuSlot { menu, slot: at };
    let _ = host.transaction(|host, tx| {
        let Some(held) = cursor.resource(host) else {
            let Some(here) = at.resource(host) else {
                return Ok(());
            };
            let have = at.amount(host);
            let want = if right { have.div_ceil(2) } else { have };
            let out = at.extract(host, tx, &here, want);
            return match cursor.insert(host, tx, &here, out) == out {
                true => Ok(()),
                false => Err(()),
            };
        };
        let carried = cursor.amount(host);
        let put = at.insert(host, tx, &held, if right { 1 } else { carried });
        if put > 0 {
            return match cursor.extract(host, tx, &held, put) == put {
                true => Ok(()),
                false => Err(()),
            };
        }
        let Some(here) = at.resource(host).filter(|here| *here != held) else {
            return Ok(());
        };
        let have = at.amount(host);
        let swapped = at.extract(host, tx, &here, have) == have
            && cursor.extract(host, tx, &held, carried) == carried
            && at.insert(host, tx, &held, carried) == carried
            && cursor.insert(host, tx, &here, have) == have;
        match swapped {
            true => Ok(()),
            false => Err(()),
        }
    });
}

/// `QUICK_MOVE` on menu slot `at` of a menu of `size` container slots, `count` of them backed
/// (`ChestMenu.quickMoveStack`): a container slot's stack goes to the inventory, hotbar end
/// first; an inventory slot's into the backed slots in order. Onto matching stacks first, then
/// into empty slots (`moveItemStackTo`), all or nothing.
fn click_quick_move(
    host: &mut dyn ServerHost,
    menu: MenuRef,
    at: usize,
    size: usize,
    count: usize,
) {
    let from = ItemPlace::MenuSlot { menu, slot: at };
    let Some(resource) = from.resource(host) else {
        return;
    };
    let (first, len, reverse) = match at < size {
        true => (size, 36, true),
        false => (0, count, false),
    };
    let _ = host.transaction(|host, tx| {
        let mut left = from.amount(host);
        for merge in [true, false] {
            for i in 0..len {
                if left == 0 {
                    return Ok(());
                }
                let slot = if reverse {
                    first + len - 1 - i
                } else {
                    first + i
                };
                let to = ItemPlace::MenuSlot { menu, slot };
                match to.resource(host) {
                    Some(here) if merge && here == resource => {}
                    None if !merge => {}
                    _ => continue,
                }
                let moved = to.insert(host, tx, &resource, left);
                if moved > 0 {
                    if from.extract(host, tx, &resource, moved) != moved {
                        return Err(());
                    }
                    left -= moved;
                }
            }
        }
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_item_places::tests::{guest, open_to_lan};
    use super::*;
    use horizon_client::items::ItemStack;
    use mod_api::{MenuReplace, MenuSlotBacking};

    /// Guest 1 (UUID 123) holding a shulker box in hotbar slot 0 with 64 stone in inventory
    /// slot 9, and a 27-slot mod chest menu open: its window.
    fn backpack_menu(game: &mut Game<'_>) -> i32 {
        open_to_lan(game);
        let mut player = guest(1);
        player.selected = 0;
        player.inventory[0] = Some(ItemStack::new("minecraft:shulker_box", 1));
        player.inventory[9] = Some(ItemStack::new("minecraft:stone", 64));
        game.lan.as_mut().unwrap().hub.guests.push(player);
        let host: &mut dyn ServerHost = game;
        host.open_menu(
            123,
            3,
            mod_api::Text::literal("pack"),
            Vec::new(),
            MenuReplace::CloseFirst,
        )
        .unwrap()
        .unwrap()
    }

    fn held_slot(n: usize) -> ItemPlace {
        ItemPlace::Nested {
            parent: Box::new(ItemPlace::PlayerHand {
                player: 123,
                hand: Hand::Main,
            }),
            slot: n,
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_shift_click_into_a_held_backpack_writes_through_its_handler() {
        with_game(|game| {
            let window = backpack_menu(game);
            let host: &mut dyn ServerHost = game;
            let menu = host.menu(123, window).unwrap();
            // Unbacked, the menu is the mod's own, as before.
            assert_eq!(menu.slot_backing(host, 0), Some(MenuSlotBacking::Native(0)));
            assert!(!game.place_menu_click(123, window, 27, 0, 1));

            let player = menu.fields().player;
            let places: Vec<_> = (0..27)
                .map(|slot| MenuSlotPlace::HeldItem {
                    hand: Hand::Main,
                    slot,
                })
                .collect();
            assert!(!game.player_back_mod_menu_slots(player, window + 1, places.clone()));
            assert!(game.player_back_mod_menu_slots(player, window, places));
            let host: &mut dyn ServerHost = game;
            assert_eq!(
                menu.slot_backing(host, 0),
                Some(MenuSlotBacking::Place(MenuSlotPlace::HeldItem {
                    hand: Hand::Main,
                    slot: 0
                }))
            );

            // Inventory slot 9 is menu slot 27: a shift-click moves the stone into the box.
            assert!(game.place_menu_click(123, window, 27, 0, 1));
            let host: &mut dyn ServerHost = game;
            assert_eq!(held_slot(0).amount(host), 64);
            assert_eq!(ItemPlace::MenuSlot { menu, slot: 27 }.stack(host), None);
            assert_eq!(ItemPlace::MenuSlot { menu, slot: 0 }.amount(host), 64);

            // The box's own hotbar slot (menu slot 54) takes no click.
            assert!(game.place_menu_click(123, window, 54, 0, 0));
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.cursor().stack(host), None);
            let held = ItemPlace::PlayerHand {
                player: 123,
                hand: Hand::Main,
            };
            assert_eq!(held.amount(host), 1, "the box stays in the hand");

            // The box gone from the hand, the menu closes at the next tick.
            let resource = held.resource(host).unwrap();
            let taken = host.transaction(|host, tx| Ok::<_, ()>(held.extract(host, tx, &resource, 1)));
            assert_eq!(taken, Ok(1));
            game.tick_menu_slot_places();
            let host: &mut dyn ServerHost = game;
            assert_eq!(host.menu(123, window), None);
        });
    }
}
