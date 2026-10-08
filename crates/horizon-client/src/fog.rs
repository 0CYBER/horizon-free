//! `FogRenderer` and its fog environments (atmospheric with rain fog, water
//! with water vision, lava, powder snow, blindness), the camera fog type
//! (`Camera.getFluidInCamera`) and the fog colour (`computeFogColor`: sunrise
//! tint, sky-colour mix, void darkness, water-vision brightening).
use crate::{
    biome::Biomes,
    blocks::{BlockFacts, Fluid},
    environment::Sky,
    world::World,
};
use std::path::Path;

/// `FogType` of the camera position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FogType {
    Atmospheric,
    Water,
    Lava,
    PowderSnow,
}

/// The fog uniform (`FogData`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogData {
    pub color: [f32; 4],
    pub environmental_start: f32,
    pub environmental_end: f32,
    pub render_distance_start: f32,
    pub render_distance_end: f32,
    pub sky_end: f32,
    pub cloud_end: f32,
}

/// Per-biome water fog end multipliers (`visual/water_fog_end_distance`).
pub struct FogTables {
    water_fog_end: Vec<f32>,
}
impl FogTables {
    pub fn load(root: &Path, biomes: &Biomes) -> Self {
        let mut water_fog_end = vec![1.0; biomes.list.len()];
        for (name, &id) in &biomes.index {
            let path = root.join(format!(
                "data/minecraft/worldgen/biome/{}.json",
                name.trim_start_matches("minecraft:")
            ));
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            let v = &json["attributes"]["minecraft:visual/water_fog_end_distance"];
            if v["modifier"].as_str() == Some("multiply")
                && let (Some(f), Some(slot)) =
                    (v["argument"].as_f64(), water_fog_end.get_mut(id as usize))
            {
                *slot = f as f32;
            }
        }
        Self { water_fog_end }
    }
}

/// Fog state that evolves over time.
#[derive(Clone, Copy, Debug, Default)]
pub struct FogState {
    /// `LocalPlayer.waterVisionTime` (0..600).
    pub water_vision_time: i32,
    /// `AtmosphericFogEnvironment.rainFogMultiplier`.
    pub rain_fog_multiplier: f32,
}
impl FogState {
    /// `LocalPlayer.aiStep`: water vision ramps up while the eye is in water.
    pub fn tick(&mut self, eye_in_water: bool) {
        self.water_vision_time = if eye_in_water {
            (self.water_vision_time + 1).min(600)
        } else {
            (self.water_vision_time - 10).max(0)
        };
    }
    /// `LocalPlayer.getWaterVision`.
    pub fn water_vision(&self, eye_in_water: bool) -> f32 {
        if !eye_in_water {
            return 0.0;
        }
        let t = self.water_vision_time as f32;
        if t >= 600.0 {
            return 1.0;
        }
        let a = (t / 100.0).clamp(0.0, 1.0);
        let b = if t < 100.0 {
            0.0
        } else {
            ((t - 100.0) / 500.0).clamp(0.0, 1.0)
        };
        a * 0.6 + b * 0.399_999_98
    }
}

/// The fog tables and evolving state kept by the game.
pub struct FogRenderer {
    pub tables: FogTables,
    pub state: FogState,
}
impl FogRenderer {
    pub fn load(root: &Path, biomes: &Biomes) -> Self {
        Self {
            tables: FogTables::load(root, biomes),
            state: FogState::default(),
        }
    }
    pub fn setup(&self, input: &FogInputs) -> FogData {
        setup_fog(&self.tables, &self.state, input)
    }
}
/// `FluidState.getHeightForCamera`.
fn camera_fluid_height(world: &World, facts: &BlockFacts, b: [i32; 3]) -> Option<(Fluid, f64)> {
    let s = facts.get(world.block(b[0], b[1], b[2]));
    if s.fluid == Fluid::None {
        return None;
    }
    let above = facts.get(world.block(b[0], b[1] + 1, b[2]));
    let source = s.fluid_level >= 8 && !s.fluid_falling;
    let height = if (source && above.full_collision) || above.fluid == s.fluid {
        1.0
    } else {
        f64::from(s.fluid_level) / 9.0
    };
    Some((s.fluid, height))
}
/// `Camera.getFluidInCamera` (near-plane points reduced to the camera and
/// the point just in front of it).
pub fn fog_type(world: &World, facts: &BlockFacts, camera: [f64; 3], forward: [f64; 3]) -> FogType {
    let block = |p: [f64; 3]| {
        [
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        ]
    };
    let b = block(camera);
    if let Some((Fluid::Water, h)) = camera_fluid_height(world, facts, b)
        && camera[1] < f64::from(b[1]) + h
    {
        return FogType::Water;
    }
    for p in [
        [
            camera[0] + forward[0] * 0.05,
            camera[1] + forward[1] * 0.05,
            camera[2] + forward[2] * 0.05,
        ],
        camera,
    ] {
        let b = block(p);
        match camera_fluid_height(world, facts, b) {
            Some((Fluid::Lava, h)) if p[1] <= f64::from(b[1]) + h => return FogType::Lava,
            Some((Fluid::Lava, _)) => {}
            _ => {
                if &*facts.get(world.block(b[0], b[1], b[2])).name == "minecraft:powder_snow" {
                    return FogType::PowderSnow;
                }
            }
        }
    }
    FogType::Atmospheric
}

