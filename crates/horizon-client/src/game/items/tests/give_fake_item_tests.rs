//! `/give` when the whole stack fits: vanilla spawns a one-tick item that
//! cannot be picked up (`ItemEntity.makeFakeItem`) and plays the pickup sound.
use super::super::native_machines::tests::server_game;
use super::*;
use horizon_core::geometry::Vec3;
use horizon_core::random::RandomSource;

fn diamonds(game: &Game<'_>) -> u32 {
    game.inventory
        .iter()
        .flatten()
        .filter(|s| &*s.item == "minecraft:diamond")
        .map(|s| s.count)
        .sum()
}

/// `GiveCommand.giveItem`'s fitted arm: `createItemStackToDrop(stack, false,
/// false)` then `makeFakeItem`, so one diamond leaves the player's head at once
/// (`PickupDelay` 32767, `Age` 5999) and cannot be taken.
#[test]
#[ignore = "daily"]
fn give_that_fits_spawns_a_fake_item_that_is_never_picked_up() {
    let mut game = server_game();
    let feet = Vec3::new(8.5, 100.0, 8.5);
    game.build_registries();
    game.player = Some(PlayerMotion::new(game.motion, feet).unwrap());
    game.set_allow_commands(true);
    assert!(game.run_command("give @s minecraft:diamond 1"));
    assert_eq!(diamonds(&game), 1, "the diamond went into the inventory");
    assert_eq!(game.dropped.len(), 1, "and one item flashed out");
    let item = &game.dropped[0];
    assert_eq!(&*item.stack.item, "minecraft:diamond");
    assert_eq!(item.stack.count, 1);
    assert_eq!(item.pickup_delay, 32767, "ItemEntity.setNeverPickUp");
    assert_eq!(item.age, 5999, "ItemEntity.makeFakeItem");
    // Thrown from the eyes, `getEyeY() - 0.3F`.
    let eye = game.player.as_ref().unwrap().dimensions.eye_height;
    assert!((item.position.y - (feet.y + f64::from(eye) - f64::from(0.3f32))).abs() < 1e-9);
    let p = item.position;
    // Standing in it, the player cannot take it (`ItemEntity.playerTouch` wants
    // a pickup delay of 0), so the inventory keeps its one diamond.
    game.player.as_mut().unwrap().position = Vec3::new(p.x, p.y, p.z);
    game.dropped[0].age = 0;
    game.tick_items();
    assert_eq!(diamonds(&game), 1, "the fake item was not picked up");
    assert_eq!(game.dropped.len(), 1);
    // Its lifetime is all but over: the next tick discards it.
    game.dropped[0].age = 5999;
    game.tick_items();
    assert!(game.dropped.is_empty(), "gone on its next tick");
}

/// `GiveCommand.giveItem`: `createItemStackToDrop(stack, false, false)` makes
/// the flash's four draws from the player's own random and then the pickup
/// sound's pitch two more, one stream, so the next throw from this player
/// continues it instead of repeating the level's.
#[test]
#[ignore = "daily"]
fn give_that_fits_draws_from_the_players_random() {
    let mut game = server_game();
    game.build_registries();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 100.0, 8.5)).unwrap());
    game.set_allow_commands(true);
    let mut random = game.player.as_ref().unwrap().random.clone();
    let velocity = horizon_client::entities::drop_motion(&mut random, game.yaw, game.pitch, false);
    let pitch = ((random.next_float() - random.next_float()) * 0.7 + 1.0) * 2.0;
    game.audio.capture = Some(Vec::new());
    assert!(game.run_command("give @s minecraft:diamond 1"));
    let flash = &game.dropped[0];
    assert_eq!(
        flash.velocity,
        Vec3::new(velocity[0], velocity[1], velocity[2]),
        "the flash scatters by the player's own random"
    );
    let played: Vec<(&str, f32)> = game
        .audio
        .capture
        .iter()
        .flatten()
        .map(|s| (s.event.as_str(), s.pitch))
        .collect();
    assert_eq!(
        played,
        [("minecraft:entity.item.pickup", pitch)],
        "and the pickup sound's pitch is the next two draws of that stream"
    );
    assert_eq!(
        game.player.as_ref().unwrap().random.state(),
        random.state(),
        "six draws of the player's random, not a fresh stream"
    );
}
