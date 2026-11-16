//! The `server_tick_rate_changed` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `/tick rate` set the server's rate, or a `/tick sprint` ended
    /// (`MinecraftServer.onTickRateChanged`, before its autosave wait is adjusted). Vanilla calls
    /// this for every `/tick rate`, the rate it already had included, and at each end of a sprint:
    /// on its own, by `/tick sprint stop` or by `/tick freeze`, the rate then being what it was.
    /// `host.tick_stats()` reads the rate. A client told the server's rate is not a server:
    /// nothing fires there. Server-side. Beyond NeoForge: there is no counterpart.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (vanilla MinecraftServer.onTickRateChanged; NeoForge adds no event or extension for it)"
    )]
    fn server_tick_rate_changed();
}
