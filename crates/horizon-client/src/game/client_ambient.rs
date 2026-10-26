//! The window's own ambient effects of entities, which a client makes for
//! itself (`level().isClientSide()` branches): endermen's and endermites'
//! portal particles, blazes' smoke and burn, eyes of ender's trail. They run
//! on the game that has the window, in-frame or following a server, never on
//! a server: nothing of them goes over the network.
use super::Game;
use horizon_client::mobs::{MobEvent, ambient};
use horizon_client::particles::{ParticleOptions, ParticleType};

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
    }
}
