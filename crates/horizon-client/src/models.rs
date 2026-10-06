//! A resource-model inspection gallery. Blockstate dispatch is still separate.
use crate::{Result, sprites::Sprites};
use horizon_assets::{
    bake::bake_model,
    model::{CuboidModel, Direction},
    resolve::ModelLibrary,
};
use std::{collections::BTreeMap, fs, path::Path};

pub const GALLERY: [&str; 12] = [
    "oak_stairs",
    "stone_slab",
    "anvil",
    "torch",
    "lantern",
    "oak_fence_post",
    "oak_fence_gate",
    "iron_bars_post",
    "cobweb",
    "grindstone",
    "ladder",
    "amethyst_cluster",
];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ModelInstance {
    pub positions: [[f32; 3]; 4],
    pub uv: [[f32; 2]; 4],
    pub layer: u32,
    pub shade: f32,
}
pub fn gallery(root: &Path, sprites: &mut Sprites) -> Result<Vec<ModelInstance>> {
    let mut models = BTreeMap::new();
    let mut pending: Vec<_> = GALLERY
        .iter()
        .map(|name| format!("minecraft:block/{name}"))
        .collect();
    while let Some(name) = pending.pop() {
        if models.contains_key(&name) {
            continue;
        }
        let (namespace, path) = name.split_once(':').ok_or("invalid model identifier")?;
        let text = fs::read_to_string(root.join(format!("assets/{namespace}/models/{path}.json")))?;
        let model = CuboidModel::from_json(&text)?;
        if let Some(parent) = &model.parent {
            pending.push(parent.clone());
        }
        models.insert(name, model);
    }
    let library = ModelLibrary::new(models)?;
    let mut instances = Vec::new();
    for (index, name) in GALLERY.iter().enumerate() {
        let model = library.resolve(&format!("minecraft:block/{name}"))?;
        let origin = [
            (index % 4) as f32 * 4.0 - 7.0,
            4.0,
            (index / 4) as f32 * 4.0 + 14.0,
        ];
        for quad in bake_model(&model)? {
            if quad.material.force_translucent || quad.tint_index != -1 {
                return Err(
                    "inspection gallery requires untinted opaque or binary-cutout materials".into(),
                );
            }
            let layer = sprites.insert(root, &quad.material.sprite)?;
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
                layer,
                shade,
            });
        }
    }
    Ok(instances)
}
