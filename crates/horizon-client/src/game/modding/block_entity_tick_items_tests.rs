use super::api_values::TryIntoApi;
use horizon_client::{
    chunk::Chunk,
    containers::{BlockEntity, ModBlockEntity},
    items::ItemStack,
};
use mod_api::{
    BlockEntityPart, BlockEntityTickEvent, BlockPart, EntityLeaveLevelEvent, Hooks,
    HostTransaction, ItemPlace, Registrar, ServerHost,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

static FIRST: AtomicU32 = AtomicU32::new(0);
static SECOND: AtomicU32 = AtomicU32::new(0);
static WRITTEN: AtomicBool = AtomicBool::new(false);
static OBSERVED: AtomicBool = AtomicBool::new(false);
static NOTIFICATIONS: AtomicU32 = AtomicU32::new(0);
struct Consume;
#[mod_api::horizon_part]
impl BlockPart for Consume {}
#[mod_api::horizon_part]
impl BlockEntityPart for Consume {
    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        if WRITTEN.swap(true, Ordering::Relaxed) {
            return;
        }
        let first = host.entity(FIRST.load(Ordering::Relaxed)).unwrap();
        let second = host.entity(SECOND.load(Ordering::Relaxed)).unwrap();
        let item = host.item_entity(first).unwrap();
        let stack = ItemStack::new("minecraft:stone", 2).try_into_api().unwrap();
        let case = std::env::var("ROW4_NATIVE_CASE").unwrap();
        if case == "unchanged" {
            *event.reply = mod_api::BlockEntityTickReply::Unchanged;
        }
        if case == "invalid_setter" {
            let invalid = ItemStack::new("minecraft:stone", 65)
                .try_into_api()
                .unwrap();
            assert!(item.set_stack(host, invalid).is_err());
            assert_eq!(item.stack(host).unwrap().count(), 5);
            return;
        }
        event.entity.data.push(6);
        event.entity.items = vec![Some(
            ItemStack::new("minecraft:stone", 6).try_into_api().unwrap(),
        )];
        if case.starts_with("wasm_") {
            super::block_entity_tick_wasm_tests::tick(host, event, first, second, stack, &case);
            return;
        }
        assert_eq!(item.set_stack(host, stack), Ok(true));
        assert_eq!(item.stack(host).unwrap().count(), 2);
        assert_eq!(
            ItemPlace::ItemEntity { id: first.id() }
                .stack(host)
                .unwrap()
                .count(),
            2
        );
        if case == "nested_abort" {
            let aborted: Result<(), ()> = host.transaction(|host, _| {
                assert_eq!(
                    item.set_stack(
                        host,
                        ItemStack::new("minecraft:stone", 1).try_into_api().unwrap()
                    ),
                    Ok(true)
                );
                assert_eq!(item.stack(host).unwrap().count(), 1);
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert_eq!(item.stack(host).unwrap().count(), 2);
        }
        assert!(second.discard(host));
        assert!(host.entity(second.id()).is_none());
        assert!(host.item_entity(second).is_none());
        assert!(
            ItemPlace::ItemEntity { id: second.id() }
                .stack(host)
                .is_none()
        );
        let level = host.level("minecraft:overworld").unwrap();
        assert!(!level.all_entities(host).unwrap().contains(&second));
        if case == "removed" {
            assert!(level.remove_block(host, event.pos, false));
        }
        if std::env::var("ROW4_NATIVE_CASE").as_deref() == Ok("invalid_stack") {
            event.entity.items = vec![Some(
                ItemStack::new("minecraft:stone", 65)
                    .try_into_api()
                    .unwrap(),
            )];
        }
    }
}
struct Observe;
#[mod_api::horizon_part]
impl BlockPart for Observe {}
#[mod_api::horizon_part]
impl BlockEntityPart for Observe {
    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        *event.reply = mod_api::BlockEntityTickReply::Unchanged;
        if OBSERVED.swap(true, Ordering::Relaxed) {
            return;
        }
        let case = std::env::var("ROW4_NATIVE_CASE").unwrap();
        if matches!(
            case.as_str(),
            "invalid_stack" | "wasm_malformed" | "invalid_setter"
        ) {
            return;
        }
        let first = host.entity(FIRST.load(Ordering::Relaxed)).unwrap();
        assert_eq!(
            host.item_entity(first)
                .unwrap()
                .stack(host)
                .unwrap()
                .count(),
            2
        );
        assert!(host.entity(SECOND.load(Ordering::Relaxed)).is_none());
        if case == "later_remove" {
            assert!(first.discard(host));
        }
    }
}
struct Unrelated;
#[mod_api::horizon_part]
impl BlockPart for Unrelated {}
#[mod_api::horizon_part]
impl BlockEntityPart for Unrelated {
    fn block_entity_tick(&self, _: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.data.is_empty() {
            event.entity.data.push(42);
        }
    }
}
struct Fixture;
#[mod_api::__subscriptions]
impl Hooks for Fixture {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let kind = registrar.block_entity_type("ticker").blocks(&[]).build();
        registrar.block_entity_part(&kind, Consume).unwrap();
        registrar.block_entity_part(&kind, Observe).unwrap();
        let other = registrar.block_entity_type("other").blocks(&[]).build();
        registrar.block_entity_part(&other, Unrelated).unwrap();
    }
    fn entity_leave_level(&self, host: &mut dyn ServerHost, event: &mut EntityLeaveLevelEvent<'_>) {
        if event.entity == SECOND.load(Ordering::Relaxed)
            || event.entity == FIRST.load(Ordering::Relaxed)
        {
            NOTIFICATIONS.fetch_add(1, Ordering::Relaxed);
            if std::env::var("ROW4_NATIVE_CASE").as_deref() == Ok("reentrant") {
                let first = host.entity(FIRST.load(Ordering::Relaxed)).unwrap();
                let item = host.item_entity(first).unwrap();
                assert_eq!(item.stack(host).unwrap().count(), 2);
                assert_eq!(
                    item.set_stack(
                        host,
                        ItemStack::new("minecraft:stone", 1).try_into_api().unwrap()
                    ),
                    Ok(true)
                );
            }
        }
    }
}

