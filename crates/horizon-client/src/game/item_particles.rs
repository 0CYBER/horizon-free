//! The sprites of item particles: `BreakingItemParticle.ItemParticleProvider.getSprite`
//! resolves the stack's model for the ground and picks a layer's particle sprite.
use super::Game;
use horizon_assets::item_model::DisplayContext;
use horizon_client::{
    items::ItemStack,
    particles::{self, ParticleOptions, ParticleType},
};

impl Game<'_> {
    /// `ItemParticleOption(ITEM, stack)`: the layer particle sprites of the
    /// stack's model resolved for the ground, with seed 0.
    pub(super) fn item_particle(&self, stack: &ItemStack) -> ParticleOptions {
        let state = self.resolve_item(stack, DisplayContext::Ground, None, 0);
        ParticleOptions::item(state.layers.iter().map(|l| l.particle.clone()).collect())
    }
    /// `LivingEntity.spawnItemParticles(stack, count)` for the local player,
    /// drawing from the world random as its eating sounds do.
    pub(super) fn eat_particles(&mut self, stack: &ItemStack, count: u32) {
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let eye_y = self.eye_position().map_or(p.y, |e| e[1]);
        let options = self.item_particle(stack);
        let mut spawned = Vec::new();
        particles::item_use_particles(
            &mut self.world.random,
            self.pitch,
            self.yaw,
            [p.x, p.y, p.z],
            eye_y,
            count,
            |p, v| spawned.push((p, v)),
        );
        for (p, v) in spawned {
            self.particles.add(&options, p, v);
        }
    }
    /// The slime ball, snowball and cobweb models behind `item_slime`,
    /// `item_snowball` and `item_cobweb`, resolved on first use.
    pub(super) fn ensure_item_particle_sprites(&mut self) {
        if !self.particles.item_sprites.is_empty() {
            return;
        }
        for (ty, item) in [
            (ParticleType::ItemSlime, "minecraft:slime_ball"),
            (ParticleType::ItemSnowball, "minecraft:snowball"),
            (ParticleType::ItemCobweb, "minecraft:cobweb"),
        ] {
            let sprites = self
                .item_particle(&ItemStack::new(item, 1))
                .item
                .unwrap_or_default();
            self.particles.item_sprites.insert(ty, sprites);
        }
    }
}
