//! The player's ambient sound handlers: `BiomeAmbientSoundsHandler` (biome
//! loops with 40-tick cross-fades, random additions, cave mood) driven by the
//! `minecraft:audio/ambient_sounds` attribute, `UnderwaterAmbientSoundHandler`
//! and the underwater loop (`UnderLiquidAmbientSoundInstance`).
use horizon_core::random::{LegacyRandom, RandomSource};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

/// `AmbientMoodSettings`.
#[derive(Clone, Debug, PartialEq)]
pub struct Mood {
    pub sound: String,
    pub tick_delay: i32,
    pub block_search_extent: i32,
    pub offset: f64,
}
/// `AmbientSounds`: the attribute value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AmbientSounds {
    pub r#loop: Option<String>,
    pub mood: Option<Mood>,
    /// (sound, tick chance)
    pub additions: Vec<(String, f64)>,
}
impl AmbientSounds {
    fn parse(v: &Value) -> Self {
        let mood = &v["mood"];
        let addition =
            |a: &Value| Some((a["sound"].as_str()?.to_owned(), a["tick_chance"].as_f64()?));
        Self {
            r#loop: v["loop"].as_str().map(str::to_owned),
            mood: mood["sound"].as_str().map(|sound| Mood {
                sound: sound.to_owned(),
                tick_delay: mood["tick_delay"].as_i64().unwrap_or(6000) as i32,
                block_search_extent: mood["block_search_extent"].as_i64().unwrap_or(8) as i32,
                offset: mood["offset"].as_f64().unwrap_or(2.0),
            }),
            // A single addition or a list of them.
            additions: match &v["additions"] {
                Value::Array(list) => list.iter().filter_map(addition).collect(),
                single => addition(single).into_iter().collect(),
            },
        }
    }
}
/// Ambient sound attributes by dimension type and biome.
#[derive(Clone, Debug, Default)]
pub struct AmbientTables {
    dimensions: HashMap<String, AmbientSounds>,
    biomes: HashMap<String, AmbientSounds>,
}
impl AmbientTables {
    pub fn load(root: &Path) -> Self {
        let read = |dir: &str| -> HashMap<String, AmbientSounds> {
            let Ok(entries) = fs::read_dir(root.join("data/minecraft").join(dir)) else {
                return HashMap::new();
            };
            entries
                .flatten()
                .filter_map(|e| {
                    let path = e.path();
                    let name = path.file_stem()?.to_str()?.to_owned();
                    let json: Value = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
                    let v = &json["attributes"]["minecraft:audio/ambient_sounds"];
                    (!v.is_null()).then(|| (format!("minecraft:{name}"), AmbientSounds::parse(v)))
                })
                .collect()
        };
        Self {
            dimensions: read("dimension_type"),
            biomes: read("worldgen/biome"),
        }
    }
    /// `EnvironmentAttributes.AMBIENT_SOUNDS` at a position.
    pub fn get(&self, dimension: &str, biome: &str) -> AmbientSounds {
        self.biomes
            .get(biome)
            .or_else(|| self.dimensions.get(dimension))
            .cloned()
            .unwrap_or_default()
    }
}
/// The sound engine side of the handlers.
pub trait AmbientOutput {
    /// A relative tickable instance; None when it could not start.
    fn play_tracked(&mut self, event: &str, volume: f32, looping: bool) -> Option<u64>;
    fn set_volume(&mut self, id: u64, volume: f32);
    fn stop(&mut self, id: u64);
    fn is_active(&self, id: u64) -> bool;
    /// A positional one-shot (`SimpleSoundInstance`, linear attenuation).
    fn play_at(&mut self, event: &str, at: [f64; 3]);
}
impl AmbientOutput for crate::audio::Audio {
    fn play_tracked(&mut self, event: &str, volume: f32, looping: bool) -> Option<u64> {
        crate::audio::Audio::play_tracked(self, event, volume, looping)
    }
    fn set_volume(&mut self, id: u64, volume: f32) {
        self.set_tracked_volume(id, volume);
    }
    fn stop(&mut self, id: u64) {
        self.stop_tracked(id);
    }
    fn is_active(&self, id: u64) -> bool {
        self.tracked_active(id)
    }
    fn play_at(&mut self, event: &str, at: [f64; 3]) {
        self.play(event, Some(at), 1.0, 1.0);
    }
}
/// `BiomeAmbientSoundsHandler.LoopSoundInstance`.
struct LoopSound {
    id: Option<u64>,
    fade: i32,
    direction: i32,
    stopped: bool,
}
impl LoopSound {
    fn tick(&mut self) -> f32 {
        if self.fade < 0 {
            self.stopped = true;
        }
        self.fade += self.direction;
        (self.fade as f32 / 40.0).clamp(0.0, 1.0)
    }
    fn fade_out(&mut self) {
        self.fade = self.fade.min(40);
        self.direction = -1;
    }
    fn fade_in(&mut self) {
        self.fade = self.fade.max(0);
        self.direction = 1;
    }
}
/// What the handlers read about the player and level each tick.
pub struct AmbientContext<'a> {
    pub sounds: &'a AmbientSounds,
    /// Feet and eye position.
    pub position: [f64; 3],
    pub eye_y: f64,
    pub underwater: bool,
    /// Raw (sky, block) light at a block.
    pub light: &'a dyn Fn([i32; 3]) -> (u8, u8),
}
/// The local player's ambient sound handlers.
pub struct Ambience {
    /// The client level's random.
    random: LegacyRandom,
    loops: Vec<(String, LoopSound)>,
    previous_loop: Option<String>,
    pub moodiness: f32,
    underwater_delay: i32,
    /// The underwater loop instance and its fade.
    underwater_loop: Option<(u64, i32)>,
    /// UnderLiquidSubSound instances (stop when the player leaves the water).
    sub_sounds: Vec<u64>,
    was_underwater: bool,
}
impl Default for Ambience {
    fn default() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as i64);
        Self::new(seed)
    }
}
impl Ambience {
    pub fn new(seed: i64) -> Self {
        Self {
            random: LegacyRandom::new(seed),
            loops: Vec::new(),
            previous_loop: None,
            moodiness: 0.0,
            underwater_delay: 0,
            underwater_loop: None,
            sub_sounds: Vec::new(),
            was_underwater: false,
        }
    }
    /// Stop everything (leaving the world).
    pub fn clear(&mut self, out: &mut impl AmbientOutput) {
        for (_, l) in self.loops.drain(..) {
            if let Some(id) = l.id {
                out.stop(id);
            }
        }
        if let Some((id, _)) = self.underwater_loop.take() {
            out.stop(id);
        }
        for id in self.sub_sounds.drain(..) {
            out.stop(id);
        }
        self.previous_loop = None;
        self.moodiness = 0.0;
        self.was_underwater = false;
    }
    /// `LocalPlayer.updateIsUnderwater` enter/exit sounds and loop, then
    /// `UnderwaterAmbientSoundHandler`, `BubbleColumn` (absent) and
    /// `BiomeAmbientSoundsHandler` ticks, then the tickable instances.
    pub fn tick(&mut self, ctx: &AmbientContext, out: &mut impl AmbientOutput) {
        let feet = ctx.position;
        if !self.was_underwater && ctx.underwater {
            out.play_at("minecraft:ambient.underwater.enter", feet);
            if let Some((id, _)) = self.underwater_loop.take() {
                out.stop(id);
            }
            self.underwater_loop = out
                .play_tracked("minecraft:ambient.underwater.loop", 0.0, true)
                .map(|id| (id, 0));
        }
        if self.was_underwater && !ctx.underwater {
            out.play_at("minecraft:ambient.underwater.exit", feet);
        }
        self.was_underwater = ctx.underwater;
        self.tick_underwater(ctx, out);
        self.tick_biome(ctx, out);
        self.tick_instances(ctx, out);
    }
    fn tick_underwater(&mut self, ctx: &AmbientContext, out: &mut impl AmbientOutput) {
        self.underwater_delay -= 1;
        if self.underwater_delay <= 0 && ctx.underwater {
            let rand = self.random.next_float();
            let sound = if rand < 1.0E-4 {
                Some("minecraft:ambient.underwater.loop.additions.ultra_rare")
            } else if rand < 0.001 {
                Some("minecraft:ambient.underwater.loop.additions.rare")
            } else if rand < 0.01 {
                Some("minecraft:ambient.underwater.loop.additions")
            } else {
                None
            };
            if let Some(sound) = sound {
                self.underwater_delay = 0;
                if let Some(id) = out.play_tracked(sound, 1.0, false) {
                    self.sub_sounds.push(id);
                }
            }
        }
    }
    fn tick_biome(&mut self, ctx: &AmbientContext, out: &mut impl AmbientOutput) {
        self.loops.retain(|(_, l)| !l.stopped);
        let sounds = ctx.sounds;
        if sounds.r#loop != self.previous_loop {
            self.previous_loop = sounds.r#loop.clone();
            for (_, l) in &mut self.loops {
                l.fade_out();
            }
            if let Some(event) = &sounds.r#loop {
                if !self.loops.iter().any(|(e, _)| e == event) {
                    let id = out.play_tracked(event, 0.0, true);
                    self.loops.push((
                        event.clone(),
                        LoopSound {
                            id,
                            fade: 0,
                            direction: 0,
                            stopped: false,
                        },
                    ));
                }
                if let Some((_, l)) = self.loops.iter_mut().find(|(e, _)| e == event) {
                    l.fade_in();
                }
            }
        }
        for (sound, chance) in &sounds.additions {
            if self.random.next_double() < *chance
                && let Some(id) = out.play_tracked(sound, 1.0, false)
            {
                // forAmbientAddition: a one-shot nobody ticks.
                let _ = id;
            }
        }
        let Some(mood) = &sounds.mood else { return };
        let extent = mood.block_search_extent;
        let span = extent * 2 + 1;
        let mut offset = || self.random.next_int_bound(span).unwrap_or(0) - extent;
        let (dx, dy, dz) = (offset(), offset(), offset());
        let sample = [
            (ctx.position[0] + f64::from(dx)).floor() as i32,
            (ctx.eye_y + f64::from(dy)).floor() as i32,
            (ctx.position[2] + f64::from(dz)).floor() as i32,
        ];
        let (sky, block) = (ctx.light)(sample);
        if sky > 0 {
            self.moodiness -= f32::from(sky) / 15.0 * 0.001;
        } else {
            self.moodiness -= (f32::from(block) - 1.0) / mood.tick_delay as f32;
        }
        if self.moodiness >= 1.0 {
            let d = [
                f64::from(sample[0]) + 0.5 - ctx.position[0],
                f64::from(sample[1]) + 0.5 - ctx.eye_y,
                f64::from(sample[2]) + 0.5 - ctx.position[2],
            ];
            let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let source = distance + mood.offset;
            out.play_at(
                &mood.sound,
                [
                    ctx.position[0] + d[0] / distance * source,
                    ctx.eye_y + d[1] / distance * source,
                    ctx.position[2] + d[2] / distance * source,
                ],
            );
            self.moodiness = 0.0;
        } else {
            self.moodiness = self.moodiness.max(0.0);
        }
    }
    /// The tickable instances' own ticks (SoundEngine.tickInGameSound).
    fn tick_instances(&mut self, ctx: &AmbientContext, out: &mut impl AmbientOutput) {
        for (_, l) in &mut self.loops {
            let volume = l.tick();
            if let Some(id) = l.id {
                if l.stopped {
                    out.stop(id);
                } else {
                    out.set_volume(id, volume);
                }
            }
        }
        // UnderLiquidAmbientSoundInstance: +1 per tick under water, -2 out of it.
        if let Some((id, fade)) = self.underwater_loop.as_mut() {
            if *fade >= 0 {
                *fade += if ctx.underwater { 1 } else { -2 };
                *fade = (*fade).min(40);
                out.set_volume(*id, (*fade as f32 / 40.0).clamp(0.0, 1.0));
            } else {
                out.stop(*id);
                self.underwater_loop = None;
            }
        }
        // UnderLiquidSubSound: stops when the player leaves the water.
        self.sub_sounds.retain(|&id| {
            let keep = ctx.underwater && out.is_active(id);
            if !keep {
                out.stop(id);
            }
            keep
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        next: u64,
        live: HashMap<u64, (String, f32)>,
        at: Vec<String>,
    }
    impl AmbientOutput for Fake {
        fn play_tracked(&mut self, event: &str, volume: f32, _looping: bool) -> Option<u64> {
            self.next += 1;
            self.live.insert(self.next, (event.to_owned(), volume));
            Some(self.next)
        }
        fn set_volume(&mut self, id: u64, volume: f32) {
            if let Some(v) = self.live.get_mut(&id) {
                v.1 = volume;
            }
        }
        fn stop(&mut self, id: u64) {
            self.live.remove(&id);
        }
        fn is_active(&self, id: u64) -> bool {
            self.live.contains_key(&id)
        }
        fn play_at(&mut self, event: &str, _at: [f64; 3]) {
            self.at.push(event.to_owned());
        }
    }
    fn ctx<'a>(
        sounds: &'a AmbientSounds,
        underwater: bool,
        light: &'a dyn Fn([i32; 3]) -> (u8, u8),
    ) -> AmbientContext<'a> {
        AmbientContext {
            sounds,
            position: [0.5, 10.0, 0.5],
            eye_y: 11.62,
            underwater,
            light,
        }
    }

    #[test]
    fn loops_cross_fade_over_forty_ticks() {
        let mut a = Ambience::new(1);
        let mut out = Fake::default();
        let dark = |_: [i32; 3]| (15u8, 0u8);
        let crimson = AmbientSounds {
            r#loop: Some("crimson".into()),
            ..Default::default()
        };
        for _ in 0..40 {
            a.tick(&ctx(&crimson, false, &dark), &mut out);
        }
        assert_eq!(out.live.values().map(|v| v.1).collect::<Vec<_>>(), [1.0]);
        let warped = AmbientSounds {
            r#loop: Some("warped".into()),
            ..Default::default()
        };
        for _ in 0..20 {
            a.tick(&ctx(&warped, false, &dark), &mut out);
        }
        let mut volumes: Vec<_> = out.live.values().map(|v| (v.0.clone(), v.1)).collect();
        volumes.sort_by(|x, y| x.0.cmp(&y.0));
        assert_eq!(
            volumes,
            [("crimson".to_owned(), 0.5), ("warped".to_owned(), 0.5)]
        );
        for _ in 0..30 {
            a.tick(&ctx(&warped, false, &dark), &mut out);
        }
        assert_eq!(
            out.live.values().map(|v| v.0.as_str()).collect::<Vec<_>>(),
            ["warped"]
        );
    }

    #[test]
    fn mood_builds_in_the_dark_and_plays_a_cave_sound() {
        let mut a = Ambience::new(2);
        let mut out = Fake::default();
        let cave = |_: [i32; 3]| (0u8, 0u8);
        let sounds = AmbientSounds {
            mood: Some(Mood {
                sound: "minecraft:ambient.cave".into(),
                tick_delay: 6000,
                block_search_extent: 8,
                offset: 2.0,
            }),
            ..Default::default()
        };
        // Block light 0: +1/6000 per tick (float), so the sound plays around tick 6000.
        let mut ticks = 0;
        while out.at.is_empty() && ticks < 7000 {
            a.tick(&ctx(&sounds, false, &cave), &mut out);
            ticks += 1;
        }
        assert!((5990..=6010).contains(&ticks), "{ticks}");
        assert_eq!(out.at, ["minecraft:ambient.cave"]);
        assert_eq!(a.moodiness, 0.0);
    }

    #[test]
    fn entering_water_plays_enter_and_fades_the_loop_in() {
        let mut a = Ambience::new(3);
        let mut out = Fake::default();
        let lit = |_: [i32; 3]| (15u8, 0u8);
        let none = AmbientSounds::default();
        a.tick(&ctx(&none, true, &lit), &mut out);
        assert_eq!(out.at, ["minecraft:ambient.underwater.enter"]);
        let id = a.underwater_loop.unwrap().0;
        assert_eq!(out.live[&id].1, 1.0 / 40.0);
        a.tick(&ctx(&none, false, &lit), &mut out);
        assert_eq!(out.at.last().unwrap(), "minecraft:ambient.underwater.exit");
        // Fade 1 -> -1 stops on the next tick.
        a.tick(&ctx(&none, false, &lit), &mut out);
        assert!(!out.live.contains_key(&id));
    }
}
