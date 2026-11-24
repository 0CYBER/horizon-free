//! `FireworkRocketEntity.explode`'s game event and sound gate, and the
//! `ITEM_USED` an `useOn` counts. Prepared resources only.
use super::Game;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_client::{
    chunk::Chunk,
    items::ItemStack,
    mobs::Who,
    particles::{FireworkExplosion, FireworkShape, ParticleLevel},
};
use horizon_core::{geometry::Vec3, position::ChunkPos, random::LegacyRandom};
use std::sync::Arc;

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
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
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap(),
    );
    game.world.random = horizon_core::random::LevelRandom::for_level(LegacyRandom::new(114));
    test(&mut game);
}

/// One `small_ball` that twinkles, as `/summon` reads it: a `Starter` of 16
/// ticks that plays a blast first and a twinkle last.
fn twinkling() -> FireworkExplosion {
    FireworkExplosion {
        shape: FireworkShape::SmallBall,
        colors: vec![0xFF5555],
        fade: Vec::new(),
        trail: true,
        twinkle: true,
    }
}

fn tick_particles(game: &mut Game<'_>) {
    let level = ParticleLevel {
        world: &game.world,
        facts: game.res.facts(),
        collision: game.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    game.particles.tick(&level);
}

/// `FireworkParticles.Starter.playSound`: a silent rocket launches quietly (as
/// it already did) and bursts quietly too, drawing its sparks and flash only.
#[test]
#[ignore = "daily"]
fn a_silent_rocket_bursts_without_blast_or_twinkle() {
    with_game(|game| {
        let stack = ItemStack::new("minecraft:firework_rocket", 1);
        let mut launching = game.new_rocket(
            Vec3::new(6.5, 201.5, 10.5),
            &stack,
            false,
            &mut LegacyRandom::new(0),
        );
        launching.silent = true;
        launching.lifetime = 100;
        game.rockets.push(launching);
        game.audio.capture = Some(Vec::new());
        game.tick_rockets();
        let heard = game.audio.capture.take().unwrap_or_default();
        assert!(
            heard
                .iter()
                .all(|s| s.event != "minecraft:entity.firework_rocket.launch"),
            "a silent rocket launches quietly: {heard:?}"
        );
        game.rockets.clear();

        let (quiet_at, loud_at) = ([6.5, 201.5, 10.5], [12.5, 201.5, 10.5]);
        let mut quiet = game.new_rocket(
            Vec3::new(quiet_at[0], quiet_at[1], quiet_at[2]),
            &stack,
            false,
            &mut LegacyRandom::new(0),
        );
        quiet.silent = true;
        quiet.explosions = vec![twinkling()];
        let mut loud = game.new_rocket(
            Vec3::new(loud_at[0], loud_at[1], loud_at[2]),
            &stack,
            false,
            &mut LegacyRandom::new(0),
        );
        loud.explosions = vec![twinkling()];
        game.explode_rocket(&quiet);
        game.explode_rocket(&loud);
        game.particles.sounds.clear();
        for _ in 0..20 {
            tick_particles(game);
        }
        let bursts: Vec<(&str, [f64; 3])> = game
            .particles
            .sounds
            .iter()
            .filter(|s| s.0.starts_with("minecraft:entity.firework_rocket."))
            .map(|s| (s.0, s.1))
            .collect();
        assert_eq!(
            bursts,
            vec![
                ("minecraft:entity.firework_rocket.blast", loud_at),
                ("minecraft:entity.firework_rocket.twinkle", loud_at),
            ],
            "only the loud rocket bangs and twinkles, both at its own position"
        );
        assert!(
            !game.particles.is_empty(),
            "the silent rocket still drew its sparks and flash"
        );
    });
}

/// Recording of `Game::post_game_event`, as the sculk tests read it.
fn start_events() {
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));
}

fn take_explosions() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| {
        std::mem::take(e.borrow_mut().as_mut().unwrap())
            .into_iter()
            .filter(|e| e.0 == "minecraft:explode")
            .collect()
    })
}

fn stop_events() {
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = None);
}

