//! The window's own ambient effects of entities, which a client makes for
//! itself (`level().isClientSide()` branches): endermen's and endermites'
//! portal particles, blazes' smoke and burn, eyes of ender's trail. They run
//! on the game that has the window, in-frame or following a server, never on
//! a server: nothing of them goes over the network.
use super::Game;
use horizon_client::mobs::{MobEvent, ambient};
use horizon_client::particles::{ParticleEngine, ParticleOptions, ParticleType};
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
        self.effect_swirls();
    }

    /// `LivingEntity.tickEffects` on a client: the swirls around an entity
    /// with visible effects, from the particles its synched data lists. The
    /// window's player has its own effects, a server's mobs and players the
    /// list the server sent, and the mobs and guests of a game in the frame
    /// theirs.
    fn effect_swirls(&mut self) {
        if let Some(p) = self.player.as_ref() {
            let effects = &self.potions.effects.list;
            let list: Vec<_> = effects
                .iter()
                .filter(|e| e.visible)
                .filter_map(|e| effect_particle(e.id, e.ambient))
                .collect();
            let ambient = effects.iter().filter(|e| e.visible).all(|e| e.ambient);
            let invisible = effects.iter().any(|e| e.id == "invisibility");
            let size = (
                f64::from(p.dimensions.width),
                f64::from(p.dimensions.height),
            );
            swirl(
                &mut self.particles,
                p.position,
                size,
                &list,
                ambient,
                invisible,
            );
        }
        let guests = self.lan.iter().flat_map(|host| host.models.values());
        for m in self.mobs.iter().chain(guests) {
            if !m.alive() {
                continue;
            }
            let size = (f64::from(m.width()), f64::from(m.height()));
            let invisible = m.invisible || m.has_effect("minecraft:invisibility");
            if m.extra.effect_particles.is_empty() {
                // A mob of this game: its own effects, visible and not ambient.
                if m.extra.effects.is_empty() {
                    continue;
                }
                let list: Vec<_> = m
                    .extra
                    .effects
                    .iter()
                    .filter_map(|e| effect_particle(e.0, false))
                    .collect();
                swirl(
                    &mut self.particles,
                    m.position,
                    size,
                    &list,
                    false,
                    invisible,
                );
            } else {
                let list = &m.extra.effect_particles;
                let ambient = m.extra.effect_ambient;
                swirl(
                    &mut self.particles,
                    m.position,
                    size,
                    list,
                    ambient,
                    invisible,
                );
            }
        }
    }
}

/// `MobEffect.particleFactory`: the omens have particles of their own, any
/// other effect an entity effect in its colour, faint when ambient.
pub(super) fn effect_particle(effect: &str, ambient: bool) -> Option<ParticleOptions> {
    Some(match effect.trim_start_matches("minecraft:") {
        "trial_omen" => ParticleOptions::simple(ParticleType::TrialOmen),
        "raid_omen" => ParticleOptions::simple(ParticleType::RaidOmen),
        other => {
            let color = horizon_client::brewing::effect(other)?.color;
            let channel = |shift: u32| ((color >> shift) & 255) as f32 / 255.0;
            // MobEffect.AMBIENT_ALPHA is 38 of 255.
            let alpha = if ambient { 38.0 / 255.0 } else { 1.0 };
            ParticleOptions::colored(
                ParticleType::EntityEffect,
                [channel(16), channel(8), channel(0), alpha],
            )
        }
    })
}

/// One tick of `LivingEntity.tickEffects`' client branch: a one in four
/// chance (one in fifteen while invisible, a fifth of either when all the
/// effects are ambient) of one of the particles at a random spot of the box.
fn swirl(
    particles: &mut ParticleEngine,
    feet: Vec3,
    size: (f64, f64),
    list: &[ParticleOptions],
    ambient: bool,
    invisible: bool,
) {
    if list.is_empty() {
        return;
    }
    let bound = if invisible { 15 } else { 4 } * if ambient { 5 } else { 1 };
    let r = particles.random();
    let Ok(0) = r.next_int_bound(bound) else {
        return;
    };
    // Util.getRandom, then getRandomX(0.5), getRandomY and getRandomZ.
    let pick = r.next_int_bound(list.len() as i32).unwrap_or_default() as usize;
    let x = feet.x + size.0 * (2.0 * r.next_double() - 1.0) * 0.5;
    let y = feet.y + size.1 * r.next_double();
    let z = feet.z + size.0 * (2.0 * r.next_double() - 1.0) * 0.5;
    particles.add(&list[pick], [x, y, z], [1.0; 3]);
}
