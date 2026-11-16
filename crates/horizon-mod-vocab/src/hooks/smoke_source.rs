//! The `smoke_source` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0216

pub trait Hooks {
    /// On an authoritative server, `CampfireBlock.isSmokeSource` for the
    /// block `state` under a campfire, as the campfire is placed or the block
    /// under it changes: `Some(true)` for your own blocks makes the campfire a
    /// signal fire whose smoke rises higher, as over a hay bale. The level is
    /// read-only.
    #[hook(
        side = "server",
        contract = "replace",
        out = "smoke",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_smoke_source",
        neoforge = "none (vanilla CampfireBlock.isSmokeSource; NeoForge adds no event or extension for it)"
    )]
    fn smoke_source();
}
