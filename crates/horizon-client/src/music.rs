//! Situational music: `MusicManager.tick` choosing tracks from
//! `Minecraft.getSituationalMusic`, with the `minecraft:audio/background_music`
//! and `minecraft:audio/music_volume` environment attributes of dimension
//! types and biomes (a biome value overrides its dimension's).
use horizon_core::random::{LegacyRandom, RandomSource};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

/// `Music`: a sound event and the delay range (ticks) before it starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Music {
    pub sound: String,
    pub min_delay: i32,
    pub max_delay: i32,
    pub replace_current_music: bool,
}
impl Music {
    fn new(sound: &str, min_delay: i32, max_delay: i32, replace_current_music: bool) -> Self {
        Self {
            sound: sound.to_owned(),
            min_delay,
            max_delay,
            replace_current_music,
        }
    }
    /// `Musics.MENU`.
    pub fn menu() -> Self {
        Self::new("minecraft:music.menu", 20, 600, true)
    }
    /// `Musics.CREDITS`.
    pub fn credits() -> Self {
        Self::new("minecraft:music.credits", 0, 0, true)
    }
    /// `Musics.END_BOSS`.
    pub fn end_boss() -> Self {
        Self::new("minecraft:music.dragon", 0, 0, true)
    }
    /// `Music.CODEC`.
    fn parse(v: &Value) -> Option<Self> {
        let sound = v["sound"].as_str()?;
        let int = |k: &str| v[k].as_i64().unwrap_or(0) as i32;
        Some(Self::new(
            sound,
            int("min_delay"),
            int("max_delay"),
            v["replace_current_music"].as_bool().unwrap_or(false),
        ))
    }
}
/// `BackgroundMusic`: the attribute value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BackgroundMusic {
    pub default: Option<Music>,
    pub creative: Option<Music>,
    pub underwater: Option<Music>,
}
impl BackgroundMusic {
    fn parse(v: &Value) -> Self {
        Self {
            default: Music::parse(&v["default"]),
            creative: Music::parse(&v["creative"]),
            underwater: Music::parse(&v["underwater"]),
        }
    }
    /// `BackgroundMusic.select`.
    pub fn select(&self, creative: bool, underwater: bool) -> Option<&Music> {
        if underwater && self.underwater.is_some() {
            self.underwater.as_ref()
        } else if creative && self.creative.is_some() {
            self.creative.as_ref()
        } else {
            self.default.as_ref()
        }
    }
}
#[derive(Clone, Debug, Default)]
struct Layer {
    music: Option<BackgroundMusic>,
    volume: Option<f32>,
}
impl Layer {
    fn parse(json: &Value) -> Self {
        let attributes = &json["attributes"];
        let music = &attributes["minecraft:audio/background_music"];
        Self {
            music: (!music.is_null()).then(|| BackgroundMusic::parse(music)),
            volume: attributes["minecraft:audio/music_volume"]
                .as_f64()
                .map(|v| v.clamp(0.0, 1.0) as f32),
        }
    }
}
/// Music attributes by dimension type and biome.
#[derive(Clone, Debug, Default)]
pub struct MusicTables {
    dimensions: HashMap<String, Layer>,
    biomes: HashMap<String, Layer>,
}
impl MusicTables {
    pub fn load(root: &Path) -> Self {
        let read = |dir: &str| -> HashMap<String, Layer> {
            let Ok(entries) = fs::read_dir(root.join("data/minecraft").join(dir)) else {
                return HashMap::new();
            };
            entries
                .flatten()
                .filter_map(|e| {
                    let path = e.path();
                    let name = path.file_stem()?.to_str()?.to_owned();
                    let json: Value = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
                    Some((format!("minecraft:{name}"), Layer::parse(&json)))
                })
                .collect()
        };
        Self {
            dimensions: read("dimension_type"),
            biomes: read("worldgen/biome"),
        }
    }
    /// `EnvironmentAttributes.BACKGROUND_MUSIC` at a position.
    pub fn background(&self, dimension: &str, biome: &str) -> BackgroundMusic {
        self.biomes
            .get(biome)
            .and_then(|l| l.music.clone())
            .or_else(|| self.dimensions.get(dimension).and_then(|l| l.music.clone()))
            .unwrap_or_default()
    }
    /// `EnvironmentAttributes.MUSIC_VOLUME` at a position (default 1).
    pub fn volume(&self, dimension: &str, biome: &str) -> f32 {
        self.biomes
            .get(biome)
            .and_then(|l| l.volume)
            .or_else(|| self.dimensions.get(dimension).and_then(|l| l.volume))
            .unwrap_or(1.0)
    }
}
/// `MusicManager.MusicFrequency` (the `musicFrequency` option).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MusicFrequency {
    #[default]
    Default,
    Frequent,
    Constant,
}
impl MusicFrequency {
    pub fn parse(name: &str) -> Self {
        match name {
            "FREQUENT" => Self::Frequent,
            "CONSTANT" => Self::Constant,
            _ => Self::Default,
        }
    }
    fn max_frequency(self) -> i32 {
        match self {
            Self::Default => 20 * 1200,
            Self::Frequent => 10 * 1200,
            Self::Constant => 0,
        }
    }
    fn next_song_delay(self, music: Option<&Music>, random: &mut LegacyRandom) -> i32 {
        let Some(music) = music else {
            return self.max_frequency();
        };
        if self == Self::Constant {
            return 100;
        }
        let min = music.min_delay.min(self.max_frequency());
        let max = music.max_delay.min(self.max_frequency());
        next_int(random, min, max)
    }
}
/// `Mth.nextInt(random, min, max)`: inclusive.
fn next_int(random: &mut LegacyRandom, min: i32, max: i32) -> i32 {
    if min >= max {
        min
    } else {
        random.next_int_bound(max - min + 1).unwrap_or(0) + min
    }
}
/// What the music manager drives (the sound engine's music channel).
pub trait MusicOutput {
    /// Start a track; false when nothing can play.
    fn play(&mut self, event: &str) -> bool;
    fn stop(&mut self);
    fn is_active(&mut self) -> bool;
    /// `SoundManager.updateCategoryVolume(MUSIC, gain)`.
    fn set_gain(&mut self, gain: f32);
}
impl MusicOutput for crate::audio::Audio {
    fn play(&mut self, event: &str) -> bool {
        self.play_music(event)
    }
    fn stop(&mut self) {
        self.stop_music();
    }
    fn is_active(&mut self) -> bool {
        self.music_active()
    }
    fn set_gain(&mut self, gain: f32) {
        self.set_music_gain(gain);
    }
}
/// `MusicManager`.
pub struct MusicManager {
    random: LegacyRandom,
    /// The sound event of the current track.
    current: Option<String>,
    current_gain: f32,
    next_song_delay: i32,
    pub frequency: MusicFrequency,
}
impl Default for MusicManager {
    fn default() -> Self {
        // RandomSource.create(): seeded from the clock, not the world.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as i64);
        Self::new(seed)
    }
}
impl MusicManager {
    pub fn new(seed: i64) -> Self {
        Self {
            random: LegacyRandom::new(seed),
            current: None,
            current_gain: 1.0,
            next_song_delay: 100,
            frequency: MusicFrequency::Default,
        }
    }
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }
    /// `MusicManager.tick` with the situational music and music volume;
    /// `loading` holds back a new track (`LevelLoadingScreen`).
    pub fn tick(
        &mut self,
        music: Option<&Music>,
        volume: f32,
        loading: bool,
        out: &mut impl MusicOutput,
    ) {
        if self.current.is_some() && self.current_gain != volume && !self.fade(volume, music, out) {
            return;
        }
        let Some(music) = music else {
            self.next_song_delay = self.next_song_delay.max(100);
            return;
        };
        if let Some(current) = &self.current {
            if music.replace_current_music && music.sound != *current {
                out.stop();
                self.next_song_delay = next_int(&mut self.random, 0, music.min_delay / 2);
            }
            if !out.is_active() {
                self.current = None;
                let delay = self
                    .frequency
                    .next_song_delay(Some(music), &mut self.random);
                self.next_song_delay = self.next_song_delay.min(delay);
            }
        }
        self.next_song_delay = self.next_song_delay.min(music.max_delay);
        if self.current.is_none() && !loading {
            self.next_song_delay -= 1;
            if self.next_song_delay <= 0 {
                self.start(music, out);
            }
        }
    }
    /// `MusicManager.startPlaying`.
    fn start(&mut self, music: &Music, out: &mut impl MusicOutput) {
        // The instance is current even if it could not start; the next tick
        // finds it inactive and schedules another.
        out.play(&music.sound);
        self.current = Some(music.sound.clone());
        self.next_song_delay = i32::MAX;
    }
    /// `MusicManager.stopPlaying`.
    pub fn stop(&mut self, situational: Option<&Music>, out: &mut impl MusicOutput) {
        if self.current.take().is_some() {
            out.stop();
        }
        self.next_song_delay = self
            .frequency
            .next_song_delay(situational, &mut self.random)
            + 100;
    }
    /// `MusicManager.fadePlaying`: false once the track faded out and stopped.
    fn fade(
        &mut self,
        volume: f32,
        situational: Option<&Music>,
        out: &mut impl MusicOutput,
    ) -> bool {
        if self.current.is_none() {
            return false;
        }
        if self.current_gain == volume {
            return true;
        }
        if self.current_gain < volume {
            self.current_gain += self.current_gain.clamp(5.0E-4, 0.005);
            if self.current_gain > volume {
                self.current_gain = volume;
            }
        } else {
            self.current_gain = 0.03 * volume + 0.97 * self.current_gain;
            if (self.current_gain - volume).abs() < 1.0E-4 || self.current_gain < volume {
                self.current_gain = volume;
            }
        }
        self.current_gain = self.current_gain.clamp(0.0, 1.0);
        if self.current_gain <= 1.0E-4 {
            self.stop(situational, out);
            false
        } else {
            out.set_gain(self.current_gain);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        playing: Option<String>,
        started: Vec<String>,
        gain: f32,
    }
    impl MusicOutput for Fake {
        fn play(&mut self, event: &str) -> bool {
            self.playing = Some(event.to_owned());
            self.started.push(event.to_owned());
            true
        }
        fn stop(&mut self) {
            self.playing = None;
        }
        fn is_active(&mut self) -> bool {
            self.playing.is_some()
        }
        fn set_gain(&mut self, gain: f32) {
            self.gain = gain;
        }
    }

    #[test]
    fn menu_music_starts_within_its_delay_and_game_music_replaces_nothing() {
        let mut m = MusicManager::new(1);
        let mut out = Fake::default();
        let menu = Music::menu();
        // nextSongDelay starts at 100, capped to MENU's max delay 600: starts on tick 100.
        for _ in 0..99 {
            m.tick(Some(&menu), 1.0, false, &mut out);
        }
        assert!(out.started.is_empty());
        m.tick(Some(&menu), 1.0, false, &mut out);
        assert_eq!(out.started, ["minecraft:music.menu"]);
        // Game music does not replace the current track.
        let game = Music::new("minecraft:music.game", 12000, 24000, false);
        m.tick(Some(&game), 1.0, false, &mut out);
        assert_eq!(m.current(), Some("minecraft:music.menu"));
        // When the menu track ends, the next delay comes from the game music range.
        out.playing = None;
        m.tick(Some(&game), 1.0, false, &mut out);
        assert!(m.current().is_none());
        assert!((12000..=24000).contains(&m.next_song_delay));
    }

    #[test]
    fn replacing_music_stops_the_current_track() {
        let mut m = MusicManager::new(2);
        let mut out = Fake::default();
        m.start(&Music::menu(), &mut out);
        let end = Music::new("minecraft:music.end", 6000, 24000, true);
        m.tick(Some(&end), 1.0, false, &mut out);
        assert!(out.playing.is_none());
        assert!(m.current().is_none());
        assert!(m.next_song_delay <= 3000);
    }

    #[test]
    fn zero_music_volume_fades_out_and_stops() {
        let mut m = MusicManager::new(3);
        let mut out = Fake::default();
        let game = Music::new("minecraft:music.game", 12000, 24000, false);
        m.start(&game, &mut out);
        let mut ticks = 0;
        while m.current().is_some() && ticks < 1000 {
            m.tick(Some(&game), 0.0, false, &mut out);
            ticks += 1;
        }
        // 0.97^n < 1e-4 after ~303 ticks.
        assert!((290..=310).contains(&ticks), "{ticks}");
        assert!(out.playing.is_none());
    }

    #[test]
    fn select_prefers_underwater_then_creative() {
        let bg = BackgroundMusic {
            default: Some(Music::new("a", 0, 0, false)),
            creative: Some(Music::new("b", 0, 0, false)),
            underwater: Some(Music::new("c", 0, 0, false)),
        };
        assert_eq!(bg.select(true, true).unwrap().sound, "c");
        assert_eq!(bg.select(true, false).unwrap().sound, "b");
        assert_eq!(bg.select(false, false).unwrap().sound, "a");
    }
}
