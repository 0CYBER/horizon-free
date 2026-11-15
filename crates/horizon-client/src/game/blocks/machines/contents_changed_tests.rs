use super::{BlockEntity, Game};
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game_at};
use horizon_client::{chunk::Chunk, items::ItemStack};
use horizon_core::{geometry::Vec3, position::BlockPos, random::RandomSource};
use mod_api::*;
use std::sync::{Arc, Mutex};

const DEPOT: &str = "contents_changed:depot";

// ANCHOR: contents_changed_listener
#[derive(Clone, Debug, PartialEq, Eq)]
struct Notice {
    level: LevelRef,
    pos: [i32; 3],
    amount: u32,
    version: Option<i32>,
}

#[derive(Clone, Default)]
struct Changes(Arc<Mutex<Vec<Notice>>>);

#[mod_api::horizon_part]
impl BlockPart for Changes {
    fn contents_changed(&self, host: &mut dyn ServerHost, event: &mut ContentsChangedEvent) {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let amount = event
            .level
            .capability(host, &ITEMS, pos, None)
            .map_or(0, |items| {
                (0..items.size(host))
                    .map(|slot| items.amount(host, slot))
                    .sum()
            });
        let version = host
            .block_entity(event.level, pos)
            .and_then(|entity| entity.data(host))
            .and_then(|data| match data.get(&"Version".into()) {
                Some(Tag::Int(value)) => Some(*value),
                _ => None,
            });
        self.0.lock().unwrap().push(Notice {
            level: event.level,
            pos,
            amount,
            version,
        });
    }
}
// ANCHOR_END: contents_changed_listener

#[mod_api::horizon_part]
impl BlockEntityPart for Changes {
    fn block_entity(&self, _: LevelSide<'_>, _: &mut BlockEntityEvent<'_>) -> Option<Compound> {
        Some(Compound::from([("Version".into(), Tag::Int(0))]))
    }
}

#[derive(Clone, Copy)]
enum NoticeAction {
    Observe,
    Write,
    Panic,
    /// Records the transaction lifecycle the notice runs in.
    Lifecycle,
    /// Opens and closes a transaction of its own, then records the lifecycle.
    NestedLifecycle,
}

/// The lifecycle each notice ran in, for [`NoticeAction::Lifecycle`].
static LIFECYCLES: Mutex<Vec<TransactionLifecycle>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct Observed {
    changes: Changes,
    action: NoticeAction,
}

#[mod_api::horizon_part]
impl BlockPart for Observed {
    fn contents_changed(&self, host: &mut dyn ServerHost, event: &mut ContentsChangedEvent) {
        self.changes.contents_changed(host, event);
        match self.action {
            NoticeAction::Observe => return,
            NoticeAction::Lifecycle => {
                LIFECYCLES
                    .lock()
                    .unwrap()
                    .push(host.transaction_lifecycle());
                return;
            }
            NoticeAction::NestedLifecycle => {
                assert_eq!(host.transaction(|_, _| Ok::<_, ()>(())), Ok(()));
                LIFECYCLES
                    .lock()
                    .unwrap()
                    .push(host.transaction_lifecycle());
                return;
            }
            NoticeAction::Panic => panic!("contents notice panic probe"),
            NoticeAction::Write => {}
        }
        if event.pos != BlockPos::new(0, 64, 0) {
            return;
        }
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let source = event.level.capability(host, &ITEMS, pos, None).unwrap();
        let other = event
            .level
            .capability(host, &ITEMS, [2, 64, 0], None)
            .unwrap();
        let sticks = ItemResource::of(
            &mod_api::ItemStack::new(mod_api::__private::item_key("minecraft:stick").unwrap(), 1)
                .unwrap(),
        );
        assert_eq!(
            host.transaction(|host, tx| {
                assert_eq!(source.insert(host, tx, 0, &sticks, 2), 2);
                assert_eq!(other.insert(host, tx, 0, &sticks, 3), 3);
                Ok::<_, ()>(())
            }),
            Ok(())
        );
        assert!(
            host.block_entity(event.level, pos)
                .unwrap()
                .set_changed(host)
                .unwrap()
        );
    }
}

