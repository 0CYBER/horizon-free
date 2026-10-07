//! `useWithoutItem` interactions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionResult {
    Success,
    Consume,
    Pass,
}
#[derive(Clone, Copy, Debug)]
pub struct PlayerContext {
    pub yaw: f32,
    pub may_build: bool,
}
