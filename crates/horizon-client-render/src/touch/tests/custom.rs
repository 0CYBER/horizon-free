//! Custom buttons and the inventory's shift and mouse buttons.
use super::*;
use crate::touch::custom::UNASSIGNED;

fn with_button(key: &str) -> TouchControls {
    let mut t = TouchControls::default();
    t.viewport(1920, 1080);
    t.settings
        .custom_buttons
        .push(CustomButton::new(key.into(), 0));
    t
}

#[test]
fn inventory_shift_and_mouse_hold_independently_and_cancel_cleanly() {
    let mut t = TouchControls {
        inventory_mode: true,
        ..TouchControls::default()
    };
    for (id, control, token) in [
        (1, Control::Sneak, "key.keyboard.left.shift"),
        (2, Control::Attack, "action:key.attack"),
        (3, Control::Use, "action:key.use"),
    ] {
        assert!(t.visible(control));
        assert!(t.inventory_update(id, t.placement(control).center, true, false));
        assert_eq!(t.key_events(), [(token.into(), true)]);
    }
    // Elsewhere the touch belongs to the inventory.
    assert!(!t.inventory_update(4, [0.5, 0.5], true, false));
    t.clear_world_gesture();
    assert_eq!(t.held_custom_tokens().count(), 3);
    assert_eq!(t.poll_state(), PadState::default());
    assert!(t.inventory_update(2, [0.0, 0.0], false, true));
    assert_eq!(t.key_events(), [("action:key.attack".into(), false)]);
    t.clear();
    let released = t.key_events();
    assert!(released.contains(&("key.keyboard.left.shift".into(), false)));
    assert!(released.contains(&("action:key.use".into(), false)));
    assert_eq!(released.len(), 2);
}

#[test]
fn a_new_finger_on_a_custom_button_presses_it_with_the_inventory_open() {
    let mut t = with_button("key.keyboard.z");
    t.inventory_mode = true;
    let p = t.custom_placement(0).center;
    assert!(t.inventory_update(1, p, true, false));
    assert_eq!(t.key_events(), [("key.keyboard.z".into(), true)]);
    assert!(t.inventory_update(1, p, false, false));
    assert!(t.key_events().is_empty());
    assert!(t.inventory_update(1, p, false, true));
    assert_eq!(t.key_events(), [("key.keyboard.z".into(), false)]);
    assert_eq!(t.held_custom_tokens().count(), 0);
}

#[test]
fn the_inventory_keeps_the_players_visibility_and_placements() {
    let mut t = TouchControls::default();
    for mode in Mode::ALL {
        t.settings.mode = mode;
        t.settings.button_visibility[Control::Attack as usize] = layout::HIDDEN;
        let before: Vec<_> = Control::ALL
            .into_iter()
            .map(|c| (c, t.visible(c), t.placement(c).center))
            .collect();
        t.inventory_mode = true;
        for (control, visible, center) in before {
            assert_eq!(t.visible(control), visible);
            assert_eq!(t.placement(control).center, center);
        }
        t.inventory_mode = false;
    }
}

#[test]
fn an_unassigned_button_sends_nothing_and_falls_through() {
    let mut t = with_button(UNASSIGNED);
    let point = t.custom_placement(0).center;
    assert!(t.control_at_point(point));
    t.update(1, point, true, false);
    assert!(t.key_events().is_empty());
    assert!(matches!(role(&t, 1), Role::Look { .. }));
    assert_eq!(
        t.settings.custom_buttons[0].label(),
        crate::lang::get("horizon.touch.button")
    );
}

