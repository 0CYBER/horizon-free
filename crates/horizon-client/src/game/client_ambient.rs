//! The window's own ambient effects of entities, which a client makes for
//! itself (`level().isClientSide()` branches): endermen's and endermites'
//! portal particles, blazes' smoke and burn, eyes of ender's trail. They run
//! on the game that has the window, in-frame or following a server, never on
//! a server: nothing of them goes over the network.
use super::Game;
use horizon_client::blocks::RenderShape;
use horizon_client::mobs::{MobEvent, ambient};
use horizon_client::motion::{LAVA, WATER};
use horizon_client::particles::{ParticleOptions, ParticleType};
use horizon_client::pose::Pose;
use horizon_core::{geometry::Vec3, random::RandomSource};

impl Game<'_> {
    pub(super) fn tick_client_ambient(&mut self) {
        if self.serving.is_some() || self.options.dedicated.is_some() {
            return;
        }
        let mut events = Vec::new();
        for m in self.mobs.iter_mut().filter(|m| !m.removed) {
            ambient::client_ai_step(m, &mut events);
        }
        for event in events {
            match event {
                MobEvent::ParticleAt { kind, at, velocity } => {
                    if let Some(ty) = ParticleType::from_name(kind) {
                        self.particles
                            .add(&ParticleOptions::simple(ty), at, velocity);
                    }
                }
                // Each guest of a LAN host makes its own.
                MobEvent::LocalSound(name, at, volume, pitch) => {
                    self.audio
                        .play_unshared(&format!("minecraft:{name}"), Some(at), volume, pitch)
                }
                _ => {}
            }
        }
        self.eye_particles();
        self.sprint_dust();
    }

    /// `Entity.baseTick`'s `spawnSprintParticle` on a client: dust of the
    /// block under a sprinting player's feet streams back from them every
    /// tick, for the window's own player and for the other players it sees.
    fn sprint_dust(&mut self) {
        // Each runner's feet, movement and supporting block.
        let mut runners: Vec<(Vec3, Vec3, Option<[i32; 3]>)> = Vec::new();
        // Entity.canSpawnSprintParticle.
        if let Some(p) = self.player.as_ref()
            && p.sprinting
            && !p.spectator
            && p.pose != Pose::Crouching
            && !p.health.dead_or_dying()
            && !p.fluids.was_touching_water
            && !p.is_in_lava()
            && p.fluid_height(WATER) <= 0.0
            && p.fluid_height(LAVA) <= 0.0
        {
            let support = p.ground.supporting_block.map(|b| [b.x, b.y, b.z]);
            runners.push((p.position, p.velocity, support));
        }
        let guests = self.lan.iter().flat_map(|host| host.models.values());
        for m in self.mobs.iter().chain(guests) {
            // Another player's fluids aren't known here: none where they stand.
            let feet = [
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            ];
            if m.extra.sprinting
                && !m.extra.crouching
                && m.alive()
                && &*m.kind.id == "minecraft:player"
                && self
                    .res
                    .facts()
                    .get(self.world.block(feet[0], feet[1], feet[2]))
                    .fluid_level
                    == 0
            {
                runners.push((m.position, m.position.subtract(m.previous), None));
            }
        }
        for (position, movement, support) in runners {
            // Entity.getOnPosLegacy: getOnPos(0.2).
            let below = (position.y - f64::from(0.2f32)).floor() as i32;
            let tall = |game: &Self, s: [i32; 3]| {
                let name = game.block_name(s);
                [
                    "minecraft:fences",
                    "minecraft:walls",
                    "minecraft:fence_gates",
                ]
                .iter()
                .any(|tag| game.block_in_tag(tag, &name))
            };
            let at = match support {
                Some(s) if tall(self, s) => s,
                Some(s) => [s[0], below, s[2]],
                None => [position.x.floor() as i32, below, position.z.floor() as i32],
            };
            let state = self.world.block(at[0], at[1], at[2]);
            if self.res.facts().get(state).render_shape == RenderShape::Invisible {
                continue;
            }
            let Some((sprite, tint)) = self.particle_sprite(state, at) else {
                continue;
            };
            let width = f64::from(0.6f32);
            let r = self.particles.random();
            let mut x = position.x + (r.next_double() - 0.5) * width;
            let mut z = position.z + (r.next_double() - 0.5) * width;
            // Clamped into the block when the feet are in another column.
            if position.x.floor() as i32 != at[0] {
                x = x.clamp(f64::from(at[0]), f64::from(at[0]) + 1.0);
            }
            if position.z.floor() as i32 != at[2] {
                z = z.clamp(f64::from(at[2]), f64::from(at[2]) + 1.0);
            }
            self.particles.add(
                &ParticleOptions::block(ParticleType::Block, sprite, tint),
                [x, position.y + 0.1, z],
                [movement.x * -4.0, 1.5, movement.z * -4.0],
            );
        }
    }
}
