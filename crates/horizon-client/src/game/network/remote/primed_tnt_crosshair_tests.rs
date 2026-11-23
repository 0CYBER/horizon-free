use super::*;
use crate::game::minecarts::{CartKind, Minecart};
use horizon_client::projectiles::{Projectile, ProjectileKind};
use horizon_core::position::BlockPos;

#[derive(Clone, Default)]
struct ClickSpy(Arc<Mutex<Vec<&'static str>>>);

#[mod_api::__subscriptions]
impl Hooks for ClickSpy {
    fn client_packet_send(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::ClientPacketSendEvent<'_>,
    ) -> Flow {
        let action = match event.packet.id.path() {
            "attack" => "attack",
            "interact" => "interact",
            "use_item" => "use_item",
            "use_item_on" => "use_item_on",
            _ => return Flow::Continue,
        };
        self.0.lock().unwrap().push(action);
        Flow::Continue
    }

    fn sweep_attack(&self, _: HostSide<'_>, event: &mut mod_api::SweepAttackEvent<'_>) -> Flow {
        if event.target == "minecraft:tnt" {
            self.0.lock().unwrap().push("tnt-sweep-check");
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn the_crosshair_picks_only_visible_in_reach_primed_tnt() {
    let spy = ClickSpy::default();
    let mods = vec![mod_api::ModEntry::new(
        "tnt_pick_spy",
        Box::new(spy.clone()),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(true, |game| {
            game.set_mode_id(0);
            game.entity_reach = 3.0;
            game.block_reach = 4.5;
            game.yaw = 0.0;
            game.pitch = 0.0;
            game.add_primed_tnt(BlockPos::new(0, 65, 5), 0.0, 80, None, None);
            let id = game.tnts[0].id as i32;
            let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let air = game.res.facts().air;
            for (case, visible) in [
                ("open", true),
                ("wall", false),
                ("nearer_cow", false),
                ("farther_cow", true),
                ("out_of_reach", false),
                ("removed", false),
            ] {
                game.world
                    .set(0, 65, 4, if case == "wall" { stone } else { air })
                    .unwrap();
                game.tick_hit.set(None);
                game.mobs.clear();
                if case.ends_with("_cow") {
                    let z = if case == "nearer_cow" { 4.5 } else { 6.5 };
                    game.mobs.push(horizon_client::mobs::Mob::new(
                        id as u32 + 1,
                        cow.clone(),
                        Vec3::new(0.5, 65.0, z),
                        0.0,
                    ));
                }
                if case == "out_of_reach" {
                    game.tnts[0].position.z = 7.5;
                } else if case == "removed" {
                    game.tnts[0].position.z = 5.5;
                    assert_eq!(
                        game.pick_entity_hit_at().and_then(|(_, target)| target),
                        Some(id),
                        "the TNT is in reach before the removal packet"
                    );
                    game.remove_entity(id);
                }
                let hit = game
                    .pick_entity_hit_at()
                    .is_some_and(|(_, target)| target == Some(id));
                println!("tnt-crosshair: {case} {hit}");
                assert_eq!(hit, visible, "{case}");
                let living = (case == "nearer_cow").then_some(true);
                assert_eq!(game.crosshair_living(), living, "attack indicator: {case}");
                if case == "open" {
                    assert!(game.entity_targeted());
                    game.remote_start_attack();
                    assert_eq!(*spy.0.lock().unwrap(), ["attack", "tnt-sweep-check"]);
                    spy.0.lock().unwrap().clear();
                    game.world.set(0, 65, 6, stone).unwrap();
                    assert_eq!(game.pick().map(|hit| hit.pos), Some([0, 65, 6]));
                    game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 1));
                    game.offhand = None;
                    game.remote_use(mod_api::InteractionAttempt::Press);
                    assert_eq!(*spy.0.lock().unwrap(), ["interact", "use_item", "interact"]);
                    game.world.set(0, 65, 6, air).unwrap();
                }
            }
            check_projectile_order(game, &spy);
            check_spectator_crosshair(game);
        });
    });
    print_use_clicks();
}

/// One use click for each order of a wind charge and the TNT along the view, a stick in the main
/// hand and the off hand empty: the pick, then the packets as the server's end of the connection
/// reads them. The harness compares these lines with the pinned client's `LocalPlayer.pick` and
/// `Minecraft.startUseItem`.
fn print_use_clicks() {
    use horizon_protocol::packets::{Empty, play::Serverbound as S};

    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    crate::game::native_entity_refs::tests::with_game_at(root, |game| {
        // Play packets reach the wire only once login and configuration are done.
        let (mut server, _writer) = crate::game::remote::tests::connect_menu_test(game);
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 3.5)).unwrap());
        game.set_mode_id(0);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 1));
        game.offhand = None;
        game.entity_reach = 3.0;
        game.block_reach = 4.5;
        game.yaw = 0.0;
        game.pitch = 0.0;
        game.add_primed_tnt(BlockPos::new(0, 65, 5), 0.0, 80, None, None);
        // The oracle's game mode has the selected slot as sent (`ensureHasSentCarriedItem` adds
        // nothing); this connection has sent none yet, and its first click would carry it.
        game.remote_send_carried();
        let tnt = game.tnts[0].id as i32;
        for (order, z) in [("tnt_first", 7.35), ("charge_first", 5.0)] {
            game.creatures.projectiles.clear();
            let charge = game.add_projectile(Projectile::new(
                ProjectileKind::WindCharge,
                Vec3::new(0.5, 65.5, z),
                None,
            )) as i32;
            let name = |id: i32| match id {
                id if id == tnt => "tnt",
                id if id == charge => "wind_charge",
                _ => "other",
            };
            let picked = game.crosshair_entity(false).map_or("none", |p| name(p.id));
            println!("tnt-crosshair: pick {order} {picked}");
            // A punch fences the click: what was sent before it is dropped, what follows is the
            // click's.
            let mut fenced = |game: &mut Game<'_>, click: bool| {
                game.send(S::Punch(Empty));
                loop {
                    let (id, body) = server.recv().unwrap();
                    let line = match S::decode(id, &body).unwrap() {
                        S::Punch(_) => return,
                        S::Interact(p) => {
                            let hand = if p.hand == 0 { "main" } else { "off" };
                            format!("interact {hand} {}", name(p.id))
                        }
                        S::UseItem(p) => {
                            format!("use_item {}", if p.hand == 0 { "main" } else { "off" })
                        }
                        _ => format!("other {id}"),
                    };
                    if click {
                        println!("tnt-crosshair: use {order} {line}");
                    }
                }
            };
            fenced(game, false);
            game.remote_use(mod_api::InteractionAttempt::Press);
            fenced(game, true);
        }
    });
}

