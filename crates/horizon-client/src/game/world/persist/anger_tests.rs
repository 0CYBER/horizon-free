//! #13076: `NeutralMob`'s saved anger end time through the real summon, save
//! and tick paths.

use super::h262_enderman_absolute_anger_tests::with_game;
use super::*;

/// #13076: `NeutralMob.addPersistentAngerSaveData` writes the end time as
/// `readPersistentAngerSaveData` read it, past ones included, and a mob
/// that missed ticks while the clock went on does not move it forward.
#[test]
#[ignore = "daily"]
fn h13076_loaded_anger_end_time_is_kept() {
    with_game(|game| {
        crate::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
            game,
            horizon_core::position::ChunkPos::new(0, 0),
        );
        game.world.game_time = 100;
        assert!(game.run_command("gamerule spawn_mobs false"));
        let mut summon = |n: i32, kind: &str, end: i64, no_ai: bool| {
            let cmd = format!(
                "summon {kind} {}.5 200 4.5 {{UUID:[I;262,2,0,{n}],NoAI:{}b,NoGravity:1b,PersistenceRequired:1b,anger_end_time:{end}L}}",
                4 + n,
                u8::from(no_ai)
            );
            assert!(game.run_command(&cmd), "{cmd}");
            game.persist
                .uuids
                .iter()
                .find_map(|(id, u)| (*u == [262, 2, 0, n]).then_some(*id))
                .unwrap()
        };
        let past = summon(1, "wolf", 1, true);
        let zero = summon(2, "iron_golem", 0, true);
        let future = summon(3, "iron_golem", 999_999, false);
        let saved = |game: &mut Game<'_>, id: u32| {
            let i = game.mobs.iter().position(|m| m.id == id).unwrap();
            game.mob_tree_nbt(i).get("anger_end_time").cloned()
        };
        for (id, end) in [(past, 1), (zero, 0), (future, 999_999)] {
            assert_eq!(saved(game, id), Some(Tag::Long(end)), "as loaded");
        }
        // Five ticks of the clock in which the mobs do not tick.
        game.world.game_time += 5;
        for (id, end) in [(past, 1), (zero, 0), (future, 999_999)] {
            assert_eq!(saved(game, id), Some(Tag::Long(end)), "unticked");
        }
        // Real ticks separated by another unticked gap require post-tick settlement
        // to stamp the completed tick before the next resume.
        for step in 0..2 {
            if step == 1 {
                game.world.game_time += 5;
            }
            game.world.game_time += 1;
            game.update_chunk_tickets();
            assert!(game.entity_ticking_area()(6, 4));
            game.tick_creature_entities().unwrap();
        }
        for (id, end) in [(past, 1), (zero, 0), (future, 999_999)] {
            assert_eq!(saved(game, id), Some(Tag::Long(end)), "ticked");
        }
        let i = game.mobs.iter().position(|m| m.id == future).unwrap();
        // The countdown counted the missed ticks too.
        assert_eq!(
            i64::from(game.mobs[i].extra.anger),
            999_999 - game.world.game_time
        );
    });
}
