use super::api_values::{StackReadError, TryIntoApi};
use super::native_live_holder_tests::{mod_holder, with_mod_parts};
use horizon_client::containers::{BlockEntity, ModBlockEntity, ModContainer};
use horizon_client::items::ItemStack;
use horizon_core::nbt::Tag;
use mod_api::{
    BlockEntityItemError as Error, Host, HostTransaction, ITEMS, ItemResource, ServerHost,
    Transaction,
};
use std::sync::Arc;

fn commit(host: &mut dyn ServerHost, test: impl FnOnce(&mut dyn ServerHost, &Transaction)) {
    host.transaction(|host, tx| {
        test(host, tx);
        Ok::<_, ()>(())
    })
    .unwrap();
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn internal_slots_preserve_automation_limits_and_nested_transactions() {
    const NAME: &str = "game::native_block_entity_item_tests::internal_slots_preserve_automation_limits_and_nested_transactions";
    with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let entity = mod_holder(game);
            let get = |host: &dyn Host, slot| entity.item_slot(host, slot);
            let set = |host: &mut dyn ServerHost, tx: &Transaction, slot, stack| {
                entity.set_item_slot(host, tx, slot, stack)
            };
            let pos = (1, 64, 1);
            let stone = game.world.block(1, 64, 1);
            let granite = game
                .pack
                .assets
                .registry
                .default_state("minecraft:granite")
                .unwrap();
            let dirt = ItemStack::new("minecraft:dirt", 2).try_into_api().unwrap();
            let diamond = ItemStack::new("minecraft:diamond", 1)
                .try_into_api()
                .unwrap();
            assert_eq!(get(game, 0), Err(Error::Unavailable));
            commit(game, |host, tx| {
                assert_eq!(
                    set(host, tx, 0, Some(dirt.clone())),
                    Err(Error::Unavailable)
                );
            });
            let machine_slots = ModContainer {
                slots: 2,
                inputs: vec![0],
                outputs: vec![1],
                ..Default::default()
            };
            let replacement_slots = ModContainer {
                slots: 1,
                ..Default::default()
            };
            game.world.block_entities.set_mod_containers(
                [
                    (stone.0, Arc::new(machine_slots)),
                    (granite.0, Arc::new(replacement_slots)),
                ]
                .into_iter()
                .collect(),
            );
            assert_eq!(get(game, 0), Ok(None));
            assert_eq!(
                get(game, 2),
                Err(Error::SlotOutOfBounds { slot: 2, slots: 2 })
            );
            commit(game, |host, tx| {
                assert_eq!(set(host, tx, 1, None), Ok(false));
                assert_eq!(set(host, tx, 0, Some(dirt.clone())), Ok(true));
                let level = entity.level(host).unwrap();
                let external = level.capability(host, &ITEMS, [1, 64, 1], None).unwrap();
                assert_eq!(
                    external.insert(host, tx, 1, &ItemResource::of(&diamond), 1),
                    0
                );
            });
            let BlockEntity::Mod(holder) = game.world.block_entities.get_mut(&pos).unwrap() else {
                panic!()
            };
            holder
                .items
                .push(Some(ItemStack::new("minecraft:gold_ingot", 3)));
            game.world.modified.clear();
            let aborted: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, tx| {
                assert_eq!(
                    set(
                        host,
                        tx,
                        0,
                        Some(ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap())
                    ),
                    Ok(true)
                );
                host.transaction(|host, child| {
                    assert_eq!(set(host, child, 1, Some(diamond.clone())), Ok(true));
                    Ok(())
                })?;
                assert_eq!(get(host, 1), Ok(Some(diamond.clone())));
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert_eq!(get(game, 0), Ok(Some(dirt.clone())));
            assert_eq!(get(game, 1), Ok(None));
            commit(game, |host, tx| {
                assert_eq!(
                    set(
                        host,
                        tx,
                        0,
                        Some(ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap())
                    ),
                    Ok(true)
                );
                assert_eq!(set(host, tx, 1, Some(diamond.clone())), Ok(true));
            });
            assert_eq!(
                get(game, 0),
                Ok(Some(
                    ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap()
                ))
            );
            assert_eq!(get(game, 1), Ok(Some(diamond.clone())));
            assert!(
                game.world.modified.is_empty(),
                "raw mod slot writes are silent"
            );
            assert!(entity.set_changed(game).unwrap());
            assert!(
                !game.world.modified.is_empty(),
                "explicit notification marks the chunk"
            );
            let BlockEntity::Mod(holder) = game.world.block_entities.get(&pos).unwrap() else {
                panic!()
            };
            assert_eq!(
                holder.items[2],
                Some(ItemStack::new("minecraft:gold_ingot", 3)),
                "saved excess is not truncated"
            );
            let before_revision = holder.revision();
            assert_eq!(
                ItemStack::new("missing:item", 1).try_into_api(),
                Err(StackReadError::UnknownItem)
            );
            assert_eq!(
                ItemStack::new("minecraft:dirt", 0).try_into_api(),
                Err(StackReadError::Empty)
            );
            game.world.modified.clear();
            let mut component = ItemStack::new("minecraft:diamond", 5);
            component
                .data_mut()
                .set_other("minecraft:max_stack_size", Some(Tag::Int(4)));
            commit(game, |host, tx| {
                assert_eq!(set(host, tx, 1, Some(diamond.clone())), Ok(false));
                assert_eq!(
                    set(host, tx, 2, None),
                    Err(Error::SlotOutOfBounds { slot: 2, slots: 2 })
                );
                for (stack, error) in [
                    (
                        ItemStack::new("minecraft:diamond", 65),
                        Error::StackRejected(mod_api::StackWriteError::CountExceeded {
                            maximum: 64,
                        }),
                    ),
                    (
                        ItemStack::new("minecraft:diamond_sword", 2),
                        Error::StackRejected(mod_api::StackWriteError::CountExceeded {
                            maximum: 1,
                        }),
                    ),
                    (
                        component.clone(),
                        Error::StackRejected(mod_api::StackWriteError::CountExceeded {
                            maximum: 4,
                        }),
                    ),
                    (ItemStack::new("minecraft:air", 1), Error::InvalidStack),
                ] {
                    let stack = stack.try_into_api().unwrap();
                    assert_eq!(set(host, tx, 1, Some(stack)), Err(error));
                }
            });
            let BlockEntity::Mod(holder) = game.world.block_entities.get(&pos).unwrap() else {
                panic!()
            };
            assert_eq!(holder.revision(), before_revision);
            assert_eq!(get(game, 1), Ok(Some(diamond)));
            assert!(game.world.modified.is_empty());
            component.count = 4;
            commit(game, |host, tx| {
                assert_eq!(
                    set(host, tx, 1, Some(component.try_into_api().unwrap())),
                    Ok(true)
                );
                assert_eq!(get(host, 1), Ok(Some(component.try_into_api().unwrap())));
                assert_eq!(set(host, tx, 1, None), Ok(true));
            });
            assert_eq!(get(game, 1), Ok(None));
            let level = entity.level(game).unwrap();
            game.world.block_entities.remove(&pos);
            assert_eq!(get(game, 0), Err(Error::Stale));
            commit(game, |host, tx| {
                assert_eq!(set(host, tx, 0, None), Err(Error::Stale));
            });
            game.world
                .block_entities
                .insert(pos, BlockEntity::Chest(vec![None; 27]));
            commit(game, |host, tx| {
                assert_eq!(get(host, 0), Err(Error::Stale));
                assert_eq!(set(host, tx, 0, None), Err(Error::Stale));
            });
            game.world.set(1, 64, 1, granite).unwrap();
            let replacement = ModBlockEntity::new("live_bench:ticker".into(), Default::default());
            game.world
                .block_entities
                .insert(pos, BlockEntity::Mod(Box::new(replacement)));
            assert_eq!(get(game, 0), Err(Error::Stale));
            let replacement = game.block_entity(level, [1, 64, 1]).unwrap();
            assert_ne!(entity, replacement);
            let get = |host: &dyn Host, slot| replacement.item_slot(host, slot);
            let set = |host: &mut dyn ServerHost, tx: &Transaction, slot, stack| {
                replacement.set_item_slot(host, tx, slot, stack)
            };
            commit(game, |host, tx| {
                assert_eq!(
                    set(host, tx, 1, None),
                    Err(Error::SlotOutOfBounds { slot: 1, slots: 1 })
                );
                assert_eq!(set(host, tx, 0, Some(dirt.clone())), Ok(true));
            });
            assert_eq!(get(game, 0), Ok(Some(dirt)));
            game.native_lifecycle
                .begin_open(mod_api::Authority::IntegratedServer);
            assert_eq!(get(game, 0), Err(Error::Stale));
            assert!(game.transactions.is_empty());
        });
    });
}

