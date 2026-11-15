//! The `can_grindstone_repair` hook: declarations the mod API generator reads (see
//! `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.canGrindstoneRepair`: does the grindstone repair `stack` as
    /// it repairs a damageable item? `can_repair` arrives with vanilla's answer, whether the
    /// stack is damageable (`ItemStack.isDamageableItem`); set it `true` or `false`. It is
    /// asked at two places, on both sides (the server's menus and the client's own result):
    /// - `GrindstoneMenu.mergeItems`, for the top stack when both inputs are single stacks of
    ///   the same item. `true` takes the repair branch: one item, the enchantments of both
    ///   merged, then all but the curses removed. Durability is merged only when the result
    ///   has durability, as vanilla checks it again on the copy. `false` takes the other
    ///   branch: two matching stacks of an item that stacks give two, and anything else
    ///   gives nothing.
    /// - The input slots' `mayPlace` (damageable or enchanted): a granted stack can be placed.
    ///   This site is Horizon's reading.
    ///
    /// A single enchanted input is never asked: that branch merges nothing. Only the
    /// `ItemPart`s attached to `stack`'s item are asked. It is given the loop `context` of the
    /// side that runs the menu and takes no `Host`, so answer from the stack and `context`
    /// alone, and fast. With no part on the stack's item the cost is one table lookup, and
    /// the stack isn't converted. The contract beyond the method's name and parameters is
    /// Horizon's reading, not checked against NeoForge's. Diverges from upstream: new hook.
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        scope = "item",
        key = "stack",
        cancellable = false,
        target = "horizon_client_lib::stations::native_can_grindstone_repair",
        neoforge = "IItemExtension.canGrindstoneRepair"
    )]
    fn can_grindstone_repair();
}