#[test]
#[ignore = "daily"]
fn native_tick_publishes_or_rejects_item_proposals_with_writeback() {
    const NAME: &str = "game::block_entity_tick_items_tests::native_tick_publishes_or_rejects_item_proposals_with_writeback";
    // Instantiate and invoke the real wasm guest on the production-sized stack.
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        if super::native_entity_refs::tests::run_fresh_cases(
            NAME,
            "ROW4_NATIVE_CASE",
            &[
                "valid",
                "unchanged",
                "invalid_stack",
                "invalid_setter",
                "wasm_valid",
                "wasm_malformed",
                "wasm_identity_readonly",
                "nested_abort",
                "later_remove",
                "removed",
                "reentrant",
                "scheduled",
                "disabled",
                "sleep",
                "unrelated",
            ],
        ) {
            return;
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("row4", Box::new(Fixture))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            super::api_values::install_parts(&mods).unwrap();
            mod_api::__private::mark_game_thread();
            let base =
                std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
            let root = super::mods::content_root(&base, &Default::default()).unwrap();
            super::native_entity_refs::tests::with_game_at(root.prepared().to_owned(), |game| {
                game.options.dedicated = Some(Vec::new());
                assert!(game.block_entity_tick_items.is_none());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let case = std::env::var("ROW4_NATIVE_CASE").unwrap();
                let pos = (1, 64, 1);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, stone).unwrap();
                let state = game.world.block(pos.0, pos.1, pos.2).0;
                game.world.block_entities.set_mod_containers(
                    [(
                        state,
                        Arc::new(horizon_client::containers::ModContainer {
                            slots: 1,
                            ..Default::default()
                        }),
                    )]
                    .into_iter()
                    .collect(),
                );
                game.world.block_entities.insert(
                    pos,
                    BlockEntity::Mod(Box::new(ModBlockEntity::new(
                        if case == "unrelated" {
                            "row4:other"
                        } else {
                            "row4:ticker"
                        }
                        .into(),
                        Default::default(),
                    ))),
                );
                for count in [5, 3] {
                    game.spawn_item(
                        ItemStack::new("minecraft:stone", count),
                        [0.0; 3],
                        [0.0; 3],
                        0,
                    );
                }
                FIRST.store(game.dropped[0].id, Ordering::Relaxed);
                SECOND.store(game.dropped[1].id, Ordering::Relaxed);
                let before = match game.world.block_entities.get(&pos).unwrap() {
                    BlockEntity::Mod(entity) => entity.revision(),
                    _ => unreachable!(),
                };
                game.world.modified.clear();
                if matches!(case.as_str(), "scheduled" | "disabled" | "sleep") {
                    use horizon_client::block_entities::Ticking;
                    game.tickets.update(None);
                    let mut states = vec![Ticking::Yes; game.pack.assets.registry.len()];
                    if case == "disabled" {
                        states[state as usize] = Ticking::No;
                    }
                    game.world.block_entities.set_states(states);
                    if case == "sleep" {
                        game.world.block_entities.sleep(pos);
                    }
                    game.tick_block_entities().unwrap();
                } else {
                    game.tick_mod_block_entity(pos);
                }
                assert!(game.block_entity_tick_items.is_none());
                let rejected = matches!(
                    case.as_str(),
                    "invalid_stack" | "wasm_malformed" | "invalid_setter" | "removed"
                );
                let expected: &[u32] = match case.as_str() {
                    "later_remove" => &[],
                    "reentrant" => &[1],
                    "disabled" | "sleep" | "unrelated" => &[5, 3],
                    _ if rejected => &[5, 3],
                    _ => &[2],
                };
                assert_eq!(
                    game.dropped
                        .iter()
                        .map(|e| e.stack.count)
                        .collect::<Vec<_>>(),
                    expected
                );
                if case == "removed" {
                    assert!(game.world.block_entities.get(&pos).is_none());
                } else {
                    let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&pos) else {
                        panic!("holder missing")
                    };
                    if case == "unchanged" {
                        assert_eq!(entity.revision(), before);
                        assert!(
                            entity.items.is_empty(),
                            "Unchanged must not normalize the original"
                        );
                        assert!(!game.world.modified.contains(&(0, 0)));
                    }
                    assert_eq!(
                        entity.data,
                        if case == "unrelated" {
                            vec![42]
                        } else if rejected
                            || matches!(case.as_str(), "disabled" | "sleep" | "unchanged")
                        {
                            vec![]
                        } else {
                            vec![6]
                        }
                    );
                    assert_eq!(
                        entity
                            .items
                            .first()
                            .and_then(|slot| slot.as_ref())
                            .map(|stack| stack.count),
                        if rejected
                            || matches!(
                                case.as_str(),
                                "disabled" | "sleep" | "unrelated" | "unchanged"
                            )
                        {
                            None
                        } else {
                            Some(6)
                        }
                    );
                }
                assert_eq!(
                    NOTIFICATIONS.load(Ordering::Relaxed),
                    if rejected || matches!(case.as_str(), "disabled" | "sleep" | "unrelated") {
                        0
                    } else if case == "later_remove" {
                        2
                    } else {
                        1
                    }
                );
                if matches!(case.as_str(), "scheduled" | "disabled" | "sleep") {
                    game.tick_block_entities().unwrap();
                } else {
                    game.tick_mod_block_entity(pos);
                }
                assert_eq!(
                    game.dropped
                        .iter()
                        .map(|e| e.stack.count)
                        .collect::<Vec<_>>(),
                    expected
                );
                if let Some(first) = game.entity(FIRST.load(Ordering::Relaxed)) {
                    let item = game.item_entity(first).unwrap();
                    assert_eq!(
                        item.set_stack(
                            game,
                            ItemStack::new("minecraft:stone", 4).try_into_api().unwrap()
                        ),
                        Ok(true)
                    );
                    assert_eq!(
                        game.dropped
                            .iter()
                            .find(|e| e.id == first.id())
                            .unwrap()
                            .stack
                            .count,
                        4
                    );
                    assert!(game.block_entity_tick_items.is_none());
                    assert!(game.block_entity_tick_key.is_none());
                }
            });
        });
    });
}
