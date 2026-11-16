//! A hoglin that retaliates keeps attacking its own attacker when a
//! neighbouring hoglin is hit in the same tick: `HoglinAi.broadcastAttackTarget`
//! gives each visible adult hoglin the nearer of its current target and the
//! new one (`setAttackTargetIfCloserThanCurrent`)
//! (tools/test_hoglin_repeat_attack_parity.py measures the same on vanilla 26.3).
use super::commands::Target;
use super::*;
use horizon_client::{chunk::Chunk, motion::PlayerMotion};
use horizon_core::position::ChunkPos;
use std::sync::Arc;

const NEAR_HOGLIN: &str = "00006093-0000-0000-0000-000000000003";
const NEAR_PIG: &str = "00006093-0000-0000-0000-00000000000d";
const HOGLIN: &str = "00006093-0000-0000-0000-000000000001";
const PIG: &str = "00006093-0000-0000-0000-00000000000b";

fn game<'a>(resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>) -> Game<'a> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res = resources.insert(loaded.resources);
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.build_registries();
    game.native_lifecycle
        .begin_open(super::native_lifecycle::Authority::IntegratedServer);
    game.set_allow_commands(true);
    for cx in 0..2 {
        game.world
            .chunks
            .insert((cx, 0), Arc::new(Chunk::empty(cx, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(cx, 0))
            .unwrap();
    }
    for cx in 0..2 {
        game.load_ticking_neighbourhood(cx, 0);
    }
    for command in [
        "difficulty normal",
        "forceload add 0 0 31 15",
        "fill 0 99 0 31 99 15 warped_nylium",
        "tick freeze",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    // Two glass pens 16 blocks apart, each with a hoglin and a sturdy NoAI pig.
    for (x0, n) in [(0, 3), (16, 1)] {
        assert!(game.run_command(&format!("fill {} 100 4 {} 102 12 glass", x0 + 4, x0 + 10)));
        assert!(game.run_command(&format!("fill {} 100 5 {} 102 11 air", x0 + 5, x0 + 9)));
        assert!(game.run_command(&format!(
            "summon hoglin {}.5 100 8.5 {{PersistenceRequired:1b,IsImmuneToZombification:1b,Health:40f,UUID:[I;24723,0,0,{n}]}}",
            x0 + 6
        )));
        assert!(game.run_command(&format!(
            "summon pig {}.5 100 8.5 {{NoAI:1b,PersistenceRequired:1b,Silent:1b,attributes:[{{id:\"minecraft:max_health\",base:100d}}],Health:100f,UUID:[I;24723,0,0,{}]}}",
            x0 + 7,
            10 + n
        )));
    }
    game
}

fn health(game: &mut Game<'_>, uuid: &str) -> f32 {
    let target = game
        .target_with_uuid(horizon_client::save::uuid_from_text(uuid).unwrap())
        .expect("the entity is in the level");
    match target {
        Target::Mob(id) => game.mobs.iter().find(|m| m.id == id).unwrap().health,
        other => panic!("{other:?} is not a mob"),
    }
}

fn step(game: &mut Game<'_>, ticks: u32) {
    for _ in 0..ticks {
        assert!(game.run_command("tick step 1"));
        game.tick_server(false).unwrap();
    }
}

#[test]
#[ignore = "daily"]
fn a_hoglin_keeps_attacking_its_attacker_beside_another_hit_hoglin() {
    let mut resources = None;
    let mut game = game(&mut resources);
    step(&mut game, 40);
    // The neighbour's hoglin first, then this one, in the same tick.
    assert!(game.run_command(&format!(
        "damage {NEAR_HOGLIN} 1 minecraft:mob_attack by {NEAR_PIG}"
    )));
    assert!(game.run_command(&format!("damage {HOGLIN} 1 minecraft:mob_attack by {PIG}")));
    step(&mut game, 5);
    let after_first = health(&mut game, PIG);
    assert!(
        after_first < 100.0,
        "the first retaliation hits ({after_first})"
    );
    let near_first = health(&mut game, NEAR_PIG);
    assert!(
        near_first < 100.0,
        "the neighbour retaliates ({near_first})"
    );
    // MeleeAttack's 40-tick cooldown: two more hits are due within 90 ticks.
    step(&mut game, 90);
    let near_later = health(&mut game, NEAR_PIG);
    assert!(
        near_later < near_first,
        "the neighbour hits again ({near_later})"
    );
    let later = health(&mut game, PIG);
    assert!(
        later < after_first,
        "this hoglin hits its attacker again ({later})"
    );
}

fn hurt_time(game: &mut Game<'_>, uuid: &str) -> i32 {
    let id = mob_id(game, uuid);
    game.mobs.iter().find(|m| m.id == id).unwrap().hurt_time
}

/// Vanilla 26.3's hit pen: the hoglin enters the fight activity at the end of
/// the tick after the hit (`HoglinAi.updateActivity` follows the brain), hits
/// in the next one, and again 41 ticks later. The pig ticks after the hoglin,
/// so it reads `HurtTime` 9 on the tick of each hit.
#[test]
#[ignore = "daily"]
fn a_hoglin_hits_from_its_second_fighting_tick_every_41_ticks() {
    let mut resources = None;
    let mut game = game(&mut resources);
    step(&mut game, 40);
    assert!(game.run_command(&format!("damage {HOGLIN} 1 minecraft:mob_attack by {PIG}")));
    let mut hits = Vec::new();
    for tick in 1..=90 {
        step(&mut game, 1);
        if hurt_time(&mut game, PIG) == 9 {
            hits.push(tick);
        }
    }
    assert_eq!(hits, [2, 43, 84]);
}

#[test]
#[ignore = "daily"]
fn deferred_stronger_hits_do_not_count_hurt_timers_down_twice() {
    use horizon_client::mobs::Damage;
    let mut resources = None;
    let mut game = game(&mut resources);
    let first = mob_id(&mut game, NEAR_HOGLIN);
    let second = mob_id(&mut game, HOGLIN);
    let target = mob_id(&mut game, PIG);
    game.creatures.ticked.extend([first, second, target]);
    game.creatures.deferred_hits = true;
    assert!(game.mob_hits_mob(target, 4.0, Damage::mob(first, [0.0; 3])));
    assert!(game.mob_hits_mob(target, 6.0, Damage::mob(second, [0.0; 3])));
    let victim = game.mobs.iter().find(|m| m.id == target).unwrap();
    assert_eq!((victim.hurt_time, victim.invulnerable), (9, 19));
    assert_eq!(victim.health, 94.0);

    // A stronger hit on the following tick also leaves the already-counted
    // timers alone rather than decrementing them again.
    let victim = game.mobs.iter_mut().find(|m| m.id == target).unwrap();
    victim.hurt_time = 8;
    victim.invulnerable = 18;
    assert!(game.mob_hits_mob(target, 8.0, Damage::mob(first, [0.0; 3])));
    let victim = game.mobs.iter().find(|m| m.id == target).unwrap();
    assert_eq!((victim.hurt_time, victim.invulnerable), (8, 18));
    assert_eq!(victim.health, 92.0);
}

/// Added controls (after the frozen baseline above).
fn mob_id(game: &mut Game<'_>, uuid: &str) -> u32 {
    match game.target_with_uuid(horizon_client::save::uuid_from_text(uuid).unwrap()) {
        Some(Target::Mob(id)) => id,
        other => panic!("{other:?}"),
    }
}

/// `setAttackTargetIfCloserThanCurrent` on the receiver: pacified ignores
/// the broadcast; a nearer current target stays; a nearer (or equally near)
/// incoming one replaces a farther one; none current takes the new one.
#[test]
#[ignore = "daily"]
fn a_broadcast_target_replaces_only_a_farther_one() {
    use horizon_client::mobs::{Who, piglin::receive_attack_target};
    use horizon_core::geometry::Vec3;
    let mut resources = None;
    let mut game = game(&mut resources);
    let receiver = mob_id(&mut game, HOGLIN);
    let (near, far) = (mob_id(&mut game, PIG), mob_id(&mut game, NEAR_PIG));
    let place = |game: &Game<'_>, id: u32| game.mobs.iter().find(|m| m.id == id).unwrap().position;
    let (near_at, far_at) = (place(&game, near), place(&game, far));
    let position = |who: Who, _current| match who {
        Who::Mob(id) if id == near => Some(near_at),
        Who::Mob(id) if id == far => Some(far_at),
        _ => None::<Vec3>,
    };
    let m = game.mobs.iter_mut().find(|m| m.id == receiver).unwrap();
    // No current target: the broadcast's is taken.
    m.ai.target = None;
    let proposal = receive_attack_target(m, Who::Mob(far), position);
    assert_eq!(proposal, Some(Who::Mob(far)), "no current target");
    assert_eq!(
        m.ai.target, None,
        "selection leaves committing to the event hook"
    );
    // Pacified: nothing changes.
    m.ai.target = None;
    m.extra.ints[0] = 200;
    assert_eq!(
        receive_attack_target(m, Who::Mob(near), position),
        None,
        "pacified"
    );
    m.extra.ints[0] = 0;
    // A nearer current target stays.
    m.ai.target = Some(Who::Mob(near));
    assert_eq!(
        receive_attack_target(m, Who::Mob(far), position),
        None,
        "nearer current"
    );
    // A nearer incoming target replaces a farther current one.
    m.ai.target = Some(Who::Mob(far));
    assert_eq!(
        receive_attack_target(m, Who::Mob(near), position),
        Some(Who::Mob(near)),
        "nearer incoming"
    );
    // A tie takes the incoming one (`getTargetNearestMe`: current only if strictly nearer).
    m.ai.target = Some(Who::Mob(far));
    assert_eq!(
        receive_attack_target(m, Who::Mob(near), |_, _| Some(near_at)),
        Some(Who::Mob(near)),
        "tie"
    );
}

/// The broadcast's receivers are the sensed visible adult hoglins
/// (`NEAREST_VISIBLE_ADULT_HOGLINS`): not a baby, not one behind stone.
#[test]
#[ignore = "daily"]
fn the_sensed_receivers_are_visible_adult_hoglins() {
    let mut resources = None;
    let mut game = game(&mut resources);
    // Inside this hoglin's pen: an adult and a baby. Beyond a stone wall: an adult.
    assert!(game.run_command(
        "summon hoglin 24.5 100 10.5 {PersistenceRequired:1b,IsImmuneToZombification:1b,NoAI:1b,UUID:[I;24723,0,0,6]}"
    ));
    assert!(game.run_command(
        "summon hoglin 24.5 100 6.5 {PersistenceRequired:1b,IsImmuneToZombification:1b,Age:-24000,UUID:[I;24723,0,0,4]}"
    ));
    assert!(game.run_command("fill 27 100 0 27 104 15 stone"));
    assert!(game.run_command(
        "summon hoglin 29.5 100 8.5 {PersistenceRequired:1b,IsImmuneToZombification:1b,NoAI:1b,UUID:[I;24723,0,0,5]}"
    ));
    step(&mut game, 40);
    let receiver = mob_id(&mut game, HOGLIN);
    let near = mob_id(&mut game, "00006093-0000-0000-0000-000000000006");
    let baby = mob_id(&mut game, "00006093-0000-0000-0000-000000000004");
    let walled = mob_id(&mut game, "00006093-0000-0000-0000-000000000005");
    let adults = &game
        .mobs
        .iter()
        .find(|m| m.id == receiver)
        .unwrap()
        .extra
        .piglin
        .adults;
    assert!(
        adults.contains(&near),
        "the visible adult neighbour: {adults:?}"
    );
    assert!(!adults.contains(&baby), "not a baby: {adults:?}");
    assert!(
        !adults.contains(&walled),
        "not one behind stone: {adults:?}"
    );
}

#[derive(Default)]
struct BroadcastTargetListener {
    state: Arc<
        std::sync::Mutex<(
            usize,
            Option<mod_api::LivingRef>,
            Option<mod_api::LivingRef>,
        )>,
    >,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for BroadcastTargetListener {
    fn living_change_target(
        &self,
        _host: &dyn mod_api::Host,
        event: &mut mod_api::LivingChangeTargetEvent<'_>,
    ) -> mod_api::Flow {
        assert!(event.is_brain, "hoglin targets use the brain event");
        let mut state = self.state.lock().unwrap();
        state.0 += 1;
        state.2 = event.original_target;
        if let Some(redirect) = state.1 {
            *event.new_target = Some(redirect);
            mod_api::Flow::Continue
        } else {
            mod_api::Flow::Cancel
        }
    }
}

#[test]
#[ignore = "daily"]
fn hoglin_broadcast_obeys_target_cancellation_and_redirection() {
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_client::mobs::{Mob, MobEvent, Who};
    use horizon_core::geometry::Vec3;

    let state = Arc::new(std::sync::Mutex::new((0, None, None)));
    let mods = vec![mod_api::ModEntry::new(
        "hoglin_target_listener",
        Box::new(BroadcastTargetListener {
            state: state.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:hoglin")
                .unwrap()
                .clone();
            for (id, x) in [(9001, 0.0), (9002, 4.0), (9003, 2.0)] {
                game.mobs
                    .push(Mob::new(id, kind.clone(), Vec3::new(x, 64.0, 0.0), 0.0));
            }
            let broadcast = || {
                vec![MobEvent::HoglinAttackTarget {
                    mob: 9001,
                    target: Who::Mob(9002),
                    player: None,
                }]
            };
            game.handle_mob_events(broadcast()).unwrap();
            assert_eq!(state.lock().unwrap().0, 1);
            assert_eq!(
                game.mobs[0].ai.target, None,
                "cancellation preserves target"
            );

            let epoch = game.native_lifecycle.epoch().unwrap();
            state.lock().unwrap().1 = Some(mod_api::LivingRef::new(mod_api::EntityRef::new(
                epoch, 9003,
            )));
            game.handle_mob_events(broadcast()).unwrap();
            assert_eq!(state.lock().unwrap().0, 2);
            assert_eq!(
                game.mobs[0].ai.target,
                Some(Who::Mob(9003)),
                "listener redirect is committed"
            );

            // A nearer current target and a pacified receiver reject the proposal
            // before listener dispatch, preserving vanilla's broadcast selection.
            game.handle_mob_events(broadcast()).unwrap();
            assert_eq!(state.lock().unwrap().0, 2);
            game.mobs[0].ai.target = None;
            game.mobs[0].extra.ints[0] = 200;
            game.handle_mob_events(broadcast()).unwrap();
            assert_eq!(state.lock().unwrap().0, 2);
            assert_eq!(game.mobs[0].ai.target, None);

            use crate::game::native_item_places::tests::{guest, open_to_lan};
            open_to_lan(game);
            for conn in [12, 13] {
                let mut placed = guest(conn);
                placed.loaded = true;
                placed.position = [if conn == 12 { 4.0 } else { 2.0 }, 64.0, 0.0];
                game.lan.as_mut().unwrap().hub.guests.push(placed);
            }
            let attacker_id = game.guest_identity(12).unwrap().1;
            let attacker = mod_api::LivingRef::new(mod_api::EntityRef::new(epoch, attacker_id));
            game.mobs[0].extra.ints[0] = 0;
            game.lan.as_mut().unwrap().mob_players.insert(9001, 13);
            let broadcast_player = || {
                vec![MobEvent::HoglinAttackTarget {
                    mob: 9001,
                    target: Who::Player,
                    player: Some(12),
                }]
            };
            state.lock().unwrap().1 = None;
            game.handle_mob_events(broadcast_player()).unwrap();
            assert_eq!(state.lock().unwrap().2, Some(attacker));
            assert_eq!(game.mobs[0].ai.target, None);
            assert_eq!(game.lan.as_ref().unwrap().mob_players[&9001], 13);
            state.lock().unwrap().1 = Some(attacker);
            game.handle_mob_events(broadcast_player()).unwrap();
            assert_eq!(state.lock().unwrap().2, Some(attacker));
            assert_eq!(game.mobs[0].ai.target, Some(Who::Player));
            assert_eq!(game.lan.as_ref().unwrap().mob_players[&9001], 12);
            // Concrete players must be compared, not the shared Who::Player tag.
            let nearer_mob = mod_api::LivingRef::new(mod_api::EntityRef::new(epoch, 9003));
            state.lock().unwrap().1 = Some(nearer_mob);
            let before = state.lock().unwrap().0;
            game.handle_mob_events(vec![MobEvent::HoglinAttackTarget {
                mob: 9001,
                target: Who::Mob(9003),
                player: None,
            }])
            .unwrap();
            assert_eq!(state.lock().unwrap().0, before + 1);
            assert_eq!(
                game.mobs[0].ai.target,
                Some(Who::Mob(9003)),
                "player at four loses to mob at two"
            );
            game.mobs[0].ai.target = Some(Who::Mob(9002));
            let nearer_player = mod_api::LivingRef::new(mod_api::EntityRef::new(
                epoch,
                game.guest_identity(13).unwrap().1,
            ));
            state.lock().unwrap().1 = Some(nearer_player);
            let broadcast_nearer_player = || {
                vec![MobEvent::HoglinAttackTarget {
                    mob: 9001,
                    target: Who::Player,
                    player: Some(13),
                }]
            };
            game.handle_mob_events(broadcast_nearer_player()).unwrap();
            assert_eq!(state.lock().unwrap().0, before + 2);
            assert_eq!(game.mobs[0].ai.target, Some(Who::Player));
            assert_eq!(game.lan.as_ref().unwrap().mob_players[&9001], 13);
            game.lan.as_mut().unwrap().mob_players.insert(9001, 12);
            game.handle_mob_events(broadcast_nearer_player()).unwrap();
            assert_eq!(state.lock().unwrap().0, before + 3);
            assert_eq!(game.lan.as_ref().unwrap().mob_players[&9001], 13);
        });
    });
}

/// The broadcast as the flush applies it (`MobEvent::HoglinAttackTarget`).
fn broadcast(game: &mut Game<'_>, mob: u32, from: u32, target: horizon_client::mobs::Who) {
    let player = game
        .lan
        .as_ref()
        .and_then(|h| h.mob_players.get(&from).copied());
    let event = horizon_client::mobs::MobEvent::HoglinAttackTarget {
        mob,
        target,
        player,
    };
    game.handle_mob_events(vec![event]).unwrap();
}

fn target_of(game: &Game<'_>, id: u32) -> Option<horizon_client::mobs::Who> {
    game.mobs.iter().find(|m| m.id == id).unwrap().ai.target
}

fn set_target(game: &mut Game<'_>, id: u32, target: Option<horizon_client::mobs::Who>) {
    game.mobs.iter_mut().find(|m| m.id == id).unwrap().ai.target = target;
}

fn place_player(game: &mut Game<'_>, x: f64, z: f64) {
    if let Some(player) = game.player.as_mut() {
        player.position = Vec3::new(x, 100.0, z);
    } else {
        game.next_entity += 1;
        let id = game.next_entity;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(x, 100.0, z)).unwrap());
        super::native_entity_refs::tests::bind_local_player(game, id, 1);
    }
}

/// A player on either side is placed and compared like a mob
/// (`getTargetNearestMe` compares positions, whoever the targets are).
#[test]
#[ignore = "daily"]
fn a_broadcast_player_is_compared_by_distance() {
    use horizon_client::mobs::Who;
    let mut resources = None;
    let mut game = game(&mut resources);
    let receiver = mob_id(&mut game, HOGLIN);
    let from = mob_id(&mut game, NEAR_HOGLIN);
    // The receiver at x 22.5, its pig at 23.5, the other pen's pig at 7.5.
    let (pig, other_pig) = (mob_id(&mut game, PIG), mob_id(&mut game, NEAR_PIG));
    // The player 2 blocks away beats the pig 15 blocks away.
    place_player(&mut game, 22.5, 10.5);
    set_target(&mut game, receiver, Some(Who::Mob(other_pig)));
    broadcast(&mut game, receiver, from, Who::Player);
    assert_eq!(
        target_of(&game, receiver),
        Some(Who::Player),
        "a nearer player"
    );
    // A current player 2 blocks away stays against the pig 15 blocks away.
    broadcast(&mut game, receiver, from, Who::Mob(other_pig));
    assert_eq!(
        target_of(&game, receiver),
        Some(Who::Player),
        "a nearer current player"
    );
    // The player 21 blocks away loses to the pig 1 block away, either way round.
    place_player(&mut game, 2.5, 2.5);
    broadcast(&mut game, receiver, from, Who::Mob(pig));
    assert_eq!(
        target_of(&game, receiver),
        Some(Who::Mob(pig)),
        "a farther current player"
    );
    broadcast(&mut game, receiver, from, Who::Player);
    assert_eq!(
        target_of(&game, receiver),
        Some(Who::Mob(pig)),
        "a farther player"
    );
}

/// A host on LAN with one placed guest (conn 2) at `guest_at`.
fn host_with_guest(game: &mut Game<'_>, guest_at: [f64; 3]) {
    use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    use horizon_protocol::packets::configuration::ClientInformation;
    let net = NetConfig {
        announce: false,
        ..NetConfig::lan(0, "hoglin-broadcast-test".into(), 2)
    };
    let settings = super::lan::LanSettings {
        port: 0,
        guest_commands: true,
        force_game_mode: false,
    };
    let hub = HubConfig {
        default_game_mode: 0,
        guest_commands: true,
        force_game_mode: false,
        hardcore: false,
        view_distance: 2,
        simulation_distance: 2,
        dimension: Dim::Overworld.id().into(),
        allow_flight: false,
        seed: 0,
        max_players: 2,
        integrated: false,
        levels: Default::default(),
        level_seeds: Default::default(),
    };
    game.lan_host(settings, net, hub).unwrap();
    let (tx, _rx) = std::sync::mpsc::channel();
    let joined = Joined {
        conn: 2,
        uuid: 2,
        name: "HoglinGuest".into(),
        properties: Vec::new(),
        info: ClientInformation::new(2),
        brand: "test".into(),
        sender: PlayerSender::new(tx, Arc::default()),
        latency: Arc::new(std::sync::atomic::AtomicI32::new(0)),
        address: "127.0.0.1:25565".parse().unwrap(),
        expected: false,
    };
    game.next_entity += 1;
    let mut guest = Guest::new(
        joined,
        i32::try_from(game.next_entity).expect("fixture entity ID fits the protocol"),
        guest_at,
        0,
    );
    guest.placed = true;
    guest.loaded = true;
    game.lan.as_mut().unwrap().hub.guests.push(guest);
}

/// On LAN, `Who::Player` is each mob's own player (`mob_players`): a hoglin
/// that a guest's attacker is broadcast to compares that guest, not the
/// host, and once it takes the guest it is bound to them.
#[test]
#[ignore = "daily"]
fn a_guest_broadcast_is_placed_as_the_guest() {
    use horizon_client::mobs::Who;
    let mut resources = None;
    let mut game = game(&mut resources);
    // The host 21 blocks from the receiver, the guest 2 blocks.
    place_player(&mut game, 2.5, 2.5);
    host_with_guest(&mut game, [22.5, 100.0, 10.5]);
    let receiver = mob_id(&mut game, HOGLIN);
    let from = mob_id(&mut game, NEAR_HOGLIN);
    let other_pig = mob_id(&mut game, NEAR_PIG);
    let bound = |game: &Game<'_>| {
        let host = game.lan.as_ref().unwrap();
        host.mob_players.get(&receiver).copied()
    };
    // From a hoglin that sees the host: the host is farther than the pig 15 blocks away.
    set_target(&mut game, receiver, Some(Who::Mob(other_pig)));
    broadcast(&mut game, receiver, from, Who::Player);
    assert_eq!(
        target_of(&game, receiver),
        Some(Who::Mob(other_pig)),
        "the host is farther"
    );
    assert_eq!(bound(&game), None);
    // From a hoglin bound to the guest: the guest is nearer, and the receiver
    // now sees that guest.
    game.lan.as_mut().unwrap().mob_players.insert(from, 2);
    broadcast(&mut game, receiver, from, Who::Player);
    assert_eq!(
        target_of(&game, receiver),
        Some(Who::Player),
        "the guest is nearer"
    );
    assert_eq!(bound(&game), Some(2), "bound to the guest");
}
