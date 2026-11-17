use super::*;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_client::{chunk::Chunk, motion::PlayerMotion};
use horizon_core::position::ChunkPos;
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
    options.game_dir = Some(
        std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
            .join(format!("h116-{:?}", std::thread::current().id())),
    );
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
    game.native_lifecycle
        .begin_open(mod_api::Authority::IntegratedServer);
    game.build_registries();
    game.set_allow_commands(true);

    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
    game.creative = true;
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("tick freeze"));
    assert!(game.run_command("fill 0 198 0 15 199 15 stone"));
    test(&mut game);
}

fn source(game: &mut Game<'_>, x: i32, z: i32, block: &str) {
    assert!(game.world.chunks.contains_key(&(0, 0)));
    assert!(game.run_command(&format!("setblock {x} 199 {z} end_stone strict")));
    assert!(game.run_command(&format!("setblock {x} 200 {z} {block} strict")));
    assert!(game.run_command(&format!("execute if block {x} 199 {z} end_stone")));
}
fn launch(game: &mut Game<'_>, kind: &str, n: i32, x: i32, z: i32, owner: bool) -> u32 {
    let own = if owner { ",Owner:[I;116,0,0,99]" } else { "" };
    assert!(game.run_command(&format!("summon {kind} {x}.5 202.5 {z}.5 {{UUID:[I;116,0,0,{n}],NoGravity:1b,Motion:[0d,-2d,0d]{own}}}")));
    let p = game
        .creatures
        .projectiles
        .iter()
        .find(|p| game.persist.uuid_of(p.id) == Some([116, 0, 0, n]))
        .unwrap();
    assert_eq!(p.kind.id(), format!("minecraft:{kind}"));
    assert_eq!(p.position, Vec3::new(x as f64 + 0.5, 202.5, z as f64 + 0.5));
    assert_eq!(p.velocity, Vec3::new(0.0, -2.0, 0.0));
    assert!(p.no_gravity);
    assert_eq!(p.owner_uuid, owner.then_some([116, 0, 0, 99]));
    if owner {
        assert!(matches!(p.owner, Some(Who::Mob(_))));
    }
    p.id
}
fn dropped(game: &Game<'_>, item: &str) {
    assert_eq!(game.dropped.len(), 1, "actual unfiltered item collection");
    assert_eq!(&*game.dropped[0].stack.item, item);
    assert_eq!(game.dropped[0].stack.count, 1);
}
#[test]
#[ignore = "daily"]
fn a_registered_arrow_collision_keeps_the_actual_projectile_for_chorus_loot() {
    with_game(|game| {
        source(game, 4, 4, "chorus_flower");
        assert!(game.dropped.is_empty());
        let id = launch(game, "arrow", 1, 4, 4, false);
        game.tick_projectiles().unwrap();
        assert!(
            game.run_command("execute if block 4 200 4 air"),
            "actual collision broke the source"
        );
        assert!(
            game.creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)
                .unwrap()
                .arrow
                .as_ref()
                .unwrap()
                .in_ground
        );
        dropped(game, "minecraft:chorus_flower");
    });
}
#[test]
#[ignore = "daily"]
fn a_registered_snowball_collision_keeps_the_actual_projectile_for_chorus_loot() {
    with_game(|game| {
        source(game, 4, 4, "chorus_flower");
        let id = launch(game, "snowball", 2, 4, 4, false);
        game.tick_projectiles().unwrap();
        assert!(game.run_command("execute if block 4 200 4 air"));
        assert!(!game.creatures.projectiles.iter().any(|p| p.id == id));
        dropped(game, "minecraft:chorus_flower");
    });
}
#[test]
#[ignore = "daily"]
fn denied_hits_other_blocks_and_direct_destroy_keep_their_drop_contracts() {
    with_game(|game| {
        source(game, 4, 4, "chorus_flower");
        assert!(game.run_command("gamerule projectiles_can_break_blocks false"));
        launch(game, "arrow", 3, 4, 4, false);
        game.tick_projectiles().unwrap();
        assert!(game.run_command("execute if block 4 200 4 chorus_flower"));
        assert!(game.dropped.is_empty());
        assert!(game.run_command("gamerule projectiles_can_break_blocks true"));
        assert!(
            game.ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING)
        );
        assert!(game.run_command("gamerule mob_griefing"));
        assert!(game.run_command(
            "summon pig 12.5 200 12.5 {UUID:[I;116,0,0,99],NoAI:1b,NoGravity:1b,Health:10f}"
        ));
        source(game, 8, 4, "chorus_flower");
        launch(game, "arrow", 4, 8, 4, true);
        game.tick_projectiles().unwrap();
        assert!(game.run_command("execute if block 8 200 4 chorus_flower"));
        assert!(game.dropped.is_empty());
        source(game, 12, 4, "chorus_flower");
        launch(game, "ender_pearl", 5, 12, 4, false);
        game.tick_projectiles().unwrap();
        assert!(game.run_command("execute if block 12 200 4 chorus_flower"));
        assert!(game.dropped.is_empty());
        source(game, 4, 8, "stone");
        launch(game, "arrow", 6, 4, 8, false);
        game.tick_projectiles().unwrap();
        assert!(game.run_command("execute if block 4 200 8 stone"));
        assert!(game.dropped.is_empty());
        assert!(game.run_command("gamerule block_drops false"));
        source(game, 8, 8, "chorus_flower");
        launch(game, "snowball", 7, 8, 8, false);
        game.tick_projectiles().unwrap();
        assert!(game.run_command("execute if block 8 200 8 air"));
        assert!(game.dropped.is_empty());
        assert!(game.run_command("gamerule block_drops true"));
        source(game, 4, 12, "chorus_flower");
        assert!(game.run_command("setblock 4 200 12 air destroy"));
        assert!(game.dropped.is_empty());
        source(game, 8, 12, "dirt");
        assert!(game.run_command("setblock 8 200 12 air destroy"));
        dropped(game, "minecraft:dirt");
    });
}
#[test]
#[ignore = "daily"]
fn default_drop_rng_continues_identically_with_table_sequence_or_level_fallback() {
    for named in [false, true] {
        with_game(|game| {
            let root =
                std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
            let mut table: serde_json::Value = serde_json::from_slice(
                &std::fs::read(root.join("data/minecraft/loot_table/blocks/gravel.json"))
                    .unwrap(),
            )
            .unwrap();
            if !named {
                table.as_object_mut().unwrap().remove("random_sequence");
            }
            let files = vec![(
                "loot_table/minecraft:blocks/gravel".to_owned(),
                vec![serde_json::to_vec(&table).unwrap()],
            )];
            let registry = game.pack.assets.registry;
            // The level callback requires its catalog for the world lifetime.
            // These two fixture catalogs live only for this test process.
            let items: &'static horizon_client::items::Items = Box::leak(Box::new(
                horizon_client::items::Items::load(
                    &root,
                    registry,
                    game.res.facts(),
                    &horizon_world::loot::PackOverlay {
                        files: &files,
                        conditions: &horizon_world::conditions::LoadConditions::default(),
                    },
                    game.res.item_defaults(),
                )
                .unwrap(),
            ));
            let state = registry.default_state("minecraft:gravel").unwrap();
            assert_eq!(
                items
                    .block_loot_table(registry, state)
                    .and_then(|id| items.loot_sequence(&id))
                    .is_some(),
                named
            );
            source(game, 4, 4, "gravel");
            game.level.seed = 116;
            game.world.random.set_seed(116);
            game.rand_sequences = Default::default();
            game.with_level(|b, level, ticks| {
                level.items = items;
                assert!(b.destroy_block(level, ticks, BlockPos::new(4, 200, 4), true));
            });
            game.process_level_events().unwrap();
            assert_eq!(game.dropped.len(), 1);
            let pool = game.rand_sequences.to_nbt();
            let sequence_words = match pool
                .get("sequences")
                .and_then(|v| v.get("minecraft:blocks/gravel"))
                .and_then(|v| v.get("source"))
            {
                Some(horizon_core::nbt::Tag::LongArray(v)) => v.clone(),
                _ => vec![],
            };
            assert_eq!(
                !sequence_words.is_empty(),
                named,
                "actual selected random source"
            );
            let mut random = game.world.random.clone();
            let continuation: Vec<i64> = (0..4).map(|_| random.next_long()).collect();
            let drops: Vec<_> = game
                .dropped
                .iter()
                .map(|d| (d.stack.item.to_string(), d.stack.count))
                .collect();
            println!(
                "h116-continuation {}",
                serde_json::json!({"named":named,"level_continuation":continuation,"sequence_words":sequence_words,"drops":drops})
            );
        });
    }
}

