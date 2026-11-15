//! The `makes_piglins_neutral` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.makesPiglinsNeutral`: does `stack`, worn by `wearer`, keep
    /// calm piglins from targeting it, as gold armour does (`PiglinAi.isWearingSafeArmor`)?
    /// `neutral` arrives with vanilla's answer, whether the stack is in
    /// `#minecraft:piglin_safe_armor`; set it `true` to make the piece count as gold, `false`
    /// to make it not count though the tag lists it, or leave it to keep the tag's answer. Asked
    /// for each stack in a player's armour slots, in the order feet, legs, chest, head, until one
    /// counts. A player wearing a piece that counts is not the target a calm piglin picks
    /// (`NEAREST_TARGETABLE_PLAYER_NOT_WEARING_GOLD`); an angry piglin attacks anyway. Only the
    /// `ItemPart`s attached to `stack`'s item are asked. It is asked whenever the server builds
    /// a player's view for the mobs, several times a tick, so the answer must be a pure, fast
    /// function of `stack` and `wearer`. An item with no such part costs one table lookup.
    /// Server-side. The contract beyond the method's name and parameters is Horizon's reading,
    /// not checked against NeoForge's.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "item",
        key = "stack",
        host = "read",
        neoforge = "IItemExtension.makesPiglinsNeutral"
    )]
    fn makes_piglins_neutral();
}
