//! The `anvil_craft_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0237

pub trait Hooks {
    /// A player took an anvil's result at `pos` and the levels, inputs and anvil wear are done (`AnvilCraftEvent.Post`); a broken anvil is already air. `craft` holds the inputs and the result as they were before. Not called when `anvil_craft_pre` cancelled.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "AnvilCraftEvent.Post"
    )]
    fn anvil_craft_post();
}
