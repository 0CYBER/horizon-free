//! `/execute store … bossbar` keeps the number it is given, and
//! `/bossbar set … name` compares components rather than their spelling:
//! `CustomBossEvent.setValue`/`setMax` store the value and clamp only the
//! derived progress, and `BossBarCommands.setName` uses `Component.equals`.
use super::*;
use crate::game::game_tests::tests::server_game;

fn game() -> Game<'static> {
    let resources = Box::leak(Box::new(None));
    let mut game = server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    game
}

/// Runs `command`: whether it succeeded and what the issuer was told.
fn run(game: &mut Game<'_>, command: &str) -> (bool, Vec<String>) {
    game.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said = game.chat_capture.take().unwrap_or_default();
    (
        ok,
        said.iter().map(horizon_client::text::Text::plain).collect(),
    )
}

/// The bar's stored `(value, max)`.
fn bar(game: &Game<'_>) -> (i32, i32) {
    let b = game.bosses.bars.get("demo:bar").expect("the bar");
    (b.value, b.max)
}

#[test]
#[ignore = "daily"]
fn bossbar_store_keeps_the_number_it_is_given() {
    let mut game = game();
    assert!(game.run_command("bossbar add demo:bar Demo"));
    assert!(game.run_command("bossbar set demo:bar max 10"));
    // CustomBossEvent.setValue stores the number and clamps only the progress.
    game.world.game_time = 50;
    assert!(
        game.run_command("execute store result bossbar demo:bar value run time query gametime")
    );
    assert_eq!(bar(&game), (50, 10), "value is not clamped to max");
    // `get` reads the stored number (BossBarCommands.getValue).
    let (ok, said) = run(&mut game, "bossbar get demo:bar value");
    assert!(ok);
    assert_eq!(said, ["Custom bossbar [Demo] has a value of 50"]);
    // CustomBossEvent.setMax stores the new max and leaves the value alone.
    game.world.game_time = 5;
    assert!(game.run_command("execute store result bossbar demo:bar max run time query gametime"));
    assert_eq!(bar(&game), (50, 5), "value survives a smaller max");
    // setMax does not raise a max below one to one.
    game.world.game_time = 0;
    assert!(game.run_command("execute store result bossbar demo:bar max run time query gametime"));
    assert_eq!(bar(&game), (50, 0));
}

#[test]
#[ignore = "daily"]
fn bossbar_name_compares_components_not_their_spelling() {
    let mut game = game();
    assert!(game.run_command("bossbar add demo:bar Demo"));
    // `{"text":"Demo"}` is the component `Demo`, so the name is unchanged
    // (BossBarCommands.setName compares with Component.equals).
    let (ok, said) = run(&mut game, r#"bossbar set demo:bar name {"text":"Demo"}"#);
    assert!(!ok, "an equal component is refused");
    assert_eq!(
        said,
        ["Nothing changed. That's already the name of this bossbar"]
    );
    // A different component still renames the bar.
    let (ok, _) = run(&mut game, r#"bossbar set demo:bar name {"text":"Other"}"#);
    assert!(ok);
}
