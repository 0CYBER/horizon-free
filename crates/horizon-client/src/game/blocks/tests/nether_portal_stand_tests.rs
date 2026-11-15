//! Armor stands in a nether portal and the pause screen in one:
//! `Entity.handlePortal` carries a stand through like any other living entity
//! (`ArmorStand.getDefaultDimensions`; a marker's 0 by 0 box and a `NoGravity`
//! stand's `noPhysics` enter nothing), and `LocalPlayer.handlePortalTransitionEffect`
//! closes only the screens whose `Screen.isAllowedInPortal` is false.

use super::*;
use crate::game::game_tests::tests::server_game;
use horizon_client::entities::ArmorStand;
use horizon_client::motion::PlayerMotion;

/// `ArmorStand`'s portal: stone at `y`, nether portal blocks at `y + 1` and
/// `y + 2` in the column of `(x, z)`.
fn put_portal(game: &mut Game<'_>, x: i32, z: i32) {
    let registry = game.pack.assets.registry;
    let stone = registry.default_state("minecraft:stone").unwrap();
    let portal = registry.default_state("minecraft:nether_portal").unwrap();
    game.world.set(x, 63, z, stone).unwrap();
    game.world.set(x, 64, z, portal).unwrap();
    game.world.set(x, 65, z, portal).unwrap();
}

#[test]
#[ignore = "daily"]
fn armor_stands_in_a_nether_portal_go_through_and_cool_down() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 65.0, 8.5)).unwrap());
    game.load_ticking_neighbourhood(0, 0);
    // A stand standing in the portal at (4, 64..65, 4); a marker whose 0 by 0
    // box sweeps no block in the one at (8, 64..65, 8); and a `NoGravity` stand
    // (`ArmorStand.hasPhysics` false, so `Entity.isAffectedByBlocks` skips its
    // sweep) in the one at (12, 64..65, 12).
    put_portal(&mut game, 4, 4);
    put_portal(&mut game, 8, 8);
    put_portal(&mut game, 12, 12);
    game.armor_stands
        .push(ArmorStand::new(1, Vec3::new(4.5, 64.0, 4.5), 0.0));
    let mut marker = ArmorStand::new(2, Vec3::new(8.5, 64.0, 8.5), 0.0);
    marker.marker = true;
    game.armor_stands.push(marker);
    let mut floating = ArmorStand::new(3, Vec3::new(12.5, 64.0, 12.5), 0.0);
    floating.no_gravity = true;
    game.armor_stands.push(floating);
    game.update_chunk_tickets();
    assert!(
        game.entity_ticking_area()(4, 4),
        "chunk (0, 0) ticks entities"
    );
    // `Entity.handlePortal` runs at the start of the entity's tick, the sweep
    // of the blocks it moved through at the end of the last one.
    for _ in 0..3 {
        game.handle_entity_portals();
        game.find_entity_portals();
    }
    assert_eq!(
        game.armor_stands.iter().map(|a| a.id).collect::<Vec<_>>(),
        vec![2, 3],
        "the stand in the portal crossed, the marker and the NoGravity stand did not"
    );
    let arrival = game
        .portals
        .arrivals
        .iter()
        .find(|a| {
            horizon_client::save::text(a.tag.get("id")).as_deref() == Some("minecraft:armor_stand")
        })
        .expect("the stand waits for its exit to load");
    // `Entity.setPortalCooldown` gives it `getDimensionChangingDelay`.
    assert_eq!(
        horizon_client::save::number(arrival.tag.get("PortalCooldown")),
        Some(i64::from(ENTITY_DELAY))
    );
}

#[test]
#[ignore = "daily"]
fn the_portal_effect_keeps_the_screens_that_allow_it() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    // `Screen.isAllowedInPortal` is `isPauseScreen` for these: the pause menu,
    // the options over it and the chat stay open while the effect grows.
    for screen in [Screen::Paused, Screen::Options, Screen::Chat] {
        game.screen = screen;
        game.portals.client_inside = true;
        assert!(game.client_portal_effect(), "the effect grows");
        assert!(game.screen == screen, "{} stayed open", screen.id());
    }
    // A container screen does not: `Gui.setScreen(null)` closes it.
    game.screen = Screen::Inventory;
    game.portals.client_inside = true;
    assert!(game.client_portal_effect(), "the effect grows");
    assert!(
        game.screen == Screen::Playing,
        "the inventory closed in the portal"
    );
    // And no screen at all is nothing to close.
    game.portals.client_inside = true;
    assert!(game.client_portal_effect(), "the effect grows");
    assert!(game.screen == Screen::Playing, "playing stays playing");
}
