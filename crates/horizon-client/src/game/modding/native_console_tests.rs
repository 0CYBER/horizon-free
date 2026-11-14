//! The console for native mods: logged lines reach `console_line`, and a command's feedback
//! comes back from `server_run_command_captured`.
use super::native_guest_turn_tests::with_guests;
use horizon_client::logging;
use mod_api::{ConsoleLineEvent, ConsoleRefusal, Hooks, ServerHost};
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

/// The console's feedback comes back as plain text. A command that ran and failed is `Ok` with
/// `ran` false and its message, not a refusal, and one leading slash is dropped.
#[test]
#[ignore = "daily"]
fn a_captured_command_returns_its_feedback() {
    with_guests(|game| {
        let (ran, lines) = game
            .server_run_command_captured("/time query gametime")
            .unwrap();
        assert!(ran);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].starts_with("The game time is"), "{lines:?}");
        let (ran, lines) = game
            .server_run_command_captured(" time query gametime ")
            .unwrap();
        assert!(ran && lines.len() == 1, "{lines:?}");
        let (ran, lines) = game
            .server_run_command_captured("gamerule nosuchrule")
            .unwrap();
        assert!(!ran);
        assert!(!lines.is_empty());
        assert!(game.chat_capture.is_none(), "the outer capture is put back");
    });
}

/// The dedicated console's own commands are not reached: they answer as a command that failed.
#[test]
#[ignore = "daily"]
fn the_dedicated_consoles_own_commands_are_not_reached() {
    with_guests(|game| {
        for line in ["stop", "op nobody", "whitelist list", "ban nobody"] {
            let (ran, lines) = game.server_run_command_captured(line).unwrap();
            assert!(!ran, "{line}: {lines:?}");
            assert!(!lines.is_empty(), "{line}");
        }
    });
}

/// Each refusal is its own result and runs nothing: a blank line (empty, spaces, a lone slash,
/// a slash and spaces) and no server open to players.
#[test]
#[ignore = "daily"]
fn each_console_refusal_has_its_own_name_and_runs_nothing() {
    with_guests(|game| {
        for blank in ["", "   ", "/", " / ", "/   "] {
            assert_eq!(
                game.server_run_command_captured(blank),
                Err(ConsoleRefusal::Blank),
                "{blank:?}"
            );
        }
        assert!(game.chat_capture.is_none(), "nothing was captured");
        let host = game.lan.take();
        assert_eq!(
            game.server_run_command_captured("time query gametime"),
            Err(ConsoleRefusal::NotOpen)
        );
        assert_eq!(
            game.server_run_command_captured(""),
            Err(ConsoleRefusal::NotOpen),
            "no server is the first reason"
        );
        game.lan = host;
        assert!(
            game.server_run_command_captured("time query gametime")
                .is_ok()
        );
    });
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
        let (ran, lines) = game
            .server_run_command_captured("time query gametime")
            .unwrap();
        assert!(ran && lines.len() == 1, "{lines:?}");
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
