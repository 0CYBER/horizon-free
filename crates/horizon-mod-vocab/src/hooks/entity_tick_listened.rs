//! The `entity_tick_listened` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0156

pub trait Hooks {
    /// Whether a mod listens to `entity_tick_pre` and `entity_tick_post`: a mod
    /// that hooks them hooks this one too and sets `listened` (HEAD, `*listened = true`).
    /// Without it the game tells no entity's tick, and keeps no list of them. Asked once for
    /// each family (mobs, items, minecarts, boats) each tick. Server-side.
    #[hook(
        side = "server",
        contract = "collect",
        out = "listened",
        cancellable = false,
        neoforge = "none (Horizon-only; NeoForge has no counterpart); an opt-in gate for a hook"
    )]
    fn entity_tick_listened();
}
