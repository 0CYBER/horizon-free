//! The `server_stopping` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0007

pub trait Hooks {
    /// The server begins teardown (NeoForge's `ServerStoppingEvent`), including
    /// after startup or the tick loop failed:
    /// the start of vanilla's `MinecraftServer.stopServer`, with the worlds
    /// still loaded and the players still connected and not yet saved, so a
    /// mod can save its own data or tell them. `world_lifecycle`'s
    /// `WorldCloseFinal` follows once the worlds are saved. A server whose loop
    /// fails still fires this before the final save attempt and `server_stopped`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ServerStoppingEvent",
        fabric = "ServerLifecycleEvents.SERVER_STOPPING",
        cancellable = false
    )]
    fn server_stopping();
}