fn check_projectile_order(game: &mut Game<'_>, spy: &ClickSpy) {
    game.add_primed_tnt(BlockPos::new(0, 65, 5), 0.0, 80, None, None);
    let id = game.tnts[0].id as i32;
    for (z, tnt_first) in [(7.35, true), (5.0, false)] {
        game.creatures.projectiles.clear();
        let charge = game.add_projectile(Projectile::new(
            ProjectileKind::WindCharge,
            Vec3::new(0.5, 65.5, z),
            None,
        )) as i32;
        assert!(game.redirectable_hit().is_some());
        assert_eq!(
            game.pick_entity_hit(),
            Some(tnt_first.then_some(id)),
            "wind charge at z={z}"
        );

        spy.0.lock().unwrap().clear();
        game.remote_start_attack();
        let expected_attack: &[&str] = if tnt_first {
            &["attack", "tnt-sweep-check"]
        } else {
            &["attack"]
        };
        assert_eq!(*spy.0.lock().unwrap(), expected_attack);

        // `Minecraft.startUseItem`: each hand's turn interacts with the crosshair's entity
        // whatever it is (`MultiPlayerGameMode.interact` always sends the packet). A wind
        // charge passes as primed TNT does (`Entity.interact`), so the packets are the same
        // and only their target tells the two apart.
        assert_eq!(
            game.crosshair_entity(false).map(|pick| pick.id),
            Some(if tnt_first { id } else { charge }),
            "use target with the wind charge at z={z}"
        );
        spy.0.lock().unwrap().clear();
        game.remote_use(mod_api::InteractionAttempt::Press);
        assert_eq!(*spy.0.lock().unwrap(), ["interact", "use_item", "interact"]);
    }
    game.creatures.projectiles.clear();
    game.remove_entity(id);
}

