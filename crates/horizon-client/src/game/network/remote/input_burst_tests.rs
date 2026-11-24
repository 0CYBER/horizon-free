//! Bursts of key presses and clicks on a server's client: a player's act on the next tick in the
//! order they came, and a mod's loop of `click_mapping` acts 64 entries of each queue a tick, oldest
//! first, with nothing dropped below the queue's maximum.
use super::*;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// How many of each packet `packets` holds: punches, player actions (drops and swaps) and slot
/// changes, in that order.
fn counts(packets: &[String]) -> [usize; 3] {
    [
        "minecraft:punch",
        "minecraft:player_action",
        "minecraft:set_carried_item",
    ]
    .map(|id| packets.iter().filter(|p| *p == id).count())
}

/// A player's burst between two ticks, keys and buttons: the number key selects at the tick's
/// pick, the click punches, then the drop keys and the swap act in the order pressed, the first
/// drop telling the server of the new slot (`ensureHasSentCarriedItem`). One press of each a tick
/// for twenty ticks acts once a tick, on its own tick.
#[test]
#[ignore = "daily"]
fn a_players_burst_acts_on_the_next_tick_in_the_order_pressed() {
    with_pressing_game(|game, packets, _| {
        game.set_mode_id(1);
        game.forced_hit = None;
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:stone", 64));
        game.key_pressed(KeyCode::Digit2, KeyCode::Digit2).unwrap();
        game.play_mouse(MouseButton::Left, true).unwrap();
        game.play_mouse(MouseButton::Left, false).unwrap();
        game.key_pressed(KeyCode::KeyQ, KeyCode::KeyQ).unwrap();
        game.key_pressed(KeyCode::KeyF, KeyCode::KeyF).unwrap();
        game.key_pressed(KeyCode::KeyQ, KeyCode::KeyQ).unwrap();
        assert_eq!(
            sent(packets),
            Vec::<String>::new(),
            "nothing before the tick"
        );
        game.remote_handle_input();
        assert_eq!(
            sent(packets),
            [
                "minecraft:punch",
                "minecraft:set_carried_item",
                "minecraft:player_action",
                "minecraft:player_action",
                "minecraft:player_action"
            ]
        );
        assert_eq!(game.selected, 1);

        for _ in 0..20 {
            game.play_mouse(MouseButton::Left, true).unwrap();
            game.play_mouse(MouseButton::Left, false).unwrap();
            game.key_pressed(KeyCode::KeyQ, KeyCode::KeyQ).unwrap();
            game.remote_handle_input();
            assert_eq!(
                sent(packets),
                ["minecraft:punch", "minecraft:player_action"]
            );
        }
    });
}

/// A mod's loop queues 150 attack clicks, 150 drops and 70 number keys before a tick: each tick
/// acts on 64 of each queue, oldest first, until all have acted. The number keys select the slot
/// of the last press each tick takes, and the tick's first drop tells the server of it.
#[test]
#[ignore = "daily"]
fn a_mods_burst_acts_64_entries_of_each_queue_a_tick() {
    with_pressing_game(|game, packets, triggered| {
        game.set_mode_id(1);
        game.forced_hit = None;
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:stone", 64));
        for _ in 0..150 {
            assert_eq!(game.click_mapping("key.attack"), Ok(()));
            assert_eq!(game.click_mapping("key.drop"), Ok(()));
        }
        for press in 0..70 {
            let key = if press < 64 {
                "key.hotbar.4"
            } else {
                "key.hotbar.7"
            };
            assert_eq!(game.click_mapping(key), Ok(()));
        }
        let mut ticks = Vec::new();
        let mut selected = Vec::new();
        for _ in 0..4 {
            game.remote_handle_input();
            ticks.push(counts(&sent(packets)));
            selected.push(game.selected);
        }
        assert_eq!(ticks, [[64, 64, 1], [64, 64, 1], [22, 22, 0], [0, 0, 0]]);
        assert_eq!(selected, [3, 6, 6, 6]);
        assert_eq!(triggered.lock().unwrap().len(), 150);
    });
}