#[test]
fn custom_keys_hold_through_motion_and_release_after_the_last_finger() {
    let mut t = with_button("key.keyboard.z");
    let p = t.custom_placement(0).center;
    t.update(1, p, true, false);
    t.update(2, p, true, false);
    assert!(t.clicked && t.owns_pointer(1));
    assert_eq!(t.key_events(), [("key.keyboard.z".into(), true)]);
    // Sliding off the button neither turns the camera nor releases the key.
    assert_eq!(t.update(1, [0.99, 0.1], false, false), [0.0; 2]);
    assert!(t.key_events().is_empty());
    t.update(1, p, false, true);
    assert!(t.key_events().is_empty());
    t.update(2, p, false, true);
    assert_eq!(t.key_events(), [("key.keyboard.z".into(), false)]);
    assert_eq!(t.held_custom_tokens().count(), 0);
}

#[test]
fn clear_releases_custom_keys_and_scaling_keeps_saved_sizes() {
    let mut t = with_button("action:key.jump");
    let stored = t.settings.custom_buttons[0].layouts[0].size;
    t.settings.button_scale = 2.0;
    assert_eq!(t.custom_placement(0).size, stored * 2.0);
    t.update(1, t.custom_placement(0).center, true, false);
    assert_eq!(t.key_events(), [("action:key.jump".into(), true)]);
    t.clear();
    assert_eq!(t.key_events(), [("action:key.jump".into(), false)]);
    assert_eq!(t.settings.custom_buttons[0].layouts[0].size, stored);
}

#[test]
fn custom_buttons_mirror_win_over_built_ins_and_count_as_shortcuts() {
    let mut t = with_button("key.keyboard.z");
    // Put it on top of the jump button: the custom button wins.
    t.settings.custom_buttons[0].layouts[0].center = t.settings.placement(Control::Jump).center;
    let p = t.custom_placement(0).center;
    t.update(1, p, true, false);
    assert!(!t.state().held(crate::gamepad::Button::South));
    assert!(t.auxiliary_at(p));
    t.clear();
    t.settings.left_handed = true;
    assert!((t.custom_placement(0).center[0] - (1.0 - p[0])).abs() < 1e-5);
}

#[test]
fn saved_buttons_survive_and_bad_ones_are_dropped() {
    let mut s = Settings::default();
    s.custom_buttons
        .push(CustomButton::new(UNASSIGNED.into(), 0));
    s.custom_buttons
        .push(CustomButton::new("action:key.jump".into(), 1));
    s.custom_buttons
        .push(CustomButton::new("action:no-such-action".into(), 2));
    s.custom_buttons
        .push(CustomButton::new("key.keyboard.no-such-key".into(), 3));
    s.custom_buttons[1].layouts[2].size = f32::NAN;
    s.sanitize();
    assert_eq!(s.custom_buttons.len(), 2);
    assert_eq!(s.custom_buttons[1].layouts[2].size, 0.15);
    let read: Settings = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
    assert_eq!(read.custom_buttons, s.custom_buttons);
    s.custom_buttons = (0..40)
        .map(|i| CustomButton::new("key.keyboard.z".into(), i))
        .collect();
    s.sanitize();
    assert_eq!(s.custom_buttons.len(), 32);
    // Files saved before custom buttons existed have none.
    let old: Settings = serde_json::from_str(r#"{"mode":"tap"}"#).unwrap();
    assert!(old.custom_buttons.is_empty());
}

#[test]
fn custom_buttons_are_drawn() {
    let a = crate::hud::HudAtlas::build(&crate::resource_source::ResourceSource::directory(
        std::path::PathBuf::from("/nonexistent/horizon-touch-custom"),
    ))
    .unwrap();
    let count = |t: &TouchControls| {
        let mut b = crate::hud::HudBatch::new(
            [1920, 1080],
            crate::hud::GuiScale {
                requested: 0,
                unicode: false,
            },
        );
        t.draw(&mut b, &a);
        b.vertices.len()
    };
    let mut t = TouchControls::default();
    let without = count(&t);
    t.settings
        .custom_buttons
        .push(CustomButton::new("key.keyboard.z".into(), 0));
    assert!(count(&t) > without);
    t.settings.custom_buttons[0].layouts[0].opacity = 0.0;
    assert_eq!(count(&t), without);
}
