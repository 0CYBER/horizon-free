//! `light_emission` asked of a real part on a content block, through the game's own asks: only a
//! block declaring `dynamic_light` is asked, its answer reads the position, `None` keeps the
//! state's light, an answer above 15 is stored as 15, a bulk replacement and a reload of the
//! chunk keep or ask again as the cells' states say, and `check_light` and a server's light
//! update ask again.
use super::Game;
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game_at};
use horizon_client::chunk::Chunk;
use mod_api::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};

const LAMP: &str = "lights:lamp";
const PLAIN: &str = "lights:plain";

/// Answers by height, as a crystal brighter deeper down would, and records where it is asked;
/// at y 70 it answers what `glow` holds, as an input that changes without a block change.
#[derive(Clone, Default)]
struct Depth {
    asked: Arc<Mutex<Vec<[i32; 3]>>>,
    glow: Arc<AtomicU8>,
}
impl Depth {
    fn take(&self) -> Vec<[i32; 3]> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }
}

#[mod_api::horizon_part]
impl BlockPart for Depth {
    fn light_emission(&self, _: &dyn Host, event: &mut LightEmissionEvent) -> Option<u8> {
        self.asked
            .lock()
            .unwrap()
            .push([event.pos.x, event.pos.y, event.pos.z]);
        match event.pos.y {
            y if y < 0 => Some(9),
            64 => Some(12),
            65 => None,
            66 => Some(200),
            70 => Some(self.glow.load(Ordering::Relaxed)),
            _ => Some(255),
        }
    }
}

/// A lamp declaring `dynamic_light`, and a block with the same light that doesn't; the same
/// part is attached to both.
struct Lights(Depth);

#[mod_api::__subscriptions]
impl Hooks for Lights {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for id in [LAMP, PLAIN] {
            let key = r.block_key(id);
            r.block_part(key, self.0.clone());
        }
    }

    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        for (id, dynamic) in [(LAMP, true), (PLAIN, false)] {
            let mut block = mod_api::Block::new(id, "minecraft:glass");
            block.properties.light = Some(4);
            block.properties.dynamic_light = dynamic.then_some(true);
            event.blocks.push(block);
        }
    }
}

/// The two blocks' default states, and air.
struct States {
    lamp: horizon_world::registry::BlockStateId,
    plain: horizon_world::registry::BlockStateId,
    air: horizon_world::registry::BlockStateId,
}

/// Runs `test` in a fresh process on a game whose content root holds the two blocks, with the
/// overworld's chunk (0, 0) empty and no light edits queued.
fn with_lights(name: &str, test: impl FnOnce(&mut Game<'_>, LevelRef, States, Depth)) {
    if run_fresh_cases(name, "HORIZON_DYNAMIC_LIGHT_CHILD", &["game"]) {
        return;
    }
    let depth = Depth::default();
    let mods: Mods = vec![ModEntry::new("lights", Box::new(Lights(depth.clone())))].into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let root = crate::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    with_game_at(root, |game| {
        let registry = &game.pack.assets.registry;
        let states = States {
            lamp: registry.default_state(LAMP).unwrap(),
            plain: registry.default_state(PLAIN).unwrap(),
            air: registry.default_state("minecraft:air").unwrap(),
        };
        let light = &game.pack.context.light;
        assert!(light.is_dynamic(states.lamp.0));
        assert!(
            !light.is_dynamic(states.plain.0),
            "the declaration gates the ask"
        );
        assert_eq!(light.emission(states.lamp.0), 4);
        let mut chunk = Chunk::empty(0, 0);
        chunk.lit = true;
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.world.light_edits.clear();
        let level = game.level("minecraft:overworld").unwrap();
        test(game, level, states, depth);
    });
}

/// The stored answer of the cell at (1, y, 2) of chunk (0, 0).
fn answer(game: &Game<'_>, y: i32) -> Option<u8> {
    let chunk = game.world.chunks.get(&(0, 0)).unwrap();
    let cell = chunk.dynamic_cell(1, y, 2).unwrap();
    chunk.dynamic_emission.get(&cell).copied()
}

#[test]
#[ignore = "daily"]
fn a_dynamic_light_part_answers_by_position_for_declared_blocks_only() {
    with_lights(
        "game::dynamic_light_emission_tests::a_dynamic_light_part_answers_by_position_for_declared_blocks_only",
        |game, _, States { lamp, plain, air }, depth| {
            for (y, state) in [(-10, lamp), (64, lamp), (65, lamp), (66, lamp), (67, lamp)] {
                game.world
                    .set_with_notification(1, y, 2, state, false)
                    .unwrap();
            }
            game.world
                .set_with_notification(1, 68, 2, plain, false)
                .unwrap();
            game.ask_edited_light_emission();
            assert_eq!(
                depth.take(),
                [[1, -10, 2], [1, 64, 2], [1, 65, 2], [1, 66, 2], [1, 67, 2]],
                "the undeclared block at y 68 is not asked"
            );
            // Deeper is brighter; None keeps the state's 4 (no entry); 200 and 255 are stored
            // as 15.
            assert_eq!(
                [-10, 64, 65, 66, 67, 68].map(|y| answer(game, y)),
                [Some(9), Some(12), None, Some(15), Some(15), None]
            );

            // A bulk edit of the section holding y 64 to 67 replaces only the cell at y 66.
            let s = game
                .world
                .chunks
                .get(&(0, 0))
                .unwrap()
                .section_index(64)
                .unwrap();
            let chunk = Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap());
            let mut values: [u32; 4096] = std::array::from_fn(|i| chunk.sections[s].get(i));
            values[2 << 8 | 2 << 4 | 1] = air.0;
            chunk.replace_blocks(s, &values);
            assert_eq!(
                [-10, 64, 66, 67].map(|y| answer(game, y)),
                [Some(9), Some(12), None, Some(15)]
            );

            // The column comes in again, its answers gone: every lamp is asked once more.
            let mut reloaded = (**game.world.chunks.get(&(0, 0)).unwrap()).clone();
            reloaded.dynamic_emission.clear();
            reloaded.dynamic_pending = true;
            game.world.chunks.remove(&(0, 0));
            game.world.chunks.insert((0, 0), Arc::new(reloaded));
            game.ask_chunk_light_emission((0, 0));
            let mut asked = depth.take();
            asked.sort();
            assert_eq!(asked, [[1, -10, 2], [1, 64, 2], [1, 65, 2], [1, 67, 2]]);
            assert_eq!(
                [-10, 64, 65, 66, 67].map(|y| answer(game, y)),
                [Some(9), Some(12), None, None, Some(15)]
            );
            assert!(!game.world.chunks.get(&(0, 0)).unwrap().dynamic_pending);
        },
    );
}

