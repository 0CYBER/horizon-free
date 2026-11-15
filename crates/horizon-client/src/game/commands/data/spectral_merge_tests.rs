//! `/data merge` on a spectral arrow re-reads its glow `Duration`:
//! `SpectralArrow.readAdditionalSaveData` runs on every `Entity.load`, so a
//! merge restarts the glow with the value it writes (200 when it writes none).
use super::saved_tag_tests::with_command_game;
use super::*;
use horizon_client::{mobs::Who, projectiles::Projectile};

/// What a command says, and whether it succeeded.
fn say(game: &mut Game<'_>, command: &str) -> (bool, String) {
    game.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said = game.chat_capture.take().unwrap_or_default();
    (
        ok,
        said.iter().map(horizon_client::text::Text::plain).collect(),
    )
}

const ARROW: &str = "00001234-0000-0000-0000-000000000001";

/// The live spectral arrow, as the entity the command targets.
fn arrow(game: &Game<'_>) -> Projectile {
    let Some(Target::Projectile(id)) = game.target_with_uuid([4660, 0, 0, 1]) else {
        panic!("the summoned spectral arrow is a projectile");
    };
    game.creatures
        .projectiles
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .expect("the summoned spectral arrow is live")
}

fn duration(game: &Game<'_>) -> i32 {
    arrow(game).arrow.unwrap().spectral_duration
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_merged_duration_restarts_a_spectral_arrows_glow() {
    with_command_game("duration", |game| {
        assert!(game.run_command(
            "summon minecraft:spectral_arrow 4.5 200 4.5 {UUID:[I;4660,0,0,1],NoGravity:1b,Motion:[0d,0d,0d],Duration:120}"
        ));
        assert!(game.run_command(
            "summon minecraft:pig 4.5 200 4.5 {UUID:[I;4660,0,0,2],NoAI:1b,NoGravity:1b,Health:10f}"
        ));
        // The arrow loaded its own saved duration.
        assert_eq!(duration(game), 120);
        // `Entity.load` reads Duration again: the live glow takes the new value
        // and the next `/data get` reads it back.
        assert!(game.run_command(&format!("data merge entity {ARROW} {{Duration:600}}")));
        assert_eq!(duration(game), 600);
        let (_, said) = say(game, &format!("data get entity {ARROW} Duration"));
        assert!(said.ends_with(": 600"), "{said}");
        // What the arrow's target glows for: `SpectralArrow.doPostHurtEffects`.
        let Some(Target::Mob(pig)) = game.target_with_uuid([4660, 0, 0, 2]) else {
            panic!("the summoned pig is a mob");
        };
        let p = arrow(game);
        game.arrow_post_hurt_effects(&p, Who::Mob(pig));
        let m = game.mobs.iter().find(|m| m.id == pig).unwrap();
        assert!(
            m.extra.effects.contains(&("minecraft:glowing", 600, 0)),
            "{:?}",
            m.extra.effects
        );
        // A compound without Duration reads the constructor's 200.
        assert!(game.run_command(&format!("data remove entity {ARROW} Duration")));
        assert_eq!(duration(game), 200);
    });
}
