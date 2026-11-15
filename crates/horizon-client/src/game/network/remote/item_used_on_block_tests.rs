//! ITEM_USED_ON_BLOCK after the block phase sees the stack copied before it (arismoko/horizon#2940).
use super::*;
use horizon_client::advancements as a;
use serde_json::json;

/// One `item_used_on_block` advancement per stick count the tool must have.
pub(super) fn install_criteria(game: &mut Game<'_>, counts: &[u32], item: &str) {
    let mut tree = a::Tree::default();
    for count in counts {
        let trigger = "minecraft:item_used_on_block".to_owned();
        let id = format!("test:sticks{count}");
        let index = tree.list.len();
        tree.by_id.insert(id.clone(), index);
        tree.by_trigger
            .entry(trigger.clone())
            .or_default()
            .push((index, 0));
        let tool = json!({"items": item, "count": count});
        tree.list.push(a::Advancement {
            id,
            parent: None,
            display: None,
            criteria: vec![a::Criterion {
                name: "seen".into(),
                trigger,
                conditions: (if *count == 0 {
                    json!({})
                } else {
                    json!({"location": {"type": "minecraft:match_tool", "predicate": tool}})
                })
                .into(),
            }],
            requirements: vec![vec!["seen".into()]],
            rewards: Default::default(),
        });
    }
    game.adv.tree = Arc::new(tree);
    game.adv.loaded = true;
}

#[test]
#[ignore = "daily"]
fn the_item_phase_criterion_sees_the_stack_from_before_the_block_phase() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "item_used_on_block_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(false, |game| {
            install_criteria(game, &[1, 2], "minecraft:stick");
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.forced_hit = Some(hit());
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 2));
            // The block's turn takes a stick from the hand and passes; the
            // item's turn then consumes the click.
            *observer.0.lock().unwrap() = Controls {
                block_answer: Some(Answer::Pass),
                take_in_block: true,
                item_on_answer: Some(Answer::Consume),
                ..Controls::default()
            };
            assert!(use_block(game, false));
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [
                    Step::Phase(UsePhase::Block, LogicalSide::Server),
                    Step::Phase(UsePhase::ItemAfterBlock, LogicalSide::Server),
                ]
            );
            assert_eq!(game.held().map(|stack| stack.count), Some(1));
            assert!(game.adv.progress.has("test:sticks2", "seen"));
            assert!(!game.adv.progress.has("test:sticks1", "seen"));
        });
    });
}

#[test]
#[ignore = "daily"]
fn compass_criterion_sees_two_compasses_before_the_block_takes_one() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "item_used_on_block_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(false, |game| {
            install_criteria(game, &[1, 2], "minecraft:compass");
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:lodestone").unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.forced_hit = Some(hit());
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:compass", 2));
            // The block's turn takes a stick from the hand and passes; the
            // item's turn then consumes the click.
            *observer.0.lock().unwrap() = Controls {
                block_answer: Some(Answer::Pass),
                take_in_block: true,

                ..Controls::default()
            };
            assert!(use_block(game, false));

            assert_eq!(game.held().map(|stack| stack.count), Some(1));
            assert!(game.adv.progress.has("test:sticks2", "seen"));
            assert!(!game.adv.progress.has("test:sticks1", "seen"));
        });
    });
}

/// `ServerPlayerGameMode.useItemOn` hands the block the stack in hand after
/// the block phase's listeners: one that takes the last fire charge and lets
/// the click go on leaves TNT nothing to light it with.
#[test]
#[ignore = "daily"]
fn tnt_reads_the_hand_the_block_phase_listener_left() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "item_used_on_block_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(false, |game| {
            let tnt = game
                .pack
                .assets
                .registry
                .default_state("minecraft:tnt")
                .unwrap();
            game.world.set(0, 64, 0, tnt).unwrap();
            game.forced_hit = Some(hit());
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:fire_charge", 1));
            *observer.0.lock().unwrap() = Controls {
                take_in_block: true,
                ..Controls::default()
            };
            use_block(game, false);
            assert!(game.held().is_none(), "the listener took the fire charge");
            assert!(game.tnts.is_empty(), "nothing primed");
            assert_eq!(game.world.block(0, 64, 0), tnt);
        });
    });
}
