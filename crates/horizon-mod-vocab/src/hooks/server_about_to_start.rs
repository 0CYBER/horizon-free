//! The `server_about_to_start` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0004

pub trait Hooks {
    /// The server is about to open its worlds: nothing of the world is
    /// loaded yet (NeoForge's `ServerAboutToStartEvent`, "called before the
    /// server begins loading anything"). Vanilla's `MinecraftServer.loadLevel`
    /// start. Server-side: the single player server thread once Play is
    /// pressed, a dedicated server once its settings are read. Each is
    /// followed by `server_started` or
    /// `world_lifecycle`'s `WorldOpenFailed`. `dedicated` tells a dedicated server from the single
    /// player one.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ServerAboutToStartEvent",
        cancellable = false
    )]
    fn server_about_to_start();
}
