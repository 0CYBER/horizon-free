//! The `on_caught_fire` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// On an authoritative server, a neighbour of fire passed its burn roll.
    /// `state` is still at `pos`; `face` points toward the fire. The spread,
    /// burn and (when replacing with fire) ignite cancel gates run first.
    /// The listener may write or schedule ticks, but the pending fire/air
    /// replacement overwrites any write at `pos`, then TNT is primed.
    /// Fire spread has no igniter and an empty stack, so neither is passed.
    /// Lava, lightning, spread into air, flint and steel, TNT use, burning
    /// projectiles, redstone and dispensers are outside this spread-only hook.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::on_caught_fire::GameLevel<'_>::native_on_caught_fire",
        neoforge = "IBlockExtension.onCaughtFire"
    )]
    fn on_caught_fire();
}
