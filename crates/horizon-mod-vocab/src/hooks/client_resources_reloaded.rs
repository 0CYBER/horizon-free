//! The `client_resources_reloaded` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0099

pub trait Hooks {
    /// Render-thread notification after a resource-pack reload has applied successfully.
    /// Atlas, models, renderer and client caches already refer to the new resources.
    /// Selection/F3+T and server-pack reloads share this point; initial startup does not
    /// (`client_setup` is the once-only moment after the initial load). Runs in
    /// Reloading: published asset/data queries may refresh caches, but world
    /// handles and transactions remain unavailable.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ClientResourceLoadFinishedEvent"
    )]
    fn client_resources_reloaded();
}
