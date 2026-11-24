//! Mod menu slots backed by storage that is not a block entity (arismoko/horizon#3385):
//! NeoForge 26.3's `StackCopySlot` and `ResourceHandlerSlot`. A mod opens its menu with
//! `player_open_mod_menu` (arismoko/horizon#3384) and backs its container slots with
//! `player_back_mod_menu_slots`. The guest's `native_menu` then mirrors those places, so the
//! client sees ordinary synced slots, and a pickup or shift-click on the menu goes through the
//! places' own handlers in one transaction. Nothing runs per tick: the mirror is refreshed after
//! each click on the menu and at each change notice (an `ItemPlace` write to the backing
//! player's places, queued as `NativeMenuEvent::PlacesChanged` and taken after the write), and
//! only slots whose stack changed are written. A menu no mod backed never reaches this module's
//! work: `GuestMenus::slot_places` stays `None` and every path below returns first.
use super::Game;
use super::api_values::TryIntoEngine;
use super::native_menus::native_menu_inventory_slot;
use horizon_client::items::ItemStack;
use horizon_client::lan::{
    guest::Guest,
    menu::{NativeMenuEvent, Slots},
};
use mod_api::__private::game::*;
use mod_api::{
    Hand, HostTransaction, ItemPlace, MenuRef, MenuSlotBackRefusal, MenuSlotPlace, ServerHost,
};

