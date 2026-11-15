//! The `entity_interact` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0057

pub trait Hooks {
    /// A player right-clicks the entity `entity` (`Entity.getId`) of type
    /// `kind` (`minecraft:cow`) with `hand` (`Hand::Main` or `Hand::Off`), holding `stack` (none in an
    /// empty hand): where `Player.interactOn` starts, before the entity's own
    /// `interact` and the item's `interactLivingEntity`. Both logical sides
    /// answer independently, once per hand, main hand before off hand.
    /// A consuming client result ends the loop; only Success swings there.
    /// SuccessServer and Consume consume without a client swing. Fail,
    /// Pass and TryWithEmptyHand fall through on this entity path.
    /// A server-only answer leaves the client's off-hand prediction unchanged.
    /// ClientHost permits reads and local effects, not authoritative writes.
    #[hook(
        side = "both",
        version = 2,
        contract = "replace",
        out = "result",
        neoforge = "PlayerInteractEvent.EntityInteract"
    )]
    fn entity_interact();
}
