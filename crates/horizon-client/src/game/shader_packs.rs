//! Shader packs, for development until the Shader Packs page exists: `HORIZON_SHADER_PACK=<name>`
//! runs `shaderpacks/<name>` (a folder or a zip beside `options.txt`) over the world. The
//! programs of the current dimension's folder are loaded when the dimension changes (spec 1.2).

use horizon_client::camera::CameraFluid;
use horizon_client::gfx::pack_uniforms::PackInputs;
use horizon_shaders::{BlockIds, Macros, Pack, Properties, dimension_folder, load_post};

use super::{Dim, Game};
use horizon_client::gfx::pack_draw::EntitySpan;
use std::cell::RefCell;
use std::collections::HashMap;

/// The ids a pack gives entities and block entities (4.1, 5.3), for the spans of the entity
/// geometry.
pub(super) struct PackIds {
    entities: HashMap<String, i32>,
    blocks: BlockIds,
    /// `blockEntityId` by block state, filled as states are drawn.
    block_states: RefCell<HashMap<u32, i32>>,
    /// The id of a name the file lacks: 65535, or 0 without the file.
    missing_entity: i32,
    missing_block: i32,
}

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
                    let mut post = load_post(&pack, &folder, &properties, &macros, &inputs);
                    let biomes = self.res.biomes();
                    let names: Vec<&str> = biomes.list.iter().map(|b| b.name.as_str()).collect();
                    post.custom.set_biomes(&names);
                    let text = pack.read_text("block.properties").unwrap_or_default();
                    let ids = BlockIds::parse(&Properties::parse(&text, &macros));
                    for note in &ids.notes {
                        log::warn!("shader pack {name}: block.properties: {note}");
                    }
                    let text = pack.read_text("entity.properties");
                    let entities = BlockIds::parse_kind(
                        &Properties::parse(text.as_deref().unwrap_or_default(), &macros),
                        "entity",
                    );
                    self.pack_ids = Some(PackIds {
                        entities: entities.plain(),
                        missing_entity: if text.is_some() { 65535 } else { 0 },
                        missing_block: if ids.is_empty() { 0 } else { 65535 },
                        blocks: ids.clone(),
                        block_states: RefCell::default(),
                    });
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
                    self.pack_ids = None;
                    log::warn!("shader pack {name}: {e}");
                    renderer.set_shader_pack(dimension, None, &[]);
                }
            }
        }
        let lerp = |(old, new): (f32, f32)| old + (new - old) * partial;
        let at = camera.floor().as_ivec3();
        let light = self.world.light(at.x, at.y, at.z);
        let biome = self
            .world
            .chunks
            .get(&(at.x >> 4, at.z >> 4))
            .map(|c| c.biome(at.x, at.y, at.z));
        let info = biome.map(|b| self.res.biomes().get(b));
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
            biome: biome.map_or(-1, i32::from),
            biome_category: info.map_or(0, |b| horizon_shaders::biome_category(&b.name)),
            biome_precipitation: info.map_or(0, |b| {
                horizon_shaders::biome_precipitation(b.precipitation, b.temperature)
            }),
            temperature: info.map_or(0.0, |b| b.temperature),
            rainfall: info.map_or(0.0, |b| b.downfall),
        }
    }
}

impl Game<'_> {
    /// Starts a span of the entity geometry at vertex `start` for entity type `name`
    /// (`minecraft:zombie`; "" for one the pack cannot name). Nothing without a pack.
    pub(super) fn mark_entity(&self, start: usize, name: &str) {
        if let Some(ids) = &self.pack_ids {
            let id = ids.entities.get(name).copied();
            self.push_span(start, "ENTITIES", id.unwrap_or(ids.missing_entity));
        }
    }

    /// Starts a span for the block entity of block state `state`.
    pub(super) fn mark_block_entity(&self, start: usize, state: u32) {
        let Some(ids) = &self.pack_ids else { return };
        let id = *ids
            .block_states
            .borrow_mut()
            .entry(state)
            .or_insert_with(|| {
                let registry = self.pack.assets.registry;
                registry
                    .state(horizon_world::registry::BlockStateId(state))
                    .and_then(|s| {
                        ids.blocks
                            .id(&s.name, |k| s.properties.get(k).map(String::as_str))
                    })
                    .unwrap_or(ids.missing_block)
            });
        self.push_span(start, "BLOCK_ENTITIES", id);
    }

    /// Starts a span of geometry that is not an entity (`PARTICLES`, `DESTROY`...).
    pub(super) fn mark_stage(&self, start: usize, stage: &str) {
        if self.pack_ids.is_some() {
            self.push_span(start, stage, 0);
        }
    }

    fn push_span(&self, start: usize, stage: &str, id: i32) {
        let mut spans = self.entity_spans.borrow_mut();
        let span = EntitySpan {
            start: start as u32,
            stage: horizon_shaders::render_stage(stage),
            id,
        };
        // A span that drew nothing gives way to the next.
        match spans.last_mut() {
            Some(last) if last.start == span.start => *last = span,
            _ => spans.push(span),
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
