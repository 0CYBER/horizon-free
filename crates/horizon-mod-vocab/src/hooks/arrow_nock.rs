//! The `arrow_nock` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player is about to nock `bow` with `hand` (`BowItem.use`, NeoForge's
    /// `ArrowNockEvent`): `has_ammo` is whether `getProjectile` found ammunition
    /// or the player is in creative. Set `cancel` to fail the use.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "ArrowNockEvent",
        cancellable = true
    )]
    fn arrow_nock();
}
