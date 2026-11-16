//! The `dragon_egg_form` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0194

pub trait Hooks {
    /// The Ender Dragon fight is about to place its first dragon egg at `pos`.
    /// Set `cancel` to leave the block unchanged. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        paper = "DragonEggFormEvent",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_dragon_egg_form"
    )]
    fn dragon_egg_form();
}
