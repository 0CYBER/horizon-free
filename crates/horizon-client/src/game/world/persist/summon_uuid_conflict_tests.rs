// Test module extracted without changing its logical path.
use super::*;
use crate::game::commands::Target;
use horizon_client::motion::PlayerMotion;

#[test]
#[ignore = "daily"]
fn constructed_and_saved_mob_uuids_survive_repeated_reconstruction() {
    // Opt in only in a fresh child; never mutate the parallel parent's environment.
    if crate::game::native_entity_refs::tests::run_fresh_cases(
        "game::persist::summon_uuid_conflict_tests::constructed_and_saved_mob_uuids_survive_repeated_reconstruction",
        "HORIZON_TEST_SEEDS",
        &["1"],
    ) {
        return;
    }
    use horizon_core::random::RandomSource;

    // Independent Java 48-bit LCG calculation: uniquifier 12345 multiplied
    // (wrapping i64) by 1181783497276652981 seeds Entity.random with
    // -2257288423974277811. Two nextLongs with UUID version/variant masks
    // yield these words; nextFloat consumes the yaw, then nextInt is below.
    // A pig reconstructed without finalization consumes no further own draws.
    const CONSTRUCTOR_UUID: [i32; 4] = [1462068314, 225001827, -1901508738, 42864579];
    const NEXT_DRAW: i32 = -742666194;
    with_game(|game, _| {
        for saved_uuid in [None, Some([160, 0, 0, 987])] {
            let mut tag = Tag::compound([("id", Tag::String("minecraft:pig".into()))]);
            if let Some(uuid) = saved_uuid
                && let Tag::Compound(fields) = &mut tag
            {
                fields.insert("UUID".into(), Tag::IntArray(uuid.to_vec()));
            }
            // Reset only this test thread, after resource/game setup has drawn.
            horizon_client::random_source::set_thread_uniquifier(12345).unwrap();
            let mut id = game.load_mob(tag, None).unwrap();
            let uuid = saved_uuid.unwrap_or(CONSTRUCTOR_UUID);
            assert_eq!(game.persist.uuid_of(id), Some(uuid));
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == id)
                    .unwrap()
                    .random
                    .clone()
                    .next_int(),
                NEXT_DRAW
            );
            for _ in 0..3 {
                let tag = game.command_target_nbt(Target::Mob(id)).unwrap();
                assert_eq!(entity_nbt::uuid(&tag), Some(uuid));
                game.forget_entity(id);
                game.mobs.retain(|mob| mob.id != id);
                horizon_client::random_source::set_thread_uniquifier(12345).unwrap();
                id = game.load_mob(tag, None).unwrap();
                assert_eq!(game.persist.uuid_of(id), Some(uuid));
                assert_eq!(
                    game.mobs
                        .iter()
                        .find(|mob| mob.id == id)
                        .unwrap()
                        .random
                        .clone()
                        .next_int(),
                    NEXT_DRAW
                );
            }
        }
    });
}

fn positioned(kind: &str, uuid: Option<[i32; 4]>) -> Tag {
    let pos = |v: f64| Tag::Double(v.to_bits());
    let mut tag = Tag::compound([
        ("id", Tag::String(kind.into())),
        ("Pos", Tag::List(vec![pos(4.5), pos(64.0), pos(4.5)])),
    ]);
    if let (Some(uuid), Tag::Compound(fields)) = (uuid, &mut tag) {
        fields.insert("UUID".into(), Tag::IntArray(uuid.to_vec()));
    }
    tag
}

#[test]
#[ignore = "daily"]
fn cloud_fangs_and_kept_entities_keep_their_constructor_uuid() {
    // Opt in only in a fresh child; never mutate the parallel parent's environment.
    if crate::game::native_entity_refs::tests::run_fresh_cases(
        "game::persist::summon_uuid_conflict_tests::cloud_fangs_and_kept_entities_keep_their_constructor_uuid",
        "HORIZON_TEST_SEEDS",
        &["1"],
    ) {
        return;
    }
    use horizon_core::random::RandomSource;

    // Independent Java LCG calculation, as in the mob test above: uniquifier 12345
    // gives this Entity.random's first two nextLongs, the UUID. The constructor takes
    // exactly one uniquifier step, so the next Entity.random is the one the uniquifier
    // squared seeds, whose first nextInt is below.
    const CONSTRUCTOR_UUID: [i32; 4] = [1462068314, 225001827, -1901508738, 42864579];
    const NEXT_ENTITY_FIRST_INT: i32 = -705649180;
    const SAVED: [i32; 4] = [160, 0, 0, 987];
    const KINDS: [&str; 3] = [
        "minecraft:area_effect_cloud",
        "minecraft:evoker_fangs",
        "minecraft:marker",
    ];
    with_game(|game, _| {
        for kind in KINDS {
            for saved in [None, Some(SAVED)] {
                horizon_client::random_source::set_thread_uniquifier(12345).unwrap();
                let tag = positioned(kind, saved);
                let id = if kind == "minecraft:marker" {
                    game.load_other(tag, kind).unwrap()
                } else {
                    game.load_unlisted((0, 0), kind, saved, tag).unwrap();
                    if kind == "minecraft:evoker_fangs" {
                        game.fangs.list.last().unwrap().id
                    } else {
                        game.breath_clouds.last().unwrap().id
                    }
                };
                let expected = saved.unwrap_or(CONSTRUCTOR_UUID);
                assert_eq!(game.persist.uuid_of(id), Some(expected), "{kind} {saved:?}");
                assert_eq!(
                    game.uuid_for(id),
                    expected,
                    "{kind} {saved:?}: saving draws no second UUID"
                );
                assert_eq!(
                    horizon_client::random_source::entity().next_int(),
                    NEXT_ENTITY_FIRST_INT,
                    "{kind} {saved:?}: one constructor draw"
                );
            }
        }
    });
}

