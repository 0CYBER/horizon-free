//! The `dispenser_acts` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A dispenser at `pos` in `dimension`, facing `facing`, is about to act on
    /// `stack` (`DispenserBlock.getDispenseMethod`), once per dispenser action
    /// on the authoritative server (not droppers). It runs before the faced
    /// block's `dispense_method` and the item's `dispense_item`, so a mod that
    /// cannot list every item or block at registration answers here for all of
    /// them, vanilla ones included. A mod's answer wins: neither part is asked
    /// and vanilla's behaviour doesn't run. The answer is `dispense_item`'s:
    /// a remainder of None empties the slot, `Some(stack)` replaces it (the
    /// hook owns consumption: no extra item is removed afterwards), and `event`
    /// is the level event played with the smoke: 1000 the click, 1001 the fail
    /// sound, or 0 for nothing at all. Leave answer None and the faced block's
    /// and the item's parts decide, as they did before.
    #[hook(
        side = "server",
        contract = "replace",
        out = "answer",
        neoforge = "none (vanilla DispenserBlock.getDispenseMethod; NeoForge adds no event or extension for it)"
    )]
    fn dispenser_acts();
}
