//! Local particle and firework requests from native client hooks.
use super::Game;
use horizon_client::particles::ParticleOptions;
use mod_api::{FireworksRequest, client::particles::PositionSource};

/// The most explosions one `create_fireworks` request draws (a crafted rocket holds at
/// most 7 stars; a mod's show can be larger).
const MAX_FIREWORK_EXPLOSIONS: usize = 256;

fn finite_options(options: &ParticleOptions) -> bool {
    options.color.into_iter().all(f32::is_finite)
        && options.to_color.into_iter().all(f32::is_finite)
        && options.scale.is_finite()
        && options
            .trail
            .as_ref()
            .is_none_or(|(at, _)| at.iter().all(|v| v.is_finite()))
        && options
            .geyser
            .is_none_or(|(_, impulse)| impulse.is_finite())
        && options.vibration.as_ref().is_none_or(|(source, at, _)| {
            at.iter().all(|v| v.is_finite())
                && match source {
                    PositionSource::Block(_) => true,
                    PositionSource::Entity { y_offset, .. } => y_offset.is_finite(),
                    _ => false,
                }
        })
}

impl Game<'_> {
    /// `ClientLevel.addParticle`: request one particle in this client's loaded world.
    /// Options and provider input slots pass through unchanged, including encoded
    /// color or size values in `velocity`. Normal distance, settings and population
    /// limits still apply. True means submitted, not necessarily visible: an unknown
    /// provider or missing sprite set draws nothing. False on a server, without a
    /// client player/level, or for non-finite numeric input. API @API:client_particles@.
    pub(super) fn add_particle(
        &mut self,
        options: &ParticleOptions,
        at: [f64; 3],
        velocity: [f64; 3],
    ) -> bool {
        if !self.native_window()
            || self.player.is_none()
            || !at.into_iter().all(f64::is_finite)
            || !velocity.into_iter().all(f64::is_finite)
            || !finite_options(options)
        {
            return false;
        }
        self.particles.add(options, at, velocity);
        true
    }
    /// `ClientLevel.createFireworks`: queue a firework's explosions in this client's loaded
    /// world through the starter a bursting rocket uses, its blast and twinkle sounds gated by
    /// `audible` as a rocket's by its silence. No entity, no damage, nothing on the server.
    /// True means queued; false on a server, without a client player/level, for non-finite
    /// input or for more than [`MAX_FIREWORK_EXPLOSIONS`] explosions. API @API:client_fireworks@.
    pub(super) fn create_fireworks(&mut self, request: &FireworksRequest) -> bool {
        use super::api_values::TryIntoEngine;
        if !self.native_window()
            || self.player.is_none()
            || !request
                .at
                .into_iter()
                .chain(request.velocity)
                .all(f64::is_finite)
            || request.explosions.len() > MAX_FIREWORK_EXPLOSIONS
        {
            return false;
        }
        let Ok(explosions) = request
            .explosions
            .iter()
            .map(|explosion| explosion.try_into_engine(&()))
            .collect();
        let camera = self.eye_position().unwrap_or(request.at);
        self.particles.fireworks(
            request.at,
            request.velocity,
            explosions,
            camera,
            request.audible,
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::particles::{ParticleLevel, ParticleType};
    use mod_api::{ClientHost, FireworkExplosion, FireworkShape};

    #[test]
    fn nonfinite_nested_particle_parameters_are_rejected() {
        let good = ParticleOptions::simple(ParticleType::Smoke);
        assert!(finite_options(&good));
        let mut bad = good.clone();
        bad.to_color[0] = f32::NAN;
        assert!(!finite_options(&bad));
        bad = good.clone();
        bad.trail = Some(([f64::INFINITY, 0.0, 0.0], 3));
        assert!(!finite_options(&bad));
        bad = good.clone();
        bad.geyser = Some((1, f32::NAN));
        assert!(!finite_options(&bad));
        bad = good;
        bad.vibration = Some((
            PositionSource::Entity {
                id: 1,
                y_offset: f32::NAN,
            },
            [0.0; 3],
            3,
        ));
        assert!(!finite_options(&bad));
        bad.vibration = Some((PositionSource::Block([0; 3]), [0.0, f64::NAN, 0.0], 3));
        assert!(!finite_options(&bad));
    }

    /// A client mod's fireworks burst either way, and blast and twinkle only when audible,
    /// as a rocket that isn't silent; a server draws nothing.
    #[test]
    #[ignore = "daily"]
    fn a_client_mods_fireworks_sound_only_when_audible() {
        use crate::game::native_entity_refs::tests::with_client_game;
        with_client_game(|game| {
            let eye = horizon_core::geometry::Vec3::new(0.5, 200.0, 0.5);
            game.player =
                Some(horizon_client::motion::PlayerMotion::new(game.motion, eye).unwrap());
            let at = [6.5, 201.5, 10.5];
            let request = |audible| FireworksRequest {
                at,
                velocity: [0.0, 0.05, 0.0],
                explosions: vec![FireworkExplosion {
                    shape: FireworkShape::Star,
                    colors: vec![0xFF5555, 0xFFFF55],
                    fade_colors: vec![0xFFFFFF],
                    trail: true,
                    twinkle: true,
                }],
                audible,
            };
            for (audible, expected) in [
                (
                    true,
                    vec![
                        "minecraft:entity.firework_rocket.blast",
                        "minecraft:entity.firework_rocket.twinkle",
                    ],
                ),
                (false, vec![]),
            ] {
                game.particles.clear();
                game.particles.sounds.clear();
                let host: &mut dyn ClientHost = game;
                assert!(host.create_fireworks(&request(audible)));
                for _ in 0..20 {
                    let level = ParticleLevel {
                        world: &game.world,
                        facts: game.res.facts(),
                        collision: game.collision,
                        player: None,
                        cached_chunk: std::cell::Cell::new(None),
                    };
                    game.particles.tick(&level);
                }
                let heard: Vec<_> = game
                    .particles
                    .sounds
                    .iter()
                    .filter(|s| s.0.starts_with("minecraft:entity.firework_rocket."))
                    .map(|s| (s.0, s.1))
                    .collect();
                let expected: Vec<_> = expected.into_iter().map(|s| (s, at)).collect();
                assert_eq!(heard, expected, "audible {audible}");
                assert!(
                    !game.particles.is_empty(),
                    "the sparks drew, audible {audible}"
                );
            }
            let host: &mut dyn ClientHost = game;
            let mut bad = request(true);
            bad.velocity[1] = f64::NAN;
            assert!(!host.create_fireworks(&bad), "non-finite input");
            let mut most = request(false);
            most.explosions = vec![most.explosions[0].clone(); 256];
            assert!(
                host.create_fireworks(&most),
                "256 explosions, vanilla's most"
            );
            bad = request(false);
            bad.explosions = vec![bad.explosions[0].clone(); 257];
            assert!(!host.create_fireworks(&bad), "257 explosions");
            game.options.execution_role =
                crate::game::execution_role::ExecutionRole::IntegratedServer;
            let host: &mut dyn ClientHost = game;
            assert!(
                !host.create_fireworks(&request(true)),
                "servers draw nothing"
            );
        });
    }
}