const PIG: &str = "NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Silent:1b";

fn pig(tag: &str, n: i32, riders: &str) -> String {
    format!("{{id:\"minecraft:pig\",UUID:[I;160,0,0,{n}],Tags:[\"{tag}\"],{PIG}{riders}}}")
}
fn riders(list: &[String]) -> String {
    format!(",Passengers:[{}]", list.join(","))
}
fn summon(game: &mut Game<'_>, tag: &str, n: i32, children: &str) -> bool {
    let body = pig(tag, n, children);
    let body = body.trim_start_matches("{id:\"minecraft:pig\",");
    game.run_command(&format!("summon minecraft:pig 4.5 200 4.5 {{{body}"))
}
/// The entities in the level that carry `uuid`.
fn live(game: &Game<'_>, n: i32) -> Vec<u32> {
    let mut ids: Vec<u32> = game
        .persist
        .uuids
        .iter()
        .filter(|(_, u)| **u == [160, 0, 0, n])
        .filter(|(id, _)| game.level_entity(**id as i32).is_some())
        .map(|(id, _)| *id)
        .collect();
    ids.sort_unstable();
    ids
}
fn mob_tagged(game: &Game<'_>, tag: &str) -> Vec<u32> {
    game.mobs
        .iter()
        .filter(|m| game.tags_of(Target::Mob(m.id)).iter().any(|t| t == tag))
        .map(|m| m.id)
        .collect()
}
fn mount(game: &Game<'_>, id: u32) -> Option<u32> {
    game.mobs.iter().find(|m| m.id == id).unwrap().extra.mount
}
/// The first tag and the UUID of each saved passenger of mob `id`, in order.
fn saved_riders(game: &mut Game<'_>, id: u32) -> Vec<(String, Option<Tag>)> {
    let saved = game.command_target_nbt(Target::Mob(id)).expect("saved mob");
    let Some(Tag::List(list)) = saved.get("Passengers") else {
        return Vec::new();
    };
    list.iter()
        .map(|p| {
            let tag = match p.get("Tags") {
                Some(Tag::List(t)) => match t.first() {
                    Some(Tag::String(s)) => s.to_string().unwrap_or_default(),
                    _ => String::new(),
                },
                _ => String::new(),
            };
            (tag, p.get("UUID").cloned())
        })
        .collect()
}
fn uuid(n: i32) -> Option<Tag> {
    Some(Tag::IntArray(vec![160, 0, 0, n]))
}

#[test]
#[ignore = "daily"]
fn pruning_keeps_graph_owned_objects_without_aliasing_duplicate_owners() {
    with_game(|game, _| {
        assert!(summon(
            game,
            "gc_root",
            177,
            &riders(&[
                pig("gc_parent", 177, &riders(&[pig("gc_leaf", 178, "")])),
                pig("gc_rejected_leaf", 177, ""),
            ])
        ));
        let root = mob_tagged(game, "gc_root")[0];
        let leaf = mob_tagged(game, "gc_leaf")[0];
        let parent = mount(game, leaf).unwrap();
        let rejected_leaf = *game
            .persist
            .off_level
            .keys()
            .find(|&&id| id != parent)
            .unwrap();
        // Health zero is not removal: a frozen dying passenger still saves.
        game.mob_object_mut(parent).unwrap().health = 0.0;
        game.prune_entity_memory();
        assert_eq!(
            saved_riders(game, root),
            vec![
                ("gc_parent".into(), uuid(177)),
                ("gc_rejected_leaf".into(), uuid(177))
            ]
        );

        game.dismount_mob(rejected_leaf);
        game.prune_entity_memory();
        assert!(!game.persist.off_level.contains_key(&rejected_leaf));
        assert_eq!(live(game, 177), vec![root]);
        assert_eq!(game.vehicle_by_uuid([160, 0, 0, 177]), Some(root));

        game.dismount_mob(parent);
        game.mobs.retain(|mob| mob.id != root);
        game.prune_entity_memory();
        assert!(game.persist.off_level.contains_key(&parent));
        assert_eq!(game.persist.uuids.get(&parent), Some(&[160, 0, 0, 177]));
        assert!(live(game, 177).is_empty());
        game.dismount_mob(leaf);
        game.prune_entity_memory();
        assert!(game.persist.off_level.is_empty());
        assert!(!game.persist.uuids.contains_key(&parent));
        assert!(!game.persist.raw.contains_key(&parent));
    });
}

