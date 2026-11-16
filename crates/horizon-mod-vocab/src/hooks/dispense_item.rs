//! The `dispense_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0075

pub trait Hooks {
    /// `DispenserBlock.getDispenseMethod`: choose a behaviour for any item,
    /// including vanilla items, on the authoritative server (not droppers).
    /// The block the dispenser faces is asked first (`dispense_method`), and
    /// its answer keeps this part from being asked. Leave answer None for
    /// vanilla. Otherwise answer `(remainder, event)`:
    /// a remainder of None empties the slot, `Some(stack)` replaces it (the hook
    /// owns consumption: no extra item is removed afterwards). `event` is the
    /// level event the dispenser plays with its smoke: 1000 the click, 1001 the
    /// fail sound (`OptionalDispenseItemBehavior` without success), or 0 for
    /// nothing at all (`DispenseItemBehavior.NOOP`). To drop one normally,
    /// answer `(native_dispense_default(..), 1000)`.
    #[hook(
        side = "server",
        contract = "replace",
        out = "answer",
        scope = "item",
        key = "stack",
        neoforge = "none (vanilla DispenserBlock.getDispenseMethod; NeoForge adds no event or extension for it)"
    )]
    fn dispense_item();
}
