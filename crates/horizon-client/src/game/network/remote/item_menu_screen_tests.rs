//! Open packets from block-backed and held-item-backed menus.
use super::*;
use crate::game::{
    execution_role::ExecutionRole,
    native_entity_refs::tests::{run_fresh_cases, with_game_role},
    native_guest_turn_tests::with_guests_at,
    native_item_places::tests::{guest, open_to_lan},
};
use horizon_client::{
    block_entities::Pos,
    containers::ModBlockEntityFields,
    lan::{Out, PlayerSender},
};
use horizon_protocol::{buf::Reader, connection::Pipe};
use mod_api::{
    BlockEntityPart, ClientHost, Flow, Hooks, Host, ItemPlace, MenuPart, MenuReplace,
    MenuScreenEvent, MenuSlotPlace, ModEntry, Mods, Registrar, RegistrationContext,
    ScreenDrawEvent, ScreenEvent, ScreenMouseButtonPressedPreEvent, ScreenRenderBackgroundEvent,
    ServerHost,
};
use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

const MENU: &str = "packtest:storage";
const MACHINE: &str = "packtest:machine";
const CHEST_BLOCK: &str = "packtest:chest";
const CHEST_MENU: &str = "minecraft:generic_9x1";
const MACHINE_POS: Pos = (1, 64, 1);
const CHILD_CASE: &str = "HORIZON_ITEM_MENU_SCREEN_CASE";
static BLOCK_DRAWS: AtomicUsize = AtomicUsize::new(0);
static ITEM_DRAWS: AtomicUsize = AtomicUsize::new(0);

struct Storage;
#[mod_api::horizon_part]
impl MenuPart for Storage {
    fn menu_screen(
        &self,
        _: &dyn ClientHost,
        event: &mut MenuScreenEvent<'_>,
    ) -> Option<mod_api::Screen> {
        assert_eq!(event.menu, MENU);
        Some(mod_api::Screen {
            width: 196.0,
            height: 154.0,
            slots: vec![Some([29.0, 20.0]), Some([80.0, 30.0])],
            inventory_x: 18.0,
            inventory_y: 72.0,
            ..Default::default()
        })
    }
}

struct Machine;
#[mod_api::horizon_part]
impl mod_api::BlockPart for Machine {}

#[mod_api::horizon_part]
impl BlockEntityPart for Machine {
    fn screen(&self, _: &dyn Host, event: &mut ScreenEvent<'_>) -> Option<mod_api::Screen> {
        assert!(matches!(event.block, MACHINE | CHEST_BLOCK));
        Some(mod_api::Screen {
            width: 176.0,
            height: 165.0,
            slots: vec![Some([35.0, 41.0]), None],
            inventory_x: 8.0,
            inventory_y: 83.0,
            ..Default::default()
        })
    }

    fn screen_draw(&self, _: &dyn ClientHost, event: &mut ScreenDrawEvent<'_>) {
        assert!(matches!(event.block, MACHINE | CHEST_BLOCK));
        BLOCK_DRAWS.fetch_add(1, Ordering::Relaxed);
    }
}

