//! `LevelRef::explosion_destroy_blocks`' contract through both mod targets: a native call of the
//! handle method and a wasm guest's call through the generated forwarder, each on a fresh game,
//! on the stack a wasm guest runs on.
#![cfg(test)]
use crate::game::Game;
use crate::game::native_entity_refs::tests::{bind_local_player, run_on_guest_stack, with_game};
use crate::game::native_explosion_resistance_tests::{BOTH, Via, wasm_answer};
use horizon_client::{chunk::Chunk, mobs::Who, motion::PlayerMotion};
use horizon_core::{geometry::Vec3, position::ChunkPos, random::RandomSource};
use mod_api::{
    BlockPart, EntityRef, ExplosionDestroyError, ExplosionDetonateEvent, ExplosionInteraction,
    ExplosionStartEvent, Flow, ModEntry, OnBlockExplodedEvent, Registrar, RegistrationContext,
    ServerHost, wire::Writer,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// One `explosion_destroy_blocks` call.
struct Blast {
    source: Option<EntityRef>,
    center: [f64; 3],
    radius: f32,
    fire: bool,
    interaction: ExplosionInteraction,
}

/// A sourceless blast of radius 4 at `[5.5, 80.5, 5.5]` with `interaction`.
fn blast(interaction: ExplosionInteraction) -> Blast {
    Blast {
        source: None,
        center: [5.5, 80.5, 5.5],
        radius: 4.0,
        fire: false,
        interaction,
    }
}

/// Destroys `positions` with `blast` in the overworld through `via` and checks the answer.
fn destroy(
    game: &mut Game<'_>,
    via: Via,
    positions: &[[i32; 3]],
    blast: Blast,
    expected: Result<u32, ExplosionDestroyError>,
) {
    let level = game.level("minecraft:overworld").unwrap();
    match via {
        Via::Native => assert_eq!(
            level.explosion_destroy_blocks(
                game,
                positions,
                blast.source,
                blast.center,
                blast.radius,
                blast.fire,
                blast.interaction
            ),
            expected
        ),
        Via::Wasm => {
            let mut input = Writer::new();
            input.put(&level);
            input.put(&positions.to_vec());
            input.put(&blast.source);
            input.put(&blast.center);
            input.put(&blast.radius);
            input.put(&blast.fire);
            input.put(&blast.interaction);
            wasm_answer(
                game,
                level,
                "level_explosion_destroy_blocks",
                input,
                &expected,
            );
        }
    }
}

/// What the test mod does and saw.
#[derive(Clone, Default)]
struct Watch {
    /// `explosion_start` cancels.
    cancel: Arc<AtomicBool>,
    /// `explosion_detonate` takes `[1, 80, 1]` out and puts `[3, 80, 3]` in.
    edit: Arc<AtomicBool>,
    /// The detonate lists heard.
    detonated: Arc<Mutex<Vec<Vec<[i32; 3]>>>>,
    /// Where dirt's part heard `on_block_exploded`.
    exploded: Arc<Mutex<Vec<[i32; 3]>>>,
}
#[derive(Clone)]
struct Marker(Arc<Mutex<Vec<[i32; 3]>>>);
#[mod_api::horizon_part]
impl BlockPart for Marker {
    fn on_block_exploded(&self, _: &mut dyn ServerHost, event: &mut OnBlockExplodedEvent<'_>) {
        self.0
            .lock()
            .unwrap()
            .push([event.pos.x, event.pos.y, event.pos.z]);
    }
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Watch {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let dirt = r.block_key("minecraft:dirt");
        r.block_part(dirt, Marker(self.exploded.clone()));
    }
    fn explosion_start(&self, _: &mut dyn ServerHost, _: &mut ExplosionStartEvent<'_>) -> Flow {
        if self.cancel.load(Ordering::Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn explosion_detonate(&self, _: &mut dyn ServerHost, event: &mut ExplosionDetonateEvent<'_>) {
        self.detonated.lock().unwrap().push(event.blocks.clone());
        if self.edit.load(Ordering::Relaxed) {
            event.blocks.retain(|p| *p != [1, 80, 1]);
            event.blocks.push([3, 80, 3]);
        }
    }
}

/// Runs `test` on a fresh server game with the watching mod and a loaded, empty overworld chunk
/// (0, 0), for each target.
fn each_target(test: impl Fn(&mut Game<'_>, Via, &Watch) + Send + Sync + 'static) {
    run_on_guest_stack(move || {
        for via in BOTH {
            let watch = Watch::default();
            let mods: mod_api::Mods = vec![ModEntry::new("watch", Box::new(watch.clone()))].into();
            mod_platform::testing::with_mods(mods.clone(), || {
                crate::game::api_values::install_parts(&mods).unwrap();
                with_game(|game| {
                    game.build_registries();
                    game.load_server_loot();
                    game.set_allow_commands(true);
                    game.options.dedicated = Some(Vec::new());
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.block_ticks
                        .add_empty_chunk(ChunkPos::new(0, 0))
                        .unwrap();
                    test(game, via, &watch);
                });
            });
        }
    });
}

fn name(game: &Game<'_>, pos: [i32; 3]) -> String {
    let state = game.world.block(pos[0], pos[1], pos[2]);
    game.res.facts().get(state).name.to_string()
}

fn set(game: &mut Game<'_>, pos: [i32; 3], block: &str) {
    assert!(game.run_command(&format!(
        "setblock {} {} {} minecraft:{block}",
        pos[0], pos[1], pos[2]
    )));
}

/// How many of `item` lie dropped.
fn dropped(game: &Game<'_>, item: &str) -> u32 {
    game.dropped
        .iter()
        .filter(|e| &*e.stack.item == item)
        .map(|e| e.stack.count)
        .sum()
}

/// The 1000 cells of a 10-block cube in chunk (0, 0).
fn cube() -> Vec<[i32; 3]> {
    (0..10)
        .flat_map(|x| (70..80).flat_map(move |y| (0..10).map(move |z| [x, y, z])))
        .collect()
}

#[test]
#[ignore = "daily"]
fn a_thousand_stone_blocks_turn_to_air_with_drops_decayed_by_the_rule() {
    each_target(|game, via, _| {
        for decay in [true, false] {
            assert!(game.run_command("fill 0 70 0 9 79 9 minecraft:stone"));
            game.dropped.clear();
            assert!(game.ui.rules.set(
                "block_explosion_drop_decay",
                horizon_client::gamerules::Value::Bool(decay),
            ));
            // Under decay a stack survives one time in `radius`: none of 1000 at a radius of
            // a billion. Without it every stone drops its cobblestone.
            let mut blast = blast(ExplosionInteraction::Block);
            blast.radius = 1.0e9;
            destroy(game, via, &cube(), blast, Ok(1000));
            assert!(
                cube().iter().all(|&p| name(game, p) == "minecraft:air"),
                "{via:?}"
            );
            let expected = if decay { 0 } else { 1000 };
            assert_eq!(
                dropped(game, "minecraft:cobblestone"),
                expected,
                "{via:?} {decay}"
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn the_keep_interaction_changes_nothing() {
    each_target(|game, via, _| {
        set(game, [1, 80, 1], "stone");
        destroy(
            game,
            via,
            &[[1, 80, 1]],
            blast(ExplosionInteraction::None),
            Ok(0),
        );
        assert_eq!(name(game, [1, 80, 1]), "minecraft:stone", "{via:?}");
    });
}

#[test]
#[ignore = "daily"]
fn a_part_hears_its_block_exploded_once() {
    each_target(|game, via, watch| {
        set(game, [2, 80, 2], "dirt");
        // A position listed twice counts once.
        let positions = [[2, 80, 2], [2, 80, 2]];
        destroy(
            game,
            via,
            &positions,
            blast(ExplosionInteraction::Block),
            Ok(1),
        );
        assert_eq!(*watch.exploded.lock().unwrap(), [[2, 80, 2]], "{via:?}");
        assert_eq!(
            *watch.detonated.lock().unwrap(),
            [vec![[2, 80, 2]]],
            "{via:?}"
        );
        assert_eq!(name(game, [2, 80, 2]), "minecraft:air", "{via:?}");
    });
}

#[test]
#[ignore = "daily"]
fn a_detonate_listener_spares_a_position_and_adds_none() {
    each_target(|game, via, watch| {
        for p in [[1, 80, 1], [2, 80, 1], [3, 80, 3]] {
            set(game, p, "stone");
        }
        watch.edit.store(true, Ordering::Relaxed);
        let positions = [[1, 80, 1], [2, 80, 1]];
        destroy(
            game,
            via,
            &positions,
            blast(ExplosionInteraction::Block),
            Ok(1),
        );
        assert_eq!(name(game, [1, 80, 1]), "minecraft:stone", "{via:?}: spared");
        assert_eq!(name(game, [2, 80, 1]), "minecraft:air", "{via:?}");
        assert_eq!(
            name(game, [3, 80, 3]),
            "minecraft:stone",
            "{via:?}: added, ignored"
        );
    });
}

#[test]
#[ignore = "daily"]
fn tnt_primes_owned_by_the_causing_entity() {
    each_target(|game, via, _| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(14.5, 80.0, 14.5)).unwrap());
        bind_local_player(game, 11, 123);
        set(game, [5, 80, 5], "tnt");
        let mut blast = blast(ExplosionInteraction::Tnt);
        // A living source is its own causing entity.
        blast.source = game.entity(11);
        assert!(blast.source.is_some());
        destroy(game, via, &[[5, 80, 5]], blast, Ok(1));
        assert_eq!(name(game, [5, 80, 5]), "minecraft:air", "{via:?}");
        let owners: Vec<Option<Who>> = game.tnts.iter().map(|t| t.owner).collect();
        assert_eq!(owners, [Some(Who::Player)], "{via:?}");
    });
}

#[test]
#[ignore = "daily"]
fn drops_of_one_item_merge_at_the_first_blocks_position() {
    each_target(|game, via, _| {
        assert!(game.ui.rules.set(
            "block_explosion_drop_decay",
            horizon_client::gamerules::Value::Bool(false),
        ));
        let positions = [[1, 80, 1], [8, 80, 8]];
        for p in positions {
            set(game, p, "cobblestone");
        }
        // Java Random seed 7: nextInt(2) is 1, so the first cell stays first.
        game.world.random.set_seed(7);
        destroy(
            game,
            via,
            &positions,
            blast(ExplosionInteraction::Block),
            Ok(2),
        );
        assert_eq!(game.world.random.next_int(), -1_552_468_968, "{via:?}");
        let drops: Vec<_> = game
            .dropped
            .iter()
            .map(|e| (e.stack.item.to_string(), e.stack.count, e.position))
            .collect();
        assert_eq!(drops.len(), 1, "{via:?}: {drops:?}");
        let (item, count, at) = &drops[0];
        assert_eq!(
            (item.as_str(), *count),
            ("minecraft:cobblestone", 2),
            "{via:?}"
        );
        // Popped from the independently calculated first shuffled cell.
        let cell = [at.x.floor() as i32, at.z.floor() as i32];
        assert_eq!(cell, [1, 1], "{via:?}: {at:?}");
    });
}

#[test]
#[ignore = "daily"]
fn an_unloaded_position_is_skipped_and_not_counted() {
    each_target(|game, via, _| {
        set(game, [1, 80, 1], "stone");
        let positions = [[1, 80, 1], [100, 80, 100]];
        destroy(
            game,
            via,
            &positions,
            blast(ExplosionInteraction::Block),
            Ok(1),
        );
        assert_eq!(name(game, [1, 80, 1]), "minecraft:air", "{via:?}");
        assert!(
            !game.world.chunks.contains_key(&(6, 6)),
            "{via:?}: never loaded"
        );
    });
}

#[test]
#[ignore = "daily"]
fn more_positions_than_the_cap_are_refused_with_nothing_changed() {
    each_target(|game, via, watch| {
        set(game, [1, 80, 1], "stone");
        let positions = vec![[1, 80, 1]; 65_537];
        destroy(
            game,
            via,
            &positions,
            blast(ExplosionInteraction::Block),
            Err(ExplosionDestroyError::TooMany(65_537)),
        );
        assert_eq!(name(game, [1, 80, 1]), "minecraft:stone", "{via:?}");
        assert!(watch.detonated.lock().unwrap().is_empty(), "{via:?}");
    });
}

#[test]
#[ignore = "daily"]
fn a_cancelled_start_destroys_nothing() {
    each_target(|game, via, watch| {
        set(game, [1, 80, 1], "stone");
        watch.cancel.store(true, Ordering::Relaxed);
        destroy(
            game,
            via,
            &[[1, 80, 1]],
            blast(ExplosionInteraction::Block),
            Ok(0),
        );
        assert_eq!(name(game, [1, 80, 1]), "minecraft:stone", "{via:?}");
        assert!(watch.detonated.lock().unwrap().is_empty(), "{via:?}");
    });
}