#[mod_api::__subscriptions]
impl Hooks for Observed {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        // An unbacked vehicle coordinate would select air if it were dispatched as a block.
        for block in [
            "minecraft:air",
            "minecraft:chest",
            "minecraft:crafter",
            "minecraft:chiseled_bookshelf",
        ] {
            let block_key = r.block_key(block);
            r.block_part(block_key, self.clone());
        }
        let kind = r.block_entity_type("depot").blocks(&[DEPOT]).build();
        r.block_entity_part(&kind, self.changes.clone()).unwrap();
    }

    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        event
            .blocks
            .push(mod_api::Block::new(DEPOT, "minecraft:white_banner"));
    }
}

fn with_changes(
    name: &str,
    action: NoticeAction,
    test: impl FnOnce(&mut Game<'_>, LevelRef, Changes),
) {
    if run_fresh_cases(name, "HORIZON_CONTENTS_CHANGED_CHILD", &["game"]) {
        return;
    }
    let changes = Changes::default();
    let mods: Mods = vec![ModEntry::new(
        "contents_changed",
        Box::new(Observed {
            changes: changes.clone(),
            action,
        }),
    )]
    .into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let root = crate::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    with_game_at(root, |game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let level = game.level("minecraft:overworld").unwrap();
        test(game, level, changes);
    });
}

