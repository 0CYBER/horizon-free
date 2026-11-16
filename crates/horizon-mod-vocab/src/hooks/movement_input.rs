//! The `movement_input` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Local player movement input, after reading it and before applying it.
    /// All seven keys are independent; opposing direction keys cancel. If any
    /// direction key changes, its keyboard direction replaces axis edits and
    /// controller input. Otherwise axis edits stand and update packet flags.
    /// With enabled listeners, false sprint holds sprint off this tick (even a
    /// passive listener's unchanged false); true requests vanilla eligibility.
    /// Without enabled listeners, vanilla latch, double tap and input are unchanged.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "MovementInputUpdateEvent",
        version = 2,
        cancellable = false
    )]
    fn movement_input(
        forward: &mut f32,
        strafe: &mut f32,
        jump: &mut bool,
        sneak: &mut bool,
        forward_key: &mut bool,
        backward_key: &mut bool,
        left_key: &mut bool,
        right_key: &mut bool,
        sprint: &mut bool,
    );
}
