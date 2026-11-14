//! The console for native mods: logged lines reach `console_line`, and a command's feedback
//! comes back from `run_command_with_output`.
use super::native_guest_turn_tests::with_guests;
use horizon_client::logging;
use mod_api::{CommandResult, ConsoleLineEvent, Hooks, ServerHost};
use std::sync::{Arc, Mutex};

/// Records the probe lines it hears and logs a line of its own for each.
struct Mirror(Arc<Mutex<Vec<(String, String, String)>>>);

#[mod_api::__subscriptions]
impl Hooks for Mirror {
    fn console_line(&self, _: &mut dyn ServerHost, event: &mut ConsoleLineEvent<'_>) {
        if event.message.starts_with("console probe") {
            self.0.lock().unwrap().push((
                event.level.to_owned(),
                event.thread.to_owned(),
                event.message.to_owned(),
            ));
            log::info!("console probe echo");
        }
    }
}

#[test]
#[ignore = "daily"]
fn logged_lines_reach_a_listener_but_not_its_own() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "mirror",
        Box::new(Mirror(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            game.native_console_lines();
            logging::write_as("Server thread", log::Level::Warn, "console probe 1");
            logging::write_as("Server thread", log::Level::Info, "console probe 2");
            game.native_console_lines();
            game.native_console_lines();
            let heard = heard.lock().unwrap();
            assert_eq!(
                *heard,
                [
                    (
                        "WARN".into(),
                        "Server thread".into(),
                        "console probe 1".into()
                    ),
                    (
                        "INFO".into(),
                        "Server thread".into(),
                        "console probe 2".into()
                    ),
                ]
            );
        });
    });
}

/// The console's feedback comes back as plain text with the result of `run_command`. A command
/// that ran and failed is `ran: false` with its message, and one leading slash is dropped.
#[test]
#[ignore = "daily"]
fn a_captured_command_returns_its_feedback() {
    with_guests(|game| {
        let result = game.native_run_command_with_output("/time query gametime");
        assert!(result.success);
        assert_eq!(result.feedback.len(), 1, "{result:?}");
        assert!(
            result.feedback[0].starts_with("The game time is"),
            "{result:?}"
        );
        let result = game.native_run_command_with_output(" time query gametime ");
        assert!(result.success && result.feedback.len() == 1, "{result:?}");
        let result = game.native_run_command_with_output("gamerule nosuchrule");
        assert!(!result.success);
        assert!(!result.feedback.is_empty());
        assert!(game.chat_capture.is_none(), "the outer capture is put back");
        // The same line through `run_command`'s path answers as the captured one does.
        assert!(game.native_run_command("time query gametime"));
        assert!(!game.native_run_command("gamerule nosuchrule"));
    });
}

/// The dedicated console's own commands are not reached: they answer as a command that failed.
#[test]
#[ignore = "daily"]
fn the_dedicated_consoles_own_commands_are_not_reached() {
    with_guests(|game| {
        for line in ["stop", "op nobody", "whitelist list", "ban nobody"] {
            let result = game.native_run_command_with_output(line);
            assert!(!result.success, "{line}: {result:?}");
            assert!(!result.feedback.is_empty(), "{line}");
        }
    });
}

/// A blank line (empty, spaces, a lone slash, a slash and spaces) and no server open to players
/// run nothing and answer as `run_command` does: `ran` false and no lines.
#[test]
#[ignore = "daily"]
fn a_blank_line_or_no_server_runs_nothing() {
    with_guests(|game| {
        for blank in ["", "   ", "/", " / ", "/   "] {
            assert_eq!(
                game.native_run_command_with_output(blank),
                CommandResult::default(),
                "{blank:?}"
            );
        }
        assert!(game.chat_capture.is_none(), "nothing was captured");
        let host = game.lan.take();
        assert_eq!(
            game.native_run_command_with_output("time query gametime"),
            CommandResult::default()
        );
        assert!(!game.native_run_command("time query gametime"));
        game.lan = host;
        assert!(
            game.native_run_command_with_output("time query gametime")
                .success
        );
    });
}

/// The lines kept are capped: at most `MAX_FEEDBACK_LINES` lines, none above `MAX_FEEDBACK_LINE_BYTES` bytes, cut
/// at a character boundary (the line below ends in a three-byte character on the cap).
#[test]
fn the_feedback_kept_is_capped() {
    use horizon_client::text::Text;
    let long = Text::literal(format!(
        "ab{}",
        "\u{20ac}".repeat(super::native_console::MAX_FEEDBACK_LINE_BYTES)
    ));
    let many: Vec<Text> = std::iter::once(long)
        .chain(
            (1..super::native_console::MAX_FEEDBACK_LINES + 40)
                .map(|n| Text::literal(format!("line {n}"))),
        )
        .collect();
    let kept = super::native_console::capped_lines(&many);
    assert_eq!(kept.len(), 256);
    assert_eq!(super::native_console::MAX_FEEDBACK_LINE_BYTES, 4096);
    // "ab" and 1364 whole euro signs (4094 bytes) fit; a 1365th would pass 4096.
    assert_eq!(kept[0].len(), 2 + 1364 * 3);
    assert!(kept[0].starts_with("ab\u{20ac}"));
    assert_eq!(kept[1], "line 1");
    assert_eq!(kept[255], "line 255");
}

/// A call made while another capture is open captures its own lines, and the outer capture
/// keeps what it had and goes on.
#[test]
#[ignore = "daily"]
fn a_nested_capture_leaves_the_outer_one_as_it_was() {
    with_guests(|game| {
        let outer = game
            .chat_capture
            .replace(vec![horizon_client::text::Text::literal("outer line")]);
        let result = game.native_run_command_with_output("time query gametime");
        assert!(result.success && result.feedback.len() == 1, "{result:?}");
        let kept: Vec<String> = game
            .chat_capture
            .take()
            .unwrap()
            .iter()
            .map(|t| t.plain())
            .collect();
        assert_eq!(kept, ["outer line"]);
        game.chat_capture = outer;
    });
}
