//! The spyglass slows the look only in first person: `MouseHandler.turnPlayer`
//! divides the sensitivity by 8 while `getCameraType().isFirstPerson() &&
//! player.isScoping()`, so a third-person camera keeps the full speed.
use super::*;
use crate::game::perspective::CameraType;
use horizon_client::items::ItemStack;
use horizon_client::mouse::degrees_per_count;

/// A server's game with a spyglass in hand and the use button held, so
/// `Player.isScoping` is true, or `None` without prepared resources.
fn scoping_game() -> Option<Game<'static>> {
    std::env::var_os("HORIZON_TEST_RESOURCES")?;
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    assert!(
        game.give(ItemStack::new("minecraft:spyglass", 1)).is_none(),
        "the spyglass fits in the empty inventory"
    );
    game.using = Some((game.selected, 0));
    Some(game)
}

#[test]
fn a_spyglass_slows_the_look_only_in_first_person() {
    let Some(mut game) = scoping_game() else {
        return;
    };
    assert!(game.scoping(), "holding the use button on a spyglass");
    let rate = |game: &mut Game<'_>, camera: CameraType| {
        game.camera_type = camera;
        degrees_per_count(game.look_options())
    };
    let first_person = rate(&mut game, CameraType::FirstPerson);
    let behind = rate(&mut game, CameraType::ThirdPersonBack);
    let front = rate(&mut game, CameraType::ThirdPersonFront);
    assert!(
        (behind / first_person - 8.0).abs() < 1e-12,
        "third person turns {behind} degrees a count, first person {first_person}"
    );
    assert_eq!(behind, front);
    // Without the spyglass, first person turns as fast as third person does.
    game.using = None;
    assert_eq!(rate(&mut game, CameraType::FirstPerson), behind);
}
