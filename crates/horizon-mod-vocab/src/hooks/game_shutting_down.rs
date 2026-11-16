//! Process shutdown hook.

pub trait Hooks {
    /// The game process is shutting down, after the client or dedicated server
    /// has stopped. Fires once on the process's game thread on an orderly exit,
    /// with the host for that side. NeoForge's `GameShuttingDownEvent`.
    #[hook(
        side = "both",
        contract = "listen",
        neoforge = "GameShuttingDownEvent",
        cancellable = false
    )]
    fn game_shutting_down();
}