struct TestMod;
#[mod_api::__subscriptions]
impl Hooks for TestMod {
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        let kind = registrar
            .block_entity_type("machine")
            .blocks(&[MACHINE, CHEST_BLOCK])
            .build();
        registrar.block_entity_part(&kind, Machine).unwrap();
        for (block, slots, menu) in [(MACHINE, 2, MENU), (CHEST_BLOCK, 9, CHEST_MENU)] {
            let block = registrar.block_key(block);
            registrar.block_container(
                &block,
                mod_api::ModContainer {
                    slots,
                    menu: Some(menu.into()),
                    ..Default::default()
                },
            );
        }
        let item = registrar.item_key("minecraft:shulker_box");
        registrar.item_container(&item, |_: &dyn Host| Some(2));
        registrar.menu_part(MENU, Storage);
    }

    fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        for block in [MACHINE, CHEST_BLOCK] {
            event
                .blocks
                .push(mod_api::Block::new(block, "minecraft:stone"));
        }
    }

    fn screen_render_background(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenRenderBackgroundEvent<'_>,
    ) {
        if host.screen_pos().is_none() && event.screen.menu == Some(MENU) {
            assert_eq!(&event.screen.rect[2..], &[196.0, 154.0]);
            ITEM_DRAWS.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn screen_mouse_button_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonPressedPreEvent<'_>,
    ) -> Flow {
        if host.screen_pos().is_none() && event.screen.menu == Some(MENU) && event.button == 0 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

fn content_root(package: &Path) -> PathBuf {
    let menu_path = package.join("data/packtest/horizon/menu");
    fs::create_dir_all(&menu_path).unwrap();
    fs::write(
        package.join("horizon-mod.toml"),
        "id = \"packtest\"\nname = \"Storage test\"\nversion = \"1.0\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n",
    )
    .unwrap();
    fs::write(menu_path.join("storage.json"), r#"{"slots": 2}"#).unwrap();
    let base = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let selected = horizon_mod_package::selection::SelectedPackage {
        path: package.to_owned(),
        package: horizon_mod_package::Package::read(package).unwrap(),
    };
    let sources = crate::game::mods::ModsState {
        dev: vec![package.to_owned()],
        selected: vec![selected].into(),
        ..Default::default()
    };
    let root = crate::game::mods::content_root(&base, &sources).unwrap();
    assert!(horizon_client::containers::mod_menu_type(MENU).is_some());
    root.prepared().to_owned()
}

fn place_machine(game: &mut Game<'_>, block: &str, slots: usize) {
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let state = game.pack.assets.registry.default_state(block).unwrap();
    let (x, y, z) = MACHINE_POS;
    game.world.set(x, y, z, state).unwrap();
    game.world.block_entities.insert(
        MACHINE_POS,
        BlockEntity::Mod(Box::new(horizon_client::containers::ModBlockEntity::new(
            MACHINE.into(),
            ModBlockEntityFields {
                items: vec![None; slots],
                ..Default::default()
            },
        ))),
    );
}

fn take_menu_packets(rx: &mpsc::Receiver<Out>) -> Vec<C> {
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
        .filter(|packet| {
            matches!(
                packet,
                C::CustomPayload(_)
                    | C::OpenScreen(_)
                    | C::ContainerSetContent(_)
                    | C::ContainerSetSlot(_)
                    | C::SetCursorItem(_)
            )
        })
        .collect()
}

fn open_item_menu(game: &mut Game<'_>, uuid: u128) -> i32 {
    let host: &mut dyn ServerHost = game;
    let player = host.player(uuid).unwrap();
    let menu = host
        .menu_keys()
        .into_iter()
        .find(|key| key.id(host) == MENU)
        .unwrap();
    let window = player
        .open_mod_menu(
            host,
            menu,
            mod_api::Text::literal("Held storage"),
            vec![7],
            None,
            MenuReplace::InPlace,
        )
        .unwrap()
        .unwrap();
    back_held_slots(host, player, window);
    window
}

fn back_held_slots(host: &mut dyn ServerHost, player: mod_api::PlayerRef, window: i32) {
    let places = (0..2)
        .map(|slot| MenuSlotPlace::HeldItem {
            hand: mod_api::Hand::Main,
            slot,
        })
        .collect();
    player.back_mod_menu_slots(host, window, places).unwrap();
}

struct MenuPackets {
    block: Vec<C>,
    item: Vec<C>,
    after_move: Vec<C>,
    chest_block: Vec<C>,
    chest_item: Vec<C>,
}

fn server_menu_packets(game: &mut Game<'_>, conn: u32) -> MenuPackets {
    let (tx, rx) = mpsc::channel();
    let host = game.lan.as_mut().unwrap();
    let uuid = host.hub.guest(conn).unwrap().uuid;
    host.hub
        .update_guest(conn, |player| {
            player.sender = PlayerSender::new(tx, Arc::default());
            player.sender.menu_extra_rev = 1;
            player.brand = "horizon".into();
            player.selected = 0;
            player.inventory[0] = Some(ItemStack::new("minecraft:shulker_box", 1));
            player.inventory[9] = Some(ItemStack::new("minecraft:stone", 7));
        })
        .unwrap();

    // The game fixture does not run the first server tick that installs this table.
    let containers = game.native_containers();
    game.world.block_entities.set_mod_containers(containers);
    place_machine(game, MACHINE, 2);
    assert_eq!(game.mod_menu_at(MACHINE_POS), Some(MENU));
    assert!(game.lan_open_container(conn, MACHINE_POS));
    assert_eq!(game.native_hidden_slots(MACHINE_POS, 2), [1]);
    let block = take_menu_packets(&rx);

    let window = open_item_menu(game, uuid);
    let item = take_menu_packets(&rx);
    let state = item
        .iter()
        .filter_map(|packet| match packet {
            C::ContainerSetContent(content) if content.container == window => Some(content.state),
            _ => None,
        })
        .next_back()
        .unwrap();
    game.lan_guest_packet(
        conn,
        S::ContainerClick(play::ContainerClick {
            container: window,
            state,
            slot: 2,
            button: 0,
            input: 1,
            changed: Vec::new(),
            carried: None,
        }),
    );
    game.run_native_menu_events();

    let held = ItemPlace::Nested {
        parent: Box::new(ItemPlace::PlayerHand {
            player: uuid,
            hand: mod_api::Hand::Main,
        }),
        slot: 0,
    };
    assert_eq!(
        held.amount(game),
        7,
        "the move writes through the held item"
    );
    let after_move = take_menu_packets(&rx);
    place_machine(game, CHEST_BLOCK, 9);
    assert!(game.lan_open_container(conn, MACHINE_POS));
    let chest_block = take_menu_packets(&rx);

    let host: &mut dyn ServerHost = game;
    let player = host.player(uuid).unwrap();
    let window = host
        .open_menu(
            uuid,
            1,
            mod_api::Text::literal("Storage in a chest layout"),
            Vec::new(),
            MenuReplace::InPlace,
        )
        .unwrap()
        .unwrap();
    back_held_slots(host, player, window);
    let chest_item = take_menu_packets(&rx);

    MenuPackets {
        block,
        item,
        after_move,
        chest_block,
        chest_item,
    }
}

fn apply(game: &mut Game<'_>, packets: Vec<C>) {
    assert!(!packets.is_empty());
    for packet in packets {
        game.handle_packet(packet).unwrap();
    }
}

fn check_client_menus(game: &mut Game<'_>, packets: MenuPackets) {
    place_machine(game, MACHINE, 2);
    apply(game, packets.block);
    assert_eq!(
        game.screen_pos(),
        Some([MACHINE_POS.0, MACHINE_POS.1, MACHINE_POS.2])
    );
    assert_eq!(game.native_screen_width(), Some(176.0));
    assert_eq!(game.native_screen_slots(), Some(vec![300]));

    let mut batch = HudBatch::scratch([640, 480]);
    assert!(game.draw_native_screen(&mut batch, 40.0, 30.0));
    assert_eq!(BLOCK_DRAWS.load(Ordering::Relaxed), 1);
    assert_eq!(ITEM_DRAWS.load(Ordering::Relaxed), 0);

    // The last click was on the matching block. This open has no block.
    game.remote.as_mut().unwrap().use_target = Some([MACHINE_POS.0, MACHINE_POS.1, MACHINE_POS.2]);
    apply(game, packets.item);
    assert_eq!(game.screen_pos(), None);
    assert_eq!(game.screen_extra_data(), Some(vec![7]));
    assert_eq!(game.native_screen_width(), Some(196.0));
    assert_eq!(game.native_screen_slots(), Some(vec![300, 301]));
    assert_eq!(game.slot_position(301), (80.0, 30.0));

    assert!(game.draw_native_screen(&mut batch, 40.0, 30.0));
    assert_eq!(BLOCK_DRAWS.load(Ordering::Relaxed), 1);
    assert_eq!(ITEM_DRAWS.load(Ordering::Relaxed), 1);
    let view = game.screen_view(&batch, Screen::Inventory);
    let mut canceled = false;
    game.native_screen_mouse_button_pressed_pre(&view, 0, 0, &mut canceled);
    assert!(canceled, "a mod can match its control by the menu id");

    apply(game, packets.after_move);
    let key = game.container.unwrap();
    let slots = game.world.block_entities.get(&key).unwrap().slots();
    let shown = slots[0].as_ref().unwrap();
    assert_eq!(
        shown.count, 7,
        "the client shows the new stack in the declared slot"
    );
    assert!(slots[1].is_none());

    check_chest_screen_origins(game, packets.chest_block, packets.chest_item);
}

fn check_chest_screen_origins(game: &mut Game<'_>, block: Vec<C>, item: Vec<C>) {
    let origin = |packet: &C| matches!(packet, C::CustomPayload(p) if p.channel == horizon_protocol::menu_extra::CHANNEL);
    assert!(block.iter().any(origin));
    assert!(!item.iter().any(origin));
    let bare_block = block.iter().filter(|p| !origin(p)).cloned().collect();

    place_machine(game, CHEST_BLOCK, 9);
    apply(game, block);
    assert_eq!(game.native_screen_slots(), Some(vec![300]));

    // The stock chest opened by the item has the same type as the last clicked block.
    let clicked = [MACHINE_POS.0, MACHINE_POS.1, MACHINE_POS.2];
    game.remote.as_mut().unwrap().use_target = Some(clicked);
    apply(game, item);
    assert_eq!(game.remote_menu_type(), Some(CHEST_MENU));
    assert_eq!(game.native_screen_slots(), None);
    assert_eq!(game.slot_count(), 45);

    // An older server that never negotiated menu-extra can open a block's bare window.
    let remote = game.remote.as_mut().unwrap();
    Arc::make_mut(&mut remote.registries).menu_extra_revision = 0;
    remote.use_target = Some(clicked);
    apply(game, bare_block);
    assert_eq!(game.native_screen_slots(), Some(vec![300]));
}

#[test]
#[ignore = "daily"]
fn item_menu_layout_keeps_held_slots_and_selects_the_matching_screen() {
    if run_fresh_cases(
        "game::remote::item_menu_screen_tests::item_menu_layout_keeps_held_slots_and_selects_the_matching_screen",
        CHILD_CASE,
        &["integrated", "remote"],
    ) {
        return;
    }
    let remote = std::env::var(CHILD_CASE).unwrap() == "remote";
    let mods: Mods = vec![ModEntry::new("packtest", Box::new(TestMod))].into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_api::__private::mark_game_thread();
    let scratch = tempfile::tempdir().unwrap();
    let root = content_root(&scratch.path().join("packtest"));

    let mut packets = None;
    if remote {
        with_game_role(root.clone(), ExecutionRole::DedicatedServer, |game| {
            open_to_lan(game);
            game.lan.as_mut().unwrap().hub.guests.push(guest(1));
            packets = Some(server_menu_packets(game, 1));
        });
    } else {
        with_guests_at(root.clone(), |game| {
            packets = Some(server_menu_packets(game, 12));
        });
    }
    with_game_role(root, ExecutionRole::Client, |game| {
        let (client, _pipe_peer) = Pipe::pair();
        let _tcp_peer = if remote {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            game.connect_remote_session(
                &listener.local_addr().unwrap().to_string(),
                "StorageTest",
                None,
                None,
                false,
                false,
            )
            .unwrap();
            Some(listener)
        } else {
            game.connect_local(client, "StorageTest").unwrap();
            None
        };
        let connection = game.remote.as_mut().unwrap();
        // The fixture starts at play-packet dispatch after menu-extra negotiation.
        Arc::make_mut(&mut connection.registries).menu_extra_revision = 1;
        connection.level_load.finish();
        check_client_menus(game, packets.unwrap());
    });
}
