//! The `player_item_mend` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0256

pub trait Hooks {
    /// Mending is about to repair a server player's item with experience from an orb. `repair` is
    /// the durability vanilla would restore and `experience` the orb's value; a listener may change
    /// either, and what is left is what the game applies to the stack and to the XP the orb keeps.
    /// Contract version `2` makes those two fields writeable: they arrive as copies of the incoming
    /// values and the dispatch writes them back after the listeners, so assigning to a field is
    /// enough. The two fields are independent: a listener that zeroes `repair` still leaves
    /// `experience` to be spent. A mod written for `1` still sees values it does not change.
    /// With no listener the values are vanilla's, bit for bit. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        version = 2,
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_item_mend();
}
