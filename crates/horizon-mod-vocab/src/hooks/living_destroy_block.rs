//! The `living_destroy_block` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mob is about to destroy a block (NeoForge's `LivingDestroyBlockEvent`):
    /// a wither breaking blocks around it, a ravager breaking leaves, the ender
    /// dragon's wall check, an enderman picking a block up, a silverfish waking
    /// its friends (the infested blocks break), a zombie breaking a door or a
    /// turtle egg. `entity` is its entity id, `kind` its type (`minecraft:wither`),
    /// `pos` the block and `state` its state. Canceling keeps the block (an
    /// enderman then holds nothing). Asked once per block, only while a mod is
    /// loaded. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "LivingDestroyBlockEvent",
        cancellable = true
    )]
    fn living_destroy_block();
}
