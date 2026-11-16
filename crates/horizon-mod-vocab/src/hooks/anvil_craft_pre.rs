//! The `anvil_craft_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0238

pub trait Hooks {
    /// A player took an anvil's result at `pos` (`AnvilCraftEvent.Pre`), before the levels are charged, the inputs used up and the anvil worn. `craft` holds the inputs and the result as they were before. Set `cancel` to refuse the take: the result stays in its slot and the inputs, levels, inventory and cursor are as before the click.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "AnvilCraftEvent.Pre",
        cancellable = true
    )]
    fn anvil_craft_pre();
}
