//! Data-driven environment attributes sampled from the pinned timelines
//! (`KeyframeTrackSampler`, `EasingType`): sky/fog colours, sky light factor,
//! celestial angles and star brightness for the overworld day cycle.
use crate::Result;
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

#[derive(Clone, Copy, Debug)]
enum Easing {
    Linear,
    Constant,
    Bezier([f32; 4]),
}
impl Easing {
    fn apply(self, x: f32) -> f32 {
        match self {
            Self::Linear => x,
            Self::Constant => 0.0,
            Self::Bezier([x1, y1, x2, y2]) => {
                let curve = |c1: f32, c2: f32| (3.0 * c1 - 3.0 * c2 + 1.0, -6.0 * c1 + 3.0 * c2, 3.0 * c1);
                let (ax, bx, cx) = curve(x1, x2);
                let (ay, by, cy) = curve(y1, y2);
                let sample = |(a, b, c): (f32, f32, f32), t: f32| ((a * t + b) * t + c) * t;
                let gradient = |t: f32| (3.0 * ax * t + 2.0 * bx) * t + cx;
                let mut t = x;
                let mut solved = false;
                for _ in 0..4 {
                    let error = sample((ax, bx, cx), t) - x;
                    if error.abs() < 1e-5 {
                        solved = true;
                        break;
                    }
                    let g = gradient(t);
                    if g < 1e-5 {
                        break;
                    }
                    t -= (error / g).clamp(-0.25, 0.25);
                }
                if !solved {
                    let (mut t0, mut t1) = (0.0f32, 1.0f32);
                    while t0 < t1 {
                        let error = sample((ax, bx, cx), t) - x;
                        if error.abs() < 1e-5 {
                            break;
                        }
                        if error < 0.0 {
                            t0 = t;
                        } else {
                            t1 = t;
                        }
                        let next = (t0 + t1) / 2.0;
                        if next == t {
                            break;
                        }
                        t = next;
                    }
                }
                sample((ay, by, cy), t)
            }
        }
    }
}
#[derive(Clone, Debug)]
struct Track {
    keyframes: Vec<(i64, [f32; 4])>,
    easing: Easing,
    multiply: bool,
}
fn parse_value(v: &Value) -> Option<[f32; 4]> {
    match v {
        Value::Number(n) => n.as_f64().map(|f| [f as f32, 0.0, 0.0, 0.0]),
        Value::Bool(b) => Some([f32::from(u8::from(*b)), 0.0, 0.0, 0.0]),
        Value::String(s) => {
            let hex = s.trim_start_matches('#');
            let c = u32::from_str_radix(hex, 16).ok()?;
            let f = |shift: u32| (c >> shift & 255) as f32 / 255.0;
            Some(if hex.len() == 8 {
                [f(16), f(8), f(0), f(24)]
            } else {
                [f(16), f(8), f(0), 1.0]
            })
        }
        _ => None,
    }
}
impl Track {
    fn sample(&self, ticks: i64, period: i64) -> [f32; 4] {
        let k = &self.keyframes;
        if k.len() == 1 {
            return k[0].1;
        }
        let t = ticks.rem_euclid(period);
        let (first, last) = (k[0], k[k.len() - 1]);
        let mut segments = vec![(last.1, last.0 - period, first.1, first.0)];
        for pair in k.windows(2) {
            segments.push((pair[0].1, pair[0].0, pair[1].1, pair[1].0));
        }
        segments.push((last.1, last.0, first.1, first.0 + period));
        let segment = segments
            .iter()
            .find(|s| t < s.3)
            .copied()
            .unwrap_or(segments[segments.len() - 1]);
        let (from, from_t, to, to_t) = segment;
        if t <= from_t {
            return from;
        }
        if t >= to_t {
            return to;
        }
        let alpha = self.easing.apply((t - from_t) as f32 / (to_t - from_t) as f32);
        std::array::from_fn(|i| from[i] + (to[i] - from[i]) * alpha)
    }
}
pub struct Environment {
    period: i64,
    tracks: HashMap<String, Track>,
    base: HashMap<String, [f32; 4]>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Sky {
    pub sky_color: [f32; 3],
    pub fog_color: [f32; 3],
    pub sky_light_factor: f32,
    pub sky_light_color: [f32; 3],
    pub block_light_tint: [f32; 3],
    pub ambient_light_color: [f32; 3],
    pub sun_angle: f32,
    pub moon_angle: f32,
    pub star_angle: f32,
    pub star_brightness: f32,
    pub sunrise_color: [f32; 4],
    pub cloud_color: [f32; 4],
    /// `gameplay/sky_light_level`, the server-side sky darkening.
    pub sky_light_level: f32,
}
impl Environment {
    pub fn load(root: &Path) -> Result<Self> {
        let timeline: Value =
            serde_json::from_slice(&fs::read(root.join("data/minecraft/timeline/day.json"))?)?;
        let dimension: Value = serde_json::from_slice(&fs::read(
            root.join("data/minecraft/dimension_type/overworld.json"),
        )?)?;
        let mut tracks = HashMap::new();
        for (name, track) in timeline["tracks"].as_object().ok_or("invalid timeline")? {
            let easing = match &track["ease"] {
                Value::String(s) if s == "constant" => Easing::Constant,
                Value::Object(o) => match o.get("cubic_bezier").and_then(Value::as_array) {
                    Some(a) if a.len() == 4 => Easing::Bezier(std::array::from_fn(|i| {
                        a[i].as_f64().unwrap_or(0.0) as f32
                    })),
                    _ => Easing::Linear,
                },
                _ => Easing::Linear,
            };
            let keyframes: Vec<_> = track["keyframes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|k| Some((k["ticks"].as_i64()?, parse_value(&k["value"])?)))
                .collect();
            if keyframes.is_empty() {
                continue;
            }
            tracks.insert(
                name.clone(),
                Track {
                    keyframes,
                    easing,
                    multiply: track["modifier"].as_str() == Some("multiply"),
                },
            );
        }
        let mut base: HashMap<String, [f32; 4]> = [
            ("minecraft:visual/block_light_tint", "#ffd88c"),
            ("minecraft:visual/sky_light_color", "#ffffff"),
            ("minecraft:visual/ambient_light_color", "#000000"),
        ]
        .into_iter()
        .filter_map(|(k, v)| Some((k.to_owned(), parse_value(&Value::String(v.into()))?)))
        .collect();
        base.insert("minecraft:visual/sky_light_factor".into(), [1.0, 0.0, 0.0, 0.0]);
        base.insert("minecraft:gameplay/sky_light_level".into(), [1.0, 0.0, 0.0, 0.0]);
        for (k, v) in dimension["attributes"].as_object().into_iter().flatten() {
            if let Some(v) = parse_value(v) {
                base.insert(k.clone(), v);
            }
        }
        Ok(Self {
            period: timeline["period_ticks"].as_i64().unwrap_or(24000),
            tracks,
            base,
        })
    }
    fn value(&self, name: &str, base: [f32; 4], ticks: i64) -> [f32; 4] {
        match self.tracks.get(name) {
            Some(track) => {
                let v = track.sample(ticks, self.period);
                if track.multiply {
                    std::array::from_fn(|i| base[i] * v[i])
                } else {
                    v
                }
            }
            None => base,
        }
    }
    fn base(&self, name: &str) -> [f32; 4] {
        self.base.get(name).copied().unwrap_or([0.0, 0.0, 0.0, 1.0])
    }
    /// Sky state for a time of day, with the biome's sky and fog colours as base values.
    pub fn sample(&self, ticks: i64, biome_sky: u32, biome_fog: u32) -> Sky {
        let rgb = |c: u32| {
            [
                (c >> 16 & 255) as f32 / 255.0,
                (c >> 8 & 255) as f32 / 255.0,
                (c & 255) as f32 / 255.0,
                1.0,
            ]
        };
        let v = |name: &str, base: [f32; 4]| self.value(name, base, ticks);
        let three = |a: [f32; 4]| [a[0], a[1], a[2]];
        let scalar = |name: &str| {
            let b = self.base(name);
            let b = if self.base.contains_key(name) { b } else { [1.0, 0.0, 0.0, 0.0] };
            let t = self.tracks.get(name);
            match t {
                Some(t) if t.multiply => b[0] * t.sample(ticks, self.period)[0],
                Some(t) => t.sample(ticks, self.period)[0],
                None => b[0],
            }
        };
        let angle = |name: &str| {
            self.tracks
                .get(name)
                .map_or(0.0, |t| t.sample(ticks, self.period)[0])
        };
        Sky {
            sky_color: three(v("minecraft:visual/sky_color", rgb(biome_sky))),
            fog_color: three(v("minecraft:visual/fog_color", rgb(biome_fog))),
            sky_light_factor: scalar("minecraft:visual/sky_light_factor"),
            sky_light_color: three(v(
                "minecraft:visual/sky_light_color",
                self.base("minecraft:visual/sky_light_color"),
            )),
            block_light_tint: three(self.base("minecraft:visual/block_light_tint")),
            ambient_light_color: three(self.base("minecraft:visual/ambient_light_color")),
            sun_angle: angle("minecraft:visual/sun_angle"),
            moon_angle: angle("minecraft:visual/moon_angle"),
            star_angle: angle("minecraft:visual/star_angle"),
            star_brightness: angle("minecraft:visual/star_brightness"),
            sunrise_color: self
                .tracks
                .get("minecraft:visual/sunrise_sunset_color")
                .map_or([0.0; 4], |t| t.sample(ticks, self.period)),
            cloud_color: v(
                "minecraft:visual/cloud_color",
                self.base("minecraft:visual/cloud_color"),
            ),
            sky_light_level: scalar("minecraft:gameplay/sky_light_level"),
        }
    }
}