/// `FireworkRocketEntity.explode`: `gameEvent(GameEvent.EXPLODE, getOwner())` at
/// the rocket, once, with and without explosions; a rocket nobody owns (a
/// dispenser's) has no source entity.
#[test]
#[ignore = "daily"]
fn a_detonating_rocket_posts_one_explode_from_its_owner() {
    with_game(|game| {
        let stack = ItemStack::new("minecraft:firework_rocket", 1);
        let mut burst = game.new_rocket(
            Vec3::new(8.5, 202.0, 8.5),
            &stack,
            false,
            &mut LegacyRandom::new(0),
        );
        burst.explosions = vec![twinkling()];
        burst.owner = Some(Who::Player);
        let mut plain = game.new_rocket(
            Vec3::new(9.5, 202.0, 8.5),
            &stack,
            false,
            &mut LegacyRandom::new(0),
        );
        plain.owner = Some(Who::Player);
        let mut stray = game.new_rocket(
            Vec3::new(10.5, 202.0, 8.5),
            &stack,
            false,
            &mut LegacyRandom::new(0),
        );
        stray.explosions = vec![twinkling()];

        start_events();
        game.explode_rocket(&burst);
        assert_eq!(
            take_explosions(),
            vec![("minecraft:explode", [8.5, 202.0, 8.5], Some(Who::Player))],
            "the burst is heard once, at the rocket, from the player who shot it"
        );
        game.explode_rocket(&plain);
        assert_eq!(
            take_explosions(),
            vec![("minecraft:explode", [9.5, 202.0, 8.5], Some(Who::Player))],
            "a rocket with no explosions still posts it"
        );
        game.explode_rocket(&stray);
        assert_eq!(
            take_explosions(),
            vec![("minecraft:explode", [10.5, 202.0, 8.5], None)],
            "`getOwner()` is null: no source entity"
        );
        stop_events();
    });
}

/// `ItemStack.useOn` awards `ITEM_USED` for `FireworkRocketItem.useOn`'s
/// SUCCESS; `FireworkRocketItem.use` while gliding awards its own.
#[test]
#[ignore = "daily"]
fn launching_from_a_block_and_while_gliding_use_the_rocket_up() {
    with_game(|game| {
        game.creative = false;
        let stack = ItemStack::new("minecraft:firework_rocket", 1);
        let hit = crate::game::Hit {
            inside: false,
            pos: [2, 199, 2],
            face: [0, 1, 0],
            point: [2.5, 200.0, 2.5],
        };
        for used in 1..=5 {
            game.inventory[game.selected] = Some(stack.clone());
            assert!(game.launch_firework(&hit, &stack), "a rocket goes up");
            assert_eq!(
                game.stats.get("used", "minecraft:firework_rocket"),
                used,
                "each right-click on the block counts one use"
            );
        }
        game.player.as_mut().unwrap().fall_flying = true;
        game.inventory[game.selected] = Some(stack.clone());
        assert!(game.boost_firework(&stack), "the gliding boost launches");
        assert_eq!(game.stats.get("used", "minecraft:firework_rocket"), 6);
    });
}