/// A mod holder with nine container slots and `keys` entries of saved data beside them.
fn container_holder(game: &mut super::Game<'_>, keys: usize) -> mod_api::BlockEntityRef {
    let entity = mod_holder(game);
    let stone = game.world.block(1, 64, 1);
    game.world.block_entities.set_mod_containers(
        [(
            stone.0,
            Arc::new(ModContainer {
                slots: 9,
                inputs: (0..9).collect(),
                outputs: (0..9).collect(),
                ..Default::default()
            }),
        )]
        .into_iter()
        .collect(),
    );
    let BlockEntity::Mod(holder) = game.world.block_entities.get_mut(&(1, 64, 1)).unwrap() else {
        panic!()
    };
    holder.items = vec![None; 9];
    for key in 0..keys {
        holder
            .nbt
            .insert(format!("k{key}").as_str().into(), Tag::Long(key as i64));
    }
    entity
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn container_moves_roll_back_slots_and_saved_data_in_any_order() {
    const NAME: &str = "game::native_block_entity_item_tests::container_moves_roll_back_slots_and_saved_data_in_any_order";
    with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let entity = container_holder(game, 3);
            let level = entity.level(game).unwrap();
            let items = level.capability(game, &ITEMS, [1, 64, 1], None).unwrap();
            let dirt =
                ItemResource::of(&ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap());
            let saved = |game: &super::Game<'_>| {
                let BlockEntity::Mod(holder) = game.world.block_entities.get(&(1, 64, 1)).unwrap()
                else {
                    panic!()
                };
                (
                    holder.items.clone(),
                    holder.nbt.clone(),
                    holder.data.clone(),
                )
            };
            commit(game, |host, tx| {
                assert_eq!(items.insert(host, tx, 0, &dirt, 10), 10)
            });
            let before = saved(game);
            // Slot writes, then a whole-entity write, then slots again, through nested
            // transactions that commit into one that aborts.
            let aborted: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, tx| {
                assert_eq!(items.extract(host, tx, 0, &dirt, 4), 4);
                assert_eq!(items.insert(host, tx, 3, &dirt, 7), 7);
                host.transaction(|host, child| {
                    assert_eq!(items.insert(host, child, 0, &dirt, 50), 50);
                    assert_eq!(entity.set_data_slots(host, vec![7]), Ok(true));
                    assert_eq!(items.extract(host, child, 3, &dirt, 2), 2);
                    Ok::<_, ()>(())
                })?;
                host.transaction(|host, child| {
                    assert_eq!(items.insert(host, child, 8, &dirt, 1), 1);
                    Err::<(), ()>(())
                })
                .unwrap_err();
                assert_eq!(items.amount(host, 8), 0, "the child's abort undid its slot");
                assert_eq!(items.amount(host, 0), 56);
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert_eq!(
                saved(game),
                before,
                "slots, saved data and data slots back as they were"
            );
            // The same writes committed stay.
            commit(game, |host, tx| {
                assert_eq!(items.extract(host, tx, 0, &dirt, 4), 4);
                assert_eq!(entity.set_data_slots(host, vec![9]), Ok(true));
                assert_eq!(items.insert(host, tx, 5, &dirt, 3), 3);
            });
            let (slots, nbt, data) = saved(game);
            assert_eq!(slots[0].as_ref().map(|s| s.count), Some(6));
            assert_eq!(slots[5].as_ref().map(|s| s.count), Some(3));
            assert_eq!(data, [9]);
            assert_eq!(nbt.len(), 3, "the saved data beside the slots is untouched");
            assert!(game.transactions.is_empty());
        });
    });
}

