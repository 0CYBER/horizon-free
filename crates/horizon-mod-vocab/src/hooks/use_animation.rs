//! The `use_animation` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0102

pub trait Hooks {
    /// `Item.getUseAnimation` of `stack` in use, for first- and third-person
    /// rendering. `animation` arrives with the item's vanilla answer; mods may
    /// change it. A `Spear` override on an item without kinetic data is refused
    /// as `None`. Asked on the client when each render consumer updates; answer
    /// from the stack alone.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "none (vanilla Item.getUseAnimation; NeoForge adds no event or extension for it)"
    )]
    fn use_animation();
}
