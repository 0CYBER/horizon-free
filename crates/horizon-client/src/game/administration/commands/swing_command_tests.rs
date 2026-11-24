//! `/swing` is `LivingEntity.swing(hand, animation, true)`: `SwingState.startIfAble`
//! refuses a swing asked for inside the first half of the running one, so a mob
//! does not replay its animation and keeps the swing's own length.
use super::super::super::native_machines::tests::server_game;
use horizon_client::mobs::ai::tick_swing;

#[test]
#[ignore = "daily"]
fn a_second_swing_inside_a_mobs_first_half_is_refused() {
    let mut game = server_game();
    game.build_registries();
    game.set_allow_commands(true);
    assert!(game.run_command("summon minecraft:pig 0.5 100 0.5"));
    // One tick into a default swing: `SwingState.ticks` is 1, inside its first half.
    game.mobs[0].swing();
    tick_swing(&mut game.mobs[0]);
    assert_eq!(
        game.mobs[0].ai.swing_time, 0,
        "the first swing's first tick"
    );
    assert!(game.run_command("swing @e[type=minecraft:pig,limit=1]"));
    assert_eq!(
        game.mobs[0].ai.swing_time, 0,
        "the command's swing is refused, not replayed"
    );
    assert_eq!(game.mobs[0].swing_duration(), 6, "and keeps its length");
}