fn dirt() -> ItemResource {
    ItemResource::of(&ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap())
}

/// A mod holder's slots, saved data and data slots, compared whole.
fn holder_state(
    game: &super::Game<'_>,
) -> (
    Vec<Option<ItemStack>>,
    horizon_core::nbt::Compound,
    Vec<i32>,
) {
    let BlockEntity::Mod(holder) = game.world.block_entities.get(&(1, 64, 1)).unwrap() else {
        panic!()
    };
    (
        holder.items.clone(),
        holder.nbt.clone(),
        holder.data.clone(),
    )
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn a_container_move_keeps_one_slot_not_the_entity() {
    const NAME: &str =
        "game::native_block_entity_item_tests::a_container_move_keeps_one_slot_not_the_entity";
    with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let entity = container_holder(game, 100_000);
            let level = entity.level(game).unwrap();
            let items = level.capability(game, &ITEMS, [1, 64, 1], None).unwrap();
            let before = holder_state(game);
            // Moves as `move_once` makes them: each in its own transaction, nested in a root.
            let root = game.transaction_open();
            for _ in 0..50 {
                for put in [true, false] {
                    (game as &mut dyn ServerHost)
                        .transaction(|host, tx| {
                            let moved = if put {
                                items.insert(host, tx, 0, &dirt(), 1)
                            } else {
                                items.extract(host, tx, 0, &dirt(), 1)
                            };
                            assert_eq!(moved, 1);
                            Ok::<_, ()>(())
                        })
                        .unwrap();
                }
            }
            assert_eq!(
                game.journal_kept(),
                (0, 1),
                "one slot kept, the entity not copied"
            );
            game.transaction_close(&root, false);
            assert_eq!(holder_state(game), before);
            // A hopper's slot has its cooldown beside it: kept whole.
            let hopper = game
                .pack
                .assets
                .registry
                .default_state("minecraft:hopper")
                .unwrap();
            game.world.set(3, 64, 1, hopper).unwrap();
            game.world.block_entities.insert(
                (3, 64, 1),
                BlockEntity::for_block("minecraft:hopper").unwrap(),
            );
            let hopper = level.capability(game, &ITEMS, [3, 64, 1], None).unwrap();
            let root = game.transaction_open();
            (game as &mut dyn ServerHost)
                .transaction(|host, tx| Ok::<_, ()>(hopper.insert(host, tx, 0, &dirt(), 1)))
                .unwrap();
            assert_eq!(game.journal_kept(), (1, 0));
            game.transaction_close(&root, false);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn a_child_whole_capture_keeps_its_older_slot_entries() {
    const NAME: &str =
        "game::native_block_entity_item_tests::a_child_whole_capture_keeps_its_older_slot_entries";
    with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let entity = container_holder(game, 3);
            let level = entity.level(game).unwrap();
            let items = level.capability(game, &ITEMS, [1, 64, 1], None).unwrap();
            let insert = |host: &mut dyn ServerHost, slot: usize, n: u32| {
                host.transaction(|host, tx| {
                    assert_eq!(items.insert(host, tx, slot, &dirt(), n), n);
                    Ok::<_, ()>(())
                })
                .unwrap()
            };
            let whole = |host: &mut dyn ServerHost, value: i32| {
                host.transaction(|host, _| {
                    assert_eq!(entity.set_data_slots(host, vec![value]), Ok(true));
                    Ok::<_, ()>(())
                })
                .unwrap()
            };
            let before = holder_state(game);
            // Each case runs in a root that aborts, and the root itself touches nothing.
            let cases: [(&str, &dyn Fn(&mut dyn ServerHost)); 4] = [
                ("a child fills an empty slot, then is kept whole", &|host| {
                    host.transaction(|host, _| {
                        insert(host, 0, 10);
                        whole(host, 7);
                        Ok::<_, ()>(())
                    })
                    .unwrap()
                }),
                ("a grandchild's slot, then the child kept whole", &|host| {
                    host.transaction(|host, _| {
                        host.transaction(|host, _| {
                            insert(host, 2, 4);
                            Ok::<_, ()>(())
                        })
                        .unwrap();
                        whole(host, 8);
                        Ok::<_, ()>(())
                    })
                    .unwrap()
                }),
                ("kept whole first, then a slot", &|host| {
                    host.transaction(|host, _| {
                        whole(host, 9);
                        insert(host, 1, 5);
                        Ok::<_, ()>(())
                    })
                    .unwrap()
                }),
                (
                    "three depths writing one slot around a whole capture",
                    &|host| {
                        host.transaction(|host, _| {
                            host.transaction(|host, _| {
                                host.transaction(|host, _| {
                                    insert(host, 4, 3);
                                    Ok::<_, ()>(())
                                })
                                .unwrap();
                                whole(host, 10);
                                Ok::<_, ()>(())
                            })
                            .unwrap();
                            insert(host, 4, 6);
                            Ok::<_, ()>(())
                        })
                        .unwrap()
                    },
                ),
            ];
            for (case, write) in cases {
                let aborted: Result<(), ()> =
                    (game as &mut dyn ServerHost).transaction(|host, _| {
                        write(host);
                        Err(())
                    });
                assert!(aborted.is_err());
                assert_eq!(holder_state(game), before, "{case}");
            }
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn an_insert_past_a_short_mod_list_rolls_back_its_length() {
    const NAME: &str = "game::native_block_entity_item_tests::an_insert_past_a_short_mod_list_rolls_back_its_length";
    with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let entity = container_holder(game, 3);
            let BlockEntity::Mod(holder) = game.world.block_entities.get_mut(&(1, 64, 1)).unwrap()
            else {
                panic!()
            };
            holder.items.clear();
            let level = entity.level(game).unwrap();
            let items = level.capability(game, &ITEMS, [1, 64, 1], None).unwrap();
            let before = holder_state(game);
            let aborted: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, tx| {
                assert_eq!(items.insert(host, tx, 5, &dirt(), 7), 7);
                Err(())
            });
            assert!(aborted.is_err());
            assert_eq!(
                holder_state(game),
                before,
                "the list is empty again, not six long"
            );
        });
    });
}

