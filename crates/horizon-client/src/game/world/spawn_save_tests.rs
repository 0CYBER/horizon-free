//! Publishing spawn batches must not expose a partially published snapshot.
use super::*;
use mod_api::{EntityJoinLevelEvent, Flow, Hooks, ServerHost};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct SaveOnSpawnJoin {
    calls: Arc<AtomicUsize>,
    succeeded: Arc<AtomicUsize>,
}
#[mod_api::horizon_mod]
impl Hooks for SaveOnSpawnJoin {
    fn entity_join_level(
        &self,
        host: &mut dyn ServerHost,
        _: &mut EntityJoinLevelEvent<'_>,
    ) -> Flow {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if host.commands().run_command("save-all flush") {
            self.succeeded.fetch_add(1, Ordering::SeqCst);
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn spawn_batches_refuse_saves_until_all_rows_are_published() {
    for planned in [false, true] {
        let root = PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
            .join(format!("save-spawn-batch-{planned}"));
        std::fs::create_dir_all(&root).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let succeeded = Arc::new(AtomicUsize::new(0));
        visibility_unwind_tests::with_save_game(&root, |game| {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mobs: Vec<_> = (1..=2)
                .map(|i| {
                    horizon_client::mobs::Mob::new(
                        u32::MAX - i,
                        kind.clone(),
                        Vec3::new(5.0 + f64::from(i), 64.0, 5.0),
                        0.0,
                    )
                })
                .collect();
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "save-spawn-batch",
                Box::new(SaveOnSpawnJoin {
                    calls: calls.clone(),
                    succeeded: succeeded.clone(),
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                mod_api::__private::mark_game_thread();
                if planned {
                    game.add_planned(
                        mobs.into_iter()
                            .map(|m| spawner::Planned::Finalized(m, None))
                            .collect(),
                        Vec::new(),
                        Vec::new(),
                    );
                } else {
                    game.add_spawned(mobs, Vec::new(), Vec::new(), false);
                }
            });
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            assert_eq!(succeeded.load(Ordering::SeqCst), 0);
            assert_eq!(game.mobs.len(), 2);
            game.save.flush().unwrap();
            assert!(game.save.load_entities(0, 0).is_empty());
            game.save_game().unwrap();
            assert_eq!(game.save.load_entities(0, 0).len(), 2);
        });
    }
}
