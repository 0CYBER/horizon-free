//! The `explosion_start` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server explosion is about to happen (`ExplosionEvent.Start`, in
    /// `ServerLevel.explode` before `ServerExplosion.explode`): every one,
    /// TNT, creepers, beds and anchors, fireballs, withers, end crystals and
    /// wind bursts. Set `cancel` and nothing of it happens: no blocks, no
    /// damage or knockback, no sound, particles or game event. Before
    /// `explosion_prime`, which a cancel skips. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "ExplosionEvent.Start",
        cancellable = true
    )]
    fn explosion_start();
}
