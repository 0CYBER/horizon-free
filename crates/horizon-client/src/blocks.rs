//! Per-state gameplay facts exported from vanilla's own block-state accessors
//! (`gameplay-facts.json`), indexed by registry state ID.
mod decode;
use crate::Result;
use horizon_world::registry::{BlockRegistry, BlockStateId};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::BufReader,
    path::Path,
    sync::Arc,
};

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
    /// `hasOffsetFunction`: the model is jittered per column.
    pub random_offset: bool,
    /// Whether the outline, not just the model, follows BlockState.getOffset.
    pub outline_offset: bool,
    /// `BlockState.isSolid()` (legacy solidity; fluid surfaces read it).
    pub legacy_solid: bool,
    /// `isFaceSturdy(.., SupportType.FULL)` by Direction ordinal.
    pub sturdy_full: u8,
    /// The block's class chain (`TransparentBlock`, `LeavesBlock`, ...), most specific first.
    pub classes: Arc<[Arc<str>]>,
    /// `getShape` (the outline used for picking) as boxes in block space,
    /// without the random offset; and its edges as LevelRenderer draws them.
    pub outline: Arc<[[f64; 6]]>,
    pub outline_edges: Arc<[[f64; 6]]>,
    /// `getMapColor(..).id` (0 = NONE).
    pub map_color: u8,
}
/// `MapColor.col` by id.
pub const MAP_COLORS: [u32; 62] = [
    0, 8368696, 16247203, 13092807, 16711680, 10526975, 10987431, 31744, 16777215, 10791096,
    9923917, 7368816, 4210943, 9402184, 16776437, 14188339, 11685080, 6724056, 15066419, 8375321,
    15892389, 5000268, 10066329, 5013401, 8339378, 3361970, 6704179, 6717235, 10040115, 1644825,
    16445005, 6085589, 4882687, 55610, 8476209, 7340544, 13742497, 10441252, 9787244, 7367818,
    12223780, 6780213, 10505550, 3746083, 8874850, 5725276, 8014168, 4996700, 4993571, 5001770,
    9321518, 2430480, 12398641, 9715553, 6035741, 1474182, 3837580, 5647422, 1356933, 6579300,
    14200723, 8365974,
];
/// The 12 edges of the unit cube (outline data predating the export).
fn cube_edges() -> Vec<[f64; 6]> {
    let mut edges = Vec::new();
    for a in 0..3 {
        for (u, v) in [(0., 0.), (1., 0.), (0., 1.), (1., 1.)] {
            let mut from = [0.0f64; 3];
            let (b, c) = ((a + 1) % 3, (a + 2) % 3);
            from[b] = u;
            from[c] = v;
            let mut to = from;
            to[a] = 1.0;
            edges.push([from[0], from[1], from[2], to[0], to[1], to[2]]);
        }
    }
    edges
}
impl StateFacts {
    pub fn has_class(&self, name: &str) -> bool {
        self.classes.iter().any(|c| &**c == name)
    }
}
pub struct BlockFacts {
    pub states: Vec<StateFacts>,
    pub air: BlockStateId,
}
impl BlockFacts {
    pub fn load(root: &Path, registry: &BlockRegistry) -> Result<Self> {
        let bytes = fs::read(root.join("gameplay-facts.json"))?;
        serde_json::from_slice::<decode::Validate>(&bytes)?;
        let raw: decode::Gameplay<'_> = serde_json::from_slice(&bytes)?;
        let entries = raw
            .states
            .0
            .as_ref()
            .ok_or("missing gameplay state facts")?;
        if entries.len() != registry.len() {
            return Err("gameplay facts do not match the block registry".into());
        }
        // Class chains and per-state behaviour flags exported from the actual block objects.
        let behaviours: Value = serde_json::from_reader(BufReader::new(File::open(
            root.join("block-behaviours.json"),
        )?))?;
        let behaviour_states = behaviours["states"]
            .as_array()
            .filter(|s| s.len() == registry.len())
            .ok_or("block behaviour facts do not match the block registry")?;
        let mut class_names: HashMap<String, Arc<str>> = HashMap::new();
        let classes: HashMap<&str, Arc<[Arc<str>]>> = behaviours["blocks"]
            .as_array()
            .ok_or("missing block behaviour classes")?
            .iter()
            .filter_map(|b| {
                let chain = b["class"]
                    .as_array()?
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|c| {
                        class_names
                            .entry(c.to_owned())
                            .or_insert_with(|| Arc::from(c))
                            .clone()
                    });
                Some((b["name"].as_str()?, chain.collect()))
            })
            .collect();
        let mut names: HashMap<String, Arc<str>> = HashMap::new();
        let mut intern = |s: &str| -> Arc<str> {
            if let Some(name) = names.get(s) {
                return name.clone();
            }
            let name: Arc<str> = Arc::from(s);
            names.insert(s.to_owned(), name.clone());
            name
        };
        let mut shapes = decode::Shapes::default();
        let mut states = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            let id = BlockStateId(index as u32);
            if entry.id != Some(index as u64) {
                return Err("gameplay facts are not in registry order".into());
            }
            let state = registry.state(id).ok_or("missing registry state")?;
            let sound = &entry.sound;
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
                "minecraft:bubble_column"
                | "minecraft:kelp"
                | "minecraft:kelp_plant"
                | "minecraft:seagrass"
                | "minecraft:tall_seagrass" => (Fluid::Water, 8, false),
                _ if prop("waterlogged") == Some("true") => (Fluid::Water, 8, false),
                _ => (Fluid::None, 0, false),
            };
            let air = matches!(
                state.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            );
            let cube = move || {
                if air {
                    Vec::new()
                } else {
                    vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]
                }
            };
            let edges = move || if air { Vec::new() } else { cube_edges() };
            let outline = shapes.intern(entry.outline, cube)?;
            let outline_edges = shapes.intern(entry.outline_edges, edges)?;
            // The export holds MapColor.col; ids are positions in MAP_COLORS.
            let col = entry.map_color.unwrap_or(0) as u32;
            let map_color = if col == 0 {
                0
            } else {
                MAP_COLORS.iter().position(|c| *c == col).unwrap_or(0) as u8
            };
            states.push(StateFacts {
                map_color,
                outline,
                outline_edges,
                name: intern(&state.name),
                emission: entry.emission as u8,
                dampening: entry.dampening as u8,
                skylight_down: entry.skylight_down,
                shape_occlusion: entry.shape_occlusion,
                can_occlude: entry.can_occlude,
                solid_render: entry.solid_render,
                hardness: entry.hardness as f32,
                requires_tool: entry.requires_tool,
                replaceable: entry.replaceable,
                random_ticks: entry.random_ticks,
                block_entity: entry.block_entity,
                render_shape: match entry.render_shape.as_deref() {
                    Some("INVISIBLE") => RenderShape::Invisible,
                    Some("ENTITYBLOCK_ANIMATED") => RenderShape::Animated,
                    _ => RenderShape::Model,
                },
                full_collision: entry.full_collision,
                friction: entry.friction as f32,
                explosion_resistance: entry.explosion_resistance as f32,
                item: intern(entry.item.as_deref().unwrap_or("minecraft:air")),
                sounds: Sounds {
                    volume: sound.volume as f32,
                    pitch: sound.pitch as f32,
                    break_sound: intern(sound.r#break.id.as_deref().unwrap_or("")),
                    step: intern(sound.step.id.as_deref().unwrap_or("")),
                    place: intern(sound.place.id.as_deref().unwrap_or("")),
                    hit: intern(sound.hit.id.as_deref().unwrap_or("")),
                    fall: intern(sound.fall.id.as_deref().unwrap_or("")),
                },
                fluid,
                fluid_level,
                fluid_falling,
                waterlogged: prop("waterlogged") == Some("true"),
                random_offset: entry.offset,
                outline_offset: entry.outline_offset.unwrap_or(entry.offset),
                legacy_solid: behaviour_states[index][1].as_u64().unwrap_or(0) & 1 != 0,
                sturdy_full: (behaviour_states[index][0].as_u64().unwrap_or(0) & 63) as u8,
                classes: classes
                    .get(state.name.as_str())
                    .cloned()
                    .ok_or("missing block behaviour class chain")?,
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