/// Each block whose slots the journal keeps one at a time, with its block entity and these
/// stacks, at `pos`.
fn container_at(
    game: &mut super::Game<'_>,
    pos: (i32, i32, i32),
    block: &str,
) -> Vec<Option<ItemStack>> {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    game.world.set(pos.0, pos.1, pos.2, state).unwrap();
    let mut entity = BlockEntity::for_block(block).unwrap();
    let mut worn = ItemStack::new("minecraft:iron_pickaxe", 1);
    worn.damage = 17;
    let stacks = [
        Some(ItemStack::new("minecraft:stone", 64)),
        None,
        Some(ItemStack::new("minecraft:dirt", 3)),
        Some(worn),
    ];
    for (slot, stack) in entity.slots_mut().iter_mut().zip(stacks) {
        *slot = stack;
    }
    let slots = entity.slots().to_vec();
    game.world.block_entities.insert(pos, entity);
    slots
}

fn slots_at(game: &super::Game<'_>, pos: (i32, i32, i32)) -> Option<Vec<Option<ItemStack>>> {
    Some(game.world.block_entities.get(&pos)?.slots().to_vec())
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn each_slot_journalled_container_aborts_to_its_exact_stacks() {
    const NAME: &str = "game::native_block_entity_item_tests::each_slot_journalled_container_aborts_to_its_exact_stacks";
    with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let entity = mod_holder(game);
            let level = entity.level(game).unwrap();
            for block in [
                "minecraft:chest",
                "minecraft:barrel",
                "minecraft:shulker_box",
            ] {
                let pos = (3, 64, 1);
                let before = container_at(game, pos, block);
                let items = level.capability(game, &ITEMS, [3, 64, 1], None).unwrap();
                let stone = ItemResource::of(&before[0].clone().unwrap().try_into_api().unwrap());
                let aborted: Result<(), ()> =
                    (game as &mut dyn ServerHost).transaction(|host, _| {
                        host.transaction(|host, tx| {
                            assert_eq!(items.extract(host, tx, 0, &stone, 10), 10);
                            assert_eq!(items.insert(host, tx, 1, &dirt(), 5), 5);
                            Ok::<_, ()>(())
                        })
                        .unwrap();
                        host.transaction(|host, tx| {
                            assert_eq!(items.extract(host, tx, 2, &dirt(), 3), 3);
                            assert_eq!(items.insert(host, tx, 0, &stone, 4), 4);
                            Ok::<_, ()>(())
                        })
                        .unwrap();
                        Err(())
                    });
                assert!(aborted.is_err());
                assert_eq!(slots_at(game, pos), Some(before), "{block}");
                game.world.block_entities.remove(&pos);
            }
            // A mod's own full-length slot list, written with `set_item_slot`.
            let entity = container_holder(game, 3);
            let before = holder_state(game);
            let stack = ItemStack::new("minecraft:dirt", 2).try_into_api().unwrap();
            let aborted: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, tx| {
                assert_eq!(entity.set_item_slot(host, tx, 3, Some(stack)), Ok(true));
                Err(())
            });
            assert!(aborted.is_err());
            assert_eq!(holder_state(game), before);
            assert!(game.transactions.is_empty());
        });
    });
}

