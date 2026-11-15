//! The `hanging_place` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0245

pub trait Hooks {
    /// A server player places a hanging entity: an item frame
    /// (`minecraft:item_frame`, `minecraft:glow_item_frame`), a painting
    /// (`minecraft:painting`) or a leash knot (`minecraft:leash_knot`, when a
    /// lead is tied to a fence that has none yet), `kind` its entity type and
    /// `pos` the block it takes (the fence's for a knot). Where
    /// `HangingEntityItem.useOn` and `LeadItem.useOn` create it, once it is
    /// known to fit and before anything is placed or used up. Set `cancel`
    /// and nothing is placed: the item stays in the player's hand. The
    /// player's state is swapped in, so `PlayerRef::tell` answers them. Only for
    /// guests of a server; `/summon`, a dispenser and the game's own player
    /// are not heard, nor is a lead tied to a mob or a boat.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (vanilla HangingEntityItem.useOn; NeoForge adds no event or extension for it); PlayerInteractEvent.RightClickBlock fires before it"
    )]
    fn hanging_place();
}
