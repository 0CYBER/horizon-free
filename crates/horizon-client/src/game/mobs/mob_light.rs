//! The light a mob is drawn with.
use super::Game;
use horizon_client::mobs::{Mob, Species};

impl Game<'_> {
    /// `EntityRenderer.getPackedLightCoords`: the light `mob` is drawn with at `alpha`, packed as
    /// the entity pass packs it (sky light in the first byte, block light in the second).
    pub(in crate::game) fn mob_packed_light(&self, mob: &Mob, alpha: f64) -> u32 {
        // The block the mob's light probe (its eye) is in, at the position this frame draws
        // rather than this tick's.
        let probe = mob.light_probe(alpha);
        let light = self.world.light(probe[0], probe[1], probe[2]);
        let mut packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
        // EntityRenderer.getBlockLightLevel: a burning mob has 15 block
        // light, and so do the Allay, Blaze, Magma Cube, Vex and Wither
        // renderers' overrides; the sky light stays the level's.
        if mob.fire_ticks > 0
            || matches!(
                mob.species(),
                Species::Allay
                    | Species::Blaze
                    | Species::MagmaCube
                    | Species::Vex
                    | Species::Wither
            )
        {
            packed = (packed & 0xFF) | ((15 * 16) << 8);
        }
        // GlowSquidRenderer.getBlockLightLevel: the body brightens as the
        // dark ticks run out (the block light is the second byte).
        if mob.species() == Species::GlowSquid {
            let lit = (packed >> 12) & 15;
            let block = horizon_client::mobs::aquatic::glow_squid_block_light(mob, lit);
            packed = (packed & 0xFF) | ((block * 16) << 8);
        }
        packed
    }
}
