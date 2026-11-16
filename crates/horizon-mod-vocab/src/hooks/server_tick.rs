//! The `server_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0009

pub trait Hooks {
    /// The authoritative server tick's phases: `ServerTickStart` before any
    /// level is ticked (NeoForge's `ServerTickEvent.Pre`), `ServerTickEnd`
    /// after the last (`.Post`). Read blocks with `native_server_block`, by
    /// dimension: which level `self.world` holds here is not defined. A paused
    /// pass fires nothing; a frozen world carries `world_running` false except
    /// during a requested step. `host.server_has_time()` says whether optional
    /// work still fits before the next tick is due (`ServerTickEvent.hasTime`).
    #[hook(side = "server", contract = "listen", fabric = ["ServerTickEvents.START_SERVER_TICK", "ServerTickEvents.END_SERVER_TICK"], neoforge = "ServerTickEvent.Pre, ServerTickEvent.Post")]
    fn server_tick();
}
