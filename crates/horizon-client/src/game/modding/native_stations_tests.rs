//! A workstation a mod opens with no block (game/modding/native_stations.rs).
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
use crate::game::native_item_places::tests::{guest, open_to_lan};
use horizon_client::{
    items::ItemStack,
    lan::{Out, PlayerSender},
};
use horizon_protocol::{
    buf::Reader,
    packets::play::{Clientbound as C, ContainerClick, ContainerClose, Serverbound},
};
use mod_api::{
    Hooks, PrepareItemCraftEvent, PrepareSmithingEvent, ServerHost, StationClosingEvent,
};
use std::sync::{Arc, Mutex, mpsc};

/// Guest 1 (UUID 123) carrying a stick, on a world open to LAN, and what their client is sent.
pub(in crate::game) fn stick_guest(game: &mut crate::game::Game<'_>) -> mpsc::Receiver<Out> {
    open_to_lan(game);
    let (tx, rx) = mpsc::channel();
    let mut player = guest(1);
    player.sender = PlayerSender::new(tx, Arc::default());
    player.inventory[5] = Some(ItemStack::new("minecraft:stick", 1));
    game.lan.as_mut().unwrap().hub.guests.push(player);
    rx
}

/// The packets sent since the last call.
pub(in crate::game) fn sent(rx: &mpsc::Receiver<Out>) -> Vec<C> {
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

pub(in crate::game) fn menu_key(id: &str) -> mod_api::MenuKey {
    mod_api::__private::menu_keys()
        .into_iter()
        .find(|key| mod_api::__private::menu_key_id(*key).as_deref() == Some(id))
        .unwrap()
}

/// A crafting table opened with no block starts with the mod's grid, works where the player
/// stands, stays open while they carry the mod's item and closes once they don't. As it closes
/// the mods keep the stacks of the slots they empty; a stack they put in a slot is ignored.
#[test]
#[ignore = "daily"]
fn a_table_with_no_block_keeps_to_its_item_and_lets_the_mods_keep_its_grid() {
    const CASE: &str = "HORIZON_PORTABLE_TABLE_CHILD";
    if run_fresh_cases(
        "game::native_stations_tests::a_table_with_no_block_keeps_to_its_item_and_lets_the_mods_keep_its_grid",
        CASE,
        &["keep"],
    ) {
        return;
    }
    static HEARD: Mutex<Vec<(i32, String, usize)>> = Mutex::new(Vec::new());
    struct Keep;
    #[mod_api::__subscriptions]
    impl Hooks for Keep {
        fn station_closing(&self, _: &mut dyn ServerHost, event: &mut StationClosingEvent<'_>) {
            let held = event.inputs.iter().flatten().count();
            HEARD
                .lock()
                .unwrap()
                .push((event.window, event.menu.to_owned(), held));
            event.inputs[0] = None;
            event.inputs[2] = event.inputs[1].clone();
        }
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("keep", Box::new(Keep))].into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    with_game(|game| {
        let rx = stick_guest(game);
        let player = game.player(123).unwrap();
        let plank = Some(ItemStack::new("minecraft:oak_planks", 1));
        let grid = vec![plank.clone(), plank.clone(), None, plank.clone(), plank];
        let stick = mod_api::__private::item_key("minecraft:stick").unwrap();
        let window = game
            .player_open_station(player, menu_key("minecraft:crafting"), grid, vec![stick])
            .unwrap()
            .unwrap();
        let table = |game: &crate::game::Game<'_>| {
            let menus = game.lan.as_ref().unwrap().menus.get(&1)?;
            let (w, _, menu, pos) = menus.crafting.table.as_ref()?;
            Some((*w, menu.slot(1).cloned(), menu.slot(0).map(|r| r.item.clone()), *pos))
        };
        let (w, first, result, pos) = table(game).unwrap();
        assert_eq!(w, window);
        assert_eq!(first.map(|s| s.item), Some("minecraft:oak_planks".into()));
        assert_eq!(result.as_deref(), Some("minecraft:crafting_table"));
        assert_eq!(pos, [0, 0, 0], "the block the player stands in");
        // No crafting table is there: the stick keeps it open.
        game.lan_tables_tick();
        assert!(table(game).is_some());
        sent(&rx);

        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.inventory[5] = None);
        game.lan_tables_tick();
        assert!(table(game).is_none());
        assert!(!game.is_portable_station(1, window));
        assert_eq!(
            *HEARD.lock().unwrap(),
            [(window, "minecraft:crafting".to_owned(), 4)]
        );
        assert!(sent(&rx).iter().any(|p| matches!(
            p,
            C::ContainerClose(ContainerClose { container }) if *container == window
        )));
        // Four planks in the grid: the mod kept one, and the one it put in an empty slot
        // made nothing.
        let planks: u32 = game.lan.as_ref().unwrap().hub.guests[0]
            .inventory
            .iter()
            .flatten()
            .filter(|s| &*s.item == "minecraft:oak_planks")
            .map(|s| s.count)
            .sum();
        assert_eq!(planks, 3);
    });
}

