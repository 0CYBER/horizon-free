use super::*;
use horizon_client::{
    chunk::Chunk,
    join_seq::JoinSeq,
    join_seq::residency::{
        Change, Journal, Kind, Owner,
        table::{EntityLookup, EntityTable, SourceEntity},
    },
};
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_mod_runtime::attachments::{AttachmentType, Holder};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const LEVEL_VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "mob-panic-test",
    name: "retained",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    First,
    Later,
    PendingDeath,
    UnloadedRow,
    Normal,
    NormalRemoval,
}

struct PanicOnDespawn {
    calls: Arc<AtomicUsize>,
    at: Option<usize>,
    force_allow: bool,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for PanicOnDespawn {
    fn mob_despawn(&self, _: &dyn mod_api::Host, event: &mut mod_api::MobDespawnEvent<'_>) {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.force_allow {
            *event.result = mod_api::Despawn::Allow;
        }
        if self.at == Some(call) {
            // An unwound answer must not become the next attempt's answer.
            *event.result = mod_api::Despawn::Allow;
            panic!("actual mob despawn listener panicked");
        }
        if self.at.is_some_and(|at| call < at) {
            *event.result = mod_api::Despawn::Deny;
        }
    }
}

/// Resolve the real carrier through the journal consumer, not a parallel test index.
fn assert_resident_mobs(
    game: &Game<'_>,
    table: &mut EntityTable,
    owner: &Owner,
    expected: &[(u32, JoinSeq)],
) {
    for &identity in expected {
        let found = table.entity_lookup(&owner.dimension, identity.0, |at| {
            let mob = &game.mobs[game.mobs.locate(at.join)?];
            Some(SourceEntity {
                id: mob.id,
                value: (mob.id, mob.join),
            })
        });
        let EntityLookup::Found { location, value } = found else {
            panic!("restored mob did not resolve: {identity:?}");
        };
        assert_eq!(location.owner, *owner);
        assert_eq!(value, identity);
    }
    table
        .end_tick()
        .expect("mob pass left an abandoned operation");
}

