//! Jukeboxes (`JukeboxBlock`, `JukeboxBlockEntity`, `JukeboxSongPlayer`):
//! inserting a `jukebox_playable` disc, ejecting it, the song with its
//! "Now Playing" overlay, note particles every second, the redstone
//! signal while playing, the comparator output, and saving.
use super::Game;
use horizon_client::{
    containers::{BlockEntity, Jukebox},
    items::ItemStack,
    particles::{ParticleOptions, ParticleType},
    text::Text,
};
use horizon_core::{position::BlockPos, random::RandomSource};
use horizon_world::level::flags as level_flags;

impl Game<'_> {
    fn jukebox(&self, pos: [i32; 3]) -> Option<&Jukebox> {
        match self.world.block_entities.get(&(pos[0], pos[1], pos[2])) {
            Some(BlockEntity::Jukebox(j)) => Some(j),
            _ => None,
        }
    }
    fn set_has_record(&mut self, pos: [i32; 3], has: bool) {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let registry = self.res.assets.registry;
        let Some(mut s) = registry.state(state).cloned() else {
            return;
        };
        s.properties.insert(
            "has_record".into(),
            if has { "true" } else { "false" }.into(),
        );
        if let Some(new) = registry.id(&s) {
            let p = BlockPos::new(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| {
                b.set_block(level, ticks, p, new, level_flags::UPDATE_CLIENTS)
            });
        }
    }
    /// `onSongChanged`: neighbours (the signal) and comparators.
    fn jukebox_changed(&mut self, pos: [i32; 3]) {
        let p = BlockPos::new(pos[0], pos[1], pos[2]);
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| {
            let block = b.block(state);
            b.update_neighbors_at(level, ticks, p, block);
        });
        self.container_changed((pos[0], pos[1], pos[2]));
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
    }
    /// `JukeboxBlock.useItemOn` / `useWithoutItem` on the jukebox at `pos`.
    pub(super) fn use_jukebox(&mut self, pos: [i32; 3], held: Option<&ItemStack>) -> bool {
        if &*self
            .res
            .facts
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name
            != "minecraft:jukebox"
        {
            return false;
        }
        if self.jukebox(pos).is_some_and(|j| j.record.is_some()) {
            self.eject_record(pos);
            return true;
        }
        let Some(stack) = held else { return false };
        let Some(song) = self
            .res
            .items
            .get(&stack.item)
            .and_then(|i| i.jukebox_song.clone())
        else {
            return false;
        };
        // JukeboxPlayable.tryInsertIntoJukebox
        let record = ItemStack {
            count: 1,
            ..stack.clone()
        };
        if !self.creative {
            self.consume_held();
        }
        self.world.block_entities.insert(
            (pos[0], pos[1], pos[2]),
            BlockEntity::Jukebox(Jukebox {
                record: Some(record),
                ticks: 0,
                playing: true,
            }),
        );
        self.set_has_record(pos, true);
        self.start_song(pos, &song);
        self.jukebox_changed(pos);
        true
    }
    /// Level event 1010: the song from the jukebox, and the "Now Playing" overlay.
    fn start_song(&mut self, pos: [i32; 3], song: &str) {
        self.stop_song(pos);
        let Some(s) = self.res.items.songs.get(song).cloned() else {
            return;
        };
        // LevelRenderer.playJukeboxSong: SimpleSoundInstance.forJukeboxSong.
        let id = self.audio.play_jukebox_song(&s.sound, pos);
        self.jukebox_sounds.insert((pos[0], pos[1], pos[2]), id);
        let description =
            horizon_client::component::to_text(&s.description, &mut |k| Text::literal(k));
        self.titles
            .set_overlay(Text::translate("record.nowPlaying", &[description]), true);
        self.update_song_volumes();
    }
    fn stop_song(&mut self, pos: [i32; 3]) {
        if let Some(id) = self.jukebox_sounds.remove(&(pos[0], pos[1], pos[2])) {
            self.audio.stop_and_release(id);
        }
    }
    /// `popOutTheItem`: the disc above the jukebox, and the song stops.
    fn eject_record(&mut self, pos: [i32; 3]) {
        let key = (pos[0], pos[1], pos[2]);
        let record = match self.world.block_entities.get_mut(&key) {
            Some(BlockEntity::Jukebox(j)) => {
                j.playing = false;
                j.ticks = 0;
                j.record.take()
            }
            _ => None,
        };
        self.stop_song(pos);
        if let Some(record) = record {
            let r = &mut self.world.random;
            // offsetRandomXZ(random, 0.7)
            let dx = f64::from(r.next_float() * 2.0 - 1.0) * 0.7 / 2.0;
            let dz = f64::from(r.next_float() * 2.0 - 1.0) * 0.7 / 2.0;
            let at = [
                f64::from(pos[0]) + 0.5 + dx,
                f64::from(pos[1]) + 1.01,
                f64::from(pos[2]) + 0.5 + dz,
            ];
            self.spawn_item(record, at, [0.0, 0.0, 0.0], 10);
        }
        self.set_has_record(pos, false);
        self.jukebox_changed(pos);
    }
    /// A broken jukebox stops its song (its disc drops with the contents).
    pub(super) fn jukebox_removed(&mut self, pos: [i32; 3]) {
        self.stop_song(pos);
    }
    /// `JukeboxSongPlayer.tick` for every playing jukebox.
    pub(super) fn tick_jukeboxes(&mut self) {
        let playing: Vec<((i32, i32, i32), String, i64)> = self
            .world
            .block_entities
            .iter()
            .filter_map(|(k, e)| match e {
                BlockEntity::Jukebox(j) if j.playing => {
                    let song = self
                        .res
                        .items
                        .get(&j.record.as_ref()?.item)?
                        .jukebox_song
                        .clone()?;
                    Some((*k, song, j.ticks))
                }
                _ => None,
            })
            .collect();
        for (key, song, ticks) in playing {
            let pos = [key.0, key.1, key.2];
            let Some(s) = self.res.items.songs.get(&song).cloned() else {
                continue;
            };
            if s.finished(ticks) {
                if let Some(BlockEntity::Jukebox(j)) = self.world.block_entities.get_mut(&key) {
                    j.playing = false;
                    j.ticks = 0;
                }
                self.stop_song(pos);
                self.jukebox_changed(pos);
                continue;
            }
            if !self.jukebox_sounds.contains_key(&key) && ticks == 0 {
                self.start_song(pos, &song);
            }
            if ticks % 20 == 0 {
                let color = f64::from(self.world.random.next_int_bound(4).unwrap_or(0)) / 24.0;
                let at = [
                    f64::from(pos[0]) + 0.5,
                    f64::from(pos[1]) + 1.2,
                    f64::from(pos[2]) + 0.5,
                ];
                self.particles.add(
                    &ParticleOptions::simple(ParticleType::Note),
                    at,
                    [color, 0.0, 0.0],
                );
            }
            if let Some(BlockEntity::Jukebox(j)) = self.world.block_entities.get_mut(&key) {
                j.ticks += 1;
            }
        }
        self.update_song_volumes();
    }
    /// Let go of songs that ended or whose jukebox stopped.
    fn update_song_volumes(&mut self) {
        let sounds: Vec<_> = self
            .jukebox_sounds
            .iter()
            .map(|(k, id)| (*k, *id))
            .collect();
        for (k, id) in sounds {
            if !self.audio.engine.is_active(id)
                || !matches!(self.world.block_entities.get(&k), Some(BlockEntity::Jukebox(j)) if j.playing)
            {
                self.audio.stop_and_release(id);
                self.jukebox_sounds.remove(&k);
            }
        }
    }
}
/// The jukebox's redstone signal (15 while a song plays) and comparator output.
pub(super) fn jukebox_signals(
    world: &horizon_client::world::World,
    items: &horizon_client::items::Items,
    pos: BlockPos,
) -> Option<(i32, i32)> {
    match world.block_entities.get(&(pos.x, pos.y, pos.z)) {
        Some(BlockEntity::Jukebox(j)) => {
            let comparator = j
                .record
                .as_ref()
                .and_then(|r| items.get(&r.item)?.jukebox_song.as_ref())
                .and_then(|s| items.songs.get(s))
                .map_or(0, |s| s.comparator);
            Some((if j.playing { 15 } else { 0 }, comparator))
        }
        _ => None,
    }
}
