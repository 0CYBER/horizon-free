//! One `replace` listener source, built into the wasm fixture and into a native test, at the real
//! point of `horizon_client::game::beacon::average(a: u32, b: u32) -> u32`: the listener's value
//! reaches the caller on both targets, and the body never runs.

/// Owns the function: ten times the larger input, where the body averages them.
pub fn own_average(args: &mut mod_points::GameBeaconAverageArgs) -> u32 {
    args.a.max(args.b) * 10
}

/// Each call's inputs and the value the caller gets.
pub const CASES: [(u32, u32, u32); 2] = [(3, 7, 70), (9, 1, 90)];

/// Counts ordinary body execution in the native fixture.
pub static BODY_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