#[test]
#[ignore = "daily"]
fn mob_despawn_listener_panic_preserves_owned_rows_and_lent_random() {
    eprintln!("MOB_PANIC_ASSERTIONS_DISABLED:{}", !cfg!(debug_assertions));
    for case in [
        Case::First,
        Case::Later,
        Case::PendingDeath,
        Case::UnloadedRow,
        Case::Normal,
        Case::NormalRemoval,
    ] {
        for dimension in [Dim::Overworld, Dim::Nether] {
            eprintln!("MOB_PANIC_CASE:{case:?}:{dimension:?}");
            let at = match case {
                Case::First | Case::UnloadedRow => Some(1),
                Case::Later | Case::PendingDeath => Some(2),
                Case::Normal | Case::NormalRemoval => None,
            };
            let calls = Arc::new(AtomicUsize::new(0));
            let mods = vec![mod_api::ModEntry::new(
                "panic-on-mob-despawn",
                Box::new(PanicOnDespawn {
                    calls: calls.clone(),
                    at,
                    force_allow: case == Case::NormalRemoval,
                }),
            )]
            .into();
            let root = std::env::temp_dir().join(format!(
                "horizon-mob-panic-{}-{:?}-{dimension:?}-{case:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            mod_platform::testing::with_mods(mods, || {
                crate::game::native_entity_refs::tests::with_game(|game| {
                    game.world_root = root.clone();
                    game.enter_level(dimension).unwrap();
                    game.options.dedicated = Some(Vec::new());
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    assert!(
                        game.entity_ticking_area()(1, 0),
                        "fixture chunk is not entity-ticking"
                    );
                    let journal = Journal::default();
                    let owner = Owner {
                        dimension: game.dim.key(),
                        kind: Kind::Mob,
                    };
                    game.mobs
                        .bind_residency(&journal, owner, |mob| (mob.id, mob.join));
                    let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    for id in [71, 72, 73, 74] {
                        let unloaded = id == 71 && case == Case::UnloadedRow;
                        let kind = if unloaded {
                            game.res
                                .mob_types()
                                .get("minecraft:zombie")
                                .unwrap()
                                .clone()
                        } else {
                            pig.clone()
                        };
                        let x = if unloaded { 33.0 } else { f64::from(id - 70) };
                        let mut mob = Mob::new(id, kind, Vec3::new(x, 80.0, 0.5), 0.0);
                        mob.no_ai = true;
                        if id == 71 && case == Case::PendingDeath {
                            // Alive at the initial death sweep; the actual base
                            // tick's burning damage kills it before listener 2.
                            mob.health = 0.5;
                            mob.fire_ticks = 20;
                        }
                        if id == 74 {
                            // A previously notified corpse is a valid retained
                            // death marker, unlike an orphan ID in this set.
                            mob.health = 0.0;
                            game.died.insert(id);
                        }
                        game.mobs.push(mob);
                    }
                    let joined: Vec<_> = game.mobs.iter().map(|mob| (mob.id, mob.join)).collect();
                    let expected = if case == Case::NormalRemoval {
                        Vec::new()
                    } else {
                        joined.clone()
                    };
                    LEVEL_VALUE
                        .set(&mut game.world.attachments, Holder::Level, 42)
                        .unwrap();
                    game.world.attachment_syncs.insert(Holder::Level);
                    let expected_syncs = game.world.attachment_syncs.clone();
                    game.world.path_writes = Some(Vec::new());
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world.set(4, 80, 4, stone).unwrap();
                    let expected_writes = game.world.path_writes.clone();
                    game.world.random =
                        horizon_core::random::LevelRandom::for_level(LegacyRandom::new(0x5eed));
                    let expected_draw = game.world.random.clone().next_long();
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        game.tick_creature_entities()
                    }));
                    assert_eq!(
                        calls.load(Ordering::SeqCst),
                        at.unwrap_or(joined.len()),
                        "actual listener count or stop boundary differed"
                    );
                    if at.is_some() {
                        let payload = result.as_ref().expect_err("listener panic was swallowed");
                        assert_eq!(
                            payload.downcast_ref::<&str>().copied(),
                            Some("actual mob despawn listener panicked")
                        );
                        assert_eq!(
                            game.world.path_writes, expected_writes,
                            "borrowed collision writes were lost"
                        );
                        assert!(
                            !game.died.contains(&71),
                            "undelivered death was marked delivered"
                        );
                    } else {
                        result.expect("unexpected listener unwind").unwrap();
                        assert_eq!(
                            game.world.path_writes,
                            Some(Vec::new()),
                            "normal pass did not consume collision writes"
                        );
                    }
                    assert_eq!(
                        game.died.contains(&74),
                        case != Case::NormalRemoval,
                        "prior death notification was forgotten or a removed corpse retained"
                    );
                    assert_eq!(
                        LEVEL_VALUE
                            .get_existing(&mut game.world.attachments, Holder::Level)
                            .unwrap(),
                        Some(42)
                    );
                    assert_eq!(
                        game.world.attachment_syncs, expected_syncs,
                        "pending attachment syncs were lost"
                    );
                    if case != Case::PendingDeath {
                        assert_eq!(
                            game.world.random.clone().next_long(),
                            expected_draw,
                            "lent world random was lost"
                        );
                    }
                    if case == Case::PendingDeath {
                        assert!(!game.mobs[0].alive(), "real burning death was not reached");
                        assert!(
                            game.creatures.ticked.contains(&71),
                            "prior real tick was lost"
                        );
                    }
                    let mut events = Vec::new();
                    journal.drain_into(&mut events);
                    let (mut depth, mut lifts) = (0usize, 0usize);
                    for event in events {
                        assert_eq!(event.owner, owner);
                        match event.change {
                            Change::Lifted => {
                                depth += 1;
                                lifts += 1;
                            }
                            Change::Restored => {
                                depth = depth.checked_sub(1).expect("unmatched mob restore")
                            }
                            _ => {}
                        }
                        journal.push(event);
                    }
                    assert!(lifts > 0, "actual lifting boundary was not observed");
                    assert_eq!(depth, 0, "mob listener abandoned the real lifted pass");
                    assert_eq!(
                        game.mobs
                            .iter()
                            .map(|mob| (mob.id, mob.join))
                            .collect::<Vec<_>>(),
                        expected,
                        "mob listener lost or reordered original rows"
                    );
                    let mut table = EntityTable::new(journal);
                    assert_resident_mobs(game, &mut table, &owner, &expected);
                    if case == Case::NormalRemoval {
                        for &(id, _) in &joined {
                            let found = table.entity_lookup(&owner.dimension, id, |at| {
                                let mob = &game.mobs[game.mobs.locate(at.join)?];
                                Some(SourceEntity {
                                    id: mob.id,
                                    value: (mob.id, mob.join),
                                })
                            });
                            assert!(matches!(found, EntityLookup::Missing));
                        }
                    }
                    if case != Case::UnloadedRow {
                        game.tick_creature_entities().unwrap();
                        assert_eq!(
                            game.mobs
                                .iter()
                                .map(|mob| (mob.id, mob.join))
                                .collect::<Vec<_>>(),
                            expected,
                            "failed listener answer leaked into the next despawn check"
                        );
                        assert_resident_mobs(game, &mut table, &owner, &expected);
                    }
                });
            });
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
