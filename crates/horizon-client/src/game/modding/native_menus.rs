//! Server menus for native mods (docs/modding/src/how-to/native-mods.md, "Server menus"): a vanilla chest menu (`generic_9x1` to `generic_9x6`) a mod opens
//! for a player and fills with items to look at. No click moves anything:
//! each reaches the mod (`native_menu_clicked`), as does the menu's closing
//! (`native_menu_closed`), and the client shows the menu as the server sends
//! it again. A stock client shows it; nothing on the player's side. A trade
//! built on these menus moves the items between the players' `ENTITY_ITEMS`
//! handlers in one transaction, so nothing moves before both sides are settled.
use super::{Game, lan::LanHost};
use horizon_client::{
    items::ItemStack,
    lan::{data::component_nbt, menu::NativeMenuEvent},
    text::Text,
};
use mod_api::__private::game::*;
use mod_api::{MenuRef, MenuReplace};

/// The inventory slot (0..9 the hotbar, 9..36 the rest) a click on `slot`
/// of a native menu of `size` slots landed on, when it is the player's
/// inventory below the menu (`ChestMenu`: the menu's slots, then the
/// inventory's 27, then the hotbar's 9).
#[allow(dead_code)]
pub(crate) fn native_menu_inventory_slot(size: usize, slot: i32) -> Option<usize> {
    let n = usize::try_from(slot).ok()?.checked_sub(size)?;
    match n {
        0..27 => Some(n + 9),
        27..36 => Some(n - 27),
        _ => None,
    }
}

/// Adds every stack of `gets` to the 36 slots of `inventory`, onto matching
/// stacks first, then into empty slots, hotbar first; false when one doesn't
/// fit whole. `max` is an item's stack size.
pub(super) fn fit(
    inventory: &mut [Option<ItemStack>],
    gets: &[ItemStack],
    max: &impl Fn(&ItemStack) -> u32,
) -> bool {
    for stack in gets {
        let mut left = stack.count;
        let max = max(stack).max(1);
        for slot in inventory.iter_mut().take(36).flatten() {
            if left > 0 && slot.same_item_same_components(stack) && slot.count < max {
                let n = (max - slot.count).min(left);
                slot.count += n;
                left -= n;
            }
        }
        for slot in inventory.iter_mut().take(36) {
            if left > 0 && slot.is_none() {
                let n = max.min(left);
                *slot = Some(ItemStack {
                    count: n,
                    ..stack.clone()
                });
                left -= n;
            }
        }
        if left > 0 {
            return false;
        }
    }
    true
}

