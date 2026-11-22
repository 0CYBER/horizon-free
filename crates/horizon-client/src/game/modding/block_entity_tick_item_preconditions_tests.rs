//! Publication-layer mutants: real Game storage, with interference after submission.
use super::{api_values::TryIntoApi, block_entity_tick_items::Key};
use horizon_client::{
    containers::{BlockEntity, ModBlockEntity},
    items::ItemStack,
};

#[test]
#[ignore = "daily"]
fn finalization_never_retargets_stale_items_or_replaced_placeholders() {
    const NAME: &str = "game::block_entity_tick_item_preconditions_tests::finalization_never_retargets_stale_items_or_replaced_placeholders";
    if super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "ROW4_PRECONDITION_CASE",
        &[
            "stack",
            "join",
            "other_level",
            "replacement",
            "empty_replacement",
            "revision",
            "unbalanced",
        ],
    ) {
        return;
    }
    super::native_live_holder_tests::with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            super::native_live_holder_tests::mod_holder(game);
            let pos = (1, 64, 1);
            let Some(BlockEntity::Mod(holder)) = game.world.block_entities.get_mut(&pos) else {
                panic!("missing holder")
            };
            let taken = std::mem::take(&mut **holder);
            let placeholder_identity = holder.revision().identity();
            let original_items = taken.items.clone();
            game.block_entity_tick_key = Some(Key {
                pos,
                placeholder_identity,
                failure_generation: game.wasm_dispatch_failures.get(),
                transaction_depth: 0,
                unbalanced: false,
                keeps_contents: false,
            });
            for count in [5, 3] {
                game.spawn_item(
                    ItemStack::new("minecraft:stone", count),
                    [0.0; 3],
                    [0.0; 3],
                    0,
                );
            }
            let first = game.dropped[0].id;
            let second = game.dropped[1].id;
            assert_eq!(
                game.stage_tick_item(first, Some(ItemStack::new("minecraft:stone", 2))),
                Some(true)
            );
            assert_eq!(game.stage_tick_item(second, None), Some(true));
            let mut snapshot = taken.try_into_api().unwrap();
            let before = snapshot.revision();
            snapshot.data.push(6);
            snapshot.items = vec![Some(
                ItemStack::new("minecraft:stone", 6).try_into_api().unwrap(),
            )];
            let case = std::env::var("ROW4_PRECONDITION_CASE").unwrap();
            match case.as_str() {
                "stack" => game.dropped[1].stack.count = 4,
                "join" => {
                    let mut entity = game.dropped.remove(1);
                    entity.join = Default::default();
                    game.dropped.push(entity);
                }
                "other_level" => {
                    let entity = game.dropped.remove(1);
                    assert!(game.enter_command_level(super::Dim::Nether).is_some());
                    game.dropped.push(entity);
                    assert!(game.enter_command_level(super::Dim::Overworld).is_some());
                }
                "replacement" | "empty_replacement" => {
                    let mut replacement = ModBlockEntity::new(
                        if case == "replacement" {
                            "replacement:ticker"
                        } else {
                            ""
                        }
                        .into(),
                        Default::default(),
                    );
                    replacement.data = vec![99];
                    game.world
                        .block_entities
                        .insert(pos, BlockEntity::Mod(Box::new(replacement)));
                }
                "unbalanced" => {
                    let _abandoned = game.transaction_open();
                }
                "revision" => {
                    snapshot =
                        mod_api::ModBlockEntity::new("wrong:identity".into(), Default::default())
                }
                _ => unreachable!("fixture case"),
            }
            game.block_entity_tick_items.as_mut().unwrap().snapshot =
                Some(Box::new((snapshot, before)));
            game.finish_tick_item_writeback(pos, taken, &original_items, (None, None), true);
            assert!(game.block_entity_tick_items.is_none());
            assert!(game.block_entity_tick_key.is_none());
            assert!(game.transactions.is_empty());
            assert_eq!(
                game.dropped
                    .iter()
                    .find(|e| e.id == first)
                    .unwrap()
                    .stack
                    .count,
                5
            );
            if case != "other_level" {
                assert_eq!(
                    game.dropped
                        .iter()
                        .find(|e| e.id == second)
                        .unwrap()
                        .stack
                        .count,
                    if case == "stack" { 4 } else { 3 }
                );
            }
            let Some(BlockEntity::Mod(holder)) = game.world.block_entities.get(&pos) else {
                panic!("holder missing")
            };
            assert_eq!(
                holder.data,
                if matches!(case.as_str(), "replacement" | "empty_replacement") {
                    vec![99]
                } else {
                    vec![]
                }
            );
            assert!(holder.items.iter().all(Option::is_none));
            if case == "other_level" {
                assert!(game.enter_command_level(super::Dim::Nether).is_some());
                assert_eq!(
                    game.dropped
                        .iter()
                        .find(|e| e.id == second)
                        .unwrap()
                        .stack
                        .count,
                    3
                );
            }
        });
    });
}
