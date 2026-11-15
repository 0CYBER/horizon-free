/// What the signal-reading glass saw: in `neighbor_changed`, whether it is powered and
/// how strongly, and whether its tick was scheduled; or that its tick ran.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: a test fixture's record, matched whole by the game's test
// no wire: testing-feature fixture state, not a mod boundary value.
pub enum SignalRead {
    /// `neighbor_changed`: `has_neighbor_signal`, `best_neighbor_signal`, the scheduling's answer.
    Changed([i32; 3], bool, i32, bool),
    /// `tick_block` ran at this position, reading the signal from its south side.
    Ticked([i32; 3], i32),
}
