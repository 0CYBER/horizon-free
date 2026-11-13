//! Sounds and particles in a level for native server mods, through a
//! `LevelRef` (docs/native-mods.md, "Entity and level handles", #7387).
//! Every player in range hears or sees them, as `ServerLevel.playSound` and
//! `sendParticles` send them. They return false on a client, for a handle of
//! an earlier open, or for arguments the network can't carry.
use super::{Dim, Game, creatures::NativeSound, lan_particles::QueuedParticle};
use horizon_client::mod_api::{LevelRef, ParticleOptions};
use horizon_client::particles::ParticleType;
use horizon_world::registry::BlockStateId;

impl Game<'_> {
    /// The level's dimension id, for a handle of this open on the server.
    fn native_level_dimension(&self, level: LevelRef) -> Option<&'static str> {
        self.native_level_world(level)?;
        [Dim::Overworld, Dim::Nether, Dim::End]
            .into_iter()
            .map(Dim::id)
            .find(|id| *id == level.dimension())
    }
    /// Plays sound event `sound` at `at` for the players in this level
    /// within range (16 blocks, times the volume above 1), as
    /// `ServerLevel.playSound(null, ...)` does: `source` indexes
    /// `SoundSource` (`master` 0, `blocks` 4...). A `minecraft:` event must be
    /// registered; a mod's goes by name and is heard where its resources
    /// define it. True means queued, not heard. API 568.
    pub(super) fn native_level_play_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: usize,
        volume: f32,
        pitch: f32,
    ) -> bool {
        let Some(dimension) = self.native_level_dimension(level) else {
            return false;
        };
        if !mod_sound_ok(sound, at, source, volume, pitch) {
            return false;
        }
        self.creatures.lan_sounds.push(NativeSound {
            dimension,
            sound: sound.to_owned(),
            at,
            source: source as i32,
            volume,
            pitch,
        });
        true
    }
    /// Sends `count` particles around `at` to the players in this level
    /// within 32 blocks, as `ServerLevel.sendParticles` does: each offset is
    /// a Gaussian times `spread`, and `speed` scales their motion (a zero
    /// count sends one particle moving along `spread`). Block particle types
    /// need `state`; item, trail and vibration particles aren't supported.
    /// True means queued, not drawn. API 568.
    pub(super) fn native_level_send_particles(
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
            options: options.clone(),
            state,
            at,
            spread,
            speed: [speed; 3],
            count,
            randomization: 0,
        });
        true
    }
}

/// Whether `id` is a sound event's id, `namespace:path` as `Identifier`
/// allows: the namespace of `[a-z0-9_.-]`, the path also with `/`.
/// Whether a native mod's sound can be sent: a namespaced event, a sound
/// source, a position inside the world's range and a finite, non-negative
/// volume and pitch.
pub(super) fn mod_sound_ok(
    sound: &str,
    at: [f64; 3],
    source: usize,
    volume: f32,
    pitch: f32,
) -> bool {
    let position_ok = at
        .into_iter()
        .all(|v| v.is_finite() && v.abs() <= f64::from(i32::MAX) / 8.0);
    let gain_ok = volume.is_finite() && volume >= 0.0 && pitch.is_finite() && pitch >= 0.0;
    sound_id(sound)
        && source < horizon_client::settings::SOUND_SOURCES.len()
        && position_ok
        && gain_ok
}

/// Whether a native mod's particles can be sent. Block particle types need
/// a known state (`state_ok`); item, trail and vibration particles aren't
/// supported by the network dispatch; a mod's own type must be namespaced.
pub(super) fn mod_particles_ok(
    options: &ParticleOptions,
    state: Option<BlockStateId>,
    state_ok: impl Fn(BlockStateId) -> bool,
    at: [f64; 3],
    spread: [f64; 3],
    speed: f64,
) -> bool {
    use ParticleType as T;
    // A native mod's own type: the name has to be namespaced, and a client
    // draws it only when its pack has the sprite set.
    if let Some(name) = &options.mod_particle {
        let namespaced = name
            .split_once(':')
            .is_some_and(|(ns, path)| !ns.is_empty() && !path.is_empty());
        if !namespaced {
            return false;
        }
    }
    let supported = match options.ty {
        T::Block | T::BlockMarker | T::FallingDust | T::DustPillar | T::BlockCrumble => {
            state.is_some_and(state_ok)
        }
        T::Item | T::Trail | T::Vibration => false,
        T::Geyser | T::GeyserPlume | T::GeyserBase | T::GeyserPoof => options
            .geyser
            .is_some_and(|(_, impulse)| impulse.is_finite()),
        _ => true,
    };
    supported
        && at.into_iter().all(f64::is_finite)
        && spread
            .into_iter()
            .all(|v| v.is_finite() && v.abs() <= f32::MAX as f64)
        && speed.is_finite()
        && speed.abs() <= f32::MAX as f64
        && options.color.into_iter().all(f32::is_finite)
        && options.to_color.into_iter().all(f32::is_finite)
        && options.scale.is_finite()
}

pub(super) fn sound_id(id: &str) -> bool {
    let Some((namespace, path)) = id.split_once(':') else {
        return false;
    };
    let allowed = |c: char, slash: bool| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c) || (slash && c == '/')
    };
    !namespace.is_empty()
        && !path.is_empty()
        && namespace.chars().all(|c| allowed(c, false))
        && path.chars().all(|c| allowed(c, true))
}
