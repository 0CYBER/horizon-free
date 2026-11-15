use super::*;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

fn fixture(
    blocks: &[([i32; 3], &str)],
    profession: u8,
    entries: &[&str],
    run: impl FnOnce(&mut Mob, &MobWorld<'_>),
) {
    let root = std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources");
    assert!(super::super::super::world::with_resource_test_world(
        std::path::Path::new(&root),
        &[],
        true,
        true,
        |world, registry| {
            for &(p, id) in blocks {
                let chunk = (p[0] >> 4, p[2] >> 4);
                if !world.chunks.contains_key(&chunk) {
                    world.chunks.insert(
                        chunk,
                        Arc::new(crate::chunk::Chunk::empty(chunk.0, chunk.1)),
                    );
                }
                world
                    .set(p[0], p[1], p[2], registry.default_state(id).unwrap())
                    .unwrap();
            }
        },
        |w, types| {
            let mut m = Mob::new(
                1,
                Arc::new(types.types["minecraft:villager"].clone()),
                Vec3::new(0.0, 64.0, 0.0),
                0.0,
            );
            super::super::super::villager::set_profession(
                &mut m,
                horizon_client::mobs::villager::profession_table(),
                usize::from(profession),
            );
            let tags = if entries.is_empty() {
                HashMap::new()
            } else {
                HashMap::from([(
                    match profession {
                        5 => "horizon:villager_secondary_poi/farmer",
                        6 => "horizon:villager_secondary_poi/fisherman",
                        _ => unreachable!("fixture uses farmers and fishermen"),
                    }
                    .to_owned(),
                    entries
                        .iter()
                        .map(|id| (*id).to_owned())
                        .collect::<HashSet<_>>(),
                )])
            };
            let mut view = w.clone();
            view.block_tags = &tags;
            SCAN_READS.with(|reads| reads.set(0));
            run(&mut m, &view);
        }
    ));
}

#[test]
#[ignore = "daily"]
fn farmer_without_entries_matches_vanilla() {
    fixture(
        &[
            ([1, 63, 0], "minecraft:farmland"),
            ([-4, 62, 4], "minecraft:farmland"),
            ([2, 63, 0], "minecraft:stone"),
        ],
        5,
        &[],
        |m, w| {
            // Independent copy of the original vanilla-only predicate and traversal.
            let mut vanilla = Vec::new();
            for x in -4..=4 {
                for y in -2..=2 {
                    for z in -4..=4 {
                        let p = [x, 64 + y, z];
                        if farmland(w, p) {
                            vanilla.push(p);
                        }
                    }
                }
            }
            sense_secondary(m, w);
            let actual = mem(m).secondary_job_site.value().unwrap();
            assert_eq!(actual, vec![[-4, 62, 4], [1, 63, 0]]);
            assert_eq!(actual, vanilla);
        },
    );
}

#[test]
#[ignore = "daily"]
fn farmer_tag_adds_to_farmland() {
    fixture(
        &[
            ([1, 63, 0], "minecraft:farmland"),
            ([0, 63, 1], "minecraft:soul_sand"),
        ],
        5,
        &["minecraft:soul_sand"],
        |m, w| {
            sense_secondary(m, w);
            assert_eq!(
                mem(m).secondary_job_site.value(),
                Some(vec![[0, 63, 1], [1, 63, 0]])
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn fisherman_with_entries_runs_the_same_sensor() {
    fixture(
        &[([1, 64, 1], "minecraft:water_cauldron")],
        6,
        &["minecraft:water_cauldron"],
        |m, w| {
            sense_secondary(m, w);
            assert_eq!(mem(m).secondary_job_site.value(), Some(vec![[1, 64, 1]]));
            assert_eq!(SCAN_READS.with(|reads| reads.get()), 405);
        },
    );
}

#[test]
#[ignore = "daily"]
fn profession_without_entries_skips_the_scan() {
    fixture(
        &[([1, 64, 1], "minecraft:water_cauldron")],
        6,
        &[],
        |m, w| {
            sense_secondary(m, w);
            assert_eq!(SCAN_READS.with(|reads| reads.get()), 0);
            assert_eq!(mem(m).secondary_job_site.value(), None);
            assert_eq!(
                mem(m).secondary_job_site.value().unwrap_or_default().len(),
                0
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn scan_bounds_are_inclusive() {
    fixture(
        &[
            ([5, 64, 0], "minecraft:soul_sand"),
            ([0, 61, 0], "minecraft:soul_sand"),
            ([-4, 62, 4], "minecraft:soul_sand"),
        ],
        6,
        &["minecraft:soul_sand"],
        |m, w| {
            sense_secondary(m, w);
            assert_eq!(mem(m).secondary_job_site.value(), Some(vec![[-4, 62, 4]]));
        },
    );
}

#[test]
#[ignore = "daily"]
fn custom_profession_detects_declared_secondary_sites() {
    fixture(
        &[
            ([1, 63, 0], "minecraft:stone"),
            ([2, 63, 0], "minecraft:farmland"),
        ],
        0,
        &[],
        |mob, world| {
            let id = |s| mod_api::Identifier::parse(s).unwrap();
            let definition = mod_api::ProfessionDefinition {
                key: mod_api::ProfessionKey(id("test:engineer")),
                workstations: vec![],
                held_poi: vec![],
                acquirable_poi: vec![],
                requested_items: vec![],
                secondary_blocks: vec![id("minecraft:stone")],
                work_sound: None,
                trades: vec![],
            };
            let table = crate::mobs::villager::ResolvedProfessions::prepare(
                vec![("test".into(), definition)],
                |_, _| true,
            )
            .unwrap();
            let mut view = world.clone();
            view.professions = &table;
            crate::mobs::villager::set_profession(mob, &table, 15);
            sense_secondary(mob, &view);
            assert_eq!(
                mem(mob).secondary_job_site.value().unwrap(),
                vec![[1, 63, 0]]
            );
        },
    );
}
