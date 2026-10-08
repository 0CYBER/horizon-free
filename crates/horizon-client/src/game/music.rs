//! `Minecraft.getSituationalMusic` and `getMusicVolume` for the music manager.
use super::Game;
use horizon_client::{blocks::Fluid, music::Music};

impl Game<'_> {
    /// `Minecraft.tick` -> `MusicManager.tick`, once per client tick.
    pub(super) fn tick_music(&mut self) {
        let (music, volume) = self.situational_music();
        // LevelLoadingScreen: no new track before the player is in the world.
        let loading = self.title.is_none() && self.player.is_none();
        self.music.tick(music.as_ref(), volume, loading, &mut self.audio);
    }
    fn situational_music(&self) -> (Option<Music>, f32) {
        // Title screen: no player, Musics.MENU at full volume.
        if self.title.is_some() {
            return (Some(Music::menu()), 1.0);
        }
        let Some(eye) = self.eye_position() else { return (Some(Music::menu()), 1.0) };
        let (x, y, z) = (eye[0].floor() as i32, eye[1].floor() as i32, eye[2].floor() as i32);
        let biome = self
            .world
            .chunks
            .get(&(x >> 4, z >> 4))
            .map_or("minecraft:plains", |c| self.res.biomes.list[c.biome(x, y, z) as usize].name.as_str());
        let dimension = self.dim.id();
        let tables = &self.res.music;
        // Player.isUnderWater: eyes in water.
        let underwater = self.res.facts.get(self.world.block(x, y, z)).fluid == Fluid::Water;
        let music = tables.background(dimension, biome).select(self.creative, underwater).cloned();
        (music, tables.volume(dimension, biome))
    }
}

impl Game<'_> {
    /// The local player's ambient sound handlers (LocalPlayer.tick).
    pub(super) fn tick_ambience(&mut self) {
        let Some(p) = self.player.as_ref() else { return };
        let feet = [p.position.x, p.position.y, p.position.z];
        let eye_y = feet[1] + f64::from(p.dimensions.eye_height);
        let (x, z) = (feet[0].floor() as i32, feet[2].floor() as i32);
        let world = &self.world;
        let biome = world
            .chunks
            .get(&(x >> 4, z >> 4))
            .map_or("minecraft:plains", |c| self.res.biomes.list[c.biome(x, feet[1].floor() as i32, z) as usize].name.as_str());
        let sounds = self.res.ambient.get(self.dim.id(), biome);
        let underwater = self.res.facts.get(world.block(x, eye_y.floor() as i32, z)).fluid == Fluid::Water;
        let light = |b: [i32; 3]| {
            let l = world.light(b[0], b[1], b[2]);
            (l >> 4, l & 15)
        };
        let ctx = horizon_client::ambience::AmbientContext { sounds: &sounds, position: feet, eye_y, underwater, light: &light };
        self.ambience.tick(&ctx, &mut self.audio);
    }
}