/// A table or station a mod opens with inputs shows the result the mods prepare for them, as
/// after a click, and a player who leaves with one open gets its inputs back without the
/// mods' say: `station_closing` is for a live close.
#[test]
#[ignore = "daily"]
fn a_station_opened_with_inputs_is_prepared_and_a_leaving_player_asks_no_mod() {
    const CASE: &str = "HORIZON_PORTABLE_PREPARE_CHILD";
    if run_fresh_cases(
        "game::native_stations_tests::a_station_opened_with_inputs_is_prepared_and_a_leaving_player_asks_no_mod",
        CASE,
        &["prepare"],
    ) {
        return;
    }
    static CLOSED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    struct Prepare;
    #[mod_api::__subscriptions]
    impl Hooks for Prepare {
        fn prepare_item_craft(
            &self,
            _: &mut dyn ServerHost,
            _: &mut PrepareItemCraftEvent<'_>,
        ) -> Option<mod_api::ItemStack> {
            mod_api::ItemStack::new(mod_api::__private::item_key("minecraft:diamond")?, 1)
        }
        fn prepare_smithing(
            &self,
            _: &mut dyn ServerHost,
            _: &mut PrepareSmithingEvent<'_>,
        ) -> Option<mod_api::ItemStack> {
            mod_api::ItemStack::new(mod_api::__private::item_key("minecraft:emerald")?, 1)
        }
        fn station_closing(&self, _: &mut dyn ServerHost, event: &mut StationClosingEvent<'_>) {
            CLOSED.lock().unwrap().push(event.menu.to_owned());
        }
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("prepare", Box::new(Prepare))].into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    with_game(|game| {
        let _rx = stick_guest(game);
        let player = game.player(123).unwrap();
        let stick = mod_api::__private::item_key("minecraft:stick").unwrap();
        // No recipe takes one dirt: the result is the mods'.
        let dirt = Some(ItemStack::new("minecraft:dirt", 1));
        let crafting = menu_key("minecraft:crafting");
        game.player_open_station(player, crafting, vec![dirt.clone()], vec![stick])
            .unwrap()
            .unwrap();
        let menus = game.lan.as_ref().unwrap().menus.get(&1).unwrap();
        let (_, _, table, _) = menus.crafting.table.as_ref().unwrap();
        assert_eq!(table.slot(0).map(|s| &*s.item), Some("minecraft:diamond"));
        // Opening the smithing table closes the crafting table, live.
        let smithing = menu_key("minecraft:smithing");
        game.player_open_station(player, smithing, vec![None, dirt, None], vec![stick])
            .unwrap()
            .unwrap();
        let menus = game.lan.as_ref().unwrap().menus.get(&1).unwrap();
        let station = menus.station.as_ref().unwrap();
        assert_eq!(station.menu.slot(3).map(|s| &*s.item), Some("minecraft:emerald"));

        game.guest_disconnected(1);
        assert_eq!(*CLOSED.lock().unwrap(), ["minecraft:crafting"]);
        let dirt: u32 = game.lan.as_ref().unwrap().hub.guests[0]
            .inventory
            .iter()
            .flatten()
            .filter(|s| &*s.item == "minecraft:dirt")
            .map(|s| s.count)
            .sum();
        assert_eq!(dirt, 2, "both inputs went back");
    });
}

