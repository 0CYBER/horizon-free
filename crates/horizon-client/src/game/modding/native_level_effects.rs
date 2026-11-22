//! Sounds and particles in a level for native server mods, through a
//! `LevelRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387).
//! Every player in range hears or sees them, as `ServerLevel.playSound` and
//! `sendParticles` send them. They return false on a client, for a handle of
//! an earlier open, or for arguments the network can't carry.
use super::native_hooks::{mod_particles_ok, mod_sound_ok};
use super::{Game, creatures::NativeSound, lan_particles::QueuedParticle};
use horizon_client::particles::ParticleOptions;
use horizon_world::registry::BlockStateId;
use mod_api::{LevelRef, SoundSource};
use std::sync::Arc;

/// The most positions one `level_send_particles_at` call takes.
pub(super) const MAX_PARTICLE_POSITIONS: usize = 4096;

impl Game<'_> {
    /// The level's dimension id, for a handle of this open on the server.
    fn native_level_dimension(&self, level: LevelRef) -> Option<&'static str> {
        self.native_level_world(level)?;
        Some(level.dimension_id())
    }
    /// Plays sound event `sound` at `at` for the players in this level
    /// within range (16 blocks, times the volume above 1), as
    /// `ServerLevel.playSound(null, ...)` does, under `source`'s volume
    /// slider. A `minecraft:` event must be registered; a mod's goes by name
    /// and is heard where its resources define it. True means queued, not
    /// heard.
    pub(super) fn level_play_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool {
        self.native_queue_sound(level, sound, at, source, volume, pitch, None)
            .is_some()
    }
    /// `level_play_sound`, leaving out the player on connection
    /// `except` (`Level.playSound(except, ...)`): the queued sound, None
    /// when it can't be sent.
    pub(super) fn native_queue_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: SoundSource,
        volume: f32,
        pitch: f32,
        except: Option<u32>,
    ) -> Option<&mut NativeSound> {
        let dimension = self.native_level_dimension(level)?;
        if !mod_sound_ok(sound, at, source.index(), volume, pitch) {
            return None;
        }
        // `Level.playSound` draws `soundSeedGenerator.nextLong` at the call;
        // without a host nothing can be sent, as the flush already did.
        let seed = self.lan.as_ref()?.hub.next_sound_seed();
        self.creatures.lan_sounds.push(NativeSound {
            dimension,
            sound: sound.to_owned(),
            at,
            source: source.index() as i32,
            volume,
            pitch,
            seed,
            except,
            entity: None,
        });
        self.creatures.lan_sounds.last_mut()
    }
    /// Sends `count` particles around `at` to the players in this level
    /// within 32 blocks, as `ServerLevel.sendParticles` does: each offset is
    /// a Gaussian times `spread`, and `speed` scales their motion (a zero
    /// count sends one particle moving along `spread`). Block particle types
    /// need `state`; item particles need a validated stack; trail and vibration aren't supported.
    /// True means queued, not drawn.
    pub(super) fn level_send_particles(
        &mut self,
        level: LevelRef,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let Some(dimension) = self.native_level_dimension(level) else {
            return false;
        };
        let registry = self.pack.assets.registry;
        let state_ok = |id| registry.state(id).is_some();
        if !mod_particles_ok(options, state, state_ok, at, spread, speed) {
            return false;
        }
        self.creatures.lan_particles.push(QueuedParticle {
            dimension,
            options: options.clone().into(),
            state,
            at,
            spread,
            speed: [speed; 3],
            count,
            randomization: 0,
        });
        true
    }
    /// `level_send_particles` around each of `positions`, in order, in one call: all of
    /// them are queued or none, if any would be refused. Each is still a request of its own,
    /// sharing one copy of `options`. More than `MAX_PARTICLE_POSITIONS` is refused.
    pub(super) fn level_send_particles_at(
        &mut self,
        level: LevelRef,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
        positions: &[[f64; 3]],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let Some(dimension) = self.native_level_dimension(level) else {
            return false;
        };
        if positions.len() > MAX_PARTICLE_POSITIONS {
            return false;
        }
        let registry = self.pack.assets.registry;
        let state_ok = |id| registry.state(id).is_some();
        if !positions
            .iter()
            .all(|&at| mod_particles_ok(options, state, state_ok, at, spread, speed))
        {
            return false;
        }
        let options = Arc::new(options.clone());
        self.creatures
            .lan_particles
            .extend(positions.iter().map(|&at| QueuedParticle {
                dimension,
                options: Arc::clone(&options),
                state,
                at,
                spread,
                speed: [speed; 3],
                count,
                randomization: 0,
            }));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::particles::ParticleType;

    fn queued_options(game: &Game<'_>, i: usize) -> Arc<ParticleOptions> {
        Arc::clone(&game.creatures.lan_particles[i].options)
    }

    /// Every position is queued, in order, or none is.
    #[test]
    #[ignore = "daily"]
    fn particles_at_queues_every_position_or_none() {
        let mut game = super::super::native_machines::tests::server_game();
        game.native_lifecycle_open_started();
        let level = game.level("minecraft:overworld").expect("a level");
        let flame = ParticleOptions::simple(ParticleType::Flame);
        let at = [[0.0, 64.0, 0.0], [1.5, 65.0, -2.0], [3.0, 66.0, 4.0]];
        let send = |game: &mut Game<'_>, positions: &[[f64; 3]]| {
            game.level_send_particles_at(level, &flame, None, positions, [0.1; 3], 0.5, 3)
        };
        assert!(send(&mut game, &at));
        let queued = &game.creatures.lan_particles;
        assert_eq!(queued.iter().map(|q| q.at).collect::<Vec<_>>(), at);
        assert!(queued.iter().all(|q| q.count == 3 && q.speed == [0.5; 3]));
        let bad = [at[0], [f64::NAN, 64.0, 0.0]];
        assert!(!send(&mut game, &bad));
        assert_eq!(game.creatures.lan_particles.len(), 3);
        // A block particle needs a state.
        let block = ParticleOptions::simple(ParticleType::Block);
        assert!(!game.level_send_particles_at(level, &block, None, &at, [0.0; 3], 0.0, 1));
        assert_eq!(game.creatures.lan_particles.len(), 3);
        assert!(send(&mut game, &[]));
        assert_eq!(game.creatures.lan_particles.len(), 3);
        // The requests share one copy of the options, and a batch over the cap is refused
        // whole.
        assert!(Arc::ptr_eq(
            &queued_options(&game, 0),
            &queued_options(&game, 2)
        ));
        let full = vec![[0.0, 64.0, 0.0]; MAX_PARTICLE_POSITIONS];
        assert!(send(&mut game, &full));
        assert_eq!(
            game.creatures.lan_particles.len(),
            3 + MAX_PARTICLE_POSITIONS
        );
        let over = vec![[0.0, 64.0, 0.0]; MAX_PARTICLE_POSITIONS + 1];
        assert!(!send(&mut game, &over));
        assert_eq!(
            game.creatures.lan_particles.len(),
            3 + MAX_PARTICLE_POSITIONS
        );
    }
}
