//! The `enchant_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0195

pub trait Hooks {
    /// A server player applied an enchantment at `pos`; `item` is the finished item and `levels` the levels charged. The player's state is not swapped in.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn enchant_item();
}
