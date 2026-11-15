//! A controller action does what its keyboard key does, on a real `Game` without a window.
//! What needs a renderer (inventory slots, screenshots) is covered by tools/pad_parity_e2e.py.

use super::super::{Audio, Game, Options, Progress, Screen, Settings, load_resources};
use horizon_client::actions::Action;
use horizon_client::command::args::GameMode;
use horizon_client::gamepad::{Button, PadState};
use horizon_client::held::Source;
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// A game at play with no world: input reaches the mappings, as in `Screen::Playing`.
use crate::game::execution_role::ExecutionRole;

fn playing(test: impl FnOnce(&mut Game)) {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!("no menu handoff"),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
    )
    .unwrap();
    game.title = None;
    game.screen = Screen::Playing;
    test(&mut game);
}

/// No window loop runs in these gameplay input tests.
struct NoEventLoop;

impl crate::game::event_loop::EventLoopCtl for NoEventLoop {
    fn exit(&self) {
        unreachable!("a gameplay pad press does not exit the event loop");
    }

    fn set_control_flow(&self, _: winit::event_loop::ControlFlow) {
        unreachable!("a gameplay pad press does not change the event loop");
    }
}

/// Hold `button` through the real press dispatcher, including its action ownership.
fn pad_press(game: &mut Game, button: Button, action: &str) {
    assert!(
        game.settings
            .controller
            .actions_of(button)
            .contains(&action)
    );
    game.pad.virtual_input = true;
    let mut state = PadState::default();
    // A trigger is held by how far it's pulled, not by a button bit.
    match button {
        Button::LeftTrigger => state.triggers[0] = 1.0,
        Button::RightTrigger => state.triggers[1] = 1.0,
        _ => state.set(button, true),
    }
    game.pad.edges.update(&state, 0.4);
    game.pad_button(&NoEventLoop, button, true).unwrap();
}

/// Release the sole held test button through the same dispatcher as its press.
fn pad_release(game: &mut Game, button: Button) {
    game.pad.edges.update(&PadState::default(), 0.4);
    game.pad_button(&NoEventLoop, button, false).unwrap();
}

#[test]
#[ignore = "daily"]
fn a_keyboard_release_keeps_the_sneak_the_pad_went_on_to_hold() {
    playing(|game| {
        // Hold mode: toggling, the pad's press would flip it.
        game.settings.controller.toggle_sneak = false;
        let shift = KeyCode::ShiftLeft;
        game.mapped_key(shift, true, false, shift, Source::Keyboard)
            .unwrap();
        pad_press(game, Button::RightStick, "key.sneak");
        game.mapped_key(shift, false, false, shift, Source::Keyboard)
            .unwrap();
        assert!(game.keys.held(Action::Sneak), "R3 is still down");
    });
}

#[test]
#[ignore = "daily"]
fn keyboard_handoff_keeps_what_the_pad_holds() {
    playing(|game| {
        game.mapped_key(KeyCode::KeyW, true, false, KeyCode::KeyW, Source::Keyboard)
            .unwrap();
        pad_press(game, Button::South, "key.jump");
        assert!(game.keys.held(Action::Jump) && game.keys.held(Action::Forward));
        // Splitscreen: keyboard and mouse move to another player.
        game.release_keyboard_mouse();
        assert!(!game.keys.held(Action::Forward), "the keyboard's walk ends");
        assert!(
            game.keys.held(Action::Jump),
            "the pad still holds jump; only the keyboard and mouse left"
        );
    });
}

#[test]
#[ignore = "daily"]
fn pad_smooth_camera_binding_toggles_the_cinematic_camera() {
    playing(|game| {
        game.settings
            .controller
            .bind("key.smoothCamera", Some(Button::Select));
        assert!(!game.smooth_camera);
        game.pad_action("key.smoothCamera", true).unwrap();
        assert!(game.smooth_camera, "as its key does in play");
    });
}

/// A player at the origin: the wheel's gates ask for one.
fn with_player(game: &mut Game) {
    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
}

#[test]
#[ignore = "daily"]
fn bumpers_step_the_hotbar_where_the_wheel_does() {
    playing(|game| {
        game.selected = 8;
        game.pad_action("pad.hotbarNext", true).unwrap();
        assert_eq!(game.selected, 8, "no player, no hotbar");
        with_player(game);
        game.pad_action("pad.hotbarNext", true).unwrap();
        assert_eq!(game.selected, 0, "wraps past the last slot");
        game.pad_action("pad.hotbarPrevious", true).unwrap();
        assert_eq!(game.selected, 8, "and back");
        game.pad_action("pad.hotbarNext", false).unwrap();
        assert_eq!(game.selected, 8, "a release does nothing");
        game.screen = Screen::Paused;
        game.pad_action("pad.hotbarNext", true).unwrap();
        assert_eq!(game.selected, 8, "a screen keeps the wheel from the hotbar");
    });
}

