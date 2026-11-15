//! The `client_stopping` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The client is closing: it fires once per game process, on the render thread,
    /// when the game shuts down, after the world the player was in is left and
    /// saved, while the game still holds its settings and resources, so a listener
    /// can read them through the `ClientHost` and save its own client state. It is the closing counterpart of `client_setup`.
    /// Closing the window, a quit shortcut and the process's stop signals all reach
    /// it; a crash does not. A dedicated server has no client and never fires it
    /// (`server_stopping` is its stop), and neither does a splitscreen guest's game, as
    /// `client_setup` skips it. Fabric's `ClientLifecycleEvents.CLIENT_STOPPING`
    /// fires earlier, before the integrated server stops, so it is not a counterpart.
    /// NeoForge has no event for this moment. A listener is a pure function of its
    /// `ClientHost` reads and its own state.
    #[hook(
        side = "client",
        contract = "listen",
        cancellable = false,
        neoforge = "ClientStoppingEvent"
    )]
    fn client_stopping();
}
