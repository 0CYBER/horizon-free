use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A server explosion is about to happen (`ExplosionEvent.Start`, in
    /// `ServerLevel.explode` before `ServerExplosion.explode`): every one,
    /// TNT, creepers, beds and anchors, fireballs, withers, end crystals and
    /// wind bursts. Set `cancel` and nothing of it happens: no blocks, no
    /// damage or knockback, no sound, particles or game event. Before
    /// `explosion_prime`, which a cancel skips. Server-side.
    pub(in crate::game) fn native_explosion_start(
        &mut self,
        explosion: &ExplosionStart<'_>,
        cancel: &mut bool,
    ) {
        self.fire_explosion_start(explosion, cancel);
    }
}
