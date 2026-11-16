//! The `player_brewed_potion` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player took `stack` from a brewing stand bottle slot
    /// (`BrewingStandMenu.PotionSlot.onTake`): `player` is their UUID and
    /// `player_entity_id` their network id.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerBrewedPotionEvent"
    )]
    fn player_brewed_potion();
}
