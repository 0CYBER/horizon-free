//! The `pickup_xp` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0212

pub trait Hooks {
    /// Before `player` picks up orb `orb` in `dimension`. Mutations to `value`
    /// and `count` take effect only when not canceled; value is clamped to
    /// 1..=i32::MAX and count to at least one.
    /// Canceling leaves the orb and pickup delay unchanged. Server-side. Diverges from upstream: renamed from `experience_orb_pickup`, to NeoForge's `PlayerXpEvent.PickupXp`.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "PlayerXpEvent.PickupXp",
        cancellable = true
    )]
    fn pickup_xp();
}