#[test]
#[ignore = "daily"]
fn breaker_context_is_target_only_and_does_not_escape_its_level_call() {
    with_game(|game| {
        source(game, 4, 4, "chorus_flower");
        source(game, 8, 4, "chorus_flower");
        source(game, 12, 4, "chorus_flower");
        let target = BlockPos::new(4, 200, 4);
        let neighbor = BlockPos::new(8, 200, 4);
        let state = game.world.block(4, 200, 4);
        // Actual loaded states and callback/evaluator; this is an isolation
        // unit, beside the registered projectile collision regressions.
        game.with_level(|_, level, _| {
            assert!(level.drop_breaker_at.is_none());
            level.drop_breaker_at = Some(target);
            level.drop_resources_now(neighbor, state);
            level.drop_resources_now(target, state);
        });
        dropped(game, "minecraft:chorus_flower");
        game.with_level(|_, level, _| assert!(level.drop_breaker_at.is_none()));
        for fail in [false, true] {
            let outcome: std::result::Result<(), ()> = game.with_level(|_, level, _| {
                level.drop_breaker_at = Some(target);
                if fail {
                    return Err(());
                }
                Ok(())
            });
            assert_eq!(outcome.is_err(), fail);
            game.with_level(|_, level, _| assert!(level.drop_breaker_at.is_none()));
        }
        // The same position on a later no-entity destruction cannot reuse
        // the former callback's presence. Nor can its neighbor.
        game.destroy_block_dropping([4, 200, 4]).unwrap();
        game.destroy_block_dropping([8, 200, 4]).unwrap();
        dropped(game, "minecraft:chorus_flower");
        let kept = game.with_level(|b, level, ticks| {
            assert!(level.drop_breaker_at.is_none());
            level.drop_tool = Some(((12, 200, 4), None));
            level.drop_into = Some(Vec::new());
            assert!(b.destroy_block(level, ticks, BlockPos::new(12, 200, 4), true));
            level.drop_into.take().unwrap()
        });
        assert!(kept.is_empty(), "machine's no-entity loot stays empty");
        dropped(game, "minecraft:chorus_flower");
    });
}
