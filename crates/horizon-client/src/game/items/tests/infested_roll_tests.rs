//! `LivingEntity.hurtServer` runs `InfestedMobEffect.onMobHurt` for a hit that
//! was not blocked, even when armor or absorption eats all of it.
use super::Game;
use crate::game::{Audio, Options, Progress, Settings, execution_role, load_resources};
use horizon_client::chunk::Chunk;
use horizon_client::effects::EffectInstance;
use horizon_client::mobs::Mob;
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use horizon_core::position::ChunkPos;
use horizon_core::random::RandomSource;
use std::sync::Arc;

/// A game over 5x5 empty chunks around (0, 0) with a stone floor at y = 3, so
/// the silverfish the roll spawns at (8.5, 4, 8.5) have a loaded chunk.
fn floored_game(root: &std::path::Path) -> Game<'static> {
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        Vec::new(),
        None,
        root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.to_path_buf();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let resources = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        resources,
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
    for cx in -2..=2 {
        for cz in -2..=2 {
            game.world
                .chunks
                .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(cx, cz))
                .unwrap();
        }
    }
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    for x in -32..48 {
        for z in -32..48 {
            game.world.set(x, 3, z, stone).unwrap();
        }
    }
    game
}

/// A game whose window player stands at (8.5, 4, 8.5) over the floor.
fn game_with_player() -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let mut game = floored_game(&root);
    game.creative = false;
    game.lan_acting = Some(7);
    let at = Vec3::new(8.5, 4.0, 8.5);
    game.player = Some(PlayerMotion::new(game.motion, at).unwrap());
    game
}

/// ... and touches one puffed (state 2) pufferfish, which hurts for 3
/// (`1 + PuffState`).
fn game_with_fish() -> Game<'static> {
    let mut game = game_with_player();
    let at = Vec3::new(8.5, 4.0, 8.5);
    let kind = game.res.mob_types()["minecraft:pufferfish"].clone();
    let mut fish = Mob::new(9001, kind, at, 0.0);
    fish.extra.size = 2;
    game.mobs.push(fish);
    game
}

fn silverfish(game: &Game<'_>) -> usize {
    game.mobs
        .iter()
        .filter(|m| m.kind.id == "minecraft:silverfish")
        .count()
}

/// `LivingEntity.hurtServer`: `success = !blocked || damage > 0.0` gates the
/// effects' `onMobHurt`, and `damage` there is what the shield left, before
/// armor and absorption. A hit absorption takes whole still rolls the 10%
/// `InfestedMobEffect.onMobHurt` chance, which `actuallyHurt` — and so
/// `CombatTracker.recordDamage` — never sees.
#[test]
#[ignore = "daily"]
fn an_absorbed_hit_still_rolls_the_infested_effect() {
    let mut g = game_with_fish();
    g.give_player_effect("minecraft:infested", 200, 0);
    g.apply_effect_status();
    // Absorption IV: 20 above the health, more than the sting's 3.
    g.add_player_effect(EffectInstance::new("absorption", 600, 4));
    assert_eq!(g.player.as_ref().unwrap().health.absorption(), 20.0);
    let mut rolls = 0;
    for i in 0..128i64 {
        // Small seeds share their first draw, so spread them over the state.
        let seed = i.wrapping_mul(0x9E37_79B9_7F4A_7C15u64 as i64);
        let p = g.player.as_mut().unwrap();
        // Back to what the first sting left, and no cooldown to refuse it.
        let _ = p.set_health(20.0, true);
        p.health.hurt.cooldown = 0;
        g.mobs.retain(|m| m.kind.id != "minecraft:silverfish");
        g.world.random.set_seed(seed);
        g.touch_pufferfish();
        let p = g.player.as_ref().unwrap();
        assert_eq!(
            (p.health.absorption(), p.health.health()),
            (17.0, 20.0),
            "seed {seed}: the sting was not absorbed whole"
        );
        let events = std::mem::take(&mut g.player.as_mut().unwrap().events);
        g.record_combat(&events);
        rolls += silverfish(&g);
    }
    assert!(
        rolls > 0,
        "no silverfish came out of 128 fully absorbed hits"
    );
}

/// `FoodData.tick` starves a player with `player.hurtServer(level,
/// damageSources().starve(), 1.0F)`, so its effects' `onMobHurt` runs too: an
/// Infested starving player rolls the same 10% chance. The starvation hurt
/// builds its own damage environment, which must carry the status flags.
#[test]
#[ignore = "daily"]
fn a_starving_infested_player_rolls_the_infested_effect() {
    let mut g = game_with_player();
    g.give_player_effect("minecraft:infested", 200, 0);
    g.apply_effect_status();
    let mut rolls = 0;
    for i in 0..128i64 {
        let seed = i.wrapping_mul(0x9E37_79B9_7F4A_7C15u64 as i64);
        g.mobs.retain(|m| m.kind.id != "minecraft:silverfish");
        g.world.random.set_seed(seed);
        let rules = g.world.health_rules;
        let resources = &g.health_resources;
        let p = g.player.as_mut().unwrap();
        let _ = p.set_health(20.0, false);
        // The empty bar's next tick is the 80th, which starves.
        p.health.food.food = 0;
        p.health.food.saturation = 0.0;
        p.health.food.exhaustion = 0.0;
        p.health.food.timer = 79;
        // No hurt cooldown to refuse it: vanilla starves every 80 ticks.
        p.health.hurt.cooldown = 0;
        p.food_tick(
            &g.loop_context,
            crate::game::entity::damage_level_of!(g),
            resources,
            rules,
        )
        .unwrap();
        let events = std::mem::take(&mut g.player.as_mut().unwrap().events);
        g.record_combat(&events);
        rolls += silverfish(&g);
    }
    assert!(rolls > 0, "no silverfish came out of 128 starvation hurts");
}
