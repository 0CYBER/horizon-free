//! The `server_stopped` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0008

pub trait Hooks {
    /// Fired once at the end of server teardown, after `server_stopping` and
    /// the final save attempt and storage release. Also fired when startup or
    /// teardown fails after startup began, including when a shutdown listener
    /// panics, on integrated and dedicated servers.
    /// Failed opens save only worlds already adopted by the server (the prepared
    /// level is installed; the session lock alone is not adoption). A failed
    /// save still reaches this hook, but does not emit `WorldCloseFinal`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ServerStoppedEvent",
        cancellable = false
    )]
    fn server_stopped();
}
