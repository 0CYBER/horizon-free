use super::*;
use crate::game::Audio;
use horizon_core::{geometry::Direction, position::BlockPos};

impl Game<'_> {
    pub(in crate::game) fn play_mining_hit_sound(
        &mut self,
        state: BlockStateId,
        pos: [i32; 3],
        face: [i32; 3],
        unshared: bool,
    ) {
        let mut cancel = false;
        // A face that names no direction (never from a block hit) skips the hook, not the sound.
        if mod_api::__private::has_scoped_hook(mod_api::HookId::PlayHitSound)
            && let Some(face) = Direction::ALL.into_iter().find(|d| d.normal() == face)
        {
            self.native_play_hit_sound(super::fire::PlayHitSoundCall {
                state,
                pos: BlockPos::new(pos[0], pos[1], pos[2]),
                face,
                cancel: &mut cancel,
            });
        }
        if cancel {
            return;
        }
        let sounds = self.res.facts().get(state).sounds.clone();
        let at = Some(pos.map(|v| f64::from(v) + 0.5));
        let volume = (sounds.volume + 1.0) / 8.0;
        let pitch = sounds.pitch * 0.5;
        let play: fn(&mut Audio, &str, Option<[f64; 3]>, f32, f32) = if unshared {
            Audio::play_unshared
        } else {
            Audio::play
        };
        play(&mut self.audio, &sounds.hit, at, volume, pitch);
    }

    pub(in crate::game) fn native_play_hit_sound(
        &mut self,
        call: super::fire::PlayHitSoundCall<'_>,
    ) {
        self.fire_play_hit_sound(call);
    }
}
