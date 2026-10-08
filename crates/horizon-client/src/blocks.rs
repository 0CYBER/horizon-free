//! Per-state gameplay facts exported from vanilla's own block-state accessors
//! (`gameplay-facts.json`), indexed by registry state ID.
use crate::Result;
use horizon_world::registry::{BlockRegistry, BlockStateId};
use serde_json::Value;
use std::{collections::HashMap, fs::File, io::BufReader, path::Path, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderShape {
    Model,
    Invisible,
    Animated,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fluid {
    None,
    Water,
    Lava,
}
#[derive(Clone, Debug)]
pub struct Sounds {
    pub volume: f32,
    pub pitch: f32,
    pub break_sound: Arc<str>,
    pub step: Arc<str>,
    pub place: Arc<str>,
    pub hit: Arc<str>,
    pub fall: Arc<str>,
}
#[derive(Clone, Debug)]
pub struct StateFacts {
    pub name: Arc<str>,
    pub emission: u8,
    pub dampening: u8,
    pub skylight_down: bool,
    pub shape_occlusion: bool,
    pub can_occlude: bool,
    pub solid_render: bool,
    pub hardness: f32,
    pub requires_tool: bool,
    pub replaceable: bool,
    pub random_ticks: bool,
    pub block_entity: bool,
    pub render_shape: RenderShape,
    pub full_collision: bool,
    pub friction: f32,
    pub explosion_resistance: f32,
    pub item: Arc<str>,
    pub sounds: Sounds,
    pub fluid: Fluid,
    /// Fluid amount 1..=8 (8 = source/falling surface), 0 when there is no fluid.
    pub fluid_level: u8,
    pub fluid_falling: bool,
    pub waterlogged: bool,
}
pub struct BlockFacts {
    pub states: Vec<StateFacts>,
    pub air: BlockStateId,
}
impl BlockFacts {
    pub fn load(root: &Path, registry: &BlockRegistry) -> Result<Self> {
        let raw: Value =
            serde_json::from_reader(BufReader::new(File::open(root.join("gameplay-facts.json"))?))?;
        let entries = raw["states"]
            .as_array()
            .ok_or("missing gameplay state facts")?;
        if entries.len() != registry.len() {
            return Err("gameplay facts do not match the block registry".into());
        }
        let mut names: HashMap<String, Arc<str>> = HashMap::new();
        let mut intern = |s: &str| -> Arc<str> {
            names
                .entry(s.to_owned())
                .or_insert_with(|| Arc::from(s))
                .clone()
        };
        let mut states = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            let id = BlockStateId(index as u32);
            if entry["id"].as_u64() != Some(index as u64) {
                return Err("gameplay facts are not in registry order".into());
            }
            let state = registry.state(id).ok_or("missing registry state")?;
            let num = |k: &str| entry[k].as_f64().unwrap_or(0.0);
            let flag = |k: &str| entry[k].as_bool().unwrap_or(false);
            let sound = &entry["sound"];
            let sound_id = |k: &str| sound[k]["id"].as_str().unwrap_or("").to_owned();
            let prop = |k: &str| state.properties.get(k).map(String::as_str);
            let (fluid, fluid_level, fluid_falling) = match state.name.as_str() {
                "minecraft:water" | "minecraft:lava" => {
                    let level: u8 = prop("level").and_then(|v| v.parse().ok()).unwrap_or(0);
                    let kind = if state.name == "minecraft:water" {
                        Fluid::Water
                    } else {
                        Fluid::Lava
                    };
                    // LiquidBlock: level 0 is the source; 1..7 flowing; >=8 falling.
                    if level == 0 {
                        (kind, 8, false)
                    } else if level >= 8 {
                        (kind, 8, true)
                    } else {
                        (kind, 8 - level, false)
                    }
                }
                "minecraft:bubble_column" | "minecraft:kelp" | "minecraft:kelp_plant"
                | "minecraft:seagrass" | "minecraft:tall_seagrass" => (Fluid::Water, 8, false),
                _ if prop("waterlogged") == Some("true") => (Fluid::Water, 8, false),
                _ => (Fluid::None, 0, false),
            };
            states.push(StateFacts {
                name: intern(&state.name),
                emission: num("emission") as u8,
                dampening: num("dampening") as u8,
                skylight_down: flag("skylight_down"),
                shape_occlusion: flag("shape_occlusion"),
                can_occlude: flag("can_occlude"),
                solid_render: flag("solid_render"),
                hardness: num("hardness") as f32,
                requires_tool: flag("requires_tool"),
                replaceable: flag("replaceable"),
                random_ticks: flag("random_ticks"),
                block_entity: flag("block_entity"),
                render_shape: match entry["render_shape"].as_str() {
                    Some("INVISIBLE") => RenderShape::Invisible,
                    Some("ENTITYBLOCK_ANIMATED") => RenderShape::Animated,
                    _ => RenderShape::Model,
                },
                full_collision: flag("full_collision"),
                friction: num("friction") as f32,
                explosion_resistance: num("explosion_resistance") as f32,
                item: intern(entry["item"].as_str().unwrap_or("minecraft:air")),
                sounds: Sounds {
                    volume: sound["volume"].as_f64().unwrap_or(1.0) as f32,
                    pitch: sound["pitch"].as_f64().unwrap_or(1.0) as f32,
                    break_sound: intern(&sound_id("break")),
                    step: intern(&sound_id("step")),
                    place: intern(&sound_id("place")),
                    hit: intern(&sound_id("hit")),
                    fall: intern(&sound_id("fall")),
                },
                fluid,
                fluid_level,
                fluid_falling,
                waterlogged: prop("waterlogged") == Some("true"),
            });
        }
        Ok(Self {
            states,
            air: registry
                .default_state("minecraft:air")
                .ok_or("missing air")?,
        })
    }
    pub fn get(&self, id: BlockStateId) -> &StateFacts {
        &self.states[id.0 as usize]
    }
    pub fn is_air(&self, id: BlockStateId) -> bool {
        matches!(
            &*self.states[id.0 as usize].name,
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        )
    }
}