#[test]
#[ignore = "daily"]
fn rejected_passenger_does_not_destroy_its_resident_duplicates_scores() {
    for score_at_destruction in [false, true] {
        with_game(|game, _| {
            assert!(summon(
                game,
                "score_root",
                240,
                &riders(&[pig(
                    "score_rejected",
                    240,
                    &riders(&[pig("score_leaf", 242, "")])
                )])
            ));
            let root = mob_tagged(game, "score_root")[0];
            let holder = super::uuid_text([160, 0, 0, 240]);
            assert!(game.run_command("scoreboard objectives add retained_uuid dummy"));
            assert!(game.run_command(&format!("scoreboard players set {holder} retained_uuid 7")));
            assert!(game.run_command("team add retained_uuid"));
            assert!(game.run_command(&format!("team join retained_uuid {holder}")));
            game.score_destroyed_entities();
            assert_eq!(
                game.score
                    .board
                    .score(&holder, "retained_uuid")
                    .map(|score| score.value),
                Some(7)
            );
            assert!(game.score.board.players_team(&holder).is_some());

            if !score_at_destruction {
                game.score.board.entity_removed(&holder);
            }
            // The former resident's actual removal still clears its UUID even
            // while its last geometry is retained for a canceled dismount.
            let mut removed = game.mob_object(root).unwrap().clone();
            removed.removed = true;
            game.persist
                .off_level
                .insert(root, super::OffLevel::Mob(removed));
            game.mobs.retain(|mob| mob.id != root);
            game.prune_entity_memory();
            assert!(game.score.board.score(&holder, "retained_uuid").is_none());
            assert!(game.score.board.players_team(&holder).is_none());
            assert!(game.run_command(&format!("scoreboard players set {holder} retained_uuid 11")));
            assert!(game.run_command(&format!("team join retained_uuid {holder}")));
            game.score_destroyed_entities();
            game.prune_entity_memory();
            assert_eq!(
                game.score
                    .board
                    .score(&holder, "retained_uuid")
                    .map(|score| score.value),
                Some(11)
            );
            assert!(game.score.board.players_team(&holder).is_some());
        });
    }
}

#[test]
#[ignore = "daily"]
fn data_load_keeps_passenger_links_on_both_sides_of_an_offlevel_object() {
    with_game(|game, _| {
        assert!(summon(
            game,
            "data_root",
            175,
            &riders(&[pig(
                "data_rejected",
                175,
                &riders(&[pig("data_leaf", 176, "")])
            )])
        ));
        let root = mob_tagged(game, "data_root")[0];
        let leaf = mob_tagged(game, "data_leaf")[0];
        let rejected = mount(game, leaf).unwrap();
        for tag in ["data_root", "data_leaf"] {
            assert!(game.run_command(&format!(
                "data merge entity @e[tag={tag},limit=1] {{Health:7.0f}}"
            )));
        }
        assert_eq!(mount(game, leaf), Some(rejected));
        assert_eq!(game.mob_object(root).unwrap().extra.rider, Some(rejected));
        assert_eq!(game.mob_object(rejected).unwrap().extra.rider, Some(leaf));
        assert!(game.mob_object(leaf).unwrap().extra.passenger);
    });
}

#[test]
#[ignore = "daily"]
fn dismount_uses_the_retained_vehicle_and_repairs_both_object_lists() {
    with_game(|game, _| {
        assert!(summon(
            game,
            "dismount_root",
            170,
            &riders(&[pig(
                "dismount_rejected",
                170,
                &riders(&[pig("dismount_leaf", 171, "")])
            )]),
        ));
        let root = mob_tagged(game, "dismount_root")[0];
        let leaf = mob_tagged(game, "dismount_leaf")[0];
        let rejected = mount(game, leaf).unwrap();
        assert!(game.level_entity(rejected as i32).is_none());
        assert_eq!(game.mob_object(rejected).unwrap().extra.rider, Some(leaf));

        game.dismount_mob(leaf);
        let leaf = game.mob_object(leaf).unwrap();
        assert_eq!(leaf.extra.mount, None);
        // Entity.getDismountLocationForPassenger: adult pig height is 0.9f.
        assert_eq!(leaf.position, Vec3::new(4.5, 200.89999997615814, 4.5));
        assert_eq!(game.mob_object(rejected).unwrap().extra.rider, None);
        assert_eq!(game.mob_object(root).unwrap().extra.rider, Some(rejected));

        // Entity.remove ejects even a passenger that never entered the level.
        game.dismount_mob(rejected);
        assert_eq!(game.mob_object(rejected).unwrap().extra.mount, None);
        assert_eq!(game.mob_object(root).unwrap().extra.rider, None);
        assert!(game.level_entity(rejected as i32).is_none());
    });
}

/// A game with a loaded, force-loaded chunk 0 0, a live pig holding UUID
/// 100 and a live armor stand holding UUID 150.
fn with_game(test: impl FnOnce(&mut Game<'_>, u32)) {
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
                .join(format!("h160-{:?}", std::thread::current().id())),
        );
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        // Setup controls: the live actors the later trees name.
        assert!(summon(game, "h160_existing", 100, ""));
        assert!(game.run_command(
            "summon minecraft:armor_stand 1.5 200 3.5 {UUID:[I;160,0,0,150],Tags:[\"h160_stand\"],NoGravity:1b}"
        ));
        let existing = live(game, 100);
        assert_eq!(existing.len(), 1);
        assert_eq!(live(game, 150).len(), 1);
        test(game, existing[0]);
    });
}

