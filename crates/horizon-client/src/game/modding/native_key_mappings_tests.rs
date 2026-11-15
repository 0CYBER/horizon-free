//! A mod's registered mapping read through the client host, fed by the control socket as by the
//! window: a tap is one click and one `input_key` press, each repeat one more click, a screen or
//! overlay opening drops them at once, a binding's modifier must be held, a rebinding moves the
//! mapping, and a mouse press counts for a mapping on its button.
use super::event_loop::EventLoopCtl;
use super::{Game, Screen};
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use mod_api::prelude::ClientInputClientHost;
use mod_api::*;
use std::sync::Mutex;
use winit::event_loop::ControlFlow;

struct NoLoop;
impl EventLoopCtl for NoLoop {
    fn exit(&self) {}
    fn set_control_flow(&self, _: ControlFlow) {}
}

/// The `input_key` events heard, as `key action`.
static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Registers `zoom` on Z, `boost` on Shift + Z and `grab` on the left mouse button, and hears
/// `input_key`.
struct Keys;

#[mod_api::__subscriptions]
impl Hooks for Keys {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.register_key_mapping(KeyRegistration::new("zoom", "key.keyboard.z"));
        r.register_key_mapping(
            KeyRegistration::new("boost", "key.keyboard.z").modifiers(ClientModifiers::new(1)),
        );
        r.register_key_mapping(KeyRegistration::new("grab", "key.mouse.left"));
    }
    fn input_key(&self, _: &mut dyn ClientHost, event: &mut InputKeyEvent<'_>) {
        HEARD
            .lock()
            .unwrap()
            .push(format!("{} {:?}", event.key, event.action));
    }
}

fn down(game: &Game<'_>, id: &str) -> bool {
    ClientInputClientHost::is_down(game, "keys", id)
}
fn click(game: &mut Game<'_>, id: &str) -> bool {
    ClientInputClientHost::consume_click(game, "keys", id)
}
fn line(game: &mut Game<'_>, line: &str) {
    game.replay_line(line, &NoLoop);
}
fn heard() -> Vec<String> {
    std::mem::take(&mut *HEARD.lock().unwrap())
}

#[test]
#[ignore = "daily"]
fn a_control_socket_key_counts_as_a_window_key() {
    if run_fresh_cases(
        "game::native_key_mappings_tests::a_control_socket_key_counts_as_a_window_key",
        "HORIZON_MOD_KEYS_CHILD",
        &["game"],
    ) {
        return;
    }
    let mods: Mods = vec![ModEntry::new("keys", Box::new(Keys))].into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    with_client_game(|game| {
        game.apply_settings();
        game.set_screen(Screen::Playing);
        heard();

        // A tap: one click, consumed once; held only while down; `input_key` hears one press
        // and one release.
        line(game, "key z tap");
        assert!(!down(game, "zoom"));
        assert!(click(game, "zoom"));
        assert!(!click(game, "zoom"));
        assert_eq!(heard(), ["key.keyboard.z Press", "key.keyboard.z Release"]);
        // The window's tap takes the same route.
        let raw = horizon_client::keybinds::raw_key_code(winit::keyboard::PhysicalKey::Code(
            winit::keyboard::KeyCode::KeyZ,
        ));
        let z = Some("key.keyboard.z");
        game.key_input(z, z, raw, KeyAction::Press, 0);
        assert!(down(game, "zoom"));
        game.key_input(z, z, raw, KeyAction::Release, 0);
        assert!(!down(game, "zoom"));
        assert!(click(game, "zoom"));
        assert!(!click(game, "zoom"));
        assert_eq!(heard(), ["key.keyboard.z Press", "key.keyboard.z Release"]);

        // Held with two repeats: three clicks.
        line(game, "key z down");
        line(game, "key z repeat");
        line(game, "key z repeat");
        assert!(down(game, "zoom"));
        line(game, "key z up");
        assert!(!down(game, "zoom"));
        let clicks: Vec<bool> = (0..4).map(|_| click(game, "zoom")).collect();
        assert_eq!(clicks, [true, true, true, false]);
        assert_eq!(heard().len(), 4);

        // A screen opening drops the clicks at once, even closed again before anyone looks.
        line(game, "key z tap");
        game.set_screen(Screen::Inventory);
        assert!(!click(game, "zoom"));
        line(game, "key z tap");
        game.set_screen(Screen::Playing);
        assert!(!click(game, "zoom"), "pressed behind the screen");
        line(game, "key z down");
        game.set_screen(Screen::Inventory);
        game.set_screen(Screen::Playing);
        assert!(!down(game, "zoom"));
        assert!(!click(game, "zoom"));
        line(game, "key z up");
        // An overlay too.
        line(game, "key z tap");
        game.open_screen_overlay("link_prompt", |game| {
            game.link_prompt = Some("https://example.com".into());
        });
        game.link_prompt = None;
        assert!(!click(game, "zoom"));

        // Shift + Z counts only with shift held; Z alone counts with it too.
        line(game, "key z tap");
        assert!(!click(game, "boost"));
        line(game, "key shift down");
        line(game, "key z tap");
        line(game, "key shift up");
        assert!(click(game, "boost"));
        assert!(!click(game, "boost"));
        let clicks: Vec<bool> = (0..3).map(|_| click(game, "zoom")).collect();
        assert_eq!(clicks, [true, true, false]);

        // Rebound in Controls to X: Z no longer counts, X does.
        game.settings
            .mods
            .set_binding("keys", "zoom", "key.keyboard.x", 0);
        game.apply_settings();
        line(game, "key z tap");
        line(game, "key x tap");
        let clicks: Vec<bool> = (0..2).map(|_| click(game, "zoom")).collect();
        assert_eq!(clicks, [true, false]);

        // A mouse press counts for a mapping on its button.
        line(game, "mouse left click");
        assert!(click(game, "grab"));
        assert!(!click(game, "grab"));

        // Another mod's id, or an unregistered one, has nothing.
        assert!(!ClientInputClientHost::consume_click(game, "other", "zoom"));
        assert!(!click(game, "missing"));
    });
}
