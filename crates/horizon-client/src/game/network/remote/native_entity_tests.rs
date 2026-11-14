//! A window joined to a server through memory, fed the server's packets as
//! it would receive them: what its native mods read of the entities and the
//! menu the server showed it.
use crate::game::{
    Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
};
use horizon_core::nbt::Tag;
use horizon_protocol::entity_data::Value;
use horizon_protocol::item::{Component, Stack, component};
use horizon_protocol::packets::play::{self, Clientbound as C};
use mod_api::EquipmentSlot;

fn spawn(dimension: &str) -> play::SpawnInfo {
    play::SpawnInfo {
        dimension_type: 0,
        dimension: dimension.into(),
        seed: 1,
        game_type: 0,
        previous_game_type: None,
        debug: false,
        flat: false,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    }
}

#[test]
#[ignore = "daily"]
fn a_client_reads_the_entities_and_menu_the_server_shows_it() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
    )
    .unwrap();
    {
        let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Setup);
        assert_eq!(
            mod_api::prelude::ClientViewClientHost::local_item_use(&game),
            None
        );
    }
    let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
    game.connect_local(pipe, "Steve").unwrap();
    let builtin = game.remote.as_ref().unwrap().builtin.clone();
    let id = |registry: &str, name: &str| builtin.get(registry).unwrap().id(name).unwrap();
    let item = |name: &str| id("minecraft:item", name);
    let add = |entity: i32, uuid: u128, kind: &str, position: [f64; 3]| {
        C::AddEntity(play::AddEntity {
            id: entity,
            uuid,
            kind: id("minecraft:entity_type", kind),
            position,
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 64,
            head_yaw: 64,
            data: 0,
        })
    };
    // No client level yet: no handle.
    assert_eq!(game.entity(20), None);
    game.handle_packet(C::Login(play::Login {
        player_id: 7,
        hardcore: false,
        levels: vec!["minecraft:overworld".into(), "minecraft:the_nether".into()],
        max_players: 20,
        chunk_radius: 8,
        simulation_distance: 8,
        reduced_debug_info: false,
        show_death_screen: true,
        limited_crafting: false,
        spawn: spawn("minecraft:overworld"),
        online_mode: false,
        enforces_secure_chat: false,
    }))
    .unwrap();

    // The window's own player, once the server's first teleport made it.
    game.handle_packet(C::PlayerPosition(play::PlayerPosition {
        teleport: 1,
        position: [0.5, 65.0, 0.5],
        delta: [0.0; 3],
        yaw: 45.0,
        pitch: 10.0,
        relatives: 0,
    }))
    .unwrap();
    let me = game.entity(7).expect("the own player");
    assert_eq!(game.entity_kind(me).as_deref(), Some("minecraft:player"));
    assert_eq!(game.entity_position(me), Some([0.5, 65.0, 0.5]));
    assert_eq!(game.entity_rotation(me), Some([45.0, 10.0]));
    // The profile the login named (the net thread's `Joined`).
    game.remote.as_mut().unwrap().profile_name = "Steve".into();
    assert_eq!(
        me.name(&game as &dyn mod_api::ClientHost).as_deref(),
        Some("Steve")
    );

    // An armor stand with a custom name and a helmet.
    let uuid = 0x0011_2233_4455_6677_8899_aabb_ccdd_eeff;
    game.handle_packet(add(20, uuid, "minecraft:armor_stand", [1.5, 64.0, 2.5]))
        .unwrap();
    game.handle_packet(C::SetEntityData(play::SetEntityData {
        id: 20,
        values: vec![(2, 6, Value::OptText(Some(Tag::String("Greeter".into()))))],
    }))
    .unwrap();
    game.handle_packet(C::SetEquipment(play::SetEquipment {
        id: 20,
        slots: vec![(5, Some(Stack::new(item("minecraft:diamond_helmet"), 1)))],
    }))
    .unwrap();
    let stand = game.entity(20).expect("the stand is tracked");
    assert_eq!(
        game.entity_kind(stand).as_deref(),
        Some("minecraft:armor_stand")
    );
    assert_eq!(game.entity_position(stand), Some([1.5, 64.0, 2.5]));
    assert_eq!(game.entity_block_position(stand), Some([1, 64, 2]));
    assert_eq!(game.entity_rotation(stand), Some([90.0, 0.0]));
    assert_eq!(game.entity_uuid(stand), Some(uuid));
    assert_eq!(
        stand.name(&game as &dyn mod_api::ClientHost).as_deref(),
        Some("Greeter")
    );
    // The client keeps the stand's equipment for rendering, but only its own
    // player is exposed as a LivingRef.
    assert_eq!(game.living(stand), None);
    let head = &game
        .armor_stands
        .iter()
        .find(|a| a.id == 20)
        .unwrap()
        .equipment[EquipmentSlot::Head as usize];
    assert_eq!(
        head.as_ref().map(|s| &*s.item),
        Some("minecraft:diamond_helmet")
    );
    // A client knows no saved data or tags, and changes nothing itself.
    assert_eq!(game.entity_nbt(stand), None);
    assert_eq!(game.entity_tags(stand), None);
    assert!(!game.entity_add_tag(stand, "seen"));
    assert!(!game.entity_set_velocity(stand, [0.0, 1.0, 0.0]));

    // An entity the client keeps no list of its own for: its type and
    // where the server put it.
    game.handle_packet(add(21, 5, "minecraft:text_display", [4.0, 70.0, -3.0]))
        .unwrap();
    let display = game.entity(21).expect("the display is tracked");
    assert_eq!(
        game.entity_kind(display).as_deref(),
        Some("minecraft:text_display")
    );
    assert_eq!(game.entity_position(display), Some([4.0, 70.0, -3.0]));
    assert_eq!(game.entity_uuid(display), Some(5));
    assert_eq!(game.entity(99), None);

    // Before the chest opens, the player's inventory menu is current.
    let player = game.local_player().unwrap();
    let inventory = game.player_container_menu(player).unwrap();
    assert_eq!(game.menu_slot_count(inventory), Some(46));
    // A chest the server opened, by its slot numbers: the chest's 9,
    // then the inventory's 27, then the hotbar's 9.
    game.handle_packet(C::OpenScreen(play::OpenScreen {
        container: 3,
        menu: id("minecraft:menu", "minecraft:generic_9x1"),
        title: Tag::String("Loot".into()),
    }))
    .unwrap();
    let mut named = Stack::new(item("minecraft:diamond"), 2);
    named.added.push((
        component::CUSTOM_NAME,
        Component::Text(Tag::String("Gem".into())),
    ));
    let mut items = vec![None; 45];
    items[0] = Some(named);
    items[36] = Some(Stack::new(item("minecraft:stone"), 5));
    game.handle_packet(C::ContainerSetContent(play::ContainerSetContent {
        container: 3,
        state: 1,
        items,
        carried: None,
    }))
    .unwrap();
    let menu = game
        .player_container_menu(player)
        .expect("a server menu is open");
    assert_eq!(game.menu_slot_count(inventory), None);
    assert_eq!(game.menu_slot_count(menu), Some(45));
    let gem = game
        .menu_slot(menu, 0)
        .flatten()
        .expect("the chest's first slot");
    assert_eq!((&*gem.item, gem.count), ("minecraft:diamond", 2));
    assert_eq!(gem.custom_name().as_deref(), Some("Gem"));
    assert_eq!(game.menu_slot(menu, 1), Some(None));
    let stone = game
        .menu_slot(menu, 36)
        .flatten()
        .expect("the hotbar's first slot");
    assert_eq!((&*stone.item, stone.count), ("minecraft:stone", 5));
    // The hotbar's first slot is the player's inventory slot 0.
    assert_eq!(game.inventory[0].as_ref().map(|s| s.count), Some(5));

    // SynchedEntityData.assignValues invokes each accessor callback before
    // applying the next value. The enderman only plays its stare sound when
    // the stared-at flag is already set at the DATA_CREEPY callback.
    use horizon_protocol::entity_data::serializer;
    game.handle_packet(add(31, 31, "minecraft:enderman", [3.0, 64.0, 0.0]))
        .unwrap();
    game.handle_packet(add(32, 32, "minecraft:enderman", [4.0, 64.0, 0.0]))
        .unwrap();
    game.audio.heard.clear();
    game.audio.capture = Some(Vec::new());
    let metadata = |id, values| C::SetEntityData(play::SetEntityData { id, values });
    let creepy = |value| (17, serializer::BOOLEAN, Value::Bool(value));
    let stared = |value| (18, serializer::BOOLEAN, Value::Bool(value));
    game.handle_packet(metadata(31, vec![creepy(true), stared(true)]))
        .unwrap();
    assert_eq!(
        game.audio.heard.get("minecraft:entity.enderman.stare"),
        None
    );
    game.handle_packet(metadata(32, vec![stared(true), creepy(true)]))
        .unwrap();
    assert_eq!(
        game.audio.heard.get("minecraft:entity.enderman.stare"),
        Some(&1)
    );
    game.handle_packet(metadata(32, vec![creepy(false), stared(false)]))
        .unwrap();
    assert_eq!(
        game.audio.heard.get("minecraft:entity.enderman.stare"),
        Some(&1)
    );
    game.mobs.iter_mut().find(|mob| mob.id == 31).unwrap().age = 400;
    game.handle_packet(metadata(31, vec![creepy(false), stared(false)]))
        .unwrap();
    assert_eq!(
        game.audio.heard.get("minecraft:entity.enderman.stare"),
        Some(&2)
    );
    assert!(game.audio.capture.as_ref().unwrap().is_empty());

    // A new client level: the handles of the old one read as None.
    game.handle_packet(C::Respawn(play::Respawn {
        spawn: spawn("minecraft:the_nether"),
        keep: 0,
    }))
    .unwrap();
    assert_eq!(game.entity_position(stand), None);
    assert_eq!(game.entity_kind(display), None);
}