/// `LevelRef::set_block` writes the world at once, outside the transaction (on `main` too), so
/// an abort after it doesn't bring the old block back. What the slot journal must not do is
/// write a kept slot into the block entity that replaced the one it was taken from.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn a_kept_slot_goes_back_only_into_the_block_entity_it_came_from() {
    const NAME: &str = "game::native_block_entity_item_tests::a_kept_slot_goes_back_only_into_the_block_entity_it_came_from";
    with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let entity = mod_holder(game);
            let level = entity.level(game).unwrap();
            let pos = (3, 64, 1);
            let replace = |game: &mut super::Game<'_>, block: &str, commit: bool| {
                container_at(game, pos, "minecraft:chest");
                let items = level.capability(game, &ITEMS, [3, 64, 1], None).unwrap();
                let stone =
                    ItemResource::of(&ItemStack::new("minecraft:stone", 1).try_into_api().unwrap());
                let result: Result<(), ()> =
                    (game as &mut dyn ServerHost).transaction(|host, tx| {
                        // Takes from the chest's full slot 0: the slot kept is its 64 stone, which
                        // an abort must not write into whatever replaced the chest.
                        assert_eq!(items.extract(host, tx, 0, &stone, 10), 10);
                        let state = host.block_state_id(block).unwrap();
                        assert!(
                            level
                                .set_block(host, [3, 64, 1], state, mod_api::BlockUpdateFlags::NONE)
                                .unwrap()
                        );
                        if commit { Ok(()) } else { Err(()) }
                    });
                assert_eq!(result.is_ok(), commit);
            };
            replace(game, "minecraft:air", false);
            assert_eq!(slots_at(game, pos), None, "removed, as on main");
            replace(game, "minecraft:barrel", false);
            let barrel = slots_at(game, pos).expect("the barrel's block entity");
            assert!(
                barrel.iter().all(Option::is_none),
                "the chest's kept slot was not written into the barrel: {barrel:?}"
            );
            replace(game, "minecraft:air", true);
            assert_eq!(slots_at(game, pos), None);
            assert!(game.transactions.is_empty());
        });
    });
}