/// `FireworkRocketEntity.tick` for a rocket attached to another player, as a
/// client ticks it: gliding, it sits at the rocket hand,
/// `Entity.getHandHoldingItemAngle` (`calculateViewVector(0, yRot + 80)`
/// scaled by 0.5 for a right main arm holding it, `yRot - 80` for a rocket
/// only in the offhand); not gliding, at the feet. It takes the holder's
/// motion (`setDeltaMovement`), or the holder's step when no motion came.
#[test]
#[ignore = "daily"]
fn another_players_boost_rocket_sits_at_their_rocket_hand() {
    with_game(|game| {
        let kind = game.res.player_type().clone().unwrap();
        let feet = Vec3::new(4.0, 210.0, 6.0);
        let mut other = horizon_client::mobs::Mob::new(43, kind, feet, 30.0);
        other.yaw = 30.0;
        other.extra.fall_flying = true;
        other
            .extra
            .set_main_hand(Some("minecraft:firework_rocket".into()));
        other.velocity = Vec3::new(0.25, -0.5, 1.0);
        game.mobs.push(other);
        let stack = ItemStack::new("minecraft:firework_rocket", 1);
        let mut rocket = game.new_rocket(
            Vec3::new(3.0, 209.0, 5.0),
            &stack,
            true,
            &mut LegacyRandom::new(0),
        );
        rocket.attached_to = Some(43);
        rocket.lifetime = 100;
        game.rockets.push(rocket);
        let offset = |game: &mut Game<'_>, hand: [f64; 3]| {
            game.tick_remote_projectiles(|_, at| at);
            let at = game.rockets[0].position;
            for (got, want) in [at.x - feet.x, at.y - feet.y, at.z - feet.z]
                .into_iter()
                .zip(hand)
            {
                assert!((got - want).abs() < 1e-3, "{at:?}: hand offset {hand:?}");
            }
        };

        // yRot + 80 = 110: (-sin 110°, 0, cos 110°) / 2, to Mth's table.
        offset(game, [-0.469_846_310_4, 0.0, -0.171_010_071_7]);
        // Only in the offhand: the left arm, yRot - 80 = -50.
        game.mobs[0].extra.set_main_hand(None);
        game.mobs[0].extra.set_slot_id(
            horizon_items::equipment::OFFHAND,
            Some("minecraft:firework_rocket".into()),
        );
        offset(game, [0.383_022_221_6, 0.0, 0.321_393_804_8]);

        game.mobs[0].extra.fall_flying = false;
        game.tick_remote_projectiles(|_, at| at);
        let r = &game.rockets[0];
        assert_eq!(r.position, feet, "no glide: handAngle is Vec3.ZERO");
        assert_eq!(
            r.velocity,
            Vec3::new(0.25, -0.5, 1.0),
            "the holder's motion"
        );

        // No motion came for the holder (a Horizon server sends none): its step.
        game.mobs[0].velocity = Vec3::ZERO;
        game.mobs[0].previous = Vec3::new(3.5, 210.25, 5.0);
        game.tick_remote_projectiles(|_, at| at);
        assert_eq!(game.rockets[0].velocity, Vec3::new(0.5, -0.25, 1.0));
    });
}

#[test]
#[ignore = "daily"]
fn a_falling_rocket_hits_the_ground_and_one_in_water_splashes() {
    with_game(|game| {
        // Commands and collisions need resident terrain for both rocket columns.
        // Use the shared chunk fixture and the player's real ticket sources.
        crate::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
            game,
            ChunkPos::new(0, 0),
        );
        game.build_registries();
        game.set_allow_commands(true);
        game.update_chunk_tickets();
        assert!(game.run_command("setblock 6 199 10 stone"));
        // Keep the falling rocket submerged through the sixth move: one
        // floating source block ends at y = 200, above its final position.
        assert!(game.run_command("setblock 20 199 10 water"));
        assert!(game.run_command("setblock 20 200 10 water"));
        for x in [6, 20] {
            assert!(game.run_command(&format!(
                "summon firework_rocket {x}.5 202 10.5 {{LifeTime:100,HasBeenShot:1b,ShotAtAngle:1b,Motion:[0d,-0.5d,0d]}}"
            )));
        }
        game.audio.heard.clear();
        start_events();
        let mut falls = Vec::new();
        for _ in 0..6 {
            game.tick_rockets();
            falls.push(game.rockets[0].fall_distance);
        }
        let events: Vec<_> = crate::game::sculk::TEST_POSTED_EVENTS.with(|v| {
            std::mem::take(v.borrow_mut().as_mut().unwrap())
                .into_iter()
                .map(|e| (e.0, e.1[0].floor() as i32))
                .filter(|e| matches!(e.0, "minecraft:hit_ground" | "minecraft:splash"))
                .collect()
        });
        stop_events();
        // The fourth move ends on the stone without a collision; the fifth lands.
        assert_eq!(falls, [0.5, 1.0, 1.5, 2.0, 0.0, 0.0]);
        assert_eq!(game.rockets[1].fall_distance, 0.0);
        assert_eq!(
            events,
            [("minecraft:splash", 20), ("minecraft:hit_ground", 6)]
        );
        assert_eq!(
            game.audio.heard.get("minecraft:entity.generic.splash"),
            Some(&1)
        );
    });
}
