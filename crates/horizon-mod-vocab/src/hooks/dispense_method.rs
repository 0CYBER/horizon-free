//! The `dispense_method` hook and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `DispenserBlock.getDispenseMethod`, before the item's `dispense_item`:
    /// the block the dispenser faces chooses what the dispense does, for any
    /// item, including vanilla items, on the authoritative server (not
    /// droppers). `state` is the block at `pos` moved by `facing` (air
    /// included), and the part is asked only for that block. An answer wins:
    /// no item part is asked and vanilla's behaviour
    /// doesn't run. The answer is `dispense_item`'s: a remainder of None
    /// empties the slot, `Some(stack)` replaces it (the hook owns
    /// consumption: no extra item is removed afterwards), and `event` is the
    /// level event played with the smoke: 1000 the click, 1001 the fail sound,
    /// or 0 for nothing at all. Leave answer None and `dispense_item` decides,
    /// as it did before.
    #[hook(
        side = "server",
        contract = "replace",
        out = "answer",
        scope = "block",
        key = "state",
        neoforge = "none (vanilla DispenserBlock.getDispenseMethod; NeoForge adds no event or extension for it)"
    )]
    fn dispense_method();
}
