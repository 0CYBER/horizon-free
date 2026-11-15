//! The bursts' sound and silence: the breeze's sound for a Wind Charged mob's
//! body (`WindChargedMobEffect.onMobRemoved`) and for a silent source no sound
//! at all (`ServerLevel.explode`'s `playSound`), and vehicles left alone by a
//! mob's blast with mob griefing off (`VehicleEntity.ignoreExplosion`).
use crate::game::commands::Target;
use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
use horizon_client::chunk::Chunk;
use horizon_world::food::Difficulty;
use std::sync::Arc;

const BREEZE_BURST: &str = "minecraft:entity.breeze.wind_burst";
const WIND_CHARGE_BURST: &str = "minecraft:entity.wind_charge.wind_burst";

/// A windowless integrated-server game on normal difficulty (a level.dat would
/// set it; the default would discard a zombie as Peaceful does) with chunk
/// (0, 0) loaded and a stone floor at y 63, which a dead mob falls onto so
/// that its burst has ground under it. `None` without the prepared resources.
fn with_game<R>(f: impl FnOnce(&mut Game<'static>) -> R) -> Option<R> {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES")?);
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .ok()?;
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
    )
    .unwrap();
    game.options.dedicated = Some(Vec::new());
    game.build_registries();
    game.set_allow_commands(true);
    game.world.health_rules.difficulty = Difficulty::Normal;
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.streaming.forced.insert((0, 0));
    assert!(game.run_command("fill 0 63 0 15 63 15 stone"));
    Some(f(&mut game))
}

/// How many times `sound` was played as a positional sound.
fn heard(game: &Game<'_>, sound: &str) -> u32 {
    game.audio.heard.get(sound).copied().unwrap_or(0)
}

/// An oak boat two blocks east of the burst's centre.
fn boat(game: &mut Game<'_>) -> u32 {
    match game
        .summon_with_nbt("minecraft:oak_boat", [6.5, 64.0, 4.5], "{}")
        .unwrap()
    {
        Some(Target::Boat(id)) => id,
        other => panic!("no boat: {other:?}"),
    }
}