/// Expected values from the official 26.3 server:
/// ServerLevel.tryAddFreshEntityWithPassengers refuses the whole tree
/// when any of its UUIDs is loaded, before anything joins.
#[test]
#[ignore = "daily"]
fn summon_refuses_a_tree_holding_a_live_uuid() {
    with_game(|game, existing| {
        let stand = live(game, 150);
        let existing_at = game
            .mobs
            .iter()
            .find(|m| m.id == existing)
            .unwrap()
            .position;
        let time = game.world.game_time;

        // A unique tree is admitted whole, each member riding its parent.
        let leaf = pig("h160_unique_leaf", 122, "");
        let child = pig("h160_unique_child", 121, &riders(&[leaf]));
        assert!(summon(game, "h160_unique", 120, &riders(&[child])));
        let [r, c, l] = [120, 121, 122].map(|n| live(game, n));
        assert_eq!([r.len(), c.len(), l.len()], [1, 1, 1]);
        assert_eq!(mount(game, c[0]), Some(r[0]));
        assert_eq!(mount(game, l[0]), Some(c[0]));

        let mut failures = Vec::new();
        let conflicts = [
            ("h160_existing_root", 100, String::new(), vec![]),
            ("h160_existing_stand_root", 150, String::new(), vec![]),
            (
                "h160_existing_child",
                101,
                riders(&[pig("h160_existing_child_child", 100, "")]),
                vec![101],
            ),
            (
                "h160_existing_leaf",
                110,
                riders(&[pig(
                    "h160_existing_leaf_child",
                    111,
                    &riders(&[pig("h160_existing_leaf_leaf", 100, "")]),
                )]),
                vec![110, 111],
            ),
        ];
        for (tag, n, children, fresh) in conflicts {
            let mobs = game.mobs.len();
            let next = game.next_entity;
            let hooks = (game.persist.joins_canceled, game.persist.joined_seq);
            let draw = game.world.random.clone().next_long();
            if summon(game, tag, n, &children) {
                failures.push(format!("{tag}: summon succeeded"));
            }
            if game.mobs.len() != mobs {
                failures.push(format!(
                    "{tag}: mob count {} expected {mobs}",
                    game.mobs.len()
                ));
            }
            if game.next_entity != next {
                failures.push(format!(
                    "{tag}: entity id {} expected {next}",
                    game.next_entity
                ));
            }
            if (game.persist.joins_canceled, game.persist.joined_seq) != hooks {
                failures.push(format!("{tag}: join hook counts changed"));
            }
            if game.world.random.clone().next_long() != draw {
                failures.push(format!("{tag}: level random advanced"));
            }
            if !mob_tagged(game, tag).is_empty() {
                failures.push(format!("{tag}: root joined"));
            }
            for n in fresh {
                if !live(game, n).is_empty() {
                    failures.push(format!("{tag}: UUID {n} joined"));
                }
            }
        }
        if live(game, 100) != vec![existing] || live(game, 150) != stand {
            failures.push(format!(
                "existing actors: UUID 100 {:?}, UUID 150 {:?}",
                live(game, 100),
                live(game, 150)
            ));
        }

        // A later unique tree and a malformed UUID (not read: a fresh one) are admitted.
        let child = riders(&[pig("h160_unique_after_child", 141, "")]);
        if !summon(game, "h160_unique_after", 140, &child)
            || live(game, 140).len() != 1
            || live(game, 141).len() != 1
        {
            failures.push("unique_after: not admitted whole".into());
        }
        for (n, value) in ["[I;1,2,3]", "[I;1,2,3,4,5]", "\"not-a-uuid\"", "[L;1L,2L]"]
            .iter()
            .enumerate()
        {
            let tag = format!("h160_malformed_{n}");
            if !game.run_command(&format!(
                "summon minecraft:pig 4.5 200 4.5 {{UUID:{value},Tags:[\"{tag}\"],{PIG}}}"
            )) || mob_tagged(game, &tag).len() != 1
            {
                failures.push(format!("{tag}: not admitted"));
            }
        }

        let after = game
            .mobs
            .iter()
            .find(|m| m.id == existing)
            .unwrap()
            .position;
        assert_eq!(after, existing_at);
        assert_eq!(game.world.game_time, time);
        assert!(
            failures.is_empty(),
            "summon live UUID admission: {failures:#?}"
        );
    });
}

