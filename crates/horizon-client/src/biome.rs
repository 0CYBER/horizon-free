//! Biome colours from the pinned biome definitions and colormaps
//! (`Biome.getGrassColor`/`getFoliageColor`/..., `BlockTintSources`).
use crate::Result;
use horizon_core::{noise::SimplexNoise, random::LegacyRandom};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrassModifier {
    None,
    DarkForest,
    Swamp,
}
#[derive(Clone, Debug)]
pub struct SpawnEntry {
    pub entity: String,
    pub weight: u32,
    pub min: u32,
    pub max: u32,
    /// A `ConstantInt` count (sampling draws nothing).
    pub constant: bool,
}
#[derive(Clone, Debug)]
pub struct BiomeInfo {
    /// `natural_mob_spawns` by category (creature, monster, ...).
    pub spawns: HashMap<String, Vec<SpawnEntry>>,
    /// `natural_mob_spawns` spawn costs by entity type.
    pub spawn_costs: HashMap<String, crate::mobs::spawn_selection::SpawnCost>,
    /// The `gameplay/creature_world_gen_spawn_probability` attribute.
    pub creature_probability: f32,
    pub name: String,
    pub temperature: f32,
    pub downfall: f32,
    pub precipitation: bool,
    pub grass: u32,
    pub modifier: GrassModifier,
    pub foliage: u32,
    pub dry_foliage: u32,
    pub water: u32,
    pub sky: Option<u32>,
    /// The biome's own fog colour attribute; otherwise the dimension's applies.
    pub fog: Option<u32>,
    pub water_fog: u32,
}
pub struct Biomes {
    pub list: Vec<BiomeInfo>,
    pub index: HashMap<String, u16>,
    info_noise: SimplexNoise,
}
fn color(v: &Value) -> Option<u32> {
    match v {
        Value::String(s) => u32::from_str_radix(s.trim_start_matches('#'), 16)
            .ok()
            .map(|c| c & 0xFF_FFFF),
        Value::Number(n) => n.as_i64().map(|c| c as u32 & 0xFF_FFFF),
        _ => None,
    }
}
fn colormap(root: &Path, name: &str) -> Result<Vec<u32>> {
    let path = root.join(format!("assets/minecraft/textures/colormap/{name}.png"));
    let mut decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(path)?));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut bytes = vec![0; reader.output_buffer_size().ok_or("colormap too large")?];
    let info = reader.next_frame(&mut bytes)?;
    let channels = info.color_type.samples();
    Ok(bytes[..info.buffer_size()]
        .chunks_exact(channels)
        .map(|p| u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2]))
        .collect())
}
/// `GrassColor.get`/`FoliageColor.get`: temperature and downfall index a 256x256 map.
fn sample(map: &[u32], temperature: f32, downfall: f32, fallback: u32) -> u32 {
    let temperature = f64::from(temperature.clamp(0.0, 1.0));
    let downfall = f64::from(downfall.clamp(0.0, 1.0)) * temperature;
    let x = ((1.0 - temperature) * 255.0) as i32;
    let y = ((1.0 - downfall) * 255.0) as i32;
    let index = (y << 8 | x) as usize;
    map.get(index).copied().unwrap_or(fallback)
}
impl Biomes {
    pub fn load(root: &Path) -> Result<Self> {
        let grass = colormap(root, "grass")?;
        let foliage = colormap(root, "foliage")?;
        let dry = colormap(root, "dry_foliage")?;
        let mut list = Vec::new();
        let mut names: Vec<_> = fs::read_dir(root.join("data/minecraft/worldgen/biome"))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        names.sort();
        for path in names {
            let name = format!(
                "minecraft:{}",
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .ok_or("bad biome")?
            );
            let v: Value = serde_json::from_slice(&fs::read(&path)?)?;
            let effects = &v["effects"];
            let attributes = &v["attributes"];
            let temperature = v["temperature"].as_f64().unwrap_or(0.5) as f32;
            let downfall = v["downfall"].as_f64().unwrap_or(0.5) as f32;
            let modifier = match effects["grass_color_modifier"].as_str() {
                Some("dark_forest") => GrassModifier::DarkForest,
                Some("swamp") => GrassModifier::Swamp,
                _ => GrassModifier::None,
            };
            let mut spawns: HashMap<String, Vec<SpawnEntry>> = HashMap::new();
            let natural = &attributes["minecraft:gameplay/natural_mob_spawns"];
            // A modifier object ({argument, modifier: overlay}) or the plain value;
            // the overlay over the dimension's empty default is the biome's value.
            let natural = if natural["argument"].is_object() {
                &natural["argument"]
            } else {
                natural
            };
            let categories = &natural["spawns_by_category"];
            for (category, entries) in categories.as_object().into_iter().flatten() {
                let list = crate::mobs::spawn_selection::parse_entries(entries);
                spawns.insert(category.clone(), list);
            }
            list.push(BiomeInfo {
                spawns,
                spawn_costs: crate::mobs::spawn_selection::parse_costs(&natural["spawn_costs"]),
                creature_probability:
                    attributes["minecraft:gameplay/creature_world_gen_spawn_probability"]
                        .as_f64()
                        .unwrap_or(0.1) as f32,
                temperature,
                downfall,
                precipitation: v["has_precipitation"].as_bool().unwrap_or(true),
                grass: color(&effects["grass_color"])
                    .unwrap_or_else(|| sample(&grass, temperature, downfall, 0xFF00FF)),
                modifier,
                foliage: color(&effects["foliage_color"])
                    .unwrap_or_else(|| sample(&foliage, temperature, downfall, 0x48B518)),
                dry_foliage: color(&effects["dry_foliage_color"])
                    .unwrap_or_else(|| sample(&dry, temperature, downfall, 0x5D4930)),
                water: color(&effects["water_color"]).unwrap_or(0x3F76E4),
                sky: color(&attributes["minecraft:visual/sky_color"]),
                fog: color(&attributes["minecraft:visual/fog_color"]),
                water_fog: color(&attributes["minecraft:visual/water_fog_color"])
                    .unwrap_or(0x050533),
                name,
            });
        }
        let index = list
            .iter()
            .enumerate()
            .map(|(i, b)| (b.name.clone(), i as u16))
            .collect();
        Ok(Self {
            list,
            index,
            info_noise: SimplexNoise::with_skipped_offsets(&mut LegacyRandom::new(2345), true),
        })
    }
    pub fn id(&self, name: &str) -> u16 {
        self.index
            .get(name)
            .or_else(|| self.index.get("minecraft:plains"))
            .copied()
            .unwrap_or(0)
    }
    pub fn get(&self, id: u16) -> &BiomeInfo {
        &self.list[usize::from(id).min(self.list.len() - 1)]
    }
    pub fn grass(&self, id: u16, x: f64, z: f64) -> u32 {
        let b = self.get(id);
        match b.modifier {
            GrassModifier::None => b.grass,
            GrassModifier::DarkForest => ((b.grass & 0xFE_FEFE) + 0x28_340A) >> 1,
            GrassModifier::Swamp => {
                if f64::from(self.info_noise.get2(x * 0.0225, z * 0.0225)) < -0.1 {
                    0x4C_763C
                } else {
                    0x6A_7039
                }
            }
        }
    }
}