fn boat_speed(game: &Game<'_>, id: u32) -> f64 {
    let v = game.boats.iter().find(|b| b.id == id).unwrap().velocity;
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

/// A zombie at (4.5, 64, 4.5) with the Wind Charged effect, `Silent` or not,
/// killed and ticked out: `LivingEntity.remove` with KILLED runs its effects
/// after `LivingEntity.tickDeath`'s twenty ticks.
fn a_wind_charged_zombie_dies(game: &mut Game<'_>, silent: bool) {
    let nbt = if silent { "{Silent:1b}" } else { "{}" };
    game.summon_with_nbt("minecraft:zombie", [4.5, 64.0, 4.5], nbt)
        .unwrap()
        .expect("summoned zombie");
    assert!(
        game.run_command(
            "effect give @e[type=minecraft:zombie,limit=1] minecraft:wind_charged 600"
        ),
        "the effect went on the zombie"
    );
    assert!(game.run_command("kill @e[type=minecraft:zombie,limit=1]"));
    for _ in 0..25 {
        game.tick_creatures().unwrap();
    }
}

/// `WindChargedMobEffect.onMobRemoved` calls `ServerLevel.explode` with
/// `SoundEvents.BREEZE_WIND_CHARGE_BURST`, so the burst is heard as the
/// breeze's, not as the wind charge's; `ServerLevel.explode` sends
/// `playSound = source == null || !source.isSilent()`, so a `Silent` mob's
/// burst is mute. The particle and the knockback stay either way.
#[test]
#[ignore = "daily"]
fn a_wind_charged_mob_bursts_with_the_breezes_sound_unless_it_is_silent() {
    let Some(outcome) = with_game(|game| {
        let mut outcome = Vec::new();
        for silent in [false, true] {
            let boat = boat(game);
            let (breeze, charge) = (heard(game, BREEZE_BURST), heard(game, WIND_CHARGE_BURST));
            game.particles.requested = Some(Vec::new());
            a_wind_charged_zombie_dies(game, silent);
            let gust = game.particles.requested.take().unwrap_or_default();
            outcome.push((
                silent,
                heard(game, BREEZE_BURST) - breeze,
                heard(game, WIND_CHARGE_BURST) - charge,
                gust.iter()
                    .filter(|(p, _, _)| {
                        p.ty == horizon_client::particles::ParticleType::GustEmitterLarge
                    })
                    .count(),
                boat_speed(game, boat) > 0.0,
            ));
            game.boats.clear();
        }
        outcome
    }) else {
        return;
    };
    assert_eq!(
        outcome,
        vec![
            // Heard once, as the breeze's, with the gust and the boat thrown.
            (false, 1, 0, 1, true),
            // A silent mob's burst is mute, and still pushes and draws.
            (true, 0, 0, 1, true),
        ]
    );
}

/// `AbstractWindCharge.explode` hands `ServerLevel.explode` the charge itself
/// as the source, so `playSound` is `!charge.isSilent()`: a `Silent` wind
/// charge's burst is mute, its gust and its push stay. The sound is the wind
/// charge's own, as `WindCharge.explode` passes it.
#[test]
#[ignore = "daily"]
fn a_silent_wind_charge_bursts_without_a_sound() {
    let Some(outcome) = with_game(|game| {
        let mut outcome = Vec::new();
        for silent in [false, true] {
            let boat = boat(game);
            let (charge, breeze) = (heard(game, WIND_CHARGE_BURST), heard(game, BREEZE_BURST));
            game.particles.requested = Some(Vec::new());
            let nbt = if silent {
                "{Silent:1b,Motion:[0.0,-1.0,0.0]}"
            } else {
                "{Motion:[0.0,-1.0,0.0]}"
            };
            game.summon_with_nbt("minecraft:wind_charge", [4.5, 66.0, 4.5], nbt)
                .unwrap()
                .expect("summoned wind charge");
            for _ in 0..40 {
                game.tick_creatures().unwrap();
            }
            let gust = game.particles.requested.take().unwrap_or_default();
            outcome.push((
                silent,
                heard(game, WIND_CHARGE_BURST) - charge,
                heard(game, BREEZE_BURST) - breeze,
                gust.iter()
                    .filter(|(p, _, _)| {
                        p.ty == horizon_client::particles::ParticleType::GustEmitterSmall
                    })
                    .count(),
                boat_speed(game, boat) > 0.0,
            ));
            game.boats.clear();
        }
        outcome
    }) else {
        return;
    };
    assert_eq!(
        outcome,
        vec![
            // Heard once, as the wind charge's, with the gust and the boat thrown.
            (false, 1, 0, 1, true),
            // A silent charge's burst is mute, and still pushes and draws.
            (true, 0, 0, 1, true),
        ]
    );
}

/// `VehicleEntity.ignoreExplosion` skips a whole blast for a boat or a
/// minecart when its indirect source is a mob and `mobGriefing` is off, so a
/// Wind Charged mob's death leaves a boat alone. The mob is the source, not a
/// breeze's charge (`ServerExplosion.canTriggerBlocks`), so the blocks that
/// react still do.
#[test]
#[ignore = "daily"]
fn a_mobs_burst_leaves_vehicles_alone_with_mob_griefing_off() {
    let Some(()) = with_game(|game| {
        assert!(game.run_command("gamerule mob_griefing false"));
        assert!(
            !game
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING)
        );
        // The rule change re-derives the difficulty from the game's
        // (`Game::sync_rules`), which no level.dat set here.
        game.world.health_rules.difficulty = Difficulty::Normal;
        // The block the burst's centre sits in is always one of the exploded
        // positions, so this door is one the blast reaches.
        assert!(game.run_command(
            "setblock 4 64 4 oak_door[half=lower,facing=north,hinge=left,open=false,powered=false]"
        ));
        let door = game.world.block(4, 64, 4);
        let boat = boat(game);
        a_wind_charged_zombie_dies(game, false);
        assert_eq!(boat_speed(game, boat), 0.0, "the boat was left alone");
        assert_ne!(
            game.world.block(4, 64, 4),
            door,
            "a mob's burst still opens doors with mob griefing off"
        );
        assert_eq!(heard(game, BREEZE_BURST), 1);
    }) else {
        return;
    };
}