/// Official 26.3 observations from tools/observe_summon_uuid.py: the
/// unique grandchild is resident, but its rejected vehicle disconnects it
/// from level ticking. Saving retains the object graph, not a stale NBT copy.
#[test]
#[ignore = "daily"]
fn a_resident_below_an_offlevel_passenger_round_trips_and_resumes_after_dismount() {
    with_game(|game, _| {
        crate::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
            game,
            ChunkPos::new(0, 0),
        );
        let baby = |tag: &str, n: i32, passengers: &str| {
            pig(tag, n, passengers).replace("NoAI:1b", "Age:-1000,NoAI:1b")
        };
        let leaf = baby("h160_grandchild", 202, "");
        let child = baby("h160_rejected", 200, &riders(&[leaf]));
        let root = baby("h160_tick_root", 200, &riders(&[child]));
        assert!(game.run_command(&format!("summon minecraft:pig 4.5 200 4.5 {root}")));
        assert!(mob_tagged(game, "h160_rejected").is_empty());
        assert_eq!(live(game, 200).len(), 1);
        assert_eq!(live(game, 202).len(), 1);
        for _ in 0..5 {
            game.tick_world(true).unwrap();
        }
        let root = mob_tagged(game, "h160_tick_root")[0];
        let grandchild = mob_tagged(game, "h160_grandchild")[0];
        assert_eq!(
            game.command_target_nbt(Target::Mob(root))
                .unwrap()
                .get("Age"),
            Some(&Tag::Int(-995))
        );
        assert_eq!(
            game.command_target_nbt(Target::Mob(grandchild))
                .unwrap()
                .get("Age"),
            Some(&Tag::Int(-1000))
        );
        let vehicle = mount(game, grandchild).expect("off-level vehicle remains attached");
        assert!(game.level_entity(vehicle as i32).is_none());

        // The real chunk write/read path must reconstruct resident descendants
        // without accidentally admitting their rejected parent.
        let chunks = game.entity_chunks(&HashSet::from([(0, 0)]), true);
        assert!(game.mobs.is_empty());
        for (key, tags) in chunks {
            game.world.queue_entity_nbt(key, tags);
        }
        game.spawn_loaded_entities();
        assert!(mob_tagged(game, "h160_rejected").is_empty());
        assert_eq!(live(game, 200).len(), 1);
        assert_eq!(live(game, 202).len(), 1);
        assert!(
            game.run_command("data merge entity @e[tag=h160_grandchild,limit=1] {Health:7.0f}")
        );
        let root = mob_tagged(game, "h160_tick_root")[0];
        let saved = game.command_target_nbt(Target::Mob(root)).unwrap();
        let Some(Tag::List(children)) = saved.get("Passengers") else {
            panic!("rejected passenger was lost")
        };
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].get("UUID"), uuid(200).as_ref());
        let Some(Tag::List(grandchildren)) = children[0].get("Passengers") else {
            panic!("resident descendant was lost")
        };
        assert_eq!(grandchildren.len(), 1);
        assert_eq!(
            grandchildren[0].get("Health"),
            Some(&Tag::Float(7.0f32.to_bits()))
        );
        assert_eq!(grandchildren[0].get("Age"), Some(&Tag::Int(-1000)));

        assert!(game.run_command("ride @e[tag=h160_grandchild,limit=1] dismount"));
        for _ in 0..5 {
            game.tick_world(true).unwrap();
        }
        let grandchild = mob_tagged(game, "h160_grandchild")[0];
        assert_eq!(mount(game, grandchild), None);
        assert_eq!(
            game.command_target_nbt(Target::Mob(grandchild))
                .unwrap()
                .get("Age"),
            Some(&Tag::Int(-995))
        );
        let saved = game.command_target_nbt(Target::Mob(root)).unwrap();
        let Some(Tag::List(children)) = saved.get("Passengers") else {
            panic!("rejected passenger was lost on dismount")
        };
        assert!(children[0].get("Passengers").is_none());
    });
}

/// Official 26.3 retains the removed root long enough to dismount its
/// rejected child at max(child.y, root.y), then seats the frozen grandchild.
#[test]
#[ignore = "daily"]
fn removed_root_retains_its_geometry_until_offlevel_passengers_dismount() {
    with_game(|game, _| {
        crate::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
            game,
            ChunkPos::new(0, 0),
        );
        game.options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let baby = |tag: &str, n: i32, passengers: &str| {
            pig(tag, n, passengers).replace("NoAI:1b", "Age:-1000,NoAI:1b")
        };
        let leaf = baby("raised_grandchild", 232, "");
        let child = baby("raised_rejected", 230, &riders(&[leaf]));
        let root = baby("raised_root", 230, &riders(&[child]));
        assert!(game.run_command(&format!("summon minecraft:pig 4.5 200 4.5 {root}")));
        for _ in 0..5 {
            game.tick_world(true).unwrap();
        }
        let chunks = game.entity_chunks(&HashSet::from([(0, 0)]), true);
        for (key, tags) in chunks {
            game.world.queue_entity_nbt(key, tags);
        }
        game.spawn_loaded_entities();
        assert!(
            game.run_command(
                "data merge entity @e[tag=raised_root,limit=1] {Pos:[4.5d,210.0d,4.5d]}"
            )
        );
        let leaf = mob_tagged(game, "raised_grandchild")[0];
        assert_eq!(game.mob_object(leaf).unwrap().position.y, 200.0);
        assert!(game.run_command("kill @e[tag=raised_root]"));
        for _ in 0..25 {
            game.tick_world(true).unwrap();
        }
        assert!(mob_tagged(game, "raised_root").is_empty());
        assert_eq!(game.mob_object(leaf).unwrap().position.y, 210.5);
        assert_eq!(
            game.command_target_nbt(Target::Mob(leaf))
                .unwrap()
                .get("Age"),
            Some(&Tag::Int(-1000))
        );
        let chunks = game.entity_chunks(&HashSet::from([(0, 0)]), true);
        for (key, tags) in chunks {
            game.world.queue_entity_nbt(key, tags);
        }
        game.spawn_loaded_entities();
        assert!(mob_tagged(game, "raised_grandchild").is_empty());
        assert!(game.persist.off_level.is_empty());
    });
}

