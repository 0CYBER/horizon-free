//! `CallFunction.execute` has no depth limit, only the command quota stops a
//! recursion; `execute if function #tag` runs the tag in an isolated frame
//! that a `return` discards (`ExecuteCommand.addIsolatedCall`, `Frame.discard`).
use super::*;
use horizon_client::datapack::FunctionLine;

fn add_function(game: &mut Game<'_>, id: &str, lines: &[&str]) {
    game.fns.store.functions.insert(
        id.into(),
        Function {
            id: id.into(),
            lines: lines
                .iter()
                .map(|l| FunctionLine::Command((*l).into()))
                .collect(),
            parameters: Vec::new(),
        },
    );
}

fn run(game: &mut Game<'_>, line: &str) {
    game.exec = ExecState::default();
    let source = game.player_source();
    let _ = game.execute_line(line, source);
}

fn score(game: &Game<'_>, holder: &str) -> Option<i32> {
    game.score.board.score(holder, "t").map(|s| s.value)
}

fn game() -> Game<'static> {
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    game.fns.loaded = true;
    run(&mut game, "scoreboard objectives add t dummy");
    game
}

#[test]
#[ignore = "daily"]
fn a_deep_recursion_runs_to_its_end_and_a_runaway_one_ends_at_the_quota_or_the_native_bound() {
    // The call nests on the native stack, which the thread that runs commands
    // reserves guest-sized; libtest's own thread is far too small for it.
    horizon_guest_stack::run_on_guest_stack(deep_recursion);
}

fn deep_recursion() {
    let mut game = game();
    add_function(
        &mut game,
        "t:countdown",
        &[
            "scoreboard players remove #n t 1",
            "execute if score #n t matches 1.. run function t:countdown",
        ],
    );
    run(&mut game, "scoreboard players set #n t 300");
    run(&mut game, "function t:countdown");
    assert_eq!(score(&game, "#n"), Some(0));

    // An unconditional recursion: on a test thread's stack it would overflow
    // long before the quota without the guest-sized stack.
    add_function(
        &mut game,
        "t:runaway",
        &["scoreboard players add #r t 1", "function t:runaway"],
    );
    // A quota below the native bound still stops the recursion.
    run(&mut game, "gamerule max_command_sequence_length 4000");
    run(&mut game, "function t:runaway");
    assert_eq!(score(&game, "#r"), Some(2000));

    // A quota above it stops at the native bound instead: 128 MiB of guest
    // stack at the 16 KiB per-level budget. Without that bound this reaches
    // 10000 (safe here), and the default 65536 would overflow the stack.
    run(&mut game, "scoreboard players set #r t 0");
    run(&mut game, "gamerule max_command_sequence_length 20000");
    run(&mut game, "function t:runaway");
    assert_eq!(score(&game, "#r"), Some(8192));
}

#[test]
#[ignore = "daily"]
fn a_function_tag_condition_stops_at_its_first_return() {
    let mut game = game();
    add_function(&mut game, "t:first", &["return 1"]);
    add_function(&mut game, "t:second", &["scoreboard players add #c t 1"]);
    add_function(
        &mut game,
        "t:failing",
        &["scoreboard players add #failing t 1", "return fail"],
    );
    add_function(&mut game, "t:third", &["scoreboard players add #third t 1"]);
    let tag = |ids: &[&str]| ids.iter().map(|id| ((*id).to_owned(), true, false)).collect();
    let tags = &mut game.fns.store.function_tags;
    tags.insert("t:p".into(), tag(&["t:first", "t:second"]));
    tags.insert("t:r".into(), tag(&["t:failing", "t:third"]));

    run(
        &mut game,
        "execute if function #t:p run scoreboard players set #ok t 1",
    );
    assert_eq!((score(&game, "#c"), score(&game, "#ok")), (None, Some(1)));

    // `return fail` discards the frame too, and its 0 is what `unless` tests.
    run(
        &mut game,
        "execute unless function #t:r run scoreboard players set #kept t 1",
    );
    assert_eq!(
        [
            score(&game, "#failing"),
            score(&game, "#third"),
            score(&game, "#kept")
        ],
        [Some(1), None, Some(1)]
    );
}
