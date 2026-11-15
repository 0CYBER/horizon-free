//! The `gather_effect_screen_tooltips` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `GatherEffectScreenTooltipsEvent`: the tooltip of the effect panel the
    /// cursor is over beside the survival or creative inventory. `tooltip`
    /// starts empty when the panel's information is fully visible, or holds
    /// the full name then duration when compact or clipped. Each line has a
    /// colour as `rgba` packs it (`u32::MAX` keeps the text's own); add, change,
    /// reorder or remove lines, and an empty list draws nothing. `effect` is
    /// the effect the panel shows, `screen` the inventory. Asked each frame
    /// a rendered panel is hovered, only while a mod listens. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "GatherEffectScreenTooltipsEvent",
        cancellable = false
    )]
    fn gather_effect_screen_tooltips();
}
