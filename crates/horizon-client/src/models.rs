//! A blockstate-driven inspection gallery, separate from gameplay simulation.
mod catalog;
use crate::{Result, sprites::Sprites};
use catalog::{StateModels, descriptor};
use horizon_assets::model::Direction;
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockRegistry;
use serde_json::{Value, json};
use std::path::Path;

type Placement = (&'static str, &'static [(&'static str, &'static str)]);
const GALLERY: [Placement; 16] = [
    (
        "oak_stairs",
        &[
            ("facing", "north"),
            ("half", "top"),
            ("shape", "inner_left"),
        ],
    ),
    ("stone_slab", &[("type", "top")]),
    ("anvil", &[("facing", "east")]),
    ("torch", &[]),
    ("lantern", &[]),
    ("oak_fence", &[("north", "true"), ("east", "true")]),
    ("oak_fence_gate", &[("facing", "west"), ("open", "true")]),
    ("iron_bars", &[("north", "true"), ("south", "true")]),
    ("cobweb", &[]),
    ("grindstone", &[("face", "wall"), ("facing", "west")]),
    ("ladder", &[("facing", "west")]),
    ("amethyst_cluster", &[("facing", "east")]),
    ("stone", &[]),
    ("stone", &[]),
    ("stone", &[]),
    ("stone", &[]),
];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ModelInstance {
    pub positions: [[f32; 3]; 4],
    pub uv: [[f32; 2]; 4],
    pub layer: u32,
    pub shade: f32,
}
pub struct Gallery {
    pub quads: Vec<ModelInstance>,
    pub states: Vec<Value>,
}
pub fn gallery(root: &Path, registry: &BlockRegistry, sprites: &mut Sprites) -> Result<Gallery> {
    let models = StateModels::load(root, registry, &GALLERY.map(|(name, _)| name), sprites)?;
    let mut instances = Vec::new();
    let mut states = Vec::new();
    for (index, (name, properties)) in GALLERY.iter().enumerate() {
        let state = registry.resolve(&descriptor(name, properties))?;
        let actual = registry
            .state(state)
            .ok_or("missing gallery registry state")?;
        let position = BlockPos::new((index % 4) as i32 * 4 - 7, 4, (index / 4) as i32 * 4 + 14);
        let origin = [position.x as f32, position.y as f32, position.z as f32];
        let selected = models.collect(state, position)?;
        states.push(json!({"block":actual.name,"properties":actual.properties,"state_id":state.0,
            "position":[position.x,position.y,position.z],
            "models":selected.iter().map(|(v,_)| json!({"model":v.model,"rotations":v.rotations,"uv_lock":v.uv_lock})).collect::<Vec<_>>()}));
        for (_, faces) in selected {
            for face in faces {
                let quad = &face.quad;
                let shade = match quad.shade_direction_override.unwrap_or(quad.facing) {
                    Direction::Down => 0.5,
                    Direction::Up => 1.0,
                    Direction::North | Direction::South => 0.8,
                    Direction::West | Direction::East => 0.6,
                };
                instances.push(ModelInstance {
                    positions: quad
                        .positions
                        .map(|v| std::array::from_fn(|i| v[i] + origin[i])),
                    uv: quad.uv,
                    layer: face.layer,
                    shade,
                });
            }
        }
    }
    Ok(Gallery {
        quads: instances,
        states,
    })
}
