//! `Minecraft.getSituationalMusic` and `getMusicVolume` for the music manager,
//! the local player's ambient sound handlers and the sound engine tick.
use super::Game;
use horizon_client::{blocks::Fluid, music::Music, sound_engine::TickContext};

impl Game<'_> {
    /// `Minecraft.tick` -> `MusicManager.tick`, once per client tick.
    pub(super) fn tick_music(&mut self) {
        let (music, volume) = self.situational_music();
        // LevelLoadingScreen: no new track before the player is in the world.
        let loading = self.title.is_none() && self.player.is_none();
        self.music
            .tick(music.as_ref(), volume, loading, &mut self.audio);
    }
    /// `Minecraft.tick` -> `SoundManager.tick(pause)`, after the music, with
    /// `pauseAllExcept(MUSIC, UI)` when the game pauses and `resume` when it
    /// carries on.
    pub(super) fn tick_sound_engine(&mut self) {
        if self.pause != self.sound_paused {
            if self.pause {
                self.audio.pause();
            } else {
                self.audio.resume();
            }
            self.sound_paused = self.pause;
        }
        let ctx = TickContext {
            player_underwater: self.ambience.was_underwater(),
            player_removed: self.player.is_none(),
        };
        self.audio.tick(self.pause, &ctx);
    }
    /// `getSituationalMusic` and `getMusicVolume`.
    fn situational_music(&self) -> (Option<Music>, f32) {
        // Title screen: no player, Musics.MENU at full volume.
        if self.title.is_some() {
            return (Some(Music::menu()), 1.0);
        }
        let Some(eye) = self.eye_position() else {
            return (Some(Music::menu()), 1.0);
        };
        let biome = self.biome_at(eye);
        let dimension = self.dim.id();
        let tables = &self.res.music;
        let volume = tables.volume(dimension, biome);
        // Minecraft.getSituationalMusic: the boss overlay's music in the End.
        if self.dim == super::Dim::End && self.boss_music() {
            return (Some(Music::end_boss()), volume);
        }
        let music = tables
            .background(dimension, biome)
            .select(self.creative, self.ambience.was_underwater())
            .cloned();
        (music, volume)
    }
    /// `BiomeManager.getNoiseBiomeAtPosition`: the quart cell holding a position.
    fn biome_at(&self, p: [f64; 3]) -> &str {
        let (x, y, z) = (
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        );
        self.world
            .chunks
            .get(&(x >> 4, z >> 4))
            .map_or("minecraft:plains", |c| {
                self.res.biomes.list[c.biome(x, y, z) as usize]
                    .name
                    .as_str()
            })
    }
    /// `isEyeInFluid(WATER)`: the eye inside the water of its block column.
    fn eye_in_water(&self, x: f64, eye_y: f64, z: f64) -> bool {
        let (bx, by, bz) = (x.floor() as i32, eye_y.floor() as i32, z.floor() as i32);
        let f = self.res.facts.get(self.world.block(bx, by, bz));
        if f.fluid != Fluid::Water {
            return false;
        }
        let above = self.res.facts.get(self.world.block(bx, by + 1, bz));
        let height = if above.fluid == Fluid::Water {
            1.0
        } else {
            f64::from(f.fluid_level) / 9.0
        };
        eye_y <= f64::from(by) + height
    }
}

impl Game<'_> {
    /// The local player's ambient sound handlers (LocalPlayer.tick).
    pub(super) fn tick_ambience(&mut self) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let feet = [p.position.x, p.position.y, p.position.z];
        let eye_y = feet[1] + f64::from(p.dimensions.eye_height);
        let sounds = self.res.ambient.get(self.dim.id(), self.biome_at(feet));
        let underwater = self.eye_in_water(feet[0], eye_y, feet[2]);
        let world = &self.world;
        let light = |b: [i32; 3]| {
            let l = world.light(b[0], b[1], b[2]);
            (l >> 4, l & 15)
        };
        let ctx = horizon_client::ambience::AmbientContext {
            sounds: &sounds,
            position: feet,
            eye_y,
            underwater,
            spectator: self.spectator(),
            light: &light,
        };
        self.ambience.tick(&ctx, &mut self.audio);
    }
}
