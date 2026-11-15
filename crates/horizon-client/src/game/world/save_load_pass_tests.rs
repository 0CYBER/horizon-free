//! Reentrant saves during a loaded entity queue's per-row join notifications.
use super::*;
use horizon_core::nbt::Tag;
use mod_api::{EntityJoinLevelEvent, Flow, Hooks, ServerHost};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct SaveOnLoadedJoin {
    calls: Arc<AtomicUsize>,
    succeeded: Arc<AtomicUsize>,
}
#[mod_api::horizon_mod]
impl Hooks for SaveOnLoadedJoin {
    fn entity_join_level(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityJoinLevelEvent<'_>,
    ) -> Flow {
        assert!(event.loaded_from_disk);
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.succeeded.store(
                usize::from(host.commands().run_command("save-all flush")),
                Ordering::SeqCst,
            );
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn loaded_queue_join_save_refuses_without_losing_remaining_rows() {
    let root = PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join("save-load-pass");
    std::fs::create_dir_all(&root).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let succeeded = Arc::new(AtomicUsize::new(2));
    visibility_unwind_tests::with_save_game(&root, |game| {
        let tags = (1..=2)
            .map(|i| {
                Tag::compound([
                    ("id", Tag::String("minecraft:item".into())),
                    (
                        "Item",
                        horizon_client::save::item_to_nbt(&ItemStack::new(
                            "minecraft:diamond",
                            i as u32,
                        )),
                    ),
                    ("UUID", Tag::IntArray(vec![i, 2, 3, 4])),
                    (
                        "Pos",
                        Tag::List(vec![
                            Tag::Double((5.0 + f64::from(i)).to_bits()),
                            Tag::Double(64.0_f64.to_bits()),
                            Tag::Double(5.0_f64.to_bits()),
                        ]),
                    ),
                ])
            })
            .collect();
        game.world.queue_entity_nbt((0, 0), tags);
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "save-loaded-join",
            Box::new(SaveOnLoadedJoin {
                calls: calls.clone(),
                succeeded: succeeded.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            mod_api::__private::mark_game_thread();
            game.spawn_loaded_entities();
        });
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "both loaded rows must notify"
        );
        assert_eq!(game.dropped.len(), 2, "both rows remain live");
        game.save.flush().unwrap();
        let disk = game.save.load_entities(0, 0);
        assert!(
            succeeded.load(Ordering::SeqCst) == 0 && disk.is_empty(),
            "nested save result {}; disk contains {} rows while both rows remain live",
            succeeded.load(Ordering::SeqCst),
            disk.len()
        );
        game.save_game().unwrap();
        assert_eq!(
            game.save.load_entities(0, 0).len(),
            2,
            "settled save must keep both rows"
        );
    });
}
