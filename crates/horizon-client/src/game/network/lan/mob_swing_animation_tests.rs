//! `ClientboundSwingAnimationPacket` carries the animation the swing started
//! with (`LivingEntity.swing`), not the default: a mob's spear thrust reaches
//! the players around as a STAB of the spear's 19 ticks, not a 6-tick whack.
use super::super::mob_swing_sync_tests::{guest_at, swings};
use crate::game::native_machines::tests::server_game;
use horizon_client::spear::Kinetic;

#[test]
#[ignore = "daily"]
fn a_mobs_spear_thrust_reaches_guests_as_a_stab() {
    let mut game = server_game();
    game.build_registries();
    game.set_allow_commands(true);
    assert!(game.run_command("difficulty normal"));
    assert!(game.run_command("summon zombie 8.5 80 8.5 {NoAI:1b,PersistenceRequired:1b}"));
    let id = game.mobs[0].id;
    let rx = guest_at(&mut game, [8.5, 80.0, 8.5]);
    let animation = Kinetic::of("minecraft:iron_spear")
        .expect("an iron spear")
        .attack_animation();
    game.mobs[0].swing_hand_modified(false, animation);
    game.lan_flush_effects();
    let sent = swings(&rx);
    assert!(
        sent.iter()
            .any(|p| p.id == id as i32 && p.kind == 2 && p.duration == 19),
        "the spear's STAB: {sent:?}"
    );
}