/// Server menus, for server mods. The players are named by UUID;
/// none of these runs while a player's state is swapped in (inside
/// `native_run_mod_command`, the menus can be opened and filled).
#[allow(dead_code)]
impl Game<'_> {
    fn native_conn(&self, uuid: u128) -> Option<u32> {
        let host = self.lan.as_ref()?;
        let g = host
            .hub
            .guests
            .iter()
            .find(|g| g.placed && g.participates_online() && g.uuid == uuid)?;
        Some(g.conn)
    }
    /// Opens a chest menu of `rows` rows (1 to 6) titled `title` for the
    /// player, showing `items` (by slot, padded with empty ones), in place of
    /// the menu they had open: its window id, a new one each time. The old
    /// menu closes as a close does, its cursor stack going back to the
    /// inventory, and `replace` says whether the client is first sent a close.
    pub(super) fn open_menu(
        &mut self,
        uuid: u128,
        rows: u8,
        title: Text,
        items: Vec<Option<ItemStack>>,
        replace: MenuReplace,
    ) -> Option<i32> {
        let conn = self.native_conn(uuid)?;
        self.close_for_open(conn, replace);
        let window = self.lan.as_mut()?.hub.open_native_menu(
            conn,
            rows,
            component_nbt(&title.component()),
            items,
        )?;
        self.player_menu_opening(conn);
        Some(window)
    }
    /// `ServerPlayer.openMenu`'s close of the menu that was open: `closeContainer`, which then
    /// tells the client, or `doCloseContainer` for a provider that skips the client's close.
    /// Either way the old menu's `removed` runs and the mods hear it close.
    fn close_for_open(&mut self, conn: u32, replace: MenuReplace) {
        match replace {
            MenuReplace::CloseFirst => self.guest_close_menus_and_tell(conn),
            MenuReplace::InPlace => self.guest_close_all(conn),
        }
    }
    /// `ServerPlayer.openMenu(provider, extraDataWriter)` for a content mod menu type and no
    /// block: the type's container slots show empty and their clicks reach the mods, as
    /// `open_menu`'s do; `extra` and `pos` go first in its `horizon:menu_extra` payload, and
    /// `replace` is the provider's choice of the client's close, as for `open_menu`.
    pub(super) fn player_open_mod_menu(
        &mut self,
        player: mod_api::PlayerRef,
        menu: mod_api::MenuKey,
        title: Text,
        extra: Vec<u8>,
        pos: Option<[i32; 3]>,
        replace: MenuReplace,
    ) -> Result<Option<i32>, mod_api::OpenMenuError> {
        use mod_api::OpenMenuError;
        let id = self.menu_key_id(menu);
        let kind =
            horizon_client::containers::mod_menu_type(&id).ok_or(OpenMenuError::NotModMenu)?;
        if extra.len() > horizon_protocol::menu_extra::MAX_DATA {
            return Err(OpenMenuError::TooLarge);
        }
        // A live handle of a connected player; a fake player has no connection.
        self.online_player_record(player)
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        let conn = self
            .native_conn(player.uuid())
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        let guest = self.lan.as_ref().and_then(|host| host.hub.guest(conn));
        let brand = guest.map_or("", |g| g.brand.as_str());
        if !horizon_client::containers::menu_reaches(brand, &id)
            // An older Horizon client reads no menu opened without a block: its
            // legacy menu-extra layout has no way to carry "no position".
            || guest.is_some_and(|g| g.sender.menu_extra_rev == 0)
        {
            return Err(OpenMenuError::PlayerUnavailable);
        }
        // The mods may stop the menu, as for a block's.
        if !self.guest_may_open(conn, &id, pos) {
            return Ok(None);
        }
        self.close_for_open(conn, replace);
        let host = self.lan.as_mut().ok_or(OpenMenuError::PlayerUnavailable)?;
        let window = host.hub.open_menu_window(
            conn,
            &id,
            component_nbt(&title.component()),
            pos.map(|[x, y, z]| (x, y, z)),
            horizon_client::lan::menu::Slots::Native,
            vec![None; kind.slots],
            Some(extra),
        );
        if window.is_some() {
            self.player_menu_opening(conn);
        }
        Ok(window)
    }
    /// The native menu the player has open as window `window`, for its slots;
    /// None for another window, a menu the game opened, or a player not here.
    pub(super) fn menu(&self, uuid: u128, window: i32) -> Option<MenuRef> {
        let conn = self.native_conn(uuid)?;
        self.lan
            .as_ref()?
            .hub
            .guest(conn)?
            .menu
            .as_ref()
            .filter(|m| {
                m.window == window && m.slots == horizon_client::lan::menu::Slots::Native
            })?;
        self.player_container_menu(self.player(uuid)?)
            .filter(|menu| menu.fields().window == window)
    }
    /// Closes the player's native menu (a `native_menu_closed` follows).
    pub(super) fn close_menu(&mut self, uuid: u128) {
        if let Some(conn) = self.native_conn(uuid)
            && let Some(host) = self.lan.as_mut()
            && host
                .hub
                .guest(conn)
                .and_then(|g| g.menu.as_ref())
                .is_some_and(|m| m.slots == horizon_client::lan::menu::Slots::Native)
        {
            self.guest_close_menus_and_tell(conn);
        }
    }
}

/// A menu a player has open, as the mods last heard it
/// (`native_player_container_open`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HeardMenu {
    /// The menu type's name; a mount's inventory has none in vanilla.
    menu: String,
    /// The block the menu is of.
    pos: Option<[i32; 3]>,
}

