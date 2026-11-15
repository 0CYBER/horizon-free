//! Admission for placement intents shares the actual connection's payload budget.
#[cfg(test)]
#[path = "settings_placement_tests.rs"]
mod tests;
use super::*;
impl Connection {
    pub fn receive_placement(
        &mut self,
        data: &[u8],
        now: Instant,
    ) -> Option<super::super::placement_sync::Frame> {
        use super::super::placement_sync::Frame;
        if !self.budget.accept(now) || !self.capability || self.authority.is_none() {
            return None;
        }
        match settings_codec::decode(data)? {
            Message::Placement(
                frame @ (Frame::Hello { .. } | Frame::Catalog { .. } | Frame::Place { .. }),
            ) => Some(frame),
            _ => None,
        }
    }
}