/// The places backing one guest's open mod menu.
pub(in crate::game) struct MenuSlotPlaces {
    /// The menu's window id: the places end with that window.
    window: i32,
    /// What container slot `n` shows; slots past these stay the mods' own.
    places: Vec<MenuSlotPlace>,
    /// The hotbar slot holding a main-hand `HeldItem` place's stack, which takes no click.
    locked: Option<usize>,
    /// The stacks the `HeldItem` places were bound to when the slots were backed, by hand (main,
    /// off): the inventory slot holding it and the stack without its contents
    /// ([`held_identity`]). Another stack there, or another hotbar slot selected, closes the menu.
    held: [Option<(usize, ItemStack)>; 2],
    /// The stacks the `ModItem` places were bound to when the slots were backed, by place:
    /// their provider, key and the stack without its contents ([`place_identity`]). Another
    /// stack there closes the menu, and a click that changes it is undone.
    mod_bound: Vec<(&'static str, u64, mod_api::ItemStack)>,
    /// A change notice is queued; further writes queue none until it is taken.
    noticed: bool,
}

/// A held container's identity: the stack without the contents its `ITEM_ITEMS` handler
/// writes (`minecraft:container`, `minecraft:bundle_contents`), so writes through the menu
/// keep it and another stack put in its place does not.
fn held_identity(stack: &ItemStack) -> ItemStack {
    let mut stack = stack.clone();
    if let Some(data) = stack.extra.as_deref_mut() {
        data.container.clear();
        data.bundle_contents.clear();
    }
    stack.tidy();
    stack
}

/// [`held_identity`] for a stack at a mod place, as the mod API reads it.
fn place_identity(stack: &mod_api::ItemStack) -> mod_api::ItemStack {
    let mut identity = stack.clone();
    let bare = mod_api::ItemStack::of(stack.item());
    for id in ["minecraft:container", "minecraft:bundle_contents"] {
        identity.copy_component_from(&bare, id);
    }
    identity
}

/// Whether each mod place in `bound` still holds the stack it was bound to, its contents aside.
fn mod_places_bound(
    host: &mut dyn ServerHost,
    bound: &[(&'static str, u64, mod_api::ItemStack)],
) -> bool {
    bound.iter().all(|&(provider, key, ref identity)| {
        let place = ItemPlace::Mod { provider, key };
        place
            .stack(host)
            .is_some_and(|stack| place_identity(&stack) == *identity)
    })
}

/// The inventory slot holding `hand`'s stack (the selected hotbar slot, or the offhand's 40) and
/// that stack, of `guest`.
fn held_by(guest: &Guest, hand: Hand) -> (usize, Option<&ItemStack>) {
    match hand {
        Hand::Off => (40, guest.offhand.as_ref()),
        _ => (
            guest.selected,
            guest.inventory.get(guest.selected).and_then(Option::as_ref),
        ),
    }
}

impl Game<'_> {
    /// `player_back_mod_menu_slots`: backs `window`'s first container slots with `places`. An
    /// empty list takes the backing away; a slot no longer backed is emptied, so the copy it
    /// showed never becomes the menu's own storage.
    pub(super) fn player_back_mod_menu_slots(
        &mut self,
        player: mod_api::PlayerRef,
        window: i32,
        places: Vec<MenuSlotPlace>,
    ) -> Result<(), MenuSlotBackRefusal> {
        let uuid = player.uuid();
        let guest = self
            .online_player_record(player)
            .ok_or(MenuSlotBackRefusal::NoPlayerOrWindow)?;
        // The stacks the held places are bound to, as they are now.
        let mut held = [None, None];
        for place in &places {
            if let MenuSlotPlace::HeldItem { hand, .. } = *place {
                let (index, stack) = held_by(guest, hand);
                held[usize::from(hand == Hand::Off)] = stack.map(|s| (index, held_identity(s)));
            }
        }
        let selected = guest.selected;
        let conn = self
            .native_conn(uuid)
            .ok_or(MenuSlotBackRefusal::NoPlayerOrWindow)?;
        let (size, slots) = self
            .lan
            .as_ref()
            .and_then(|lan| lan.hub.guest(conn)?.menu.as_ref())
            .filter(|open| open.window == window)
            .map(|open| (open.size, open.slots))
            .ok_or(MenuSlotBackRefusal::NoPlayerOrWindow)?;
        // Only a blockless mod menu's slots are the game's to show (`Slots::Native`); a block's
        // or an entity's menu reads its own storage, which a backing would not replace.
        if slots != Slots::Native {
            return Err(MenuSlotBackRefusal::UnsupportedMenu);
        }
        let menu = self
            .menu(uuid, window)
            .ok_or(MenuSlotBackRefusal::NoPlayerOrWindow)?;
        if places.len() > size {
            return Err(MenuSlotBackRefusal::TooManyPlaces);
        }
        let host: &mut dyn ServerHost = self;
        if let Some(index) = places
            .iter()
            .position(|place| !place.resolves(host, player))
        {
            return Err(MenuSlotBackRefusal::PlaceUnresolved { index });
        }
        // The stacks the mod places are bound to, as they are now.
        let mut mod_bound = Vec::new();
        for place in &places {
            if let MenuSlotPlace::ModItem { provider, key, .. } = *place
                && !mod_bound
                    .iter()
                    .any(|(p, k, _)| (*p, *k) == (provider, key))
                && let Some(stack) = (ItemPlace::Mod { provider, key }).stack(host)
            {
                mod_bound.push((provider, key, place_identity(&stack)));
            }
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
            return Err(MenuSlotBackRefusal::NoPlayerOrWindow);
        };
        let count = places.len();
        let entry = &mut lan.menus.entry(conn).or_default().slot_places;
        // The slots this window's old backing mirrored that the new one does not show.
        let before = entry
            .as_ref()
            .filter(|old| old.window == window)
            .map_or(0, |old| old.places.len());
        *entry = (count > 0).then_some(MenuSlotPlaces {
            window,
            places,
            locked,
            held,
            mod_bound,
            noticed: false,
        });
        if let Some(index) = lan.hub.guests.connection_index(conn)
            && let Some(shown) = lan.hub.guests.native_menu_mut(index)
        {
            for slot in shown.iter_mut().take(before).skip(count) {
                *slot = None;
            }
        }
        if count > 0 {
            self.refresh_menu_places(conn, uuid, menu);
        }
        if before > count
            && let Some(lan) = self.lan.as_mut()
        {
            lan.hub.resend_menu(conn, window, &self.world);
        }
        Ok(())
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

    /// Whether the stacks `conn`'s held places were bound to are still where they were: the same
    /// inventory slot (the same hotbar slot selected) holding the same stack, its contents aside.
    fn held_places_bound(&self, conn: u32, player: mod_api::PlayerRef) -> bool {
        let Some(backed) = self
            .lan
            .as_ref()
            .and_then(|lan| lan.menus.get(&conn)?.slot_places.as_ref())
        else {
            return true;
        };
        if backed.held.iter().all(Option::is_none) {
            return true;
        }
        let Some(guest) = self.online_player_record(player) else {
            return false;
        };
        [Hand::Main, Hand::Off]
            .into_iter()
            .zip(&backed.held)
            .all(|(hand, bound)| {
                let Some((index, identity)) = bound else {
                    return true;
                };
                let (now, stack) = held_by(guest, hand);
                now == *index && stack.is_some_and(|stack| held_identity(stack) == *identity)
            })
    }

    /// The stacks `conn`'s mod places were bound to.
    fn mod_bound_of(&self, conn: u32) -> Vec<(&'static str, u64, mod_api::ItemStack)> {
        self.lan
            .as_ref()
            .and_then(|lan| lan.menus.get(&conn)?.slot_places.as_ref())
            .map_or_else(Vec::new, |backed| backed.mod_bound.clone())
    }

    /// Whether the stacks `conn`'s held and mod places were bound to are still where they were.
    fn places_bound(&mut self, conn: u32, player: mod_api::PlayerRef) -> bool {
        let bound = self.mod_bound_of(conn);
        self.held_places_bound(conn, player) && mod_places_bound(self, &bound)
    }

    /// Copies each place's stack into the guest's `native_menu` where it changed, which the next
    /// container sync sends; closes the menu, as a gone block entity's, once a place stops
    /// resolving or a held place's stack is no longer the bound one. False when it closed or was
    /// not open.
    fn refresh_menu_places(&mut self, conn: u32, uuid: u128, menu: MenuRef) -> bool {
        let Some((window, count, _)) = self.menu_places_of(conn) else {
            return false;
        };
        let player = menu.fields().player;
        let mut open = self.places_bound(conn, player);
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
            let Some(index) = guests.connection_index(conn) else {
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

    /// The storage-change notice: an `ItemPlace` write to `place`, if it is a place of a player
    /// whose open menu a mod backed (their hand or inventory, a container held there, their
    /// menu's slots), queues one look at that menu's places for after the write
    /// (`NativeMenuEvent::PlacesChanged`). Any other write, and every write while no menu is
    /// backed, returns at the first checks.
    pub(super) fn notice_menu_place_write(&mut self, place: &ItemPlace) {
        let mut root = place;
        while let ItemPlace::Nested { parent, .. } = root {
            root = &**parent;
        }
        let uuid = match root {
            ItemPlace::PlayerHand { player, .. } | ItemPlace::PlayerSlot { player, .. } => *player,
            ItemPlace::MenuSlot { menu, .. } => menu.fields().player.uuid(),
            _ => return,
        };
        if self
            .lan
            .as_ref()
            .is_none_or(|lan| lan.menus.values().all(|m| m.slot_places.is_none()))
        {
            return;
        }
        let Some(conn) = self.native_conn(uuid) else {
            return;
        };
        let Some(lan) = self.lan.as_mut() else {
            return;
        };
        let Some(backed) = lan
            .menus
            .get_mut(&conn)
            .and_then(|m| m.slot_places.as_mut())
            .filter(|backed| !backed.noticed)
        else {
            return;
        };
        backed.noticed = true;
        let window = backed.window;
        lan.hub
            .native_menu_events
            .push(NativeMenuEvent::PlacesChanged { uuid, window });
    }

    /// A change notice taken (`NativeMenuEvent::PlacesChanged`): the backed menu's places looked
    /// at again, or forgotten once that menu is no longer open.
    pub(super) fn menu_places_changed(&mut self, uuid: u128, window: i32) {
        let Some(conn) = self.native_conn(uuid) else {
            return;
        };
        let Some((backed_window, ..)) = self.menu_places_of(conn) else {
            return;
        };
        if let Some(backed) = self
            .lan
            .as_mut()
            .and_then(|lan| lan.menus.get_mut(&conn)?.slot_places.as_mut())
        {
            backed.noticed = false;
        }
        if backed_window != window {
            return;
        }
        match self.menu(uuid, window) {
            Some(menu) => {
                self.refresh_menu_places(conn, uuid, menu);
            }
            None => self.forget_menu_places(conn),
        }
    }

    /// The menu `window` closed (`NativeMenuEvent::Closed`): its places end with it. Its
    /// mirrored slots went with the menu (`native_menu` is cleared on close).
    pub(super) fn menu_places_closed(&mut self, uuid: u128, window: i32) {
        if let Some(conn) = self.native_conn(uuid)
            && self.menu_places_of(conn).is_some_and(|(w, ..)| w == window)
        {
            self.forget_menu_places(conn);
        }
    }

    /// A click on a backed menu (`AbstractContainerMenu.clicked`'s `PICKUP` and `QUICK_MOVE`),
    /// done through the places' handlers in one transaction; true when it was answered here,
    /// false to leave it to the mods (`native_menu_clicked`): a click in a menu no mod backed,
    /// on a container slot past the places, outside the menu, or of another kind. Before any
    /// click on a backed menu every place must still resolve and a held or mod place's stack
    /// must still be the bound one; else the menu closes and the click does nothing.
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
        if backed_window != window {
            return false;
        }
        let Some(menu) = self.menu(uuid, window) else {
            return false;
        };
        let player = menu.fields().player;
        let places: Vec<_> = (0..count)
            .filter_map(|n| self.native_menu_place(conn, window, n))
            .collect();
        if !self.places_bound(conn, player)
            || !places.into_iter().all(|place| place.resolves(self, player))
        {
            self.forget_menu_places(conn);
            self.close_menu(uuid);
            return true;
        }
        let Ok(at) = usize::try_from(slot) else {
            return false;
        };
        let pickup = mode == 0 && matches!(button, 0 | 1);
        if !(pickup || mode == 1) || (count..size).contains(&at) || at >= size + 36 {
            return false;
        }
        let inventory = native_menu_inventory_slot(size, slot);
        // The held backpack's own slot takes no click, so it can't go into itself; a click that
        // takes a mod place's backpack out of its place is undone, for the same reason.
        if inventory.is_none() || inventory != locked {
            let bound = self.mod_bound_of(conn);
            let host: &mut dyn ServerHost = self;
            let _ = host.transaction(|host, _| {
                if pickup {
                    click_pickup(host, menu, at, button == 1);
                } else {
                    click_quick_move(host, menu, at, size, count);
                }
                match mod_places_bound(host, &bound) {
                    true => Ok(()),
                    false => Err(()),
                }
            });
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
    use super::super::native_entity_refs::tests::with_game as with_base_game;
    use super::super::native_item_places::tests::{guest, open_to_lan};
    use super::*;
    use horizon_client::items::ItemStack;
    use horizon_client::lan::menu::Slots;
    use mod_api::{MenuReplace, MenuSlotBacking};

    struct Backpack;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Backpack {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let item = r.item_key("minecraft:shulker_box");
            r.item_container(&item, |_: &dyn mod_api::Host| Some(BOX_SIZE.get()));
            r.place_provider(BACK, std::sync::Arc::new(BackSlot));
        }
    }

    std::thread_local! {
        /// The box's slots, which a test shrinks (a mod's container sized by its own state).
        static BOX_SIZE: std::cell::Cell<u32> = const { std::cell::Cell::new(27) };
    }

    /// An accessory mod's back slots ([`ItemPlace::Mod`]): back slot `key` is kept in the
    /// player's inventory slot 20 + `key`, which the menu also shows.
    const BACK: &str = "backpack_fixture:back";
    fn back_slot(key: u64) -> ItemPlace {
        ItemPlace::PlayerSlot {
            player: 123,
            slot: 20 + key as i32,
        }
    }
    struct BackSlot;
    impl mod_api::PlaceProvider for BackSlot {
        fn stack(&self, host: &mut dyn ServerHost, key: u64) -> Option<mod_api::ItemStack> {
            back_slot(key).stack(host)
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &mod_api::Transaction,
            key: u64,
            resource: &mod_api::ItemResource,
            amount: u32,
        ) -> u32 {
            back_slot(key).insert(host, tx, resource, amount)
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            tx: &mod_api::Transaction,
            key: u64,
            resource: &mod_api::ItemResource,
            amount: u32,
        ) -> u32 {
            back_slot(key).extract(host, tx, resource, amount)
        }
    }

    fn with_game(test: impl FnOnce(&mut Game<'_>)) {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "backpack_fixture",
            Box::new(Backpack),
        )]
        .into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || with_base_game(test));
    }

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
            assert_eq!(
                game.player_back_mod_menu_slots(player, window + 1, places.clone()),
                Err(MenuSlotBackRefusal::NoPlayerOrWindow)
            );
            assert_eq!(
                game.player_back_mod_menu_slots(player, window, places),
                Ok(())
            );
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

            // The box gone from the hand, the write's notice closes the menu once taken.
            let resource = held.resource(host).unwrap();
            let taken =
                host.transaction(|host, tx| Ok::<_, ()>(held.extract(host, tx, &resource, 1)));
            assert_eq!(taken, Ok(1));
            game.run_native_menu_events();
            let host: &mut dyn ServerHost = game;
            assert_eq!(host.menu(123, window), None);
        });
    }