#[test]
#[ignore = "daily"]
fn a_spectators_bumpers_set_the_flying_speed_as_the_wheel_does() {
    playing(|game| {
        with_player(game);
        game.mode = GameMode::Spectator;
        game.selected = 3;
        let speed = game.player.as_ref().unwrap().flying_speed;
        game.pad_action("pad.hotbarPrevious", true).unwrap();
        // One wheel notch up: `MouseHandler.onScroll` adds 0.005.
        let after = game.player.as_ref().unwrap().flying_speed;
        assert!((after - speed - 0.005).abs() < 1e-6, "{speed} -> {after}");
        assert_eq!(game.selected, 3, "a spectator's hotbar doesn't step");
    });
}

#[test]
#[ignore = "daily"]
fn readers_of_the_movement_keys_see_the_stick() {
    playing(|game| {
        game.focused = true;
        assert!(!game.mapping("key.forward").unwrap().unwrap().down);
        // Up and to the left, on a diagonal: two keys, as a keyboard would hold.
        game.pad.state.left = [-0.7, 0.7];
        assert!(game.mapping("key.forward").unwrap().unwrap().down);
        assert!(game.mapping("key.left").unwrap().unwrap().down);
        assert!(!game.mapping("key.back").unwrap().unwrap().down);
        assert!(!game.mapping("key.right").unwrap().unwrap().down);
        assert!(
            !game.mapping("key.jump").unwrap().unwrap().down,
            "the stick holds only the movement keys"
        );
        // A screen takes the stick away.
        game.screen = Screen::Paused;
        assert!(!game.mapping("key.forward").unwrap().unwrap().down);
    });
}

#[test]
#[ignore = "daily"]
fn pad_number_actions_select_the_hotbar_as_their_keys_do() {
    playing(|game| {
        with_player(game);
        game.settings
            .controller
            .bind("key.hotbar.3", Some(Button::DPadUp));
        // This tests the one-shot number action, not all actions sharing D-pad up
        // (the default chat binding would open a screen before the number action).
        game.pad_action("key.hotbar.3", true).unwrap();
        assert_eq!(game.hotbar_presses.entries(), [2]);
    });
}

#[test]
#[ignore = "daily"]
fn a_held_pad_toolbar_action_counts_as_the_toolbar_key() {
    playing(|game| {
        game.settings
            .controller
            .bind("key.saveToolbarActivator", Some(Button::DPadUp));
        assert!(!game.mapped_key_down("key.saveToolbarActivator"));
        pad_press(game, Button::DPadUp, "key.saveToolbarActivator");
        assert!(game.mapped_key_down("key.saveToolbarActivator"));
        assert!(!game.mapped_key_down("key.loadToolbarActivator"));
    });
}

#[test]
#[ignore = "daily"]
fn the_pads_attack_and_use_reach_what_the_mouse_tells_mods() {
    playing(|game| {
        assert_eq!(game.native_clicks(), (0, 0));
        pad_press(game, Button::RightTrigger, "key.attack");
        assert!(game.mapping("key.attack").unwrap().unwrap().down);
        assert_eq!(game.native_clicks(), (1, 0));
        pad_release(game, Button::RightTrigger);
        assert!(!game.mapping("key.attack").unwrap().unwrap().down);
        pad_press(game, Button::LeftTrigger, "key.use");
        assert!(game.mapping("key.use").unwrap().unwrap().down);
        assert_eq!(game.native_clicks(), (1, 1));
    });
}

#[test]
#[ignore = "daily"]
fn the_mouse_letting_go_leaves_what_the_pad_holds_and_back() {
    playing(|game| {
        game.mouse_mapping(MouseButton::Left, true, Source::Keyboard)
            .unwrap();
        pad_press(game, Button::RightTrigger, "key.attack");
        game.mouse_mapping(MouseButton::Left, false, Source::Keyboard)
            .unwrap();
        assert!(game.left, "the pad still holds the attack");
        // The pad let go, so its edges are no longer held either.
        pad_release(game, Button::RightTrigger);
        assert!(!game.left);

        // The other way round: the mouse button is physically down.
        game.physical_buttons.insert(MouseButton::Right);
        game.mouse_mapping(MouseButton::Right, true, Source::Keyboard)
            .unwrap();
        pad_press(game, Button::LeftTrigger, "key.use");
        pad_release(game, Button::LeftTrigger);
        assert!(game.right, "the mouse still holds the use");
        game.physical_buttons.remove(&MouseButton::Right);
        game.mouse_mapping(MouseButton::Right, false, Source::Keyboard)
            .unwrap();
        assert!(!game.right);
    });
}

#[test]
#[ignore = "daily"]
fn a_release_from_a_screen_can_not_strand_the_pads_attack() {
    playing(|game| {
        // The mouse went down in play, and came up where the mappings didn't hear it.
        game.mouse_mapping(MouseButton::Left, true, Source::Keyboard)
            .unwrap();
        game.left = false;
        pad_press(game, Button::RightTrigger, "key.attack");
        assert!(game.left);
        pad_release(game, Button::RightTrigger);
        assert!(!game.left, "nothing is physically held");
    });
}

#[test]
#[ignore = "daily"]
fn keyboard_handoff_keeps_the_attack_the_pad_holds() {
    playing(|game| {
        pad_press(game, Button::RightTrigger, "key.attack");
        game.release_keyboard_mouse();
        assert!(game.left);
    });
}
