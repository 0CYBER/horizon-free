//! The `server_starting` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0005

pub trait Hooks {
    /// Fired once immediately after `server_about_to_start`, before the server
    /// accepts play, on both integrated and dedicated servers. A failed startup
    /// still reaches `server_stopped`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ServerStartingEvent",
        cancellable = false
    )]
    fn server_starting();
}
