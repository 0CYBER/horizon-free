//! `LivingEntity.updatingUsingItem` and `releaseUsingItem` go on only while the
//! hand holds the item the use began with (`ItemStack.isSameItem`), as arismoko/horizon#3211 asks.
use super::item_use::interact_vibration_tests::with_game;
use super::*;

const FINISH: &str = "G 10,64,-4 minecraft:item_interact_finish";

/// Starts eating a golden apple with the use button held and 64 arrows in the
/// inventory, runs `ticks` ticks of the use, then `f`. Returns the interaction
/// vibrations posted after the start's.
fn eating(ticks: u32, f: impl FnOnce(&mut Game<'_>)) -> Vec<String> {
    let lines = with_game("minecraft:golden_apple", |game| {
        game.creative = false;
        game.build_registries();
        game.ui.registries.permission = 4;
        game.screen = Screen::Playing;
        game.right = true;
        game.inventory[9] = Some(ItemStack::new("minecraft:arrow", 64));
        assert!(game.start_using_item());
        for _ in 0..ticks {
            game.tick_using();
        }
        assert_eq!(game.using, Some((game.selected, ticks)));
        f(game);
    })
    .expect("prepared resources");
    let mut lines: Vec<_> = lines
        .into_iter()
        .filter(|l| l.contains("item_interact"))
        .collect();
    assert_eq!(lines.remove(0), "G 10,64,-4 minecraft:item_interact_start");
    lines
}

fn arrows(game: &Game<'_>) -> u32 {
    game.inventory[9].as_ref().map_or(0, |s| s.count)
}

#[test]
#[ignore = "daily"]
fn a_bow_put_in_place_of_the_apple_stops_the_use() {
    let lines = eating(5, |game| {
        *game.held_mut() = Some(ItemStack::new("minecraft:bow", 1));
        game.tick_using();
        assert!(game.using.is_none());
        game.right = false;
        assert!(!game.release_using_item());
        assert_eq!(arrows(game), 64, "no arrow is fired");
    });
    assert_eq!(lines, [FINISH]);
}

#[test]
#[ignore = "daily"]
fn replacing_the_apple_with_air_stops_the_use() {
    let lines = eating(5, |game| {
        assert!(game.run_command("item replace entity @s weapon.mainhand with air"));
        game.tick_using();
        assert!(game.using.is_none());
    });
    assert_eq!(lines, [FINISH]);
}

#[test]
#[ignore = "daily"]
fn clearing_the_apple_ends_the_use_silently() {
    let lines = eating(5, |game| {
        assert!(game.run_command("clear @s minecraft:golden_apple"));
        game.tick_using();
        assert!(game.using.is_none());
    });
    assert!(lines.is_empty(), "{lines:?}");
}

#[test]
#[ignore = "daily"]
fn another_apple_stack_goes_on_with_the_use() {
    eating(5, |game| {
        assert!(game.run_command("item replace entity @s weapon.mainhand with golden_apple 3"));
        for _ in 5..32 {
            assert!(game.using.is_some());
            game.tick_using();
        }
        assert!(game.using.is_none());
        assert_eq!(game.held().map(|s| s.count), Some(2));
    });
}

#[test]
#[ignore = "daily"]
fn releasing_right_after_a_bow_replaced_the_apple_fires_nothing() {
    let lines = eating(20, |game| {
        *game.held_mut() = Some(ItemStack::new("minecraft:bow", 1));
        game.right = false;
        assert!(game.release_using_item());
        assert!(game.using.is_none());
        assert_eq!(arrows(game), 64, "no arrow is fired");
    });
    assert_eq!(lines, [FINISH]);
}

#[test]
#[ignore = "daily"]
fn a_command_putting_the_apple_back_keeps_the_use() {
    let lines = eating(5, |game| {
        // A mod's raw write puts a bow in; the command puts an apple back
        // before the next tick: `useItem` is still the apple, so the use goes on.
        *game.held_mut() = Some(ItemStack::new("minecraft:bow", 1));
        assert!(game.run_command("item replace entity @s weapon.mainhand with golden_apple"));
        game.tick_using();
        assert_eq!(game.using, Some((game.selected, 6)));
    });
    assert!(lines.is_empty(), "{lines:?}");
}