/// The cost a slot journal saves, measured: opt-in (`HORIZON_JOURNAL_TIMING=1`), as wall-clock
/// time depends on the machine. `a_container_move_keeps_one_slot_not_the_entity` is the
/// regression test; this prints the numbers for a pull request.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn timing_container_moves_beside_saved_data() {
    const NAME: &str =
        "game::native_block_entity_item_tests::timing_container_moves_beside_saved_data";
    if std::env::var_os("HORIZON_JOURNAL_TIMING").is_none() {
        return;
    }
    with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let time = |game: &mut super::Game<'_>, keys: usize| {
                let entity = container_holder(game, keys);
                let level = entity.level(game).unwrap();
                let items = level.capability(game, &ITEMS, [1, 64, 1], None).unwrap();
                let start = std::time::Instant::now();
                commit(game, |host, _| {
                    for _ in 0..500 {
                        host.transaction(|host, tx| {
                            assert_eq!(items.insert(host, tx, 0, &dirt(), 1), 1);
                            Ok::<_, ()>(())
                        })
                        .unwrap();
                        host.transaction(|host, tx| {
                            assert_eq!(items.extract(host, tx, 0, &dirt(), 1), 1);
                            Ok::<_, ()>(())
                        })
                        .unwrap();
                    }
                });
                start.elapsed()
            };
            time(game, 1);
            let small = time(game, 1);
            let large = time(game, 100_000);
            eprintln!("1,000 container moves: {small:?} beside 1 key, {large:?} beside 100,000");
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn storage_asks_a_game_container_by_kind_and_rolls_back_with_its_transaction() {
    const NAME: &str = "game::native_block_entity_item_tests::storage_asks_a_game_container_by_kind_and_rolls_back_with_its_transaction";
    with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let entity = mod_holder(game);
            let level = entity.level(game).unwrap();
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            game.world.set(3, 64, 1, chest).unwrap();
            game.world
                .block_entities
                .insert((3, 64, 1), BlockEntity::Chest(vec![None; 27]));
            let of = |id: &str| ItemResource::of(&ItemStack::new(id, 1).try_into_api().unwrap());
            let (stone, dirt) = (of("minecraft:stone"), of("minecraft:dirt"));
            let storage = level
                .capability(game, &mod_api::STORAGE, [3, 64, 1], None)
                .unwrap();
            assert_eq!(storage.space_for(game, &stone), Ok(27 * 64));
            commit(game, |host, tx| {
                assert_eq!(storage.insert(host, tx, &stone, 100), 100);
                assert_eq!(storage.insert(host, tx, &dirt, 5), 5);
            });
            assert_eq!(storage.amount(game, &stone), Ok(100));
            assert_eq!(
                storage.space_for(game, &stone),
                Ok(24 * 64 + 28),
                "two stone slots, one dirt"
            );
            let aborted: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, tx| {
                assert_eq!(storage.extract(host, tx, &stone, 70), 70);
                assert_eq!(storage.insert(host, tx, &dirt, 2000), 25 * 64 + 59);
                Err(())
            });
            assert!(aborted.is_err());
            assert_eq!(storage.amount(game, &stone), Ok(100));
            assert_eq!(storage.amount(game, &dirt), Ok(5));
            let BlockEntity::Chest(slots) = game.world.block_entities.get(&(3, 64, 1)).unwrap()
            else {
                panic!()
            };
            let counts: Vec<u32> = slots.iter().flatten().map(|s| s.count).collect();
            assert_eq!(
                counts,
                [64, 36, 5],
                "slots back as they were, in their order"
            );
            let mut page = mod_api::StoragePage::new(64).unwrap();
            assert_eq!(storage.page(game, 0, &mut page), Ok(None));
            assert_eq!(page.entries().len(), 3);
            // A furnace's top takes only into its input, and its bottom gives only its result.
            let furnace = game
                .pack
                .assets
                .registry
                .default_state("minecraft:furnace")
                .unwrap();
            game.world.set(5, 64, 1, furnace).unwrap();
            game.world.block_entities.insert(
                (5, 64, 1),
                BlockEntity::for_block("minecraft:furnace").unwrap(),
            );
            let top = level
                .capability(
                    game,
                    &mod_api::STORAGE,
                    [5, 64, 1],
                    Some(mod_api::Direction::Up),
                )
                .unwrap();
            let bottom = level
                .capability(
                    game,
                    &mod_api::STORAGE,
                    [5, 64, 1],
                    Some(mod_api::Direction::Down),
                )
                .unwrap();
            commit(game, |host, tx| {
                assert_eq!(top.insert(host, tx, &stone, 200), 64, "the input slot only");
                assert_eq!(bottom.extract(host, tx, &stone, 64), 0, "not the input");
            });
            assert!(game.transactions.is_empty());
        });
    });
}