/// Official 26.3 teleports passengers before their copied vehicle. The
/// previously rejected child takes the UUID, so the copied root is off-level.
#[test]
#[ignore = "daily"]
fn dimension_travel_admits_the_duplicate_child_before_the_copied_root() {
    with_game(|game, _| {
        game.options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        game.world_root = game.save.root.clone();
        game.enter_level(super::super::Dim::Nether).unwrap();
        assert_eq!(game.dim, super::super::Dim::Nether);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        game.enter_level(super::super::Dim::Overworld).unwrap();
        assert_eq!(game.dim, super::super::Dim::Overworld);
        let baby = |tag: &str, n: i32, passengers: &str| {
            pig(tag, n, passengers).replace("NoAI:1b", "Age:-1000,NoAI:1b")
        };
        let grandchild = baby("h160_travel_grandchild", 222, "");
        let child = baby("h160_travel_rejected", 220, &riders(&[grandchild]));
        let root = baby("h160_travel_root", 220, &riders(&[child]));
        assert!(game.run_command(&format!("summon minecraft:pig 4.5 200 4.5 {root}")));
        for _ in 0..5 {
            game.tick_world(true).unwrap();
        }

        let root_id = mob_tagged(game, "h160_travel_root")[0];
        assert!(
            game.teleport_vehicle_across(
                crate::game::commands::Target::Mob(root_id),
                super::super::Dim::Nether,
                [4.5, 200.0, 4.5],
                None,
                false,
                crate::game::vehicle_teleport::TeleportMotion::Stop,
            )
            .unwrap()
        );
        assert!(mob_tagged(game, "h160_travel_root").is_empty());
        assert!(mob_tagged(game, "h160_travel_rejected").is_empty());
        assert!(mob_tagged(game, "h160_travel_grandchild").is_empty());
        game.enter_level(super::super::Dim::Nether).unwrap();
        assert!(mob_tagged(game, "h160_travel_root").is_empty());
        let child = mob_tagged(game, "h160_travel_rejected");
        let grandchild = mob_tagged(game, "h160_travel_grandchild");
        assert_eq!(child.len(), 1);
        assert_eq!(grandchild.len(), 1);
        assert_eq!(live(game, 220), child);
        assert_eq!(mount(game, grandchild[0]), Some(child[0]));
        let copied_root = mount(game, child[0]).expect("copied root object remains attached");
        assert!(game.level_entity(copied_root as i32).is_none());
        for _ in 0..5 {
            game.tick_world(true).unwrap();
        }
        for id in [child[0], grandchild[0]] {
            assert_eq!(
                game.command_target_nbt(Target::Mob(id)).unwrap().get("Age"),
                Some(&Tag::Int(-1000))
            );
        }
        // A ghost root is not independently saved by the level. Its resident
        // descendants must not be promoted to chunk roots on unload/reload.
        let chunks = game.entity_chunks(&HashSet::from([(0, 0)]), true);
        for (key, tags) in chunks {
            game.world.queue_entity_nbt(key, tags);
        }
        game.spawn_loaded_entities();
        assert!(mob_tagged(game, "h160_travel_rejected").is_empty());
        assert!(mob_tagged(game, "h160_travel_grandchild").is_empty());
        assert!(game.persist.off_level.is_empty());
    });
}

