//! The `client_tick_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Once every client tick, paused or not and on the title screen too, before the window's tick runs (NeoForge's
    /// `ClientTickEvent.Pre`): the window's `Game` only, so not on a server's
    /// own tick (`server_tick` is that moment). Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ClientTickEvent.Pre",
        fabric = "ClientTickEvents.START_CLIENT_TICK"
    )]
    fn client_tick_pre();
}