fn check_spectator_crosshair(game: &mut Game<'_>) {
    game.set_mode_id(3);
    game.entity_reach = 3.0;
    game.block_reach = 4.5;
    game.carts.push(Minecart::new(
        700,
        CartKind::Chest,
        Vec3::new(0.5, 65.25, 6.5),
    ));
    assert!(game.spectator_crosshair(), "the chest cart is in reach");
    game.add_primed_tnt(BlockPos::new(0, 65, 5), 0.0, 80, None, None);
    assert!(!game.spectator_crosshair(), "the TNT covers the chest cart");
    game.tnts[0].position.x = 1.5;
    assert!(game.spectator_crosshair(), "the ray misses the TNT's box");

    game.carts.clear();
    game.tnts[0].position.x = 0.5;
    let barrel = game
        .pack
        .assets
        .registry
        .default_state("minecraft:barrel")
        .unwrap();
    game.world.set(0, 65, 7, barrel).unwrap();
    game.tnts[0].position.z = 9.5;
    assert!(game.spectator_crosshair(), "the barrel is in block reach");
    // LocalPlayer.pick sees the closer entity within block reach and
    // returns a miss after applying the shorter entity reach.
    game.tnts[0].position.z = 7.1;
    assert!(game.primed_tnt_hit(game.entity_reach).is_none());
    assert!(
        !game.spectator_crosshair(),
        "the reach-filtered pick misses"
    );
}

#[derive(Clone, Debug, PartialEq)]
enum ServerNotice {
    Attack(i32),
    InteractSpecific(i32, [f64; 3]),
    Interact(i32, Hand),
}

#[derive(Clone, Default)]
struct HookSpy(Arc<Mutex<Vec<ServerNotice>>>);

#[mod_api::__subscriptions]
impl Hooks for HookSpy {
    fn attack_entity(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::AttackEntityEvent<'_>,
    ) -> Flow {
        assert_eq!(event.kind, "minecraft:tnt");
        self.0
            .lock()
            .unwrap()
            .push(ServerNotice::Attack(event.entity));
        Flow::Continue
    }

    fn entity_interact_specific(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::EntityInteractSpecificEvent<'_>,
    ) -> Flow {
        assert_eq!(event.kind, "minecraft:tnt");
        self.0
            .lock()
            .unwrap()
            .push(ServerNotice::InteractSpecific(event.entity, event.location));
        Flow::Continue
    }

    fn entity_interact(
        &self,
        host: HostSide<'_>,
        event: &mut mod_api::EntityInteractEvent<'_>,
    ) -> Option<Answer> {
        assert!(matches!(host, HostSide::Server(_)));
        assert_eq!(event.kind, "minecraft:tnt");
        self.0
            .lock()
            .unwrap()
            .push(ServerNotice::Interact(event.entity, event.hand));
        None
    }
}

#[test]
#[ignore = "daily"]
fn primed_tnt_packets_reach_the_hooks_and_leave_bystanders_untouched() {
    use horizon_protocol::packets::play::{self, Serverbound as S};

    let spy = HookSpy::default();
    let mods = vec![mod_api::ModEntry::new(
        "tnt_action_spy",
        Box::new(spy.clone()),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let _guest = setup_interaction_guest(game);
            game.as_guest(12, |game| game.attack_ticks = 5).unwrap();
            game.add_primed_tnt(
                horizon_core::position::BlockPos::new(0, 65, 5),
                0.0,
                80,
                None,
                None,
            );
            let id = game.tnts[0].id as i32;
            let original_velocity = game.tnts[0].velocity;
            // A closer cow on the same ray, as when an entity has moved since
            // the client picked the packet's TNT target.
            let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(horizon_client::mobs::Mob::new(
                id as u32 + 1,
                cow,
                Vec3::new(0.5, 65.0, 4.5),
                0.0,
            ));
            let health = game.mobs[0].health;
            game.lan_guest_packet(12, S::Attack(play::Attack { id }));
            assert_eq!(game.as_guest(12, |g| g.attack_ticks).unwrap(), 0);
            let location = [0.0, 0.625, -f64::from(0.49_f32)];
            game.lan_guest_packet(
                12,
                S::Interact(play::Interact {
                    id,
                    hand: 1,
                    location,
                    secondary: false,
                }),
            );
            assert_eq!(
                *spy.0.lock().unwrap(),
                [
                    ServerNotice::Attack(id),
                    ServerNotice::InteractSpecific(id, location),
                    ServerNotice::Interact(id, Hand::Off),
                ]
            );
            assert_eq!(game.mobs[0].health, health);
            assert_eq!(game.tnts[0].fuse, 80);
            assert_eq!(game.tnts[0].velocity, original_velocity);

            spy.0.lock().unwrap().clear();
            game.tnts[0].position.z = 28.5;
            game.lan_guest_packet(12, S::Attack(play::Attack { id }));
            game.lan_guest_packet(
                12,
                S::Interact(play::Interact {
                    id,
                    hand: 1,
                    location,
                    secondary: false,
                }),
            );
            assert!(spy.0.lock().unwrap().is_empty(), "out-of-reach packets");
        });
    });
}