/// #8589: the server sends entity event 42 (the raid sweat) for a villager;
/// the client must draw the particles, as it does for the events it let
/// through before. An event with no particle table row (2) draws nothing.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_villager_raid_sweat_event_draws_particles() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
    )
    .unwrap();
    let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
    game.connect_local(pipe, "Steve").unwrap();
    let builtin = game.remote.as_ref().unwrap().builtin.clone();
    let kind = builtin
        .get("minecraft:entity_type")
        .unwrap()
        .id("minecraft:villager")
        .unwrap();
    game.handle_packet(C::Login(play::Login {
        player_id: 7,
        hardcore: false,
        levels: vec!["minecraft:overworld".into()],
        max_players: 20,
        chunk_radius: 8,
        simulation_distance: 8,
        reduced_debug_info: false,
        show_death_screen: true,
        limited_crafting: false,
        spawn: spawn("minecraft:overworld"),
        online_mode: false,
        enforces_secure_chat: false,
    }))
    .unwrap();
    game.handle_packet(C::PlayerPosition(play::PlayerPosition {
        teleport: 1,
        position: [0.5, 65.0, 0.5],
        delta: [0.0; 3],
        yaw: 45.0,
        pitch: 10.0,
        relatives: 0,
    }))
    .unwrap();
    game.handle_packet(C::AddEntity(play::AddEntity {
        id: 21,
        uuid: 0x0011_2233_4455_6677_8899_aabb_ccdd_eeff,
        kind,
        // Within 32 blocks of the camera, which starts at the origin: the
        // particle engine drops a particle farther away than that.
        position: [1.5, 1.0, 2.5],
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 64,
        head_yaw: 64,
        data: 0,
    }))
    .unwrap();
    // `len` counts only particles already ticked in; `made` counts every
    // particle created, which is what the event handler does at once.
    let made = |g: &Game| g.particles.made().values().sum::<u64>();
    let before = made(&game);
    game.handle_packet(C::EntityEvent(play::EntityEvent { id: 21, event: 2 }))
        .unwrap();
    assert_eq!(made(&game), before, "event 2 has no particle row");
    game.handle_packet(C::EntityEvent(play::EntityEvent { id: 21, event: 42 }))
        .unwrap();
    assert!(made(&game) > before, "event 42 draws the sweat");
}

impl super::Remote {
    /// Lists this client's own player with `latency`, as the server's player info update does.
    pub(in crate::game) fn list_self(&mut self, latency: i32) {
        let mut me = horizon_client::tab_list::TabPlayer::new(self.profile, "SessionReadTest");
        me.latency = latency;
        self.tab_players.insert(self.profile, me);
    }
}