    /// The guest's record, to change their hands as a packet or a command would.
    fn edit_guest(game: &mut Game<'_>, edit: impl FnOnce(&mut horizon_client::lan::guest::Guest)) {
        let guests = &mut game.lan.as_mut().unwrap().hub.guests;
        let index = guests.connection_index(1).unwrap();
        guests.update(index, edit);
    }

    fn held_places(hand: Hand, slots: std::ops::Range<usize>) -> Vec<MenuSlotPlace> {
        slots
            .map(|slot| MenuSlotPlace::HeldItem { hand, slot })
            .collect()
    }

    /// Stone in the box, inventory slot 9 and the menu's own slots: un-backing keeps it at 64.
    fn stone(game: &mut Game<'_>, menu: MenuRef) -> u32 {
        let host: &mut dyn ServerHost = game;
        let boxed: u32 = (0..27).map(|n| held_slot(n).amount(host)).sum();
        let own: u32 = (0..27)
            .map(|slot| match menu.slot_backing(host, slot) {
                // A backed slot is a view of the box, already counted above.
                Some(MenuSlotBacking::Native(_)) => ItemPlace::MenuSlot { menu, slot }.amount(host),
                _ => 0,
            })
            .sum();
        let carried = ItemPlace::PlayerSlot {
            player: 123,
            slot: 9,
        }
        .amount(host);
        boxed + own + carried
    }

