//! `ItemEntity.areMergable` and `merge` on stacks with their own
//! `max_stack_size`: dropped items merge up to `ItemStack.getMaxStackSize`,
//! which is the stack's own size and not the item's default.
use super::native_machines::tests::server_game;
use super::*;
use horizon_core::nbt::Tag;

/// Stone with its own `max_stack_size`, or with the component removed
/// (`!minecraft:max_stack_size`, vanilla's `getOrDefault` default of one).
fn stone(size: Option<i32>, count: u32) -> ItemStack {
    let mut stack = ItemStack::new("minecraft:stone", count);
    let data = stack.data_mut();
    match size {
        Some(n) => data.set_other("minecraft:max_stack_size", Some(Tag::Int(n))),
        None => data.set_other(
            "!minecraft:max_stack_size",
            Some(Tag::Compound(Default::default())),
        ),
    };
    stack.tidy();
    stack
}

/// `ItemEntity.areMergable` and `merge`: two stacks that do not fit one stack
/// of their own size stay apart, and the rest merge.
#[test]
fn items_merge_only_up_to_their_own_max_stack_size() {
    if std::env::var_os("HORIZON_TEST_RESOURCES").is_none() {
        return;
    }
    let mut game = server_game();
    let counts = |game: &Game<'_>| {
        let mut counts: Vec<u32> = game.dropped.iter().map(|item| item.stack.count).collect();
        counts.sort_unstable();
        counts
    };
    let at = [0.5, 70.0, 0.5];
    // `Entity.baseTick` counts the tick before `ItemEntity.tick` merges, so an
    // item merges on its second tick at the earliest: `tickCount % 2` for an
    // item that moved between blocks, as these falling ones do.
    let spawn = |game: &mut Game<'_>, stacks: &[ItemStack]| {
        game.dropped.clear();
        for stack in stacks {
            game.spawn_item(stack.clone(), at, [0.0; 3], 0);
            game.dropped.last_mut().unwrap().ticks = 1;
        }
        game.ticked_entities_through = game.next_entity;
    };
    // Two and one of a stone that holds four fit one stack, so they merge.
    spawn(&mut game, &[stone(Some(4), 2), stone(Some(4), 1)]);
    game.tick_items();
    assert_eq!(counts(&game), [3]);
    // Two and three add up to five, so `areMergable` refuses.
    spawn(&mut game, &[stone(Some(4), 2), stone(Some(4), 3)]);
    game.tick_items();
    assert_eq!(counts(&game), [2, 3]);
    // A removed component holds one, so two of them stay apart as well.
    spawn(&mut game, &[stone(None, 1), stone(None, 1)]);
    game.tick_items();
    assert_eq!(counts(&game), [1, 1]);
}