/// A hook an open asks (`prepare_item_craft` on the restored grid) can't open another station
/// for the same player: it would take the outer one's place and lose its inputs. Neither can one
/// a click in the open table asks, while the table is out of the player's menus. Once the open
/// or the click is done, the next open works.
#[test]
#[ignore = "daily"]
fn a_station_opening_refuses_another_from_its_hooks() {
    const CASE: &str = "HORIZON_PORTABLE_REENTRY_CHILD";
    if run_fresh_cases(
        "game::native_stations_tests::a_station_opening_refuses_another_from_its_hooks",
        CASE,
        &["reenter"],
    ) {
        return;
    }
    type Answer = Result<Option<i32>, mod_api::OpenStationError>;
    static ANSWERS: Mutex<Vec<Answer>> = Mutex::new(Vec::new());
    struct Reenter;
    #[mod_api::__subscriptions]
    impl Hooks for Reenter {
        fn prepare_item_craft(
            &self,
            host: &mut dyn ServerHost,
            _: &mut PrepareItemCraftEvent<'_>,
        ) -> Option<mod_api::ItemStack> {
            let player = host.player(123)?;
            let stone =
                mod_api::ItemStack::new(mod_api::__private::item_key("minecraft:stone")?, 1);
            let stick = mod_api::__private::item_key("minecraft:stick")?;
            let crafting = menu_key("minecraft:crafting");
            let answer = player.open_station(host, crafting, vec![stone], vec![stick]);
            ANSWERS.lock().unwrap().push(answer);
            None
        }
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("reenter", Box::new(Reenter))].into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    with_game(|game| {
        let _rx = stick_guest(game);
        let player = game.player(123).unwrap();
        let stick = mod_api::__private::item_key("minecraft:stick").unwrap();
        let crafting = menu_key("minecraft:crafting");
        let dirt = Some(ItemStack::new("minecraft:dirt", 1));
        let window = game
            .player_open_station(player, crafting, vec![dirt], vec![stick])
            .unwrap()
            .unwrap();
        assert_eq!(
            *ANSWERS.lock().unwrap(),
            [Err(mod_api::OpenStationError::Busy)]
        );
        let menus = game.lan.as_ref().unwrap().menus.get(&1).unwrap();
        let (w, _, table, _) = menus.crafting.table.as_ref().unwrap();
        assert_eq!(*w, window);
        assert_eq!(
            table.slot(1).map(|s| &*s.item),
            Some("minecraft:dirt"),
            "the outer table keeps its grid"
        );
        ANSWERS.lock().unwrap().clear();
        game.lan_guest_packet(
            1,
            Serverbound::ContainerClick(ContainerClick {
                container: window,
                state: 0,
                slot: -999,
                button: 0,
                input: 0,
                changed: Vec::new(),
                carried: None,
            }),
        );
        assert_eq!(
            *ANSWERS.lock().unwrap(),
            [Err(mod_api::OpenStationError::Busy)],
            "a click asks the mods with the table busy"
        );
        let menus = game.lan.as_ref().unwrap().menus.get(&1).unwrap();
        let (w, _, table, _) = menus.crafting.table.as_ref().unwrap();
        assert_eq!(*w, window);
        assert_eq!(
            table.slot(1).map(|s| &*s.item),
            Some("minecraft:dirt"),
            "the clicked table keeps its grid"
        );
        assert!(!menus.station_busy, "the click puts the mark back");
        assert!(
            game.player_open_station(player, crafting, Vec::new(), vec![stick])
                .unwrap()
                .is_some(),
            "the next open works"
        );
    });
}
