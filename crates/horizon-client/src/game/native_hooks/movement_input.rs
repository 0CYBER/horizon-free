use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Local player movement input, after reading it and before applying it.
    pub(in crate::game) fn native_movement_input(
        &self,
        call: mod_api::__private::MovementInputCall<'_>,
    ) {
        self.fire_movement_input(call);
    }
}

impl Game<'_> {
    /// Apply the listener's keys or independent axes before local movement and
    /// packet construction. A key edit wins over all axis edits, including a
    /// controller's old vector; opposing keys remain independently observable.
    pub(in crate::game) fn mod_movement_keys(
        &self,
        mut keys: horizon_client::sprint::Keys,
    ) -> horizon_client::sprint::Keys {
        if !mod_api::__private::enabled_listened(mod_api::HookId::MovementInput) {
            return keys;
        }
        let original_keys = [keys.forward, keys.backward, keys.left, keys.right];
        let [mut strafe, mut forward] = keys.direction();
        let original_axes = [strafe, forward];
        self.native_movement_input(mod_api::__private::MovementInputCall {
            forward: &mut forward,
            strafe: &mut strafe,
            jump: &mut keys.jump,
            sneak: &mut keys.shift,
            forward_key: &mut keys.forward,
            backward_key: &mut keys.backward,
            left_key: &mut keys.left,
            right_key: &mut keys.right,
            sprint: &mut keys.sprint,
        });
        if [keys.forward, keys.backward, keys.left, keys.right] != original_keys {
            // KeyboardInput.calculateImpulse and tick: discard the stale
            // analog vector and derive the normalized direction from the keys.
            keys.analog = None;
        } else if [strafe, forward] != original_axes {
            if forward != original_axes[1] {
                keys.forward = forward > 0.0;
                keys.backward = forward < 0.0;
            }
            if strafe != original_axes[0] {
                keys.left = strafe > 0.0;
                keys.right = strafe < 0.0;
            }
            keys.analog = Some([strafe, forward]);
        }
        // A subscribed listener owns sprint for this tick. Without one, the
        // vanilla latch and double-tap behavior above are left untouched.
        keys.sprint_suppressed = !keys.sprint;
        keys
    }

    #[cfg(test)]
    pub(in crate::game) fn movement_dispatch_fixture(&self, fire: bool) -> f32 {
        let mut keys = horizon_client::sprint::Keys::default();
        let (mut forward, mut strafe) = (1.0, 0.0);
        let call = mod_api::__private::MovementInputCall {
            forward: &mut forward,
            strafe: &mut strafe,
            jump: &mut keys.jump,
            sneak: &mut keys.shift,
            forward_key: &mut keys.forward,
            backward_key: &mut keys.backward,
            left_key: &mut keys.left,
            right_key: &mut keys.right,
            sprint: &mut keys.sprint,
        };
        if fire {
            self.native_movement_input(call);
        } else {
            mod_api::dispatch_movement_input(self, call);
        }
        forward
    }
}
