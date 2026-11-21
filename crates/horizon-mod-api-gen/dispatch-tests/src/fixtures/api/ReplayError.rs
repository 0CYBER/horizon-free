#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayError {
    InvalidId,
    NotFound,
    Unreadable,
    NotOpen,
    InvalidSpeed,
    BackwardSeek,
    Unavailable,
    WorldBusy,
    Superseded,
}
