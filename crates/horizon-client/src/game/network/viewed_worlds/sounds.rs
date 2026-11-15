//! A viewed level's sounds (arismoko/horizon#3983, gap 12), as Immersive Portals' `CrossPortalSound` replays a
//! far level's sound through a portal: the engine only delivers the sound; a client mod's
//! `viewed_level_sound` says where in the player's own level it plays (`play_at`), and with
//! which volume and pitch. Nothing plays it otherwise, and nothing of it reaches the player's
//! level's own sound hooks.
use super::routing::RouteError;
use crate::game::{Game, level_sounds::LevelSound, native_hooks::level_sound_playable};
use horizon_protocol::{
    packets::play::{self, Clientbound as C},
    viewed_levels::ViewedLevelsError,
};
use mod_api::{HookId, Identifier, LevelRef, SoundSource};

#[cfg(test)]
thread_local! {
    /// How many times this thread asked whether `viewed_level_sound` has a listener.
    pub(super) static LISTENED_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether a mod listens to `viewed_level_sound`, asked once per payload before it is decoded:
/// without one, the decoder skips a run's sounds unread (`Message::decode_with`).
pub(super) fn listened() -> bool {
    #[cfg(test)]
    LISTENED_CHECKS.with(|n| n.set(n.get() + 1));
    mod_api::__private::listened(HookId::ViewedLevelSound)
}

/// A sound as the server sent it for a viewed level: its event (a registry id or an inline name),
/// source, position in that level (an entity-bound sound's, its emitter's there), volume, pitch
/// and seed.
pub(super) struct ViewedSound<'a> {
    pub sound: &'a (i32, Option<(String, Option<f32>)>),
    pub source: i32,
    pub position: [f64; 3],
    pub volume: f32,
    pub pitch: f32,
    pub seed: i64,
}

impl<'a> ViewedSound<'a> {
    /// A sound at a position, from fixed-point eighths of a block.
    pub(super) fn at_position(p: &'a play::Sound) -> Self {
        Self {
            sound: &p.sound,
            source: p.source,
            position: [p.x, p.y, p.z].map(|c| f64::from(c) / 8.0),
            volume: p.volume,
            pitch: p.pitch,
            seed: p.seed,
        }
    }
}

/// A viewed level's sound as it plays in the player's level, once the listeners answered.
#[derive(Debug, PartialEq)]
pub(super) struct ViewedPlay {
    pub name: String,
    pub source: SoundSource,
    pub at: [f64; 3],
    pub volume: f32,
    pub pitch: f32,
    pub seed: i64,
}

impl Game<'_> {
    /// A sound in a `run` of viewed level `level`, kept by the decoder because a mod listens: its
    /// play clientbound id and bytes, which the codec read exactly. One bound to an entity is at its emitter in the viewed world, and plays nothing
    /// when the world holds no such entity; it plays where the listener puts it, and does not
    /// follow the emitter.
    pub(super) fn viewed_level_sound(
        &mut self,
        level: i32,
        id: i32,
        body: &[u8],
    ) -> Result<(), RouteError> {
        let packet = C::decode(id, body).map_err(|_| ViewedLevelsError::MalformedPacket(id))?;
        let play = match &packet {
            C::Sound(p) => self.ask_viewed_level_sound(level, ViewedSound::at_position(p)),
            C::SoundEntity(p) => {
                let emitter = self.remote.as_ref().and_then(|r| {
                    let entity = r.viewed.get(level)?.entities.get(p.id)?;
                    Some(entity.position)
                });
                emitter.and_then(|position| {
                    let sound = ViewedSound {
                        sound: &p.sound,
                        source: p.source,
                        position,
                        volume: p.volume,
                        pitch: p.pitch,
                        seed: p.seed,
                    };
                    self.ask_viewed_level_sound(level, sound)
                })
            }
            _ => None,
        };
        if let Some(play) = play {
            self.audio.play_on_seeded(
                play.source.index(),
                &play.name,
                Some(play.at),
                play.volume,
                play.pitch,
                play.seed,
            );
        }
        Ok(())
    }

    /// `viewed_level_sound` for sound `p` of viewed level `level`: where it plays in the player's
    /// level, with the volume and pitch the listeners left and the server's seed. None when they
    /// left `play_at` None, or left a position that is not finite, or a volume or pitch that is
    /// not finite and at least 0 (`level_sound_playable`).
    pub(super) fn ask_viewed_level_sound(
        &mut self,
        level: i32,
        p: ViewedSound<'_>,
    ) -> Option<ViewedPlay> {
        let name = self
            .remote_sound_name(p.sound)
            .filter(|name| !self.pvp_mutes_sound(name))?;
        let sound = Identifier::parse(&name).ok()?;
        let handle = self.viewed_level_handle(level)?;
        let mut source = LevelSound::new(&name, p.position, p.volume, p.pitch).source;
        if let Some(sent) = usize::try_from(p.source)
            .ok()
            .and_then(|s| SoundSource::ALL.get(s))
        {
            source = *sent;
        }
        let (mut volume, mut pitch, mut play_at) = (p.volume, p.pitch, None);
        self.native_viewed_level_sound(mod_api::__private::ViewedLevelSoundCall {
            level: handle,
            sound: &sound,
            source,
            position: p.position,
            volume: &mut volume,
            pitch: &mut pitch,
            seed: p.seed,
            play_at: &mut play_at,
        });
        let at = play_at.filter(|&at| level_sound_playable(at, volume, pitch))?;
        Some(ViewedPlay {
            name,
            source,
            at,
            volume,
            pitch,
            seed: p.seed,
        })
    }

    /// The handle of viewed level `level` (`host.level`'s for it): its key and the client's epoch.
    fn viewed_level_handle(&self, level: i32) -> Option<LevelRef> {
        let epoch = self.native_lifecycle.epoch()?;
        let world = self.remote.as_ref()?.viewed.get(level)?;
        Some(LevelRef::new(
            epoch,
            mod_api::__private::dimension_key(&world.level),
        ))
    }
}
