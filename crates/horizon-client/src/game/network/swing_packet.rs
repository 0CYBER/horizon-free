use super::*;

impl Game<'_> {
    /// A locally requested swing sends Punch only when the item's Part permits it.
    pub(super) fn remote_swing_packet(&mut self) {
        if self.try_swing_hand(self.hand, SwingAnimation::DEFAULT) {
            self.send(horizon_protocol::packets::play::Serverbound::Punch(
                horizon_protocol::packets::Empty,
            ));
        }
    }
}