/// `ServerLevel.tryAddFreshEntityWithPassengers` / `PersistentEntitySectionManager.addEntity`:
/// vanilla adds a tree's members in order and skips one whose UUID an earlier member took;
/// that one still rides its vehicle and saves under its Passengers (#11807).
#[test]
#[ignore = "daily"]
fn summon_skips_a_uuid_taken_earlier_in_the_same_tree() {
    with_game(|game, _| {
        let mut failures = Vec::new();
        let pigs = game.mobs.len();
        if !summon(
            game,
            "h160_same_uuid",
            102,
            &riders(&[pig("h160_same_uuid_child", 102, "")]),
        ) {
            failures.push("same_uuid: summon failed".into());
        }
        let same = mob_tagged(game, "h160_same_uuid");
        if live(game, 102) != same || same.len() != 1 {
            failures.push(format!(
                "same_uuid: live 102 {:?} root {same:?}",
                live(game, 102)
            ));
        }
        if !mob_tagged(game, "h160_same_uuid_child").is_empty() {
            failures.push("same_uuid: child joined".into());
        }
        if let [root] = same[..]
            && saved_riders(game, root) != vec![("h160_same_uuid_child".into(), uuid(102))]
        {
            failures.push(format!(
                "same_uuid: saved riders {:?}",
                saved_riders(game, root)
            ));
        }

        let siblings = riders(&[
            pig("h160_sibling_a", 104, ""),
            pig("h160_sibling_b", 104, ""),
        ]);
        if !summon(game, "h160_sibling", 103, &siblings) {
            failures.push("sibling: summon failed".into());
        }
        let root = mob_tagged(game, "h160_sibling");
        let a = mob_tagged(game, "h160_sibling_a");
        if live(game, 104) != a || a.len() != 1 || root.len() != 1 {
            failures.push(format!("sibling: live 104 {:?} a {a:?}", live(game, 104)));
        } else if mount(game, a[0]) != Some(root[0]) {
            failures.push("sibling: a does not ride the root".into());
        }
        if !mob_tagged(game, "h160_sibling_b").is_empty() {
            failures.push("sibling: b joined".into());
        }
        if let [root] = root[..] {
            let saved = saved_riders(game, root);
            let expected = vec![
                ("h160_sibling_a".to_string(), uuid(104)),
                ("h160_sibling_b".to_string(), uuid(104)),
            ];
            if saved != expected {
                failures.push(format!("sibling: saved riders {saved:?}"));
            }
        }

        let leaf = riders(&[pig("h160_leaf_root_leaf", 130, "")]);
        let child = riders(&[pig("h160_leaf_root_child", 131, &leaf)]);
        if !summon(game, "h160_leaf_root", 130, &child) {
            failures.push("leaf_root: summon failed".into());
        }
        let child = mob_tagged(game, "h160_leaf_root_child");
        if !mob_tagged(game, "h160_leaf_root_leaf").is_empty() || child.len() != 1 {
            failures.push("leaf_root: leaf joined or child missing".into());
        } else if saved_riders(game, child[0]) != vec![("h160_leaf_root_leaf".into(), uuid(130))] {
            failures.push(format!(
                "leaf_root: saved riders {:?}",
                saved_riders(game, child[0])
            ));
        }
        if game.mobs.len() != pigs + 5 {
            failures.push(format!(
                "internal: mob count {} expected {}",
                game.mobs.len(),
                pigs + 5
            ));
        }
        assert!(
            failures.is_empty(),
            "summon in-tree UUID admission: {failures:#?}"
        );
    });
}

/// Where each pig of a boat or minecart root's tree stands in the Nether, by
/// the official 26.3 server (`tools/observe_summon_uuid.py --cases root_child
/// --action travel --root-type oak_boat|minecart --final-restart`, with and
/// without `--dismount-after-travel grandchild|rejected`).
/// Heights are the rejected pig's, then its grandchild's.
struct RootTravel {
    root: &'static str,
    /// After `Entity.teleportCrossDimension` ejected and remounted them. A
    /// grandchild that leaves the pig stays where it is, and so does the pig.
    seated: [f64; 2],
    /// After the pig leaves the retained root: it is seated on the pig again.
    rejected_dismounted: [f64; 2],
}

const OAK_BOAT_TRAVEL: RootTravel = RootTravel {
    root: "minecraft:oak_boat",
    seated: [200.5625, 201.01249998807907],
    rejected_dismounted: [200.5625, 201.0625],
};

const MINECART_TRAVEL: RootTravel = RootTravel {
    root: "minecraft:minecart",
    seated: [200.69999998807907, 201.14999997615814],
    rejected_dismounted: [200.69999998807907, 201.19999998807907],
};

fn age(game: &mut Game<'_>, id: u32) -> Option<Tag> {
    game.command_target_nbt(Target::Mob(id))
        .unwrap()
        .get("Age")
        .cloned()
}

/// Write and read back the entities of chunk 0 0, as an unload or a restart does.
fn reload_chunk(game: &mut Game<'_>) {
    let chunks = game.entity_chunks(&HashSet::from([(0, 0)]), true);
    for (key, tags) in chunks {
        game.world.queue_entity_nbt(key, tags);
    }
    game.spawn_loaded_entities();
}