/// What a mount's inventory is called, as vanilla's `HorseInventoryMenu` has
/// no menu type.
const MOUNT_MENU: &str = "horizon:mount_inventory";

/// The menu `conn` has open, whichever way the game keeps it: a block's
/// container, a trade, a crafting table, a workstation, a lectern or a
/// mount's inventory. Not the player's own inventory menu.
pub(super) fn open_menu_of(host: &LanHost, conn: u32) -> Option<(i32, &str, Option<[i32; 3]>)> {
    let g = host.hub.guest(conn)?;
    let play = host.play.get(&conn);
    let menus = host.menus.get(&conn);
    if let Some(m) = &g.menu {
        let menu = host.hub.registry_name("minecraft:menu", m.kind)?;
        // A vehicle's menu is keyed by a stand-in, not a block.
        let pos = m
            .pos
            .filter(|p| p.1 != horizon_client::lan::menu::VEHICLE_KEY_Y)
            .map(|p| [p.0, p.1, p.2]);
        return Some((m.window, menu, pos));
    }
    let play = play?;
    let (window, menu, pos) = if let Some(gm) = &play.merchant {
        (gm.window, "minecraft:merchant", None)
    } else if let Some((window, _, _, pos)) = menus.and_then(|m| m.crafting.table.as_ref()) {
        (*window, "minecraft:crafting", Some(*pos))
    } else if let Some(st) = menus.and_then(|m| m.station.as_ref()) {
        (
            st.window,
            super::lan_stations::menu_of(st.kind)?.0,
            Some(st.pos),
        )
    } else if let Some(l) = &play.lectern {
        (
            l.window,
            "minecraft:lectern",
            Some([l.key.0, l.key.1, l.key.2]),
        )
    } else if let Some(m) = &play.mount_window {
        (m.window, MOUNT_MENU, None)
    } else {
        return None;
    };
    Some((window, menu, pos))
}

