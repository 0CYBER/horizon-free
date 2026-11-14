//! Fork-shaped conversion of the upstream declaration.

pub trait Hooks {
    /// `ItemStack.interactLivingEntity`: after ordinary entity interactions
    /// pass, for an alive native Mob and a nonempty stack in `hand`.
    /// Both logical sides answer independently. A consuming client result
    /// ends the hand loop; only Success swings there. SuccessServer and
    /// Consume consume without a client swing; Fail, Pass and TryWithEmptyHand
    /// fall through. A server-only answer leaves client off-hand prediction
    /// unchanged. ClientHost permits reads and local effects, not authoritative
    /// writes. Armor stands, other players and dragon parts are not covered.
    #[hook(
        side = "both",
        version = 2,
        contract = "replace",
        out = "result",
        scope = "item",
        key = "stack",
        neoforge = "none (Item.interactLivingEntity)"
    )]
    fn item_interact_mob();
}
