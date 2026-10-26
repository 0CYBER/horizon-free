//! Shader packs, for development until the Shader Packs page exists: `HORIZON_SHADER_PACK=<name>`
//! runs `shaderpacks/<name>` (a folder or a zip beside `options.txt`) over the world. The
//! programs of the current dimension's folder are loaded when the dimension changes (spec 1.2).

use horizon_client::camera::CameraFluid;
use horizon_client::gfx::pack_uniforms::PackInputs;
use horizon_shaders::{BlockIds, Macros, Pack, Properties, dimension_folder, load_post};

use super::{Dim, Game};

impl Game<'_> {
    /// Loads the pack for the current dimension when it changed, and returns this frame's
    /// inputs to the pack's uniforms.
    pub(super) fn shader_pack_inputs(&mut self, camera: glam::DVec3, partial: f32) -> PackInputs {
        let Some(name) = std::env::var_os("HORIZON_SHADER_PACK") else {
            return PackInputs::default();
        };
        let dimension = match self.dim {
            Dim::Overworld => 0,
            Dim::Nether => -1,
            Dim::End => 1,
        };
        if let Some(renderer) = &mut self.renderer
            && renderer.shader_pack_dimension() != Some(dimension)
        {
            let name = name.to_string_lossy();
            let started = std::time::Instant::now();
            let path = std::env::current_dir()
                .unwrap_or_default()
                .join("shaderpacks")
                .join(&*name);
            match Pack::open(&path) {
                Ok(pack) => {
                    let macros = Macros::standard();
                    let text = pack.read_text("shaders.properties").unwrap_or_default();
                    let properties = Properties::parse(&text, &macros);
                    for note in &properties.notes {
                        log::warn!(
                            "shader pack {name}: shaders.properties:{}: {}",
                            note.line,
                            note.message
                        );
                    }
                    let folder = dimension_folder(&pack, dimension);
                    let inputs = renderer.pack_inputs();
                    let post = load_post(&pack, &folder, &properties, &macros, &inputs);
                    let text = pack.read_text("block.properties").unwrap_or_default();
                    let ids = BlockIds::parse(&Properties::parse(&text, &macros));
                    for note in &ids.notes {
                        log::warn!("shader pack {name}: block.properties: {note}");
                    }
                    let started_ids = std::time::Instant::now();
                    let table = block_id_table(&self.pack, &ids);
                    log::info!(
                        "shader pack {name}: mc_Entity for {} of {} block sprites in {} ms",
                        table.chunks(2).filter(|p| p[0] != -1).count(),
                        table.len() / 2,
                        started_ids.elapsed().as_millis()
                    );
                    if let Some((programs, notes)) =
                        renderer.set_shader_pack(dimension, Some(post), &table)
                    {
                        for note in notes {
                            log::warn!("shader pack {name}: {note}");
                        }
                        log::info!(
                            "shader pack {name}: folder `{folder}`, {} programs ({}) in {} ms",
                            programs.len(),
                            programs.join(" "),
                            started.elapsed().as_millis()
                        );
                    }
                }
                Err(e) => {
                    log::warn!("shader pack {name}: {e}");
                    renderer.set_shader_pack(dimension, None, &[]);
                }
            }
        }
        let lerp = |(old, new): (f32, f32)| old + (new - old) * partial;
        let at = camera.floor().as_ivec3();
        let light = self.world.light(at.x, at.y, at.z);
        PackInputs {
            day_time: self.world.day_time,
            rain: lerp(self.world.weather.rain_level),
            thunder: lerp(self.world.weather.thunder_level),
            eye_in: match self.camera_fluid(camera.to_array()) {
                CameraFluid::None => 0,
                CameraFluid::Water => 1,
                CameraFluid::Lava => 2,
                CameraFluid::PowderSnow => 3,
            },
            blindness: f32::from(u8::from(self.potions.effects.get("blindness").is_some())),
            eye_light: [i32::from(light & 15), i32::from(light >> 4)],
            hide_gui: self.ui.hide_gui,
            brightness: self.settings.gamma,
        }
    }
}

/// `mc_Entity` (x, y) of each block sprite layer, as (id, fluid) pairs: terrain vertices carry
/// their sprite layer, not their block. A layer takes the id of the block named like its sprite
/// (`block/oak_leaves` for `oak_leaves`), else the id most blocks drawing it share (ties: the
/// lowest); -1 when no block drawing it has one. y is 1 for the fluid sprites, -1 otherwise.
fn block_id_table(pack: &super::PackResources, ids: &BlockIds) -> Vec<i32> {
    use std::collections::{BTreeMap, BTreeSet};
    let layers = pack
        .render
        .sprite_layers
        .values()
        .max()
        .map_or(0, |l| l + 1) as usize;
    let mut table = vec![-1; layers * 2];
    if ids.is_empty() {
        return table;
    }
    let registry = pack.assets.registry;
    let catalog = &pack.render.model_catalog;
    let sprite = |name: &str| pack.render.sprite_layers.get(name).copied();
    // (layer, id) -> (a block named like the sprite, the blocks).
    let mut votes: BTreeMap<(u32, i32), (bool, BTreeSet<&str>)> = BTreeMap::new();
    for i in 0..registry.len() {
        let state_id = horizon_world::registry::BlockStateId(i as u32);
        let Some(state) = registry.state(state_id) else {
            continue;
        };
        let Some(id) = ids.id(&state.name, |k| state.properties.get(k).map(String::as_str)) else {
            continue;
        };
        let block = state.name.strip_prefix("minecraft:").unwrap_or(&state.name);
        let mut vote = |layer: u32| {
            let named = pack
                .layer_names
                .get(&layer)
                .and_then(|n| n.strip_prefix("block/"))
                == Some(block);
            let v = votes.entry((layer, id)).or_default();
            v.0 |= named;
            v.1.insert(&state.name);
        };
        // Fluids draw from their own sprites.
        let fluid = match block {
            "water" => ["water_still", "water_flow"],
            "lava" => ["lava_still", "lava_flow"],
            _ => ["", ""],
        };
        for f in fluid.iter().filter(|f| !f.is_empty()) {
            if let Some(l) = sprite(&format!("minecraft:block/{f}")) {
                vote(l);
            }
        }
        let origin = horizon_core::position::BlockPos::new(0, 0, 0);
        for (_, model) in catalog.collect(state_id, origin).unwrap_or_default() {
            for face in &model.faces {
                vote(face.layer);
            }
        }
    }
    let mut best: BTreeMap<u32, (bool, usize, i32)> = BTreeMap::new();
    for ((layer, id), (named, blocks)) in votes {
        let candidate = (named, blocks.len(), -id);
        let b = best.entry(layer).or_insert(candidate);
        if candidate > *b {
            *b = candidate;
        }
    }
    for (layer, (_, _, id)) in best {
        if let Some(slot) = table.get_mut(layer as usize * 2) {
            *slot = -id;
        }
    }
    for f in ["water_still", "water_flow", "lava_still", "lava_flow"] {
        if let Some(l) = sprite(&format!("minecraft:block/{f}"))
            && let Some(slot) = table.get_mut(l as usize * 2 + 1)
        {
            *slot = 1;
        }
    }
    table
}