/// Where the game calls the menu hook points.
impl Game<'_> {
    /// `ServerPlayer.openMenu` / `openHorseInventory`: called only after
    /// constructing the menu, queuing its open packet and sending full contents.
    /// Remember it before dispatch so a hook can close it synchronously.
    pub(super) fn player_menu_opening(&mut self, conn: u32) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some((_, menu, pos)) = open_menu_of(host, conn) else {
            return;
        };
        let menu = HeardMenu {
            menu: menu.to_owned(),
            pos,
        };
        host.menus.entry(conn).or_default().menu_heard = Some(menu.clone());
        self.player_menu_opened(conn, menu);
    }

    /// `conn`'s menu closes with the guest in the level in place
    /// (`ServerPlayer.doCloseContainer`): the mods hear it, if they heard it
    /// open.
    pub(super) fn player_menu_closing(&mut self, conn: u32) {
        let heard = self
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            .and_then(|p| p.menu_heard.take());
        if let Some(old) = heard {
            self.player_menu_closed(conn, old);
        }
    }

    fn player_menu_opened(&mut self, conn: u32, menu: HeardMenu) {
        let Some((uuid, name)) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone()))
        else {
            return;
        };
        let dimension = self.dim.id();
        self.player_hook(conn, move |g| {
            g.native_player_container_open(uuid, &name, dimension, &menu.menu, menu.pos);
        });
    }

    fn player_menu_closed(&mut self, conn: u32, menu: HeardMenu) {
        let Some((uuid, name)) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone()))
        else {
            return;
        };
        let dimension = self.dim.id();
        self.player_hook(conn, move |g| {
            g.native_player_container_close(uuid, &name, dimension, &menu.menu, menu.pos);
        });
    }

    /// Runs a player hook with the guest's state in place, or as it is when
    /// the guest has no body to swap in.
    fn player_hook(&mut self, conn: u32, f: impl FnOnce(&mut Self)) {
        let mut f = Some(f);
        if self.lan_acting != Some(conn) {
            self.as_guest(conn, |g| {
                if let Some(f) = f.take() {
                    f(g);
                }
            });
        }
        if let Some(f) = f {
            f(self);
        }
    }

    /// The clicks on and closings of native menus the hub collected, each
    /// told to the mods with the hub in `self.lan`, no player's state
    /// swapped in.
    pub(super) fn run_native_menu_events(&mut self) {
        loop {
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if host.hub.native_menu_events.is_empty() {
                return;
            }
            match host.hub.native_menu_events.remove(0) {
                NativeMenuEvent::Click {
                    uuid,
                    window,
                    slot,
                    button,
                    mode,
                } => self.native_menu_clicked(uuid, window, slot, button, mode),
                NativeMenuEvent::Closed { uuid, window } => self.native_menu_closed(uuid, window),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::api_values::TryIntoApi;
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_item_places::tests::{guest, open_to_lan};
    use super::*;
    use horizon_client::lan::{Out, PlayerSender};
    use horizon_protocol::{
        buf::Reader,
        packets::play::{self, Clientbound as C, Serverbound as S},
    };
    use mod_api::{HostTransaction, ItemPlace, ItemResource, ServerHost};
    use std::sync::{Arc, mpsc};

    /// Guest 1 (UUID 123) on a world open to LAN, and what their client is sent.
    fn watched_guest(game: &mut Game<'_>) -> mpsc::Receiver<Out> {
        open_to_lan(game);
        let (tx, rx) = mpsc::channel();
        let mut player = guest(1);
        player.sender = PlayerSender::new(tx, Arc::default());
        game.lan.as_mut().unwrap().hub.guests.push(player);
        rx
    }

    /// The packets sent since the last call.
    fn sent(rx: &mpsc::Receiver<Out>) -> Vec<C> {
        rx.try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                _ => Vec::new(),
            })
            .map(|body| {
                let mut reader = Reader::new(&body);
                let id = reader.varint().unwrap();
                C::decode(id, reader.rest()).unwrap()
            })
            .collect()
    }

    fn open(game: &mut Game<'_>, rows: u8, title: &str, replace: MenuReplace) -> i32 {
        let host: &mut dyn ServerHost = game;
        host.open_menu(
            123,
            rows,
            mod_api::Text::literal(title),
            Vec::new(),
            replace,
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn container_hooks_follow_actual_open_boundaries_without_a_tick() {
        use mod_api::{Hooks, ModEntry, Mods, PlayerContainerCloseEvent, PlayerContainerOpenEvent};
        use std::sync::Mutex;
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_menus::tests::container_hooks_follow_actual_open_boundaries_without_a_tick",
            "HORIZON_CONTAINER_BOUNDARY_CASE",
            &["same_tick", "replace", "mount", "guest"],
        ) {
            return;
        }
        struct Watch {
            events: Arc<Mutex<Vec<(bool, String)>>>,
            packets: Arc<Mutex<Option<mpsc::Receiver<Out>>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Watch {
            fn player_container_open(
                &self,
                _host: &mut dyn ServerHost,
                event: &mut PlayerContainerOpenEvent<'_>,
            ) {
                // Both packets must precede the callback, not just the open's return.
                let packets = self.packets.lock().unwrap();
                let packets = sent(packets.as_ref().unwrap());
                let open = packets
                    .iter()
                    .find_map(|packet| match packet {
                        C::OpenScreen(p) => Some(p.container),
                        C::MountScreenOpen(p) => Some(p.container),
                        _ => None,
                    })
                    .unwrap();
                assert!(packets.iter().any(|packet| {
                    matches!(packet, C::ContainerSetContent(p) if p.container == open)
                }));
                self.events
                    .lock()
                    .unwrap()
                    .push((true, event.menu.to_string()));
            }
            fn player_container_close(
                &self,
                _host: &mut dyn ServerHost,
                event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                self.events
                    .lock()
                    .unwrap()
                    .push((false, event.menu.to_string()));
            }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let packets = Arc::new(Mutex::new(None));
        let mods: Mods = vec![ModEntry::new(
            "watch",
            Box::new(Watch {
                events: events.clone(),
                packets: packets.clone(),
            }),
        )]
        .into();
        mod_platform::__private::install(mods.clone());
        crate::game::api_values::install_parts(&mods).unwrap();
        with_game(|game| {
            *packets.lock().unwrap() = Some(watched_guest(game));
            let case = std::env::var("HORIZON_CONTAINER_BOUNDARY_CASE").unwrap();
            // Own inventory and unsuccessful construction have no open boundary.
            game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 0 }));
            game.lan_open_mount(1, u32::MAX);
            assert!(!game.lan_open_container(1, (0, 64, 0)));
            assert!(events.lock().unwrap().is_empty());
            if case == "mount" {
                game.mobs.push(horizon_client::mobs::Mob::new(
                    31,
                    game.res.mob_types().get("minecraft:horse").unwrap().clone(),
                    horizon_core::geometry::Vec3::new(0.0, 0.0, 0.0),
                    0.0,
                ));
                game.lan_open_mount(1, 31);
                assert_eq!(
                    open_menu_of(game.lan.as_ref().unwrap(), 1),
                    Some((1, "horizon:mount_inventory", None))
                );
                game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 1 }));
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "horizon:mount_inventory".into()),
                        (false, "horizon:mount_inventory".into()),
                    ]
                );
                return;
            }
            let first = if case == "guest" {
                let mut window = None;
                game.as_guest(1, |game| {
                    window = Some(open(game, 1, "first", MenuReplace::CloseFirst));
                });
                window.unwrap()
            } else {
                open(game, 1, "first", MenuReplace::CloseFirst)
            };
            assert_eq!(first, 1);
            assert_eq!(
                open_menu_of(game.lan.as_ref().unwrap(), 1),
                Some((1, "minecraft:generic_9x1", None))
            );
            assert_eq!(
                *events.lock().unwrap(),
                vec![(true, "minecraft:generic_9x1".into())]
            );
            if case == "replace" {
                assert_eq!(open(game, 2, "second", MenuReplace::InPlace), 2);
                // A stale close must not notify the replacement.
                game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 1 }));
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "minecraft:generic_9x1".into()),
                        (false, "minecraft:generic_9x1".into()),
                        (true, "minecraft:generic_9x2".into()),
                    ]
                );
                game.close_menu(123);
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "minecraft:generic_9x1".into()),
                        (false, "minecraft:generic_9x1".into()),
                        (true, "minecraft:generic_9x2".into()),
                        (false, "minecraft:generic_9x2".into()),
                    ]
                );
            } else {
                game.lan_guest_packet(
                    1,
                    S::ContainerClose(play::ContainerClose { container: first }),
                );
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "minecraft:generic_9x1".into()),
                        (false, "minecraft:generic_9x1".into()),
                    ]
                );
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_slot_written_outside_a_click_is_one_set_slot_at_the_next_sync() {
        with_game(|game| {
            let rx = watched_guest(game);
            let window = open(game, 1, "shop", MenuReplace::CloseFirst);
            assert_eq!(window, 1);
            let host: &mut dyn ServerHost = game;
            assert_eq!(host.menu(123, 2), None, "another window");
            assert_eq!(host.menu(124, 1), None, "another player");
            let menu = host.menu(123, 1).unwrap();
            assert_eq!(menu.container_id(host), Some(1));
            sent(&rx);

            let diamond = ItemStack::new("minecraft:diamond", 1)
                .try_into_api()
                .unwrap();
            let slot = ItemPlace::MenuSlot { menu, slot: 4 };
            let written = host.transaction(|host, tx| {
                Ok::<_, ()>(slot.insert(host, tx, &ItemResource::of(&diamond), 3))
            });
            assert_eq!(written, Ok(3));
            assert!(sent(&rx).is_empty(), "nothing before the tick's sync");

            game.lan.as_mut().unwrap().hub.sync_containers(&game.world);
            let packets = sent(&rx);
            assert_eq!(packets.len(), 1, "{packets:?}");
            let C::ContainerSetSlot(set) = &packets[0] else {
                panic!("{packets:?}");
            };
            // The open's content took state 1; this is the next.
            assert_eq!((set.container, set.state, set.slot), (1, 2, 4));
            assert_eq!(set.item.as_ref().map(|item| item.count), Some(3));
            game.lan.as_mut().unwrap().hub.sync_containers(&game.world);
            assert!(sent(&rx).is_empty(), "sent once");

            // Closed, the menu answers nothing.
            game.close_menu(123);
            let host: &mut dyn ServerHost = game;
            assert_eq!(host.menu(123, 1), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_menu_opened_in_place_sends_no_close_returns_the_cursor_and_ignores_the_old_window() {
        with_game(|game| {
            let rx = watched_guest(game);
            let closes = |packets: &[C]| -> Vec<i32> {
                packets
                    .iter()
                    .filter_map(|p| match p {
                        C::ContainerClose(close) => Some(close.container),
                        _ => None,
                    })
                    .collect()
            };
            let opens = |packets: &[C]| -> Vec<(i32, horizon_core::nbt::Tag)> {
                packets
                    .iter()
                    .filter_map(|p| match p {
                        C::OpenScreen(open) => Some((open.container, open.title.clone())),
                        _ => None,
                    })
                    .collect()
            };
            // From the inventory nothing is open, so nothing closes.
            assert_eq!(open(game, 1, "one", MenuReplace::CloseFirst), 1);
            let packets = sent(&rx);
            assert_eq!(closes(&packets), Vec::<i32>::new());
            assert_eq!(opens(&packets).len(), 1);

            let stone = ItemStack::new("minecraft:stone", 4);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.carried = Some(stone.clone()));
            game.lan.as_mut().unwrap().hub.native_menu_events.clear();
            assert_eq!(open(game, 2, "two", MenuReplace::InPlace), 2);
            let packets = sent(&rx);
            assert_eq!(
                closes(&packets),
                Vec::<i32>::new(),
                "no close: the mouse stays"
            );
            assert_eq!(
                opens(&packets),
                vec![(2, component_nbt(&Text::literal("two").component()))]
            );
            let g = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(g.carried, None, "the cursor stack went back");
            assert_eq!(
                g.inventory.iter().flatten().cloned().collect::<Vec<_>>(),
                vec![stone]
            );
            assert_eq!(g.menu.as_ref().map(|m| (m.window, m.size)), Some((2, 18)));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.native_menu_events,
                vec![NativeMenuEvent::Closed {
                    uuid: 123,
                    window: 1
                }],
                "the old menu closed on the server"
            );

            // The old window's click and close land on nothing.
            game.lan.as_mut().unwrap().hub.native_menu_events.clear();
            game.lan_guest_packet(
                1,
                S::ContainerClick(play::ContainerClick {
                    container: 1,
                    state: 1,
                    slot: 0,
                    button: 0,
                    input: 0,
                    changed: Vec::new(),
                    carried: None,
                }),
            );
            game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 1 }));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.native_menu_events,
                Vec::new()
            );
            let host: &mut dyn ServerHost = game;
            assert_eq!(host.menu(123, 1), None);
            assert!(host.menu(123, 2).is_some(), "the new menu stays open");

            // The default sends the close first.
            sent(&rx);
            assert_eq!(open(game, 3, "three", MenuReplace::CloseFirst), 3);
            let packets = sent(&rx);
            assert_eq!(closes(&packets), vec![2]);
            assert_eq!(opens(&packets).len(), 1);
            let close_at = packets
                .iter()
                .position(|p| matches!(p, C::ContainerClose(_)))
                .unwrap();
            let open_at = packets
                .iter()
                .position(|p| matches!(p, C::OpenScreen(_)))
                .unwrap();
            assert!(close_at < open_at);
        });
    }

    /// The content mod menu type the mod-menu tests open: a prepared registry slot
    /// (`minecraft:generic_9x1`) for a synthetic definition, in the fresh process each case
    /// runs in, which keeps the one-shot menu table out of other fixtures.
    const MOD_MENU: &str = "minecraft:generic_9x1";

    fn register_mod_menu() {
        assert!(horizon_client::containers::register_mod_menu_types([(
            MOD_MENU.into(),
            9,
            0,
            Vec::new(),
            None,
            None,
        )]));
    }

    /// [`watched_guest`] on a Horizon client that reads a menu opened with no block, and the
    /// key of [`MOD_MENU`].
    fn mod_menu_guest(game: &mut Game<'_>) -> (mpsc::Receiver<Out>, mod_api::MenuKey) {
        let rx = watched_guest(game);
        game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.brand = "horizon".into();
            g.sender.menu_extra_rev = 1;
        });
        let key = mod_api::__private::menu_keys()
            .into_iter()
            .find(|key| mod_api::__private::menu_key_id(*key).as_deref() == Some(MOD_MENU))
            .unwrap();
        (rx, key)
    }

    #[test]
    #[ignore = "daily"]
    fn a_mod_menu_opened_in_place_sends_no_close_and_still_closes_the_old_menu() {
        use mod_api::{PlayerContainerCloseEvent, PlayerContainerOpenEvent};
        use std::sync::Mutex;
        const CASE: &str = "HORIZON_MOD_MENU_REPLACE_CHILD";
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_menus::tests::a_mod_menu_opened_in_place_sends_no_close_and_still_closes_the_old_menu",
            CASE,
            &["close_first", "in_place"],
        ) {
            return;
        }
        // What the mods heard, in order.
        static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());
        struct Listen;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Listen {
            fn player_container_open(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerContainerOpenEvent<'_>,
            ) {
                HEARD.lock().unwrap().push(format!("open {}", event.menu));
            }
            fn player_container_close(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                HEARD.lock().unwrap().push(format!("close {}", event.menu));
            }
        }
        let heard = || std::mem::take(&mut *HEARD.lock().unwrap());
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("listen", Box::new(Listen))].into();
        mod_platform::__private::install(mods.clone());
        crate::game::api_values::install_parts(&mods).unwrap();
        let replace = match std::env::var(CASE).unwrap().as_str() {
            "close_first" => MenuReplace::CloseFirst,
            _ => MenuReplace::InPlace,
        };
        register_mod_menu();
        with_game(|game| {
            let (rx, key) = mod_menu_guest(game);
            // Where the game keeps what the mods last heard the player open.
            game.lan.as_mut().unwrap().play.entry(1).or_default();
            let player = game.player(123).unwrap();
            assert_eq!(open(game, 3, "old", MenuReplace::CloseFirst), 1);
            assert_eq!(heard(), ["open minecraft:generic_9x3"]);
            sent(&rx);
            let stone = ItemStack::new("minecraft:stone", 4);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.carried = Some(stone.clone()));

            let window = game
                .player_open_mod_menu(player, key, Text::literal("new"), vec![7], None, replace)
                .unwrap()
                .unwrap();
            assert_eq!(window, 2);
            // The old menu's close, heard once, before anything of the new one.
            assert_eq!(
                heard(),
                [
                    "close minecraft:generic_9x3".to_owned(),
                    format!("open {MOD_MENU}")
                ]
            );
            let packets: Vec<String> = sent(&rx)
                .iter()
                .map(|p| match p {
                    C::ContainerClose(close) => format!("close {}", close.container),
                    C::CustomPayload(payload) => format!("payload {}", payload.channel),
                    C::OpenScreen(open) => format!("open {}", open.container),
                    C::ContainerSetContent(content) => format!(
                        "content {} state {} slots {}",
                        content.container,
                        content.state,
                        content.items.len()
                    ),
                    other => format!("{other:?}"),
                })
                .collect();
            // The client's close of the old window for `CloseFirst` only (queued before the
            // removal, as `closeContainer` does), the inventory with the cursor stack back in
            // it, then the new menu's extra data, screen and contents.
            let mut expected = vec![
                "content 0 state 1 slots 46",
                "payload horizon:menu_extra",
                "open 2",
                "content 2 state 1 slots 45",
            ];
            if replace == MenuReplace::CloseFirst {
                expected.insert(0, "close 1");
            }
            assert_eq!(packets, expected);
            let g = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(g.carried, None, "the cursor stack went back");
            assert_eq!(
                g.inventory.iter().flatten().cloned().collect::<Vec<_>>(),
                vec![stone]
            );
            assert_eq!(g.menu.as_ref().map(|m| (m.window, m.size)), Some((2, 9)));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.native_menu_events,
                vec![NativeMenuEvent::Closed {
                    uuid: 123,
                    window: 1
                }],
                "the old menu closed on the server"
            );

            // Both notifications were delivered inline, without a second close.
            assert!(heard().is_empty());
        });
    }

    #[test]
    #[ignore = "needs prepared resources; isolated menu registration"]
    fn mod_menu_backend_payload_and_refusals() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_menus::tests::mod_menu_backend_payload_and_refusals",
            "HORIZON_MOD_MENU_BACKEND_CHILD",
            &["backend"],
        ) {
            return;
        }
        register_mod_menu();
        with_game(|game| {
            let (rx, key) = mod_menu_guest(game);
            let player = game.player(123).unwrap();
            let old = open(game, 1, "old", MenuReplace::CloseFirst);
            sent(&rx);
            let bytes = vec![0, 255, 17];
            let window = game
                .player_open_mod_menu(
                    player,
                    key,
                    Text::literal("new"),
                    bytes.clone(),
                    None,
                    MenuReplace::CloseFirst,
                )
                .unwrap()
                .unwrap();
            assert_eq!(window, old + 1);
            let packets = sent(&rx);
            let payload_at = packets
                .iter()
                .position(|p| {
                    matches!(p,
                        C::CustomPayload(p) if p.channel == horizon_protocol::menu_extra::CHANNEL
                    )
                })
                .unwrap();
            let open_at = packets
                .iter()
                .position(|p| {
                    matches!(p,
                        C::OpenScreen(p) if p.container == window
                    )
                })
                .unwrap();
            assert!(payload_at < open_at, "extra data must precede its window");
            let C::CustomPayload(payload) = &packets[payload_at] else {
                unreachable!()
            };
            assert_eq!(
                horizon_protocol::menu_extra::decode(&payload.data).unwrap(),
                horizon_protocol::menu_extra::Message {
                    window,
                    pos: None,
                    data: bytes
                }
            );
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .as_ref()
                    .map(|m| (m.window, m.pos)),
                Some((window, None))
            );

            for (revision, extra, expected) in [
                (1, vec![0; 33 * 1024], mod_api::OpenMenuError::TooLarge),
                (0, vec![1], mod_api::OpenMenuError::PlayerUnavailable),
            ] {
                game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.sender.menu_extra_rev = revision;
                });
                game.lan.as_mut().unwrap().hub.native_menu_events.clear();
                assert_eq!(
                    game.player_open_mod_menu(
                        player,
                        key,
                        Text::literal("refused"),
                        extra,
                        None,
                        MenuReplace::InPlace,
                    ),
                    Err(expected)
                );
                assert_eq!(
                    game.lan.as_ref().unwrap().hub.guests[0]
                        .menu
                        .as_ref()
                        .map(|m| m.window),
                    Some(window),
                    "refusal preserves the current menu"
                );
                assert!(game.lan.as_ref().unwrap().hub.native_menu_events.is_empty());
                assert!(sent(&rx).is_empty(), "refusal sends neither close nor open");
            }
        });
    }

    #[test]
    fn menu_clicks_map_to_the_inventory_below() {
        // A 6-row chest: 54 slots, then the inventory's 27, then the hotbar.
        assert_eq!(native_menu_inventory_slot(54, 3), None);
        assert_eq!(native_menu_inventory_slot(54, 54), Some(9));
        assert_eq!(native_menu_inventory_slot(54, 80), Some(35));
        assert_eq!(native_menu_inventory_slot(54, 81), Some(0));
        assert_eq!(native_menu_inventory_slot(54, 89), Some(8));
        assert_eq!(native_menu_inventory_slot(54, 90), None);
        assert_eq!(native_menu_inventory_slot(54, -999), None);
        assert_eq!(native_menu_inventory_slot(9, 9), Some(9));
    }
}
