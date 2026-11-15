//! `execute if items entity <dropped item> contents ...`: `ItemEntity.getSlot`.
use super::*;
use horizon_client::items::ItemStack;

/// `ItemEntity.getSlot(0)` returns its stack (`SlotAccess.of(this::getItem,
/// this::setItem)`), and `SlotRanges` maps `contents`, `container.0` and
/// `hotbar.0` to slot 0, so a dropped item's `contents` is the stack it holds.
#[test]
#[ignore = "daily"]
fn a_dropped_item_entity_holds_its_stack_in_contents() {
    use crate::game::game_tests::tests::server_game;
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    game.set_allow_commands(true);
    assert!(game.run_command("scoreboard objectives add held dummy"));
    game.spawn_item(
        ItemStack::new("minecraft:elytra", 1),
        [4.5, 200.0, 4.5],
        [0.0, 0.0, 0.0],
        0,
    );
    let held = |game: &Game<'_>| {
        game.score
            .board
            .score("#seen", "held")
            .map(|s| s.value)
            .unwrap_or(0)
    };
    assert!(game.run_command(
        "execute as @e[type=item] if items entity @s contents * run scoreboard players add #seen held 1"
    ));
    assert_eq!(held(&game), 1, "a dropped item's contents is its stack");
    assert!(game.run_command(
        "execute as @e[type=item] unless items entity @s contents minecraft:diamond run scoreboard players add #seen held 1"
    ));
    assert_eq!(held(&game), 2, "the stack is the elytra, not a diamond");
    assert!(game.run_command(
        "execute as @e[type=item] if items entity @s contents minecraft:elytra run scoreboard players add #seen held 1"
    ));
    assert_eq!(held(&game), 3);
    // A count predicate reads the stack's count (one elytra, not two).
    assert!(
        game.run_command(
            "execute as @e[type=item] unless items entity @s contents minecraft:elytra[count=2] run scoreboard players add #seen held 1"
        )
    );
    assert_eq!(held(&game), 4);
}