/// Everything `FogRenderer.setupFog` reads for one frame.
pub struct FogInputs<'a> {
    pub fog_type: FogType,
    pub sky: &'a Sky,
    /// Biome water fog colour (`visual/water_fog_color`).
    pub water_fog_color: [f32; 3],
    pub biome: u16,
    pub render_distance_chunks: i32,
    pub camera_y: f64,
    pub min_y: i32,
    /// Flat worlds use a 1-block void darkness range.
    pub flat: bool,
    pub forward: [f32; 3],
    pub rain: f32,
    pub thunder: f32,
    /// Dimensions with a sky disc (overworld): sunrise tint and sky mix apply.
    pub skybox: bool,
    /// Blindness: (remaining duration ticks, infinite).
    pub blindness: Option<(i32, bool)>,
}

/// `FogRenderer.setupFog` + `computeFogColor`.
pub fn setup_fog(tables: &FogTables, state: &FogState, input: &FogInputs) -> FogData {
    let rd = input.render_distance_chunks as f32 * 16.0;
    let sky = input.sky;
    let eye_in_water = input.fog_type == FogType::Water;
    let water_vision = state.water_vision(eye_in_water);
    // Base colour of the applicable environment.
    let mut color = match input.fog_type {
        FogType::Lava => rgb(0x99_1A00),
        FogType::PowderSnow => rgb(0x9F_BACC),
        FogType::Water => input.water_fog_color,
        FogType::Atmospheric => {
            let mut fog = sky.fog_color;
            if input.skybox && input.render_distance_chunks >= 4 {
                let sun_x = if sky.sun_angle.to_radians().sin() > 0.0 {
                    -1.0
                } else {
                    1.0
                };
                let looking = input.forward[0] * sun_x;
                let alpha = sky.sunrise_color[3];
                if looking > 0.0 && alpha > 0.0 {
                    let t = looking * alpha;
                    fog = std::array::from_fn(|i| fog[i] + (sky.sunrise_color[i] - fog[i]) * t);
                }
            }
            let mut sky_color = sky.sky_color;
            if input.rain > 0.0 {
                let m = 1.0 - input.rain * 0.5;
                let mb = 1.0 - input.rain * 0.4;
                sky_color = [sky_color[0] * m, sky_color[1] * m, sky_color[2] * mb];
            }
            if input.thunder > 0.0 {
                let m = 1.0 - input.thunder * 0.5;
                sky_color = sky_color.map(|c| c * m);
            }
            let sky_fog_end = (512.0f32 / 16.0).min(input.render_distance_chunks as f32);
            let mix = (sky_fog_end / 32.0).clamp(0.0, 1.0).mul_add(0.75, 0.25);
            let mix = 1.0 - mix.powf(0.25);
            std::array::from_fn(|i| fog[i] + (sky_color[i] - fog[i]) * mix)
        }
    };
    // Void darkness and blindness.
    let range = if input.flat { 1.0 } else { 32.0 };
    let mut darkness =
        ((range + input.min_y as f32 - input.camera_y as f32) / range).clamp(0.0, 1.0);
    if let Some((duration, infinite)) = input.blindness {
        darkness = if !infinite && duration < 20 {
            darkness.max(duration as f32 / 20.0)
        } else {
            1.0
        };
    }
    if darkness > 0.0 && !matches!(input.fog_type, FogType::Lava | FogType::PowderSnow) {
        let brightness = (1.0 - darkness) * (1.0 - darkness);
        color = color.map(|c| c * brightness);
    }
    let brighten = if eye_in_water { water_vision } else { 0.0 };
    if color.iter().all(|&c| c != 0.0) {
        let scale = 1.0 / color[0].max(color[1]).max(color[2]);
        color = color.map(|c| c + (c * scale - c) * brighten);
    }
    let mut fog = FogData {
        color: [color[0], color[1], color[2], 1.0],
        environmental_start: 0.0,
        environmental_end: 1024.0,
        render_distance_start: 0.0,
        render_distance_end: rd,
        sky_end: rd.min(512.0),
        cloud_end: 2048.0,
    };
    match input.fog_type {
        FogType::Lava => {
            fog.environmental_start = 0.25;
            fog.environmental_end = 1.0;
            fog.sky_end = 1.0;
            fog.cloud_end = 1.0;
        }
        FogType::PowderSnow => {
            fog.environmental_start = 0.0;
            fog.environmental_end = 2.0;
            fog.sky_end = 2.0;
            fog.cloud_end = 2.0;
        }
        _ if input.blindness.is_some() => {
            let (duration, infinite) = input.blindness.unwrap_or((0, true));
            let distance = if infinite {
                5.0
            } else {
                rd + (5.0 - rd) * (duration as f32 / 20.0).min(1.0)
            };
            fog.environmental_start = distance * 0.25;
            fog.environmental_end = distance;
            fog.sky_end = distance * 0.8;
            fog.cloud_end = distance * 0.8;
        }
        FogType::Water => {
            fog.environmental_start = -8.0;
            let factor = tables
                .water_fog_end
                .get(input.biome as usize)
                .copied()
                .unwrap_or(1.0);
            fog.environmental_end = 96.0 * factor * water_vision.max(0.25);
            fog.sky_end = fog.environmental_end;
            fog.cloud_end = fog.environmental_end;
        }
        FogType::Atmospheric => {
            fog.environmental_start = sky.fog_start + -160.0 * state.rain_fog_multiplier;
            let min_end = sky.fog_end.min(96.0);
            fog.environmental_end = min_end.max(sky.fog_end + -256.0 * state.rain_fog_multiplier);
            fog.sky_end = rd.min(512.0);
            fog.cloud_end = 2048.0f32.min(128.0 * 16.0);
        }
    }
    let span = (rd / 10.0).clamp(4.0, 64.0);
    fog.render_distance_start = rd - span;
    fog.render_distance_end = rd;
    fog
}
/// `AtmosphericFogEnvironment.updateRainFogState`.
pub fn update_rain_fog(
    state: &mut FogState,
    rain: f32,
    sky_light: u8,
    biome_rains: bool,
    delta_ticks: f32,
) {
    let sky = ((f32::from(sky_light) - 8.0) / 7.0).clamp(0.0, 1.0);
    let target = rain * sky * if biome_rains { 1.0 } else { 0.5 };
    state.rain_fog_multiplier += (target - state.rain_fog_multiplier) * delta_ticks * 0.2;
}
fn rgb(c: u32) -> [f32; 3] {
    [
        (c >> 16 & 255) as f32 / 255.0,
        (c >> 8 & 255) as f32 / 255.0,
        (c & 255) as f32 / 255.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inputs(sky: &Sky) -> FogInputs<'_> {
        FogInputs {
            fog_type: FogType::Atmospheric,
            sky,
            water_fog_color: [0.0, 0.0, 0.2],
            biome: 0,
            render_distance_chunks: 12,
            camera_y: 70.0,
            min_y: -64,
            flat: false,
            forward: [0.0, 0.0, 1.0],
            rain: 0.0,
            thunder: 0.0,
            skybox: true,
            blindness: None,
        }
    }
    #[test]
    fn render_distance_fog_span() {
        let sky = Sky {
            fog_end: 1024.0,
            fog_color: [0.5; 3],
            sky_color: [0.5; 3],
            ..Sky::default()
        };
        let tables = FogTables {
            water_fog_end: vec![1.0],
        };
        let fog = setup_fog(&tables, &FogState::default(), &inputs(&sky));
        // 12 chunks = 192 blocks, span clamp(19.2, 4, 64).
        assert!((fog.render_distance_start - (192.0 - 19.2)).abs() < 1e-4);
        assert_eq!(fog.render_distance_end, 192.0);
        assert_eq!(fog.sky_end, 192.0);
    }
    #[test]
    fn water_vision_ramps() {
        let mut s = FogState::default();
        for _ in 0..100 {
            s.tick(true);
        }
        assert!((s.water_vision(true) - 0.6).abs() < 1e-6);
        for _ in 0..500 {
            s.tick(true);
        }
        assert_eq!(s.water_vision(true), 1.0);
        assert_eq!(s.water_vision(false), 0.0);
    }
    #[test]
    fn rain_fog_pulls_fog_in() {
        let sky = Sky {
            fog_end: 1024.0,
            ..Sky::default()
        };
        let tables = FogTables {
            water_fog_end: vec![1.0],
        };
        let state = FogState {
            rain_fog_multiplier: 1.0,
            ..FogState::default()
        };
        let fog = setup_fog(&tables, &state, &inputs(&sky));
        assert_eq!(fog.environmental_start, -160.0);
        assert_eq!(fog.environmental_end, 768.0);
    }
}