/// A block entity's whole state, and a hopper's cooldown beside it, compared as text: what an
/// abort must put back whatever kind of container it is.
fn whole_state(game: &super::Game<'_>, pos: (i32, i32, i32)) -> String {
    format!(
        "{:?} {:?}",
        game.world.block_entities.get(&pos),
        game.hoppers.get(&pos).map(|h| h.cooldown)
    )
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn storage_writes_abort_to_the_exact_state_of_every_container_kind() {
    const NAME: &str = "game::native_block_entity_item_tests::storage_writes_abort_to_the_exact_state_of_every_container_kind";
    with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let entity = mod_holder(game);
            let level = entity.level(game).unwrap();
            let pos = (3, 64, 1);
            let of = |id: &str| ItemResource::of(&ItemStack::new(id, 1).try_into_api().unwrap());
            // Each kind with an item it takes through the side asked, and why it is here.
            let kinds = [
                ("minecraft:chest", None, "minecraft:dirt"),
                ("minecraft:barrel", None, "minecraft:dirt"),
                ("minecraft:shulker_box", None, "minecraft:dirt"),
                ("minecraft:dispenser", None, "minecraft:dirt"),
                // An insert into an empty hopper sets its cooldown.
                ("minecraft:hopper", None, "minecraft:dirt"),
                // An input into an empty furnace restarts its cooking.
                (
                    "minecraft:furnace",
                    Some(mod_api::Direction::Up),
                    "minecraft:raw_iron",
                ),
                // A disc starts its song at the root commit.
                ("minecraft:jukebox", None, "minecraft:music_disc_cat"),
                // A pot keeps its item in its saved data.
                ("minecraft:decorated_pot", None, "minecraft:dirt"),
                // Blaze powder goes to its fuel slot, the one slot it takes.
                ("minecraft:brewing_stand", None, "minecraft:blaze_powder"),
            ];
            let mut answered = Vec::new();
            for (block, side, item) in kinds {
                // Placed as the game places it, so each gets its own block entity.
                for name in ["minecraft:air", block] {
                    let state = game.pack.assets.registry.default_state(name).unwrap();
                    game.level_set_block(level, [3, 64, 1], state, mod_api::BlockUpdateFlags::NONE)
                        .unwrap();
                }
                game.hoppers.remove(&pos);
                let Some(storage) = level.capability(game, &mod_api::STORAGE, [3, 64, 1], side)
                else {
                    continue;
                };
                let resource = of(item);
                let before = whole_state(game, pos);
                let aborted: Result<(), ()> =
                    (game as &mut dyn ServerHost).transaction(|host, _| {
                        host.transaction(|host, tx| {
                            assert_eq!(storage.insert(host, tx, &resource, 1), 1, "{block}");
                            Ok::<_, ()>(())
                        })
                        .unwrap();
                        host.transaction(|host, tx| {
                            assert_eq!(storage.extract(host, tx, &resource, 1), 1, "{block}");
                            assert_eq!(storage.insert(host, tx, &resource, 1), 1, "{block}");
                            Ok::<_, ()>(())
                        })
                        .unwrap();
                        Err(())
                    });
                assert!(aborted.is_err());
                assert_eq!(whole_state(game, pos), before, "{block} after the abort");
                // Committed, the item is there.
                commit(game, |host, tx| {
                    assert_eq!(storage.insert(host, tx, &resource, 1), 1, "{block}")
                });
                assert_eq!(storage.amount(game, &resource), Ok(1), "{block}");
                answered.push(block);
            }
            eprintln!("STORAGE answered for: {answered:?}");
            for block in [
                "minecraft:chest",
                "minecraft:barrel",
                "minecraft:shulker_box",
                "minecraft:dispenser",
                "minecraft:hopper",
                "minecraft:furnace",
                "minecraft:jukebox",
                "minecraft:brewing_stand",
            ] {
                assert!(answered.contains(&block), "{block} answers STORAGE");
            }
            assert!(
                !answered.contains(&"minecraft:decorated_pot"),
                "a pot takes items from hoppers only, never through the capability"
            );
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn an_aborted_bulk_transfer_conserves_every_item() {
    const NAME: &str =
        "game::native_block_entity_item_tests::an_aborted_bulk_transfer_conserves_every_item";
    with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let entity = mod_holder(game);
            let level = entity.level(game).unwrap();
            let (a, b) = ((3, 64, 1), (5, 64, 1));
            container_at(game, a, "minecraft:chest");
            container_at(game, b, "minecraft:barrel");
            let from = level
                .capability(game, &mod_api::STORAGE, [3, 64, 1], None)
                .unwrap();
            let to = level
                .capability(game, &mod_api::STORAGE, [5, 64, 1], None)
                .unwrap();
            let kinds: Vec<ItemResource> = ["minecraft:stone", "minecraft:dirt"]
                .iter()
                .map(|id| ItemResource::of(&ItemStack::new(id, 1).try_into_api().unwrap()))
                .collect();
            let total = |game: &mut super::Game<'_>| {
                let mut n = 0;
                for kind in &kinds {
                    n += from.amount(game, kind).unwrap() + to.amount(game, kind).unwrap();
                }
                n
            };
            let (before_a, before_b) = (slots_at(game, a), slots_at(game, b));
            let before = total(game);
            let aborted: Result<u64, mod_api::StorageError> = (game as &mut dyn ServerHost)
                .transaction(|host, tx| {
                    let moved =
                        mod_api::transfer_matching(host, &from, &to, |_| true, u64::MAX, tx)?;
                    assert!(moved > 0);
                    Err(mod_api::StorageError::ZeroPage)
                });
            assert!(aborted.is_err());
            assert_eq!((slots_at(game, a), slots_at(game, b)), (before_a, before_b));
            let moved = (game as &mut dyn ServerHost)
                .transaction(|host, tx| {
                    mod_api::transfer_matching(host, &from, &to, |_| true, u64::MAX, tx)
                })
                .unwrap();
            assert_eq!(moved, 68, "64 stone, 3 dirt and the worn pickaxe");
            assert_eq!(total(game), before);
            assert!(game.transactions.is_empty());
        });
    });
}

