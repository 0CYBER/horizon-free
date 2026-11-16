//! The `highlight_tip` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.getHighlightTip`: the name drawn above the
    /// hotbar when `stack` is selected. `name` arrives with what
    /// `Hud.extractSelectedItemName` draws (the hover name in its rarity
    /// colour, italic when the stack has a `custom_name`); the text the part
    /// leaves is drawn in its place, centred at the same height and fade, cut
    /// at the screen edge as a long custom name is. Asked once each client
    /// tick while the name shows. The display timer still restarts only when
    /// the selected item or its hover name changes, so a tip that changes
    /// every tick doesn't keep the name up. Only the `ItemPart`s attached to
    /// `stack`'s item are asked. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "IItemExtension.getHighlightTip"
    )]
    fn highlight_tip();
}
