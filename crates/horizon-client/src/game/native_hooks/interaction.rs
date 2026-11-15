use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) struct InteractionDecision {
    pub canceled: bool,
    pub swing: bool,
}

impl InteractionDecision {
    pub fn canceled_swing(self) -> Option<bool> {
        self.canceled.then_some(self.swing)
    }
}

impl Game<'_> {
    /// An interaction key acts on what the crosshair points at
    /// (`InputEvent.InteractionKeyMappingTriggered`), while playing: `key` says which.
    /// The attack fires when an attack is allowed and again each tick a held attack
    /// key keeps digging a block; the use key fires once for each hand it tries, the
    /// main hand first; pick block fires only with something aimed at. Set `canceled`
    /// to skip the action: a cancelled use also skips the other hand. A cancelled
    /// attack or use still swings `hand` unless the mod clears `swing_hand` (it
    /// starts true). A window client only, including single player; a mod
    /// that claims an item's use (`mod_use_start`) sees the event first.
    /// Uncancelled attacks and uses also honour false by suppressing natural swings.
    /// Pick only vetoes its packet and never swings.
    pub(in crate::game) fn native_interaction_key_mapping_triggered(
        &mut self,
        key: InteractionKey,
        hand: Hand,
        attempt: InteractionAttempt,
        swing_hand: &mut bool,
        canceled: &mut bool,
    ) {
        self.fire_interaction_key_mapping_triggered(key, hand, attempt, swing_hand, canceled);
    }
    /// Preserve cancellation and swing independently for both native and wasm dispatch.
    pub(in crate::game) fn native_interaction(
        &mut self,
        key: InteractionKey,
        hand: Hand,
        attempt: InteractionAttempt,
    ) -> InteractionDecision {
        let (mut swing, mut canceled) = (true, false);
        self.native_interaction_key_mapping_triggered(
            key,
            hand,
            attempt,
            &mut swing,
            &mut canceled,
        );
        let swing = match key {
            InteractionKey::PickBlock => false,
            _ => swing,
        };
        if key != InteractionKey::PickBlock
            && let Some(remote) = self.remote.as_mut()
        {
            remote.interaction_swing[usize::from(hand == Hand::Off)] = swing;
        }
        InteractionDecision { canceled, swing }
    }
}