/// `check_light` asks a declared block's part again once at the next update however often it
/// is called, and asks nothing of an undeclared block; it is refused on a client and for another
/// dimension's handle. A server's light update asks a client's part again, for the changed
/// sections only.
#[test]
#[ignore = "daily"]
fn check_light_and_a_light_update_ask_the_part_again() {
    with_lights(
        "game::dynamic_light_emission_tests::check_light_and_a_light_update_ask_the_part_again",
        |game, level, States { lamp, plain, .. }, depth| {
            depth.glow.store(5, Ordering::Relaxed);
            game.world
                .set_with_notification(1, 70, 2, lamp, false)
                .unwrap();
            game.world
                .set_with_notification(1, 71, 2, plain, false)
                .unwrap();
            game.ask_edited_light_emission();
            assert_eq!(depth.take(), [[1, 70, 2]]);
            assert_eq!(answer(game, 70), Some(5));
            game.world.light_edits.clear();

            // The part's input changes without a block change: the answer stays until a check.
            depth.glow.store(11, Ordering::Relaxed);
            for _ in 0..3 {
                assert!(game.level_check_light(level, [1, 70, 2]));
            }
            assert!(game.level_check_light(level, [1, 71, 2]));
            assert_eq!(
                game.world.light_edits,
                [((1, 70, 2), lamp.0, lamp.0), ((1, 71, 2), plain.0, plain.0)],
                "repeated checks queue one edit"
            );
            assert_eq!(answer(game, 70), Some(5));
            game.ask_edited_light_emission();
            assert_eq!(
                depth.take(),
                [[1, 70, 2]],
                "asked once, the undeclared block not at all"
            );
            assert_eq!(answer(game, 70), Some(11));
            game.world.light_edits.clear();

            // After the ask a check queues again.
            assert!(game.level_check_light(level, [1, 70, 2]));
            assert_eq!(game.world.light_edits.len(), 1);
            game.world.light_edits.clear();

            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            assert!(
                !game.level_check_light(nether, [1, 70, 2]),
                "another dimension"
            );
            let role = game.options.execution_role;
            game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
            assert!(!game.level_check_light(level, [1, 70, 2]), "a client");
            game.options.execution_role = role;
            assert!(game.world.light_edits.is_empty());

            // A light update for another section asks nothing; one for the lamp's asks it once.
            depth.glow.store(3, Ordering::Relaxed);
            let s = game
                .world
                .chunks
                .get(&(0, 0))
                .unwrap()
                .section_index(70)
                .unwrap();
            game.ask_updated_light_emission((0, 0), &[s + 1]);
            assert!(depth.take().is_empty());
            assert_eq!(answer(game, 70), Some(11));
            game.ask_updated_light_emission((0, 0), &[s]);
            assert_eq!(depth.take(), [[1, 70, 2]]);
            assert_eq!(answer(game, 70), Some(3));
        },
    );
}
