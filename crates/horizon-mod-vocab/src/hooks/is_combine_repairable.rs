//! The `is_combine_repairable` hook: declarations the mod API generator reads (see
//! `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.isCombineRepairable`: may two of `stack`'s item be combined
    /// in a crafting grid to repair it (`RepairItemRecipe.canCombine`)? `repairable` arrives
    /// with vanilla's answer, whether the stack has durability (`minecraft:max_damage` and
    /// `minecraft:damage`); set it `true` or `false`. It is asked for each of the two stacks,
    /// once they are found to be single stacks of the same item, and both must answer `true`.
    /// A `true` for a stack with no durability (a maximum damage of 0) is refused with a
    /// warning, and vanilla's `false` is kept, because the result needs a positive maximum.
    /// The anvil's combining of two of the same item keeps vanilla's test and doesn't ask.
    /// It is asked on both sides, wherever the grid's result is matched: a crafting table, the
    /// inventory grid, a crafter and the client's own result. Only the `ItemPart`s attached to
    /// `stack`'s item are asked. It is given the loop `context` of the side that runs the
    /// match and takes no `Host`, so answer from the stack and `context` alone, and fast: a
    /// match may ask it many times. With no part on the stack's item the cost is one table
    /// lookup, and the stack isn't converted. The contract beyond the method's name and
    /// parameters is Horizon's reading, not checked against NeoForge's. Diverges from
    /// upstream: new hook.
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        scope = "item",
        key = "stack",
        cancellable = false,
        target = "horizon_client_lib::crafting::native_hooks::native_is_combine_repairable",
        neoforge = "IItemExtension.isCombineRepairable"
    )]
    fn is_combine_repairable();
}