/// The comparison behind `STORAGE`'s guidance, in the engine: `cells` barrel pairs, each a
/// source full of stacks and an empty sink, moved 8 items a pipe a tick, one move in ten
/// aborted after it wrote. Today's pipe asks `ITEMS` on both sides and calls `move_resources`
/// every tick, busy or not; the `STORAGE` pipe keeps its two handlers and stops asking once a
/// source has nothing left. Opt-in (`HORIZON_STORAGE_TIMING=1`, `HORIZON_STORAGE_CELLS` for the
/// grid size, 1,024 by default), as wall-clock time depends on the machine; it prints the pipe
/// code's time per tick, busy and idle, and fails if an item is made or lost.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn timing_storage_against_items_across_barrel_cells() {
    const NAME: &str =
        "game::native_block_entity_item_tests::timing_storage_against_items_across_barrel_cells";
    if std::env::var_os("HORIZON_STORAGE_TIMING").is_none() {
        return;
    }
    let cells: i32 = std::env::var("HORIZON_STORAGE_CELLS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(1024);
    with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let entity = mod_holder(game);
            let level = entity.level(game).unwrap();
            let side = (cells as f64).sqrt().ceil() as i32;
            for cx in 0..=(side * 2) / 16 {
                for cz in 0..=side / 16 {
                    game.world.chunks.insert(
                        (cx, cz),
                        Arc::new(horizon_client::chunk::Chunk::empty(cx, cz)),
                    );
                }
            }
            let at = |cell: i32| (cell % side * 2, cell / side);
            let kinds = ["minecraft:stone", "minecraft:dirt", "minecraft:cobblestone"];
            let fill = |game: &mut super::Game<'_>| {
                for cell in 0..cells {
                    let (x, z) = at(cell);
                    for (y, full) in [(64, true), (66, false)] {
                        let state = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:barrel")
                            .unwrap();
                        game.world.set(x, y, z, state).unwrap();
                        let mut barrel = BlockEntity::for_block("minecraft:barrel").unwrap();
                        if full {
                            for (slot, stack) in barrel.slots_mut().iter_mut().enumerate() {
                                *stack = Some(ItemStack::new(kinds[slot % 3], 64));
                            }
                        }
                        game.world.block_entities.insert((x, y, z), barrel);
                    }
                }
            };
            let resources: Vec<ItemResource> = kinds
                .iter()
                .map(|id| ItemResource::of(&ItemStack::new(id, 1).try_into_api().unwrap()))
                .collect();
            let total = |game: &mut super::Game<'_>| {
                let mut n = 0u64;
                for cell in 0..cells {
                    let (x, z) = at(cell);
                    for y in [64, 66] {
                        let storage = level
                            .capability(game, &mod_api::STORAGE, [x, y, z], None)
                            .unwrap();
                        for kind in &resources {
                            n += storage.amount(game, kind).unwrap();
                        }
                    }
                }
                n
            };
            let report = |label: &str,
                          busy: (u32, std::time::Duration),
                          idle: std::time::Duration| {
                eprintln!(
                    "{label}: {cells} cells, {:.3} ms a tick while moving ({} ticks), {:.3} ms a tick idle",
                    busy.1.as_secs_f64() * 1e3 / f64::from(busy.0.max(1)),
                    busy.0,
                    idle.as_secs_f64() * 1e3 / 200.0
                );
            };
            // Today's pipe.
            fill(game);
            let before = total(game);
            let mut moves = 0u32;
            let mut run_items = |game: &mut super::Game<'_>| {
                let mut moved = 0;
                for cell in 0..cells {
                    let (x, z) = at(cell);
                    let below =
                        level.capability(game, &ITEMS, [x, 64, z], Some(mod_api::Direction::Up));
                    let above =
                        level.capability(game, &ITEMS, [x, 66, z], Some(mod_api::Direction::Down));
                    let (Some(below), Some(above)) = (below, above) else {
                        continue;
                    };
                    moves += 1;
                    let abort = moves.is_multiple_of(10);
                    let result = (game as &mut dyn ServerHost).transaction(|host, tx| {
                        let n = mod_api::move_resources(host, &below, &above, |_| true, 8, tx)
                            .map_err(|_| 0u32)?;
                        if abort && n > 0 { Err(n) } else { Ok(n) }
                    });
                    moved += result.unwrap_or(0);
                }
                moved
            };
            let (mut ticks, start) = (0, std::time::Instant::now());
            while run_items(game) > 0 && ticks < 2000 {
                ticks += 1;
            }
            let busy = (ticks, start.elapsed());
            let start = std::time::Instant::now();
            for _ in 0..200 {
                run_items(game);
            }
            report("ITEMS, every tick", busy, start.elapsed());
            assert_eq!(total(game), before, "ITEMS: every item is still there");
            // The STORAGE pipe.
            fill(game);
            let before = total(game);
            let pipes: Vec<_> = (0..cells)
                .map(|cell| {
                    let (x, z) = at(cell);
                    let from = level
                        .capability(
                            game,
                            &mod_api::STORAGE,
                            [x, 64, z],
                            Some(mod_api::Direction::Up),
                        )
                        .unwrap();
                    let to = level
                        .capability(
                            game,
                            &mod_api::STORAGE,
                            [x, 66, z],
                            Some(mod_api::Direction::Down),
                        )
                        .unwrap();
                    (from, to)
                })
                .collect();
            let mut awake = vec![true; pipes.len()];
            let mut page = mod_api::StoragePage::new(mod_api::MAX_STORAGE_PAGE).unwrap();
            let mut moves = 0u32;
            let mut run_storage = |game: &mut super::Game<'_>| {
                let mut moved = 0;
                for (cell, (from, to)) in pipes.iter().enumerate() {
                    if !awake[cell] {
                        continue;
                    }
                    from.page(game, 0, &mut page).unwrap();
                    let Some((kind, _)) = page.entries().first().cloned() else {
                        awake[cell] = false;
                        continue;
                    };
                    moves += 1;
                    let abort = moves.is_multiple_of(10);
                    let result = (game as &mut dyn ServerHost).transaction(|host, tx| {
                        let n =
                            mod_api::transfer(host, from, to, &kind, 8, tx).map_err(|_| 0u64)?;
                        if abort && n > 0 { Err(n) } else { Ok(n) }
                    });
                    moved += result.unwrap_or(0);
                }
                moved
            };
            let (mut ticks, start) = (0, std::time::Instant::now());
            while run_storage(game) > 0 && ticks < 2000 {
                ticks += 1;
            }
            let busy = (ticks, start.elapsed());
            let start = std::time::Instant::now();
            for _ in 0..200 {
                run_storage(game);
            }
            report("STORAGE, sleeping when empty", busy, start.elapsed());
            assert_eq!(total(game), before, "STORAGE: every item is still there");
            assert!(game.transactions.is_empty());
        });
    });
}
