//! The `creator_mod_id` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `IItemExtension.getCreatorModId`: the mod that made `stack`, which a
    /// recipe viewer or tooltip names, instead of the mod that registered its
    /// item. `mod_id` arrives as the item id's namespace; set it to another
    /// mod (a universal bucket holding that mod's fluid, a facade of its
    /// block) or `None` for no mod. Asked by `ItemResource::creator_mod_id`,
    /// from client tooltips and viewers as well as the server: answer from
    /// the stack alone. Only the `ItemPart`s attached to `stack`'s item are
    /// asked: an item with none costs one table lookup.
    #[hook(
        side = "both",
        contract = "collect",
        out = "mod_id",
        scope = "item",
        key = "stack",
        host = "read",
        neoforge = "IItemExtension.getCreatorModId"
    )]
    fn creator_mod_id();
}
