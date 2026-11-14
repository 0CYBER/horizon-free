//! `Strider.canAddPassenger` refuses a rider while the strider's eye is in
//! lava, though `Strider.mobInteract` still consumes the click; a strider
//! standing in lava with its eye above the surface and one in water both take
//! the rider.
use super::tests::server_game;
use crate::game::*;
use horizon_client::{items::ItemStack, mobs::Mob};
use horizon_core::geometry::Vec3;

/// Runs the fluid scan `Entity.updateFluidInteraction` would do on the mob's
/// next tick, over the game's current world.
fn scan_fluids(game: &mut Game<'_>) {
    let fluids = {
        let w = game.mob_world_base(&[]);
        let m = &game.mobs[0];
        w.fluids(m.bounds(), m.eye_y())
    };
    game.mobs[0].fluids = fluids;
}

#[test]
fn a_strider_with_its_eyes_in_lava_refuses_a_rider() {
    let Some(mut game) = server_game() else {
        return;
    };
    let (air, lava, water) = {
        let registry = game.pack.assets.registry;
        (
            registry.default_state("minecraft:air").unwrap(),
            registry.default_state("minecraft:lava").unwrap(),
            registry.default_state("minecraft:water").unwrap(),
        )
    };
    let kind = game
        .res
        .mob_types()
        .get("minecraft:strider")
        .unwrap()
        .clone();
    let mut strider = Mob::new(42, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    strider.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
    game.mobs.clear();
    game.mobs.push(strider);
    *game.held_mut() = None;

    // Lava over the strider's feet and its eye (y + 1.445, 0.85 of its 1.7).
    for y in [64, 65] {
        game.world.set(0, y, 0, lava).unwrap();
    }
    scan_fluids(&mut game);
    assert!(game.mobs[0].fluids.in_lava, "the strider stands in lava");
    assert!(game.mobs[0].fluids.eye_in_lava, "its eye is in the lava");
    assert!(
        game.interact_mount(0, ""),
        "Strider.mobInteract still consumes the click"
    );
    assert_eq!(
        game.riding, None,
        "Strider.canAddPassenger refuses the rider"
    );

    // Lava at its feet alone, its eye above the surface: `isEyeInFluid`, not
    // the body in lava, is what closes the seat.
    game.world.set(0, 65, 0, air).unwrap();
    scan_fluids(&mut game);
    assert!(game.mobs[0].fluids.in_lava);
    assert!(!game.mobs[0].fluids.eye_in_lava);
    assert!(game.interact_mount(0, ""));
    assert_eq!(game.riding, Some(42));

    // Water in the eyes does not stop it: only lava does.
    game.riding = None;
    for y in [64, 65] {
        game.world.set(0, y, 0, water).unwrap();
    }
    scan_fluids(&mut game);
    assert!(game.mobs[0].fluids.eye_in_water);
    assert!(!game.mobs[0].fluids.eye_in_lava);
    assert!(game.interact_mount(0, ""));
    assert_eq!(game.riding, Some(42));
}