    #[test]
    #[ignore = "daily"]
    fn un_backing_or_shortening_empties_the_mirrored_slots_and_keeps_the_items_once() {
        with_game(|game| {
            let window = backpack_menu(game);
            let host: &mut dyn ServerHost = game;
            let menu = host.menu(123, window).unwrap();
            let player = menu.fields().player;
            let places = held_places(Hand::Main, 0..27);
            assert_eq!(
                game.player_back_mod_menu_slots(player, window, places),
                Ok(())
            );
            assert!(game.place_menu_click(123, window, 27, 0, 1));
            assert_eq!(stone(game, menu), 64);
            // Shortened to slot 1 of the box: menu slot 0 shows that empty slot, the rest nothing.
            let places = held_places(Hand::Main, 1..2);
            assert_eq!(
                game.player_back_mod_menu_slots(player, window, places),
                Ok(())
            );
            let host: &mut dyn ServerHost = game;
            assert_eq!(ItemPlace::MenuSlot { menu, slot: 0 }.amount(host), 0);
            assert_eq!(held_slot(0).amount(host), 64);
            assert_eq!(stone(game, menu), 64);
            // Un-backed: the menu's own slots hold no copy to take out.
            assert_eq!(
                game.player_back_mod_menu_slots(player, window, Vec::new()),
                Ok(())
            );
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.slot_backing(host, 0), Some(MenuSlotBacking::Native(0)));
            assert_eq!(ItemPlace::MenuSlot { menu, slot: 0 }.stack(host), None);
            assert_eq!(stone(game, menu), 64);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_backing_names_why_it_refuses() {
        with_game(|game| {
            let window = backpack_menu(game);
            let host: &mut dyn ServerHost = game;
            let menu = host.menu(123, window).unwrap();
            let player = menu.fields().player;
            let back = |game: &mut Game<'_>, places| {
                game.player_back_mod_menu_slots(player, window, places)
            };
            assert_eq!(
                back(game, held_places(Hand::Main, 0..28)),
                Err(MenuSlotBackRefusal::TooManyPlaces)
            );
            // Nothing in the offhand: the second place names no storage.
            let mut places = held_places(Hand::Main, 0..1);
            places.extend(held_places(Hand::Off, 0..1));
            assert_eq!(
                back(game, places),
                Err(MenuSlotBackRefusal::PlaceUnresolved { index: 1 })
            );
            // A block's menu shows its block's slots: no backing is stored for it.
            edit_guest(game, |g| {
                g.menu.as_mut().unwrap().slots = Slots::Block((0, 64, 0));
            });
            assert_eq!(
                back(game, held_places(Hand::Main, 0..1)),
                Err(MenuSlotBackRefusal::UnsupportedMenu)
            );
            assert_eq!(game.menu_places_of(1), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_creative_shift_click_fills_the_held_backpack_not_a_copy() {
        with_game(|game| {
            let window = backpack_menu(game);
            edit_guest(game, |g| g.game_mode = 1);
            let host: &mut dyn ServerHost = game;
            let menu = host.menu(123, window).unwrap();
            let player = menu.fields().player;
            let places = held_places(Hand::Main, 0..27);
            assert_eq!(
                game.player_back_mod_menu_slots(player, window, places),
                Ok(())
            );
            assert!(game.place_menu_click(123, window, 27, 0, 1));
            let host: &mut dyn ServerHost = game;
            assert_eq!(held_slot(0).amount(host), 64, "the box in the hand holds it");
            let guest = game.lan.as_ref().unwrap().hub.guest(1).unwrap();
            let boxes = guest
                .inventory
                .iter()
                .flatten()
                .filter(|s| &*s.item == "minecraft:shulker_box")
                .count();
            assert_eq!(boxes, 1, "no second box");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_write_into_the_box_is_mirrored_at_its_notice() {
        with_game(|game| {
            let window = backpack_menu(game);
            let host: &mut dyn ServerHost = game;
            let menu = host.menu(123, window).unwrap();
            let player = menu.fields().player;
            let places = held_places(Hand::Main, 0..27);
            assert_eq!(
                game.player_back_mod_menu_slots(player, window, places),
                Ok(())
            );
            let host: &mut dyn ServerHost = game;
            let stone = ItemPlace::PlayerSlot {
                player: 123,
                slot: 9,
            }
            .resource(host)
            .unwrap();
            let put =
                host.transaction(|host, tx| Ok::<_, ()>(held_slot(4).insert(host, tx, &stone, 5)));
            assert_eq!(put, Ok(5));
            let shown = |game: &Game<'_>| {
                let guest = game.lan.as_ref().unwrap().hub.guest(1).unwrap();
                guest.native_menu[4].as_ref().map(|s| s.count)
            };
            assert_eq!(shown(game), None, "mirrored after the write, not inside it");
            game.run_native_menu_events();
            assert_eq!(shown(game), Some(5));
        });
    }

    /// The bound stack replaced by `replace`, then a shift-click: the menu closes and the click
    /// moves nothing.
    fn replaced_closes(hand: Hand, replace: impl FnOnce(&mut horizon_client::lan::guest::Guest)) {
        with_game(|game| {
            let window = backpack_menu(game);
            if hand == Hand::Off {
                edit_guest(game, |g| g.offhand = g.inventory[0].take());
            }
            let host: &mut dyn ServerHost = game;
            let menu = host.menu(123, window).unwrap();
            let player = menu.fields().player;
            let places = held_places(hand, 0..27);
            assert_eq!(
                game.player_back_mod_menu_slots(player, window, places),
                Ok(())
            );
            edit_guest(game, replace);
            assert!(game.place_menu_click(123, window, 27, 0, 1));
            let host: &mut dyn ServerHost = game;
            assert_eq!(host.menu(123, window), None);
            let carried = ItemPlace::PlayerSlot {
                player: 123,
                slot: 9,
            };
            assert_eq!(carried.amount(host), 64);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn another_stack_in_the_hand_closes_the_menu() {
        replaced_closes(Hand::Main, |g| {
            g.inventory[0] = Some(ItemStack::new("minecraft:red_shulker_box", 1));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn another_stack_in_the_offhand_closes_the_menu() {
        replaced_closes(Hand::Off, |g| {
            g.offhand = Some(ItemStack::new("minecraft:red_shulker_box", 1));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn another_hotbar_slot_selected_closes_the_menu() {
        replaced_closes(Hand::Main, |g| {
            g.inventory[1] = Some(ItemStack::new("minecraft:shulker_box", 1));
            g.selected = 1;
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_backpack_in_a_mod_place_backs_the_menu_never_goes_into_itself_and_closes_once_gone() {
        with_game(|game| {
            let window = backpack_menu(game);
            // The box worn in back slot 0 (inventory slot 20, menu slot 38) instead of held.
            edit_guest(game, |g| g.inventory[20] = g.inventory[0].take());
            let host: &mut dyn ServerHost = game;
            let menu = host.menu(123, window).unwrap();
            let player = menu.fields().player;
            let places: Vec<_> = (0..27)
                .map(|slot| MenuSlotPlace::ModItem {
                    provider: BACK,
                    key: 0,
                    slot,
                })
                .collect();
            assert_eq!(
                game.player_back_mod_menu_slots(player, window, places),
                Ok(())
            );

            // Inventory slot 9 is menu slot 27: a shift-click moves the stone into the box.
            assert!(game.place_menu_click(123, window, 27, 0, 1));
            let in_box = ItemPlace::Nested {
                parent: Box::new(back_slot(0)),
                slot: 0,
            };
            let host: &mut dyn ServerHost = game;
            assert_eq!(in_box.amount(host), 64);
            assert_eq!(ItemPlace::MenuSlot { menu, slot: 0 }.amount(host), 64);

            // Taking the box out of its place, onto the cursor or into itself, is undone.
            for mode in [0, 1] {
                assert!(game.place_menu_click(123, window, 38, 0, mode));
                let host: &mut dyn ServerHost = game;
                assert_eq!(back_slot(0).amount(host), 1, "the box stays in its place");
                assert_eq!(menu.cursor().stack(host), None);
                assert_eq!(in_box.amount(host), 64);
                assert!(host.menu(123, window).is_some());
            }

            // The box gone from its place, the write's notice closes the menu once taken.
            let host: &mut dyn ServerHost = game;
            let worn = ItemPlace::Mod {
                provider: BACK,
                key: 0,
            };
            let resource = worn.resource(host).unwrap();
            let taken =
                host.transaction(|host, tx| Ok::<_, ()>(worn.extract(host, tx, &resource, 1)));
            assert_eq!(taken, Ok(1));
            game.run_native_menu_events();
            let host: &mut dyn ServerHost = game;
            assert_eq!(host.menu(123, window), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_click_once_a_place_no_longer_resolves_closes_the_menu_and_moves_nothing() {
        with_game(|game| {
            let window = backpack_menu(game);
            edit_guest(game, |g| g.inventory[20] = g.inventory[0].take());
            let host: &mut dyn ServerHost = game;
            let menu = host.menu(123, window).unwrap();
            let player = menu.fields().player;
            // Menu slot 0 shows the box's slot 1, menu slot 1 its slot 0.
            let places = [1, 0]
                .map(|slot| MenuSlotPlace::ModItem {
                    provider: BACK,
                    key: 0,
                    slot,
                })
                .to_vec();
            assert_eq!(
                game.player_back_mod_menu_slots(player, window, places),
                Ok(())
            );
            // The box shrinks to one slot, the same stack in its place: menu slot 0 names none.
            BOX_SIZE.set(1);
            assert!(game.place_menu_click(123, window, 27, 0, 1));
            BOX_SIZE.set(27);
            let host: &mut dyn ServerHost = game;
            assert_eq!(host.menu(123, window), None, "the menu closed");
            let in_box = ItemPlace::Nested {
                parent: Box::new(back_slot(0)),
                slot: 0,
            };
            assert_eq!(in_box.amount(host), 0, "nothing went into the box");
            let carried = ItemPlace::PlayerSlot {
                player: 123,
                slot: 9,
            };
            assert_eq!(carried.amount(host), 64, "the stone stayed in the inventory");
        });
    }
}
