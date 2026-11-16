//! The `add_client_reload_listeners` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register client asset reload listeners before a reload stages its
    /// assets (NeoForge's `AddClientReloadListenersEvent`, the client's
    /// counterpart of `add_server_reload_listeners`): the first load
    /// on the loader thread, then every later reload — the player
    /// changed resource packs, F3+T was pressed, or a server sent its packs.
    /// The listener's `prepare` then reads the merged asset view with
    /// resource-pack overrides resolved, and its returned apply publishes the
    /// parsed state, all applies after all prepares, before
    /// `client_resources_reloaded`. A listener that returns an error
    /// aborts the reload and keeps every listener's previous assets.
    /// Client-side, in Reloading nested in Setup initially and World later.
    /// Later registration is on the render thread, prepare on the loader,
    /// and apply on the owning game thread; completion restores the entering
    /// phase. Startup's empty Host grants no extra access.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "AddClientReloadListenersEvent"
    )]
    fn add_client_reload_listeners();
}