fn place(game: &mut Game<'_>, level: LevelRef, pos: [i32; 3], block: &str) {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    assert_eq!(
        game.level_set_block(level, pos, state, BlockUpdateFlags::NONE),
        Ok(true)
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn explicit_notices_observe_tentative_writes_and_survive_abort() {
    with_changes(
        "game::machines::contents_changed_tests::explicit_notices_observe_tentative_writes_and_survive_abort",
        NoticeAction::Observe,
        |game, level, changes| {
            let pos = [0, 64, 0];
            place(game, level, pos, "minecraft:chest");
            let items = level.capability(game, &ITEMS, pos, None).unwrap();
            let entity = game.block_entity(level, pos).unwrap();
            let sticks = ItemResource::of(
                &mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:stick").unwrap(),
                    1,
                )
                .unwrap(),
            );
            changes.0.lock().unwrap().clear();
            assert_eq!(
                game.transaction(|host, tx| {
                    assert_eq!(items.insert(host, tx, 0, &sticks, 2), 2);
                    assert!(
                        host.transaction(|host, inner| {
                            assert_eq!(items.insert(host, inner, 0, &sticks, 7), 7);
                            Err::<(), ()>(())
                        })
                        .is_err()
                    );
                    assert_eq!(items.amount(host, 0), 2);
                    host.transaction(|host, inner| {
                        assert_eq!(items.insert(host, inner, 0, &sticks, 3), 3);
                        Ok::<_, ()>(())
                    })?;
                    assert!(changes.0.lock().unwrap().is_empty());
                    Ok::<_, ()>(())
                }),
                Ok(())
            );
            let notice = |amount| Notice {
                level,
                pos,
                amount,
                version: None,
            };
            assert_eq!(*changes.0.lock().unwrap(), [notice(5)]);
            changes.0.lock().unwrap().clear();
            game.world.block_log = Some(Vec::new());
            assert!(
                game.transaction(|host, tx| {
                    assert_eq!(items.insert(host, tx, 0, &sticks, 4), 4);
                    for _ in 0..2 {
                        assert!(entity.set_changed(host).unwrap());
                    }
                    assert_eq!(*changes.0.lock().unwrap(), [notice(9), notice(9)]);
                    assert!(level.send_block_updated(host, pos));
                    Err::<(), ()>(())
                })
                .is_err()
            );
            assert_eq!(items.amount(game, 0), 5);
            assert_eq!(*changes.0.lock().unwrap(), [notice(9), notice(9)]);
            assert!(game.world.block_log.as_ref().unwrap().contains(&(0, 64, 0)));
            assert!(entity.set_changed(game).unwrap());
            assert_eq!(
                *changes.0.lock().unwrap(),
                [notice(9), notice(9), notice(5)]
            );
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_self_write_does_not_reenter_but_another_backing_block_notifies() {
    with_changes(
        "game::machines::contents_changed_tests::a_self_write_does_not_reenter_but_another_backing_block_notifies",
        NoticeAction::Write,
        |game, level, changes| {
            let source = [0, 64, 0];
            let other = [2, 64, 0];
            for pos in [source, other] {
                place(game, level, pos, "minecraft:chest");
            }
            changes.0.lock().unwrap().clear();
            let entity = game.block_entity(level, source).unwrap();
            // No outer transaction: set_changed dispatches immediately. The listener writes
            // into itself and another block, and explicitly invalidates itself on every call.
            assert!(entity.set_changed(game).unwrap());
            assert_eq!(
                *changes.0.lock().unwrap(),
                [
                    Notice {
                        level,
                        pos: source,
                        amount: 0,
                        version: None
                    },
                    Notice {
                        level,
                        pos: other,
                        amount: 3,
                        version: None
                    },
                ]
            );
            assert_eq!(
                level
                    .capability(game, &ITEMS, source, None)
                    .unwrap()
                    .amount(game, 0),
                2
            );
            // The first dispatch is finished: the same block must notify again, not stay guarded.
            assert!(entity.set_changed(game).unwrap());
            assert_eq!(
                *changes.0.lock().unwrap(),
                [
                    Notice {
                        level,
                        pos: source,
                        amount: 0,
                        version: None
                    },
                    Notice {
                        level,
                        pos: other,
                        amount: 3,
                        version: None
                    },
                    Notice {
                        level,
                        pos: source,
                        amount: 2,
                        version: None
                    },
                    Notice {
                        level,
                        pos: other,
                        amount: 6,
                        version: None
                    },
                ]
            );
            assert_eq!(
                level
                    .capability(game, &ITEMS, source, None)
                    .unwrap()
                    .amount(game, 0),
                4
            );
            assert_eq!(
                level
                    .capability(game, &ITEMS, other, None)
                    .unwrap()
                    .amount(game, 0),
                6
            );
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_panicking_part_does_not_leave_its_contents_notice_guarded() {
    with_changes(
        "game::machines::contents_changed_tests::a_panicking_part_does_not_leave_its_contents_notice_guarded",
        NoticeAction::Panic,
        |game, level, changes| {
            let pos = [0, 64, 0];
            place(game, level, pos, "minecraft:chest");
            changes.0.lock().unwrap().clear();
            let entity = game.block_entity(level, pos).unwrap();
            for _ in 0..2 {
                assert!(
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        entity.set_changed(game).unwrap()
                    }))
                    .is_err()
                );
            }
            // Both calls reach the Part: the first unwind did not leave a stale active position.
            assert_eq!(changes.0.lock().unwrap().len(), 2);
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_double_container_notifies_each_backing_block_once() {
    with_changes(
        "game::machines::contents_changed_tests::a_double_container_notifies_each_backing_block_once",
        NoticeAction::Observe,
        |game, level, changes| {
            let base = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            for (pos, kind) in [([0, 64, 0], "left"), ([1, 64, 0], "right")] {
                let state = game
                    .with_props(base, &[("facing", "north"), ("type", kind)])
                    .unwrap();
                assert_eq!(
                    game.level_set_block(level, pos, state, BlockUpdateFlags::NONE),
                    Ok(true)
                );
            }
            let items = level.capability(game, &ITEMS, [0, 64, 0], None).unwrap();
            assert_eq!(items.size(game), 54);
            let sticks = ItemResource::of(
                &mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:stick").unwrap(),
                    1,
                )
                .unwrap(),
            );
            changes.0.lock().unwrap().clear();
            assert_eq!(
                game.transaction(|host, tx| {
                    assert_eq!(items.insert(host, tx, 0, &sticks, 3), 3);
                    assert_eq!(items.insert(host, tx, 27, &sticks, 2), 2);
                    Ok::<_, ()>(())
                }),
                Ok(())
            );
            let mut heard = changes.0.lock().unwrap().clone();
            heard.sort_by_key(|notice| notice.pos);
            assert_eq!(
                heard,
                [[0, 64, 0], [1, 64, 0]].map(|pos| Notice {
                    level,
                    pos,
                    amount: 5,
                    version: None,
                })
            );
            changes.0.lock().unwrap().clear();
            assert!(
                game.block_entity(level, [0, 64, 0])
                    .unwrap()
                    .set_changed(game)
                    .unwrap()
            );
            assert_eq!(
                *changes.0.lock().unwrap(),
                [Notice {
                    level,
                    pos: [0, 64, 0],
                    amount: 5,
                    version: None,
                }]
            );
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_native_types_payload_notifications_observe_each_write_before_commit() {
    with_changes(
        "game::machines::contents_changed_tests::a_native_types_payload_notifications_observe_each_write_before_commit",
        NoticeAction::Observe,
        |game, level, changes| {
            let pos = [0, 64, 0];
            place(game, level, pos, DEPOT);
            let entity = game.block_entity(level, pos).unwrap();
            changes.0.lock().unwrap().clear();
            assert_eq!(
                game.transaction(|host, _| {
                    for version in [11, 12] {
                        assert!(
                            entity
                                .merge_data(
                                    host,
                                    Compound::from([("Version".into(), Tag::Int(version))])
                                )
                                .unwrap()
                        );
                    }
                    assert_eq!(changes.0.lock().unwrap().len(), 2);
                    Ok::<_, ()>(())
                }),
                Ok(())
            );
            assert_eq!(
                *changes.0.lock().unwrap(),
                [11, 12].map(|version| Notice {
                    level,
                    pos,
                    amount: 0,
                    version: Some(version),
                })
            );
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_crafter_notices_its_source_but_not_a_vehicles_placeholder() {
    with_changes(
        "game::machines::contents_changed_tests::a_crafter_notices_its_source_but_not_a_vehicles_placeholder",
        NoticeAction::Observe,
        |game, level, changes| {
            use crate::game::minecarts::{CartKind, Minecart};
            let pos = [0, 66, 0];
            let base = game
                .pack
                .assets
                .registry
                .default_state("minecraft:crafter")
                .unwrap();
            let state = game
                .with_props(base, &[("orientation", "down_east")])
                .unwrap();
            assert_eq!(
                game.level_set_block(level, pos, state, BlockUpdateFlags::NONE),
                Ok(true)
            );
            let Some(BlockEntity::Crafter(crafter)) =
                game.world.block_entities.get_mut(&(0, 66, 0))
            else {
                panic!("the placed crafter is missing");
            };
            crafter.items[0] = Some(ItemStack::new("minecraft:oak_planks", 1));
            crafter.items[3] = Some(ItemStack::new("minecraft:oak_planks", 1));
            game.carts.push(Minecart::new(
                700,
                CartKind::Chest,
                Vec3::new(0.5, 65.0, 0.5),
            ));
            changes.0.lock().unwrap().clear();
            let dropped = game.dropped.len();
            let mut random = game.world.random.clone();
            game.dispense_from(BlockPos::new(0, 66, 0), state).unwrap();
            assert_eq!(
                game.carts[0].items[0],
                Some(ItemStack::new("minecraft:stick", 4))
            );
            assert_eq!(game.dropped.len(), dropped);
            assert_eq!(
                *changes.0.lock().unwrap(),
                [Notice {
                    level,
                    pos,
                    amount: 0,
                    version: None
                }]
            );
            // The existing shared lookup chooses its one candidate once. Notification never
            // resolves the destination again, and a menu's borrowed slots are not a block.
            random.next_int_bound(1).unwrap();
            assert_eq!(game.world.random.clone().next_long(), random.next_long());
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn notices_come_in_first_write_order_once_each_whatever_the_journal_keeps() {
    with_changes(
        "game::machines::contents_changed_tests::notices_come_in_first_write_order_once_each_whatever_the_journal_keeps",
        NoticeAction::Observe,
        |game, level, changes| {
            let (a, b) = ([0, 64, 0], [2, 64, 0]);
            for pos in [a, b] {
                place(game, level, pos, "minecraft:chest");
            }
            let first = level.capability(game, &ITEMS, a, None).unwrap();
            let second = level.capability(game, &ITEMS, b, None).unwrap();
            let sticks = ItemResource::of(
                &mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:stick").unwrap(),
                    1,
                )
                .unwrap(),
            );
            changes.0.lock().unwrap().clear();
            let committed = game.transaction(|host, tx| {
                assert_eq!(first.insert(host, tx, 0, &sticks, 2), 2);
                host.transaction(|host, inner| {
                    assert_eq!(second.insert(host, inner, 3, &sticks, 5), 5);
                    Ok::<_, ()>(())
                })?;
                assert_eq!(first.insert(host, tx, 1, &sticks, 1), 1);
                assert!(
                    changes.0.lock().unwrap().is_empty(),
                    "nothing told before the commit"
                );
                Ok::<_, ()>(())
            });
            assert_eq!(committed, Ok(()));
            let told: Vec<([i32; 3], u32)> = changes
                .0
                .lock()
                .unwrap()
                .iter()
                .map(|notice| (notice.pos, notice.amount))
                .collect();
            assert_eq!(told, [(a, 3), (b, 5)]);
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_mod_reads_the_open_transaction_its_depth_and_the_root_commits_phase() {
    with_changes(
        "game::machines::contents_changed_tests::a_mod_reads_the_open_transaction_its_depth_and_the_root_commits_phase",
        NoticeAction::Lifecycle,
        |game, level, _| {
            let pos = [0, 64, 0];
            place(game, level, pos, "minecraft:chest");
            let items = level.capability(game, &ITEMS, pos, None).unwrap();
            let entity = game.block_entity(level, pos).unwrap();
            let sticks = ItemResource::of(
                &mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:stick").unwrap(),
                    1,
                )
                .unwrap(),
            );
            let host: &mut dyn ServerHost = game;
            assert_eq!(host.transaction_lifecycle(), TransactionLifecycle::None);
            assert_eq!(host.transaction_current(), None);
            // A helper given no transaction writes in the open one, which the abort undoes.
            let aborted = host.transaction(|host, outer| {
                assert_eq!(host.transaction_lifecycle(), TransactionLifecycle::Open);
                assert_eq!(host.transaction_depth(outer), Some(0));
                host.transaction(|host, inner| {
                    assert_eq!(host.transaction_depth(inner), Some(1));
                    assert_eq!(host.transaction_current().as_ref(), Some(inner));
                    let current = host.transaction_current().unwrap();
                    assert_eq!(items.insert(host, &current, 0, &sticks, 4), 4);
                    Ok::<_, ()>(())
                })?;
                assert_eq!(items.amount(host, 0), 4);
                Err::<(), ()>(())
            });
            assert!(aborted.is_err());
            assert_eq!(items.amount(host, 0), 0);
            // The closed transaction is no longer open.
            let closed =
                host.transaction(|_, tx| Ok::<_, ()>(mod_api::__private::transaction_token(tx)));
            let closed = mod_api::__private::transaction_of(closed.unwrap());
            assert_eq!(host.transaction_depth(&closed), None);
            // The notice of a committed write runs while the root closes.
            LIFECYCLES.lock().unwrap().clear();
            let committed = host.transaction(|host, tx| {
                assert_eq!(items.insert(host, tx, 0, &sticks, 2), 2);
                assert!(entity.set_changed(host).unwrap());
                Ok::<_, ()>(())
            });
            assert_eq!(committed, Ok(()));
            assert_eq!(
                *LIFECYCLES.lock().unwrap(),
                [
                    TransactionLifecycle::Open,
                    TransactionLifecycle::RootClosing
                ]
            );
            assert_eq!(host.transaction_lifecycle(), TransactionLifecycle::None);
        },
    );
}

/// A notice that opens and closes a transaction of its own leaves the root's closing phase as it
/// was, and the phase ends with the root.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_notice_that_opens_a_transaction_keeps_the_root_closing_phase() {
    with_changes(
        "game::machines::contents_changed_tests::a_notice_that_opens_a_transaction_keeps_the_root_closing_phase",
        NoticeAction::NestedLifecycle,
        |game, level, _| {
            let pos = [0, 64, 0];
            place(game, level, pos, "minecraft:chest");
            let items = level.capability(game, &ITEMS, pos, None).unwrap();
            let sticks = ItemResource::of(
                &mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:stick").unwrap(),
                    1,
                )
                .unwrap(),
            );
            LIFECYCLES.lock().unwrap().clear();
            let host: &mut dyn ServerHost = game;
            let committed = host.transaction(|host, tx| {
                assert_eq!(items.insert(host, tx, 0, &sticks, 1), 1);
                Ok::<_, ()>(())
            });
            assert_eq!(committed, Ok(()));
            assert_eq!(
                *LIFECYCLES.lock().unwrap(),
                [TransactionLifecycle::RootClosing]
            );
            assert_eq!(host.transaction_lifecycle(), TransactionLifecycle::None);
        },
    );
}

/// A notice that panics while the root commits leaves no closing phase behind: after the caught
/// panic nothing is open, and the next transaction reads Open.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_panicking_root_commit_notice_leaves_the_phase_none() {
    with_changes(
        "game::machines::contents_changed_tests::a_panicking_root_commit_notice_leaves_the_phase_none",
        NoticeAction::Panic,
        |game, level, _| {
            let pos = [0, 64, 0];
            place(game, level, pos, "minecraft:chest");
            let items = level.capability(game, &ITEMS, pos, None).unwrap();
            let sticks = ItemResource::of(
                &mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:stick").unwrap(),
                    1,
                )
                .unwrap(),
            );
            let host: &mut dyn ServerHost = game;
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                host.transaction(|host, tx| {
                    assert_eq!(items.insert(host, tx, 0, &sticks, 1), 1);
                    Ok::<_, ()>(())
                })
            }));
            assert!(caught.is_err());
            assert_eq!(host.transaction_lifecycle(), TransactionLifecycle::None);
            let next = host.transaction(|host, _| {
                assert_eq!(host.transaction_lifecycle(), TransactionLifecycle::Open);
                Ok::<_, ()>(())
            });
            assert_eq!(next, Ok(()));
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn bookshelf_abort_restores_items_flags_and_last_slot_but_keeps_issued_effects() {
    with_changes(
        "game::machines::contents_changed_tests::bookshelf_abort_restores_items_flags_and_last_slot_but_keeps_issued_effects",
        NoticeAction::Observe,
        |game, level, changes| {
            use horizon_core::{position::ChunkPos, ticks::TickTypeId};
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            let book = ItemResource::of(
                &mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:book").unwrap(),
                    1,
                )
                .unwrap(),
            );
            for (index, operation) in ["insert", "extract", "merge", "replace"]
                .into_iter()
                .enumerate()
            {
                let pos = [index as i32 * 2, 64, 0];
                let key = (pos[0], pos[1], pos[2]);
                place(game, level, pos, "minecraft:chiseled_bookshelf");
                let extracting = operation == "extract";
                let Some(BlockEntity::ChiseledBookshelf(shelf)) =
                    game.world.block_entities.get_mut(&key)
                else {
                    panic!("missing bookshelf");
                };
                shelf.last_slot = 4;
                shelf.items[2] = extracting.then(|| ItemStack::new("minecraft:book", 1));
                let initial = game.world.block(pos[0], pos[1], pos[2]);
                let initial = game
                    .with_props(
                        initial,
                        &[("slot_2_occupied", if extracting { "true" } else { "false" })],
                    )
                    .unwrap();
                game.world.set(pos[0], pos[1], pos[2], initial).unwrap();
                let comparator = BlockPos::new(pos[0] + 1, pos[1], pos[2]);
                place(
                    game,
                    level,
                    [comparator.x, comparator.y - 1, comparator.z],
                    "minecraft:stone",
                );
                place(
                    game,
                    level,
                    [comparator.x, comparator.y, comparator.z],
                    "minecraft:comparator",
                );
                let state = game.world.block(comparator.x, comparator.y, comparator.z);
                let state = game.with_props(state, &[("facing", "west")]).unwrap();
                game.world
                    .set(comparator.x, comparator.y, comparator.z, state)
                    .unwrap();
                game.with_level(|b, level, ticks| b.scheduled_block_tick(level, ticks, comparator));
                assert_eq!(game.block_ticks.comparator_outputs[&comparator], 5);
                let kind = TickTypeId(u32::from(game.res.behaviours().block(state).0));
                assert!(!game.block_ticks.blocks.has_scheduled(comparator, kind));
                let entity = game.block_entity(level, pos).unwrap();
                let items = level.capability(game, &ITEMS, pos, None).unwrap();
                let expected = Notice {
                    level,
                    pos,
                    amount: u32::from(!extracting),
                    version: None,
                };
                changes.0.lock().unwrap().clear();
                game.world.modified.clear();
                game.world.block_log = Some(Vec::new());
                let aborted: Result<(), ()> = game.transaction(|host, tx| {
                    match operation {
                        "insert" => {
                            assert_eq!(items.insert(host, tx, 2, &book, 1), 1);
                            assert!(entity.set_changed(host).unwrap());
                        }
                        "extract" => {
                            assert_eq!(items.extract(host, tx, 2, &book, 1), 1);
                            assert!(entity.set_changed(host).unwrap());
                        }
                        _ => {
                            let mut stack = horizon_client::save::item_to_nbt(&ItemStack::new(
                                "minecraft:book",
                                1,
                            ));
                            let Tag::Compound(fields) = &mut stack else {
                                panic!("item is not a compound")
                            };
                            fields.insert("Slot".into(), Tag::Byte(2));
                            let data = Compound::from([("Items".into(), Tag::List(vec![stack]))]);
                            if operation == "merge" {
                                assert!(entity.merge_data(host, data).unwrap());
                            } else {
                                assert!(entity.replace_data(host, data).unwrap());
                            }
                        }
                    }
                    let Some(BlockEntity::ChiseledBookshelf(shelf)) =
                        host.world.block_entities.get(&key)
                    else {
                        panic!("missing tentative bookshelf");
                    };
                    assert_eq!(shelf.last_slot, 2, "{operation}");
                    assert_eq!(shelf.items[2].is_some(), !extracting, "{operation}");
                    let info = host
                        .pack
                        .assets
                        .registry
                        .state(host.world.block(pos[0], pos[1], pos[2]))
                        .unwrap();
                    assert_eq!(
                        info.properties["slot_2_occupied"],
                        if extracting { "false" } else { "true" }
                    );
                    assert_eq!(*changes.0.lock().unwrap(), [expected.clone()]);
                    assert!(level.send_block_updated(host, pos));
                    Err(())
                });
                assert!(aborted.is_err());
                let Some(BlockEntity::ChiseledBookshelf(shelf)) =
                    game.world.block_entities.get(&key)
                else {
                    panic!("missing restored bookshelf");
                };
                assert_eq!(shelf.last_slot, 4, "{operation}");
                assert_eq!(shelf.items[2].is_some(), extracting, "{operation}");
                assert_eq!(
                    game.world.block(pos[0], pos[1], pos[2]),
                    initial,
                    "{operation}"
                );
                assert_eq!(
                    *changes.0.lock().unwrap(),
                    [expected],
                    "no synthetic restore notice"
                );
                assert!(
                    game.world.modified.contains(&(0, 0)),
                    "issued dirty mark survives"
                );
                assert!(
                    game.world.block_log.as_ref().unwrap().contains(&key),
                    "issued data-send survives"
                );
                assert!(
                    game.block_ticks.blocks.has_scheduled(comparator, kind),
                    "issued comparator work survives"
                );
            }
        },
    );
}
