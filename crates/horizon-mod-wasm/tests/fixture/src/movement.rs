//! Shared native/wasm movement cases, selected by independently held keys.
use mod_api::MovementInputEvent;

pub fn movement_input(event: &mut MovementInputEvent) {
    if event.right_key {
        event.forward_key = true;
        event.sprint = false;
    } else if event.left_key {
        if !(event.forward_key && event.backward_key) {
            event.forward = -0.5;
        }
        event.strafe = 0.25;
        event.jump = !event.jump;
        event.sneak = !event.sneak;
        event.sprint = true;
    } else if event.backward_key {
        event.forward_key = true;
        event.left_key = true;
        event.right_key = true;
        // Direction-key edits must win over these conflicting axis edits.
        event.forward = 0.25;
        event.strafe = 0.5;
        event.sprint = true;
    } else {
        event.forward_key = true;
        event.sprint = true;
    }
}