/// A queue holds 1024 entries at most: a mod's 1100th drop, attack click or number key is refused
/// and the call returns as usual; the 1024 queued act over the next ticks.
#[test]
#[ignore = "daily"]
fn a_full_queue_refuses_the_entry_and_the_call_returns() {
    with_pressing_game(|game, packets, triggered| {
        game.set_mode_id(1);
        game.forced_hit = None;
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:stone", 64));
        for _ in 0..1100 {
            assert_eq!(game.click_mapping("key.drop"), Ok(()));
            assert_eq!(game.click_mapping("key.attack"), Ok(()));
            assert_eq!(game.click_mapping("key.hotbar.2"), Ok(()));
        }
        assert_eq!(
            game.remote.as_ref().unwrap().pending_keys.entries().len(),
            1024
        );
        assert_eq!(game.hotbar_presses.entries().len(), 1024);
        let mut acted = [0; 3];
        for _ in 0..17 {
            game.remote_handle_input();
            let tick = counts(&sent(packets));
            acted[0] += tick[0];
            acted[1] += tick[1];
        }
        acted[2] = triggered.lock().unwrap().len();
        assert_eq!(acted, [1024, 1024, 1024]);
        assert!(game.hotbar_presses.entries().is_empty());
    });
}

/// Sprint hits on targets whose `Entity.hurtClient` returns true (boats, minecarts, End crystals,
/// shulker bullets) slow the attacker's horizontal velocity by 0.6 and end sprinting; targets
/// whose `hurtClient` is false (primed TNT) leave sprint and velocity untouched.
#[test]
#[ignore = "daily"]
fn sprint_hit_on_boat_cart_crystal_or_shulker_bullet_slows_attacker() {
    with_pressing_game(|game, _, _| {
        game.boats.push(crate::game::boats::Boat::new(
            801,
            "oak".into(),
            Vec3::new(0.0, 65.0, 2.0),
            0.0,
        ));
        game.carts.push(crate::game::minecarts::Minecart::new(
            802,
            crate::game::minecarts::CartKind::Rideable,
            Vec3::new(0.0, 65.0, 2.0),
        ));
        game.crystals.push(crate::game::dragon::Crystal {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 803,
            position: Vec3::new(0.0, 65.0, 2.0),
            show_bottom: false,
            time: 0,
            beam: None,
            invulnerable: false,
            fall: (false, 0.0),
            motion: Vec3::ZERO,
            rotation: [0.0, 0.0],
        });
        let mut bullet = horizon_client::projectiles::Projectile::new(
            horizon_client::projectiles::ProjectileKind::ShulkerBullet {
                target: None,
                dir: Some(1),
                steps: 0,
                delta: [0.0; 3],
                seed: 0,
            },
            Vec3::new(0.0, 65.0, 2.0),
            None,
        );
        bullet.id = 804;
        game.creatures.projectiles.push(bullet);
        game.add_primed_tnt(
            horizon_core::position::BlockPos::new(0, 65, 2),
            0.0,
            80,
            None,
            None,
        );
        let tnt_id = game.tnts[0].id as i32;

        for (label, id, should_slow) in [
            ("boat", 801, true),
            ("minecart", 802, true),
            ("end_crystal", 803, true),
            ("shulker_bullet", 804, true),
            ("primed_tnt", tnt_id, false),
        ] {
            game.hand_strength.ticker = 20;
            let p = game.player.as_mut().unwrap();
            p.sprinting = true;
            p.velocity = Vec3::new(1.0, 0.25, -0.5);
            assert!(game.remote_attack_entity(id, 2.0), "{label}");
            let p = game.player.as_ref().unwrap();
            if should_slow {
                assert!(!p.sprinting, "{label} should stop sprinting");
                assert!((p.velocity.x - 0.6).abs() < 1e-6, "{label}: {:?}", p.velocity);
                assert!(
                    (p.velocity.y - 0.25).abs() < 1e-6,
                    "{label}: {:?}",
                    p.velocity
                );
                assert!(
                    (p.velocity.z - -0.3).abs() < 1e-6,
                    "{label}: {:?}",
                    p.velocity
                );
            } else {
                assert!(p.sprinting, "{label} must not stop sprinting");
                assert!((p.velocity.x - 1.0).abs() < 1e-6, "{label}: {:?}", p.velocity);
            }
        }
    });
}
