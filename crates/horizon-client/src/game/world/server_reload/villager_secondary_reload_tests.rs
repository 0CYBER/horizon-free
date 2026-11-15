use super::*;
use horizon_client::mobs::{self, Mob};
use horizon_core::geometry::Vec3;
use mod_api::{DataPackFilesEvent, Registrar, RegistrationContext, ServerHost};

const TAG: &str = "horizon:villager_secondary_poi/farmer";
struct Tags(Arc<AtomicBool>);
#[mod_api::__subscriptions]
impl Hooks for Tags {
    fn data_pack_files(&self, _: &mut dyn ServerHost, event: &mut DataPackFilesEvent<'_>) {
        let values = if self.0.load(Ordering::SeqCst) {
            r#"["minecraft:soul_sand"]"#
        } else {
            "[]"
        };
        event.files.push((
            "tags/horizon:block/villager_secondary_poi/farmer".into(),
            format!(r#"{{"replace":true,"values":{values}}}"#).into_bytes(),
        ));
    }
}
struct Missing;
#[mod_api::__subscriptions]
impl Hooks for Missing {
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        registrar.item_key("secondary_test:missing");
    }
}

fn memory(game: &crate::game::Game<'_>, mob: &mut Mob) -> Option<Vec<[i32; 3]>> {
    let mut world = game.mob_world_base(&[]);
    world.mob_griefing = false;
    let mut test_level = horizon_client::health::TestLevel::new(0);
    let mut level = test_level.level();
    for time in 0..100 {
        world.game_time = time;
        mobs::villager_brain::tick(mob, &world, &mut level, &mut Vec::new());
    }
    mobs::villager::ext(mob)
        .unwrap()
        .brain
        .as_ref()
        .unwrap()
        .mem
        .secondary_job_site
        .value()
}

fn fixture(run: impl FnOnce(&mut crate::game::Game<'_>, &Arc<AtomicBool>, &mut Mob)) {
    let enabled = Arc::new(AtomicBool::new(true));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "secondary_test",
            Box::new(Tags(enabled.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                let scratch = tempfile::tempdir().unwrap();
                let root = scratch.path().to_owned();
                game.load_world(request(&root)).unwrap();
                // Explicitly erase the complete sensor box so generated terrain cannot affect the fixture.
                let registry = game.pack.assets.registry;
                for x in -4..=4 {
                    for z in -4..=4 {
                        let chunk = (x >> 4, z >> 4);
                        if !game.world.chunks.contains_key(&chunk) {
                            game.world.chunks.insert(
                                chunk,
                                Arc::new(horizon_client::chunk::Chunk::empty(chunk.0, chunk.1)),
                            );
                        }
                        for y in 62..=66 {
                            game.world
                                .set(x, y, z, registry.default_state("minecraft:air").unwrap())
                                .unwrap();
                        }
                    }
                }
                for (p, id) in [
                    ([1, 63, 0], "minecraft:farmland"),
                    ([0, 63, 1], "minecraft:soul_sand"),
                ] {
                    game.world
                        .set(p[0], p[1], p[2], registry.default_state(id).unwrap())
                        .unwrap();
                }
                assert!(game.run_command("reload"));
                assert!(game.items().block_tags[TAG].contains("minecraft:soul_sand"));
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:villager")
                    .unwrap()
                    .clone();
                let mut mob = Mob::new(901, kind, Vec3::new(0.0, 64.0, 0.0), 0.0);
                mobs::villager::set_profession(
                    &mut mob,
                    horizon_client::mobs::villager::profession_table(),
                    5,
                );
                // Keep the full brain from resetting an untraded farmer with no primary POI.
                mob.extra.villager_level = 2;
                assert_eq!(memory(game, &mut mob), Some(vec![[0, 63, 1], [1, 63, 0]]));
                run(game, &enabled, &mut mob);
                let _ = std::fs::remove_dir_all(root);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn successful_reload_removes_secondary_tag_entries() {
    fixture(|game, enabled, mob| {
        enabled.store(false, Ordering::SeqCst);
        assert!(game.run_command("reload"));
        assert!(game.items().block_tags[TAG].is_empty());
        assert_eq!(memory(game, mob), Some(vec![[1, 63, 0]]));
    });
}

#[test]
#[ignore = "daily"]
fn failed_reload_preparation_keeps_secondary_tag_entries() {
    fixture(|game, enabled, mob| {
        enabled.store(false, Ordering::SeqCst);
        let old = game.world_tables.as_ref().map(|tables| tables as *const _);
        crate::game::api_values::install_parts(&[ModEntry::new(
            "secondary_test",
            Box::new(Missing),
        )])
        .unwrap();
        assert!(!game.run_command("reload"));
        assert_eq!(
            old,
            game.world_tables.as_ref().map(|tables| tables as *const _)
        );
        assert!(game.items().block_tags[TAG].contains("minecraft:soul_sand"));
        assert_eq!(memory(game, mob), Some(vec![[0, 63, 1], [1, 63, 0]]));
    });
}