/// A boat or minecart root whose pig passenger shares its UUID travels by
/// command after a save and reload. Passengers are admitted before their
/// copied vehicle, so the pig takes the UUID and the copied root is retained
/// off-level: unselectable, never ticked, yet still the pig's vehicle.
fn vehicle_root_travel(expected: &RootTravel, dismount: Option<&str>) {
    with_game(|game, _| {
        game.options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        game.world_root = game.save.root.clone();
        let ticking = crate::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood;
        ticking(game, ChunkPos::new(0, 0));
        game.enter_level(super::super::Dim::Nether).unwrap();
        ticking(game, ChunkPos::new(0, 0));
        assert!(game.run_command("forceload add 0 0"));
        game.enter_level(super::super::Dim::Overworld).unwrap();
        let baby = |tag: &str, n: i32, passengers: &str| {
            pig(tag, n, passengers).replace("NoAI:1b", "Age:-1000,NoAI:1b")
        };
        let grandchild = baby("root_travel_grandchild", 242, "");
        let rejected = baby("root_travel_rejected", 240, &riders(&[grandchild]));
        assert!(game.run_command(&format!(
            "summon {} 1.5 200 1.5 {{UUID:[I;160,0,0,240],Tags:[\"root_travel_root\"],NoGravity:1b,Passengers:[{rejected}]}}",
            expected.root
        )));
        for _ in 0..5 {
            game.tick_world(true).unwrap();
        }
        reload_chunk(game);
        assert!(mob_tagged(game, "root_travel_rejected").is_empty());
        assert_eq!(mob_tagged(game, "root_travel_grandchild").len(), 1);

        let source_root = match (game.boats.first(), game.carts.first()) {
            (Some(boat), _) => Target::Boat(boat.id),
            (_, Some(cart)) => Target::Cart(cart.id),
            _ => panic!("the summoned root is resident in the source"),
        };
        // `TeleportCommand` into another level: `Entity.teleportCrossDimension`.
        assert!(
            game.teleport_vehicle_across(
                source_root,
                super::super::Dim::Nether,
                [1.5, 200.0, 1.5],
                None,
                false,
                crate::game::vehicle_teleport::TeleportMotion::Stop,
            )
            .unwrap()
        );
        // The source keeps none of the transferred objects.
        assert!(!game.run_command("data get entity @e[tag=root_travel_root,limit=1]"));
        assert!(mob_tagged(game, "root_travel_grandchild").is_empty());
        assert!(game.persist.off_level.is_empty());

        game.enter_level(super::super::Dim::Nether).unwrap();
        assert!(!game.run_command("data get entity @e[tag=root_travel_root,limit=1]"));
        let rejected = mob_tagged(game, "root_travel_rejected");
        let grandchild = mob_tagged(game, "root_travel_grandchild");
        assert_eq!((rejected.len(), grandchild.len()), (1, 1));
        let (rejected, grandchild) = (rejected[0], grandchild[0]);
        assert_eq!(live(game, 240), vec![rejected]);
        assert_eq!(mount(game, grandchild), Some(rejected));
        let root = mount(game, rejected).expect("the copied root stays the pig's vehicle");
        assert!(game.level_entity(root as i32).is_none());
        assert!(game.boats.iter().all(|b| b.id != root));
        assert!(game.carts.iter().all(|c| c.id != root));
        assert!(game.boat_object(root).is_some() || game.cart_object(root).is_some());
        let y = |game: &Game<'_>, id: u32| game.mob_object(id).unwrap().position.y;
        assert_eq!([y(game, rejected), y(game, grandchild)], expected.seated);
        for _ in 0..5 {
            game.tick_world(true).unwrap();
        }
        // Neither the retained root nor its passengers are ticked.
        assert_eq!(mount(game, rejected), Some(root));
        assert_eq!([y(game, rejected), y(game, grandchild)], expected.seated);
        for id in [rejected, grandchild] {
            assert_eq!(age(game, id), Some(Tag::Int(-1000)));
        }

        let survivors: &[u32] =
            match dismount {
                None => &[],
                Some("grandchild") => {
                    assert!(game.run_command(
                        "execute as @e[tag=root_travel_grandchild] run ride @s dismount"
                    ));
                    assert_eq!(mount(game, grandchild), None);
                    assert_eq!([y(game, rejected), y(game, grandchild)], expected.seated);
                    for _ in 0..5 {
                        game.tick_world(true).unwrap();
                    }
                    assert_eq!(age(game, rejected), Some(Tag::Int(-1000)));
                    assert_eq!(age(game, grandchild), Some(Tag::Int(-995)));
                    &[grandchild]
                }
                Some(_) => {
                    assert!(game.run_command(
                        "execute as @e[tag=root_travel_rejected] run ride @s dismount"
                    ));
                    assert_eq!(mount(game, rejected), None);
                    assert_eq!(mount(game, grandchild), Some(rejected));
                    assert_eq!(
                        [y(game, rejected), y(game, grandchild)],
                        expected.rejected_dismounted
                    );
                    for _ in 0..5 {
                        game.tick_world(true).unwrap();
                    }
                    for id in [rejected, grandchild] {
                        assert_eq!(age(game, id), Some(Tag::Int(-995)));
                    }
                    &[rejected, grandchild]
                }
            };

        // The retained root is never saved; a passenger saves only under its root.
        reload_chunk(game);
        assert!(!game.run_command("data get entity @e[tag=root_travel_root,limit=1]"));
        assert!(game.persist.off_level.is_empty());
        let rejected_after = mob_tagged(game, "root_travel_rejected");
        let grandchild_after = mob_tagged(game, "root_travel_grandchild");
        assert_eq!(
            (rejected_after.len(), grandchild_after.len()),
            (
                usize::from(survivors.contains(&rejected)),
                usize::from(survivors.contains(&grandchild))
            )
        );
        if let ([rejected], [grandchild]) = (&rejected_after[..], &grandchild_after[..]) {
            assert_eq!(mount(game, *grandchild), Some(*rejected));
        }
        for id in rejected_after.into_iter().chain(grandchild_after) {
            assert_eq!(age(game, id), Some(Tag::Int(-995)));
        }
    });
}

#[test]
#[ignore = "daily"]
fn boat_root_refused_by_its_passenger_uuid_stays_off_level_through_travel() {
    for dismount in [None, Some("grandchild"), Some("rejected")] {
        vehicle_root_travel(&OAK_BOAT_TRAVEL, dismount);
    }
}

#[test]
#[ignore = "daily"]
fn minecart_root_refused_by_its_passenger_uuid_stays_off_level_through_travel() {
    for dismount in [None, Some("grandchild"), Some("rejected")] {
        vehicle_root_travel(&MINECART_TRAVEL, dismount);
    }
}
