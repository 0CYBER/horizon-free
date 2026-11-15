//! A real idle server tick must not sweep or read the attachment index.
use super::*;
use crate::game::native_entity_refs::tests::with_game;
use horizon_core::nbt::Tag;
use horizon_mod_runtime::attachments::ATTACHMENTS_KEY;

#[test]
#[ignore = "daily"]
fn idle_tick_does_no_index_work_with_ten_thousand_bound_holders() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        for n in 0..10_000 {
            let uuid = [n, 0, 0, 1];
            let entity = Tag::compound([
                ("id", Tag::String("minecraft:painting".into())),
                ("UUID", Tag::IntArray(uuid.to_vec())),
                ("block_pos", Tag::IntArray(vec![1, 70, 3])),
                ("variant", Tag::String("minecraft:kebab".into())),
                (
                    ATTACHMENTS_KEY,
                    Tag::compound([(
                        "idle",
                        Tag::compound([(
                            "value",
                            Tag::compound([("value", Tag::compound([("n", Tag::Int(n))]))]),
                        )]),
                    )]),
                ),
            ]);
            game.load_painting(entity, Some(uuid), (0, 0));
        }
        assert_eq!(game.paintings.len(), 10_000);
        let index = game.attachment_indexes[&Dim::Overworld].clone();
        let before = index.lock().unwrap().snapshot_named("idle", "value");
        assert_eq!(before.len(), 10_000);
        let work = index.lock().unwrap().work_count();
        let tick = game.ticks;
        let time = game.world.game_time;
        game.tick().unwrap();
        assert_eq!(game.ticks, tick + 1);
        assert_eq!(game.world.game_time, time + 1, "the world was not frozen");
        assert_eq!(
            index.lock().unwrap().work_count(),
            work,
            "idle tick touched the index"
        );
        assert_eq!(game.paintings.len(), 10_000);
        let after = index.lock().unwrap().snapshot_named("idle", "value");
        assert!(Arc::ptr_eq(&before, &after));
    });
}
