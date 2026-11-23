//! Bind prepared registry states to model choices; bake each transformed model
//! once and share its geometry across positions. Resource failure recovery and
//! resource-pack reloads remain separate work.
use crate::{Result, sprites::Sprites};
use horizon_assets::{
    bake::{BlockModelTransform, ModelQuad, RenderType, bake_block_model},
    blockstate::{
        BlockStateDefinition, BlockStateFile, Variant, VariantSet,
        bind::{Properties, PropertySchema, PropertyValues, SelectedModels},
        choice::CompiledChoices,
    },
    model::Direction,
};
use horizon_core::{
    position::BlockPos,
    random::{LegacyRandom, position_seed},
};
use horizon_world::{
    chunk::BlockState,
    registry::{BlockRegistry, BlockStateId},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    sync::OnceLock,
};

/// `prepareQuadShape`: the quad's bounds, whether it is partial (smaller than
/// the block face) and whether it is cubic (flush with the block boundary, or
/// flat on a full collision shape). A function of the quad's positions and the
/// block's `isCollisionShapeFullBlock` only, so it is baked per face, once for
/// each.
#[derive(Clone, Copy)]
pub struct QuadShape {
    /// The quad's bounds by face index (`mesher::lighting`'s `SizeInfo`
    /// order: down, up, north, south, west, east), and `1.0 - the bound`.
    pub bounds: [f32; 12],
    pub partial: bool,
    pub cubic: bool,
}
impl QuadShape {
    pub fn new(positions: &[[f32; 3]; 4], face: Direction, full: bool) -> Self {
        // `SizeInfo` indices into the face shape.
        const DOWN: usize = 0;
        const UP: usize = 1;
        const NORTH: usize = 2;
        const SOUTH: usize = 3;
        const WEST: usize = 4;
        const EAST: usize = 5;
        let (mut min, mut max) = ([32f32; 3], [-32f32; 3]);
        for p in positions {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        let mut bounds = [0f32; 12];
        for (index, value) in [
            (WEST, min[0]),
            (EAST, max[0]),
            (DOWN, min[1]),
            (UP, max[1]),
            (NORTH, min[2]),
            (SOUTH, max[2]),
        ] {
            bounds[index] = value;
            bounds[index + 6] = 1.0 - value;
        }
        let (lo, hi) = (1.0e-4f32, 0.9999f32);
        let partial =
            |a: usize, b: usize| min[a] >= lo || min[b] >= lo || max[a] <= hi || max[b] <= hi;
        let (partial, cubic) = match face {
            Direction::Down => (partial(0, 2), min[1] == max[1] && (min[1] < lo || full)),
            Direction::Up => (partial(0, 2), min[1] == max[1] && (max[1] > hi || full)),
            Direction::North => (partial(0, 1), min[2] == max[2] && (min[2] < lo || full)),
            Direction::South => (partial(0, 1), min[2] == max[2] && (max[2] > hi || full)),
            Direction::West => (partial(1, 2), min[0] == max[0] && (min[0] < lo || full)),
            Direction::East => (partial(1, 2), min[0] == max[0] && (max[0] > hi || full)),
        };
        QuadShape {
            bounds,
            partial,
            cubic,
        }
    }
}
pub struct BakedFace {
    pub quad: ModelQuad,
    pub layer: u32,
    /// The texels under the quad's UVs are all opaque (`SpriteContents.computeTransparency`):
    /// drawn in the solid layer without an alpha test.
    pub opaque: bool,
    /// Fractional alpha: drawn in the sorted translucent pass.
    pub translucent: bool,
    /// The Fusion connecting texture rule its sprite joins by ([`crate::fusion`]), plus one:
    /// two bytes, within the padding every face already has.
    pub fusion: Option<std::num::NonZeroU16>,
    /// `prepareQuadShape` of the quad, indexed by the block's `full` flag: the
    /// mesher would otherwise recompute it for every face of every block.
    pub shapes: [QuadShape; 2],
    /// The quad's own UVs in `TerrainVertex` fixed point, which the mesh
    /// otherwise recomputes for every face of every block.
    pub uv_fixed: [[u16; 2]; 4],
}
impl BakedFace {
    pub fn new(
        quad: ModelQuad,
        layer: u32,
        opaque: bool,
        translucent: bool,
        fusion: Option<std::num::NonZeroU16>,
    ) -> Self {
        let shapes = [false, true].map(|full| QuadShape::new(&quad.positions, quad.facing, full));
        let uv_fixed = crate::mesher::baked_uv(quad.uv);
        Self {
            quad,
            layer,
            opaque,
            translucent,
            fusion,
            shapes,
            uv_fixed,
        }
    }
}
pub struct BakedModel {
    pub ambient_occlusion: bool,
    /// The model's particle slot also exists for models without block faces.
    pub particle: Option<u32>,
    pub faces: Vec<BakedFace>,
    pub cube: Option<[super::cube::CubeStyle; 6]>,
}
/// Compact 1-byte summary of a [`BakedModel`]'s face cull directions.
///
/// Bit layout:
/// - bits `0..6`: `cull_mask`, bit `d` set if any face has `quad.cull == Some(d)`
/// - bit `6`: `has_uncullable`, set if any face has `quad.cull.is_none()`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CullFlags(u8);
impl CullFlags {
    const CULL_MASK_BITS: u8 = 0b0011_1111;
    const HAS_UNCULLABLE_BIT: u8 = 1 << 6;

    /// Empty model flags (no culled faces and no uncullable faces).
    pub const EMPTY: Self = Self(0);

    /// Computes the cull flags for a slice of baked faces.
    pub fn from_faces(faces: &[BakedFace]) -> Self {
        let mut bits = 0u8;
        for face in faces {
            match face.quad.cull {
                Some(d) => bits |= 1u8 << (d as u8),
                None => bits |= Self::HAS_UNCULLABLE_BIT,
            }
        }
        Self(bits)
    }

    /// Bitmask of `face.quad.cull` directions present (`1 << (d as u8)`).
    #[inline]
    pub const fn cull_mask(self) -> u8 {
        self.0 & Self::CULL_MASK_BITS
    }

    /// Whether any face has `face.quad.cull.is_none()`.
    #[inline]
    pub const fn has_uncullable(self) -> bool {
        self.0 & Self::HAS_UNCULLABLE_BIT != 0
    }

    /// Combines the cull flags of two models (e.g. multipart parts).
    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}
const _: () = {
    assert!((Direction::Down as u8) < 6);
    assert!((Direction::Up as u8) < 6);
    assert!((Direction::North as u8) < 6);
    assert!((Direction::South as u8) < 6);
    assert!((Direction::West as u8) < 6);
    assert!((Direction::East as u8) < 6);
};
type VariantKey = (String, [u8; 3], bool);
/// Empty by default: no state has a model (a dedicated server, which draws
/// nothing).
#[derive(Default)]
pub struct StateModels {
    /// Each definition's choices, a variant standing as its index in `baked`.
    choices: Vec<CompiledChoices<u32>>,
    /// All baked alternatives per definition, invalidated before mutable model access.
    bounds: OnceLock<Vec<Option<[[f32; 3]; 2]>>>,
    /// Precomputed cull flags per `baked` slot, invalidated before mutable model access.
    cull_flags: OnceLock<Vec<CullFlags>>,
    /// By state id: its definition in `choices` and the models it selects.
    states: Vec<Option<(usize, SelectedModels)>>,
    single_models: Vec<Option<u32>>,
    /// Every transformed model once, with the variant it was baked from.
    baked: Vec<(Variant, BakedModel)>,
    /// Standalone models by id: their index in `baked`.
    standalone: BTreeMap<String, u32>,
}
fn key(variant: &Variant) -> VariantKey {
    (variant.model.clone(), variant.rotations, variant.uv_lock)
}
/// Bakes model `name` of `library`, turned and UV-locked as a variant says.
fn bake_variant(
    sprites: &mut Sprites,
    library: &horizon_assets::resolve::ModelLibrary,
    name: &str,
    turns: [u8; 3],
    lock: bool,
) -> Result<BakedModel> {
    bake_resolved(sprites, &library.resolve(name)?, turns, lock)
}
/// Bakes a resolved `model`, turned and UV-locked as a variant says.
fn bake_resolved(
    sprites: &mut Sprites,
    model: &horizon_assets::resolve::ResolvedCuboidModel,
    turns: [u8; 3],
    lock: bool,
) -> Result<BakedModel> {
    let particle = model
        .material("particle")?
        .map(|material| sprites.insert(&material.sprite))
        .transpose()?;
    let faces: Vec<BakedFace> = bake_block_model(model, BlockModelTransform::new(turns, lock))?
        .into_iter()
        .map(|quad| {
            let layer = sprites.insert(&quad.material.sprite)?;
            // A model's `render_type` replaces the pass its sprite would pick.
            let (translucent, opaque) = match quad.render_type {
                Some(RenderType::Translucent | RenderType::Tripwire) => (true, false),
                Some(RenderType::Solid) => (false, true),
                Some(RenderType::Cutout | RenderType::CutoutMipped) => (false, false),
                None | Some(RenderType::Unsupported) => (
                    quad.material.force_translucent || sprites.translucent[layer as usize],
                    sprites.region_opaque(layer, &quad.uv),
                ),
            };
            Ok(BakedFace::new(quad, layer, opaque, translucent, None))
        })
        .collect::<Result<_>>()?;
    let cube = super::cube::cube_styles(&faces);
    Ok(BakedModel {
        ambient_occlusion: model.ambient_occlusion,
        particle,
        faces,
        cube,
    })
}

/// `FaceBakery.bakeQuad`: use the same centered geometry and inverse face UV
/// transformation as a blockstate variant, not the draw-time pose.
fn bake_form(
    root: &crate::resource_source::ResourceSource,
    sprites: &mut Sprites,
    models: &crate::model_library::ParsedModels,
    form: &mod_api::StandaloneModelForm,
) -> Result<BakedModel> {
    use mod_api::StandaloneModelForm;
    match form {
        StandaloneModelForm::Rotated {
            model,
            turns,
            uvlock,
            ..
        } => {
            let library = models.block_library(root, vec![model.clone()])?;
            bake_variant(sprites, &library, model, *turns, *uvlock)
        }
        StandaloneModelForm::Combined { parts, .. } => {
            let library = models.block_library(root, parts.clone())?;
            let mut combined = BakedModel {
                ambient_occlusion: true,
                particle: None,
                faces: Vec::new(),
                cube: None,
            };
            for part in parts {
                let baked = bake_variant(sprites, &library, part, [0; 3], false)?;
                if combined.particle.is_none() {
                    combined.particle = baked.particle;
                }
                combined.ambient_occlusion &= baked.ambient_occlusion;
                combined.faces.extend(baked.faces);
            }
            combined.cube = super::cube::cube_styles(&combined.faces);
            Ok(combined)
        }
    }
}
/// The definitions a blockstate file holds, bottom first: the layers of a
/// file the pack overlay stacked (`packs::BLOCKSTATE_LAYERS`), else the
/// file itself.
fn definition_layers(text: &str) -> std::result::Result<Vec<String>, String> {
    if !text.contains(crate::packs::BLOCKSTATE_LAYERS) {
        return Ok(vec![text.to_owned()]);
    }
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    match value.get(crate::packs::BLOCKSTATE_LAYERS) {
        Some(Value::Array(layers)) => Ok(layers.iter().map(Value::to_string).collect()),
        _ => Ok(vec![text.to_owned()]),
    }
}
/// One layer of block `block`'s blockstate file, read and bound to its `possible` states:
/// vanilla's variants and multipart, or the picks of the definition type a mod registered.
fn bind_layer(
    text: &str,
    block: &str,
    schema: &PropertySchema,
    possible: &[Properties],
) -> std::result::Result<(BlockStateDefinition, Vec<Option<SelectedModels>>), String> {
    match BlockStateFile::from_json(text).map_err(|e| e.to_string())? {
        BlockStateFile::Vanilla(definition) => {
            let bound = definition
                .bind(schema, possible)
                .map_err(|e| e.to_string())?;
            Ok((definition, bound))
        }
        BlockStateFile::Typed { kind, config } => {
            crate::mod_block_state_models::instantiate(&kind, block, &config, possible)
                .map(BlockStateDefinition::from_picks)
        }
    }
}
/// Each state's model from definitions bound bottom first: the topmost one
/// that binds the state, as (its layer, its selection). A higher pack's
/// definition replaces only the states it binds (`putAll` in
/// `loadBlockStateDefinitionStack`).
fn topmost(layers: Vec<Vec<Option<SelectedModels>>>) -> Vec<Option<(usize, SelectedModels)>> {
    let mut picked: Vec<Option<(usize, SelectedModels)>> = Vec::new();
    for (layer, bound) in layers.into_iter().enumerate() {
        picked.resize(picked.len().max(bound.len()), None);
        for (slot, selected) in picked.iter_mut().zip(bound) {
            if let Some(selected) = selected {
                *slot = Some((layer, selected));
            }
        }
    }
    picked
}
impl StateModels {
    pub fn load(
        root: &crate::resource_source::ResourceRoot,
        registry: &BlockRegistry,
        blocks: &[&str],
        sprites: &mut Sprites,
    ) -> Result<Self> {
        let facts: BTreeMap<String, Value> =
            serde_json::from_slice(&fs::read(root.prepared().join("model-states.json"))?)?;
        Self::from_facts(root.source(), registry, blocks, sprites, &facts)
    }
    /// Bind and bake models using an already parsed model-state document.
    pub fn from_facts(
        root: &crate::resource_source::ResourceSource,
        registry: &BlockRegistry,
        blocks: &[&str],
        sprites: &mut Sprites,
        facts: &BTreeMap<String, Value>,
    ) -> Result<Self> {
        Self::from_facts_with_models(
            root,
            registry,
            blocks,
            sprites,
            facts,
            &crate::model_library::ParsedModels::default(),
        )
    }
    pub(crate) fn from_facts_with_models(
        root: &crate::resource_source::ResourceSource,
        registry: &BlockRegistry,
        blocks: &[&str],
        sprites: &mut Sprites,
        facts: &BTreeMap<String, Value>,
        models: &crate::model_library::ParsedModels,
    ) -> Result<Self> {
        let mut choices = Vec::new();
        let mut states = vec![None; registry.len()];
        let mut variants = BTreeSet::new();
        let mut by_name: std::collections::HashMap<&str, Vec<BlockStateId>> =
            std::collections::HashMap::new();
        for i in 0..registry.len() {
            let id = BlockStateId(i as u32);
            if let Some(state) = registry.state(id) {
                by_name.entry(state.name.as_str()).or_default().push(id);
            }
        }
        for block in blocks.iter().copied().collect::<BTreeSet<_>>() {
            // Vanilla blocks come bare; content mods' blocks with their namespace.
            let (namespace, path) = block.split_once(':').unwrap_or(("minecraft", block));
            let name = format!("{namespace}:{path}");
            let facts = facts.get(&name).ok_or("missing block model state schema")?;
            let schema: PropertySchema = facts["schema"]
                .as_object()
                .ok_or("invalid model schema")?
                .iter()
                .map(|(key, value)| {
                    let values = if let Some(range) = value.get("integer") {
                        PropertyValues::Integer {
                            minimum: i32::try_from(
                                range[0]
                                    .as_i64()
                                    .ok_or("invalid integer property minimum")?,
                            )?,
                            maximum: i32::try_from(
                                range[1]
                                    .as_i64()
                                    .ok_or("invalid integer property maximum")?,
                            )?,
                        }
                    } else {
                        PropertyValues::Named(
                            value["named"]
                                .as_array()
                                .ok_or("missing named property values")?
                                .iter()
                                .map(|v| {
                                    v.as_str()
                                        .map(str::to_owned)
                                        .ok_or("invalid named property value")
                                })
                                .collect::<std::result::Result<_, _>>()?,
                        )
                    };
                    Ok((key.clone(), values))
                })
                .collect::<Result<_>>()?;
            let possible: Vec<Properties> = serde_json::from_value(facts["states"].clone())?;
            // Vanilla's BlockStateModelLoader: a definition that does not
            // read or bind is logged and left out; with none left the block
            // gets no model, and the other blocks still load. A file several
            // packs provide binds each pack's definition on its own, bottom
            // first (`loadBlockStateDefinitionStack`).
            let file = format!("assets/{namespace}/blockstates/{path}.json");
            let layers = match root
                .read_to_string(&file)
                .map_err(|e| e.to_string())
                .and_then(|text| definition_layers(&text))
            {
                Ok(layers) => layers,
                Err(e) => {
                    log::warn!("Horizon: blockstate definition {file} not loaded: {e}");
                    continue;
                }
            };
            let mut loaded = Vec::with_capacity(layers.len());
            for text in &layers {
                match bind_layer(text, &name, &schema, &possible) {
                    Ok(layer) => loaded.push(layer),
                    Err(e) => log::warn!("Horizon: blockstate definition {file} not loaded: {e}"),
                }
            }
            if loaded.is_empty() {
                continue;
            }
            let native_order: Vec<_> = by_name
                .get(name.as_str())
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .map(|&id| (id, &registry.state(id).expect("indexed state").properties))
                .collect();
            if native_order.len() != possible.len()
                || native_order
                    .iter()
                    .zip(&possible)
                    .any(|((_, a), b)| *a != b)
            {
                return Err(
                    "model state definitions disagree with the imported block registry".into(),
                );
            }
            let (definitions, bound): (Vec<_>, Vec<_>) = loaded.into_iter().unzip();
            let first = choices.len();
            for ((id, _), picked) in native_order.into_iter().zip(topmost(bound)) {
                if let Some((layer, selected)) = picked {
                    states[id.0 as usize] = Some((first + layer, selected));
                }
            }
            for definition in definitions {
                for choice in definition
                    .variants
                    .iter()
                    .flatten()
                    .map(|(_, choice)| choice)
                    .chain(
                        definition
                            .multipart
                            .iter()
                            .flatten()
                            .map(|selector| &selector.variant),
                    )
                {
                    match choice {
                        VariantSet::Single(v) => {
                            variants.insert(key(v));
                        }
                        VariantSet::Weighted(v) => variants.extend(v.iter().map(|(v, _)| key(v))),
                    }
                }
                choices.push(CompiledChoices::new(&definition)?);
            }
        }
        let names: Vec<String> = variants.iter().map(|(name, _, _)| name.clone()).collect();
        let library = match models.block_library(root, names.clone()) {
            Ok(library) => library,
            Err(e) => {
                log::warn!("Horizon: block models incomplete ({e}); using the missing model");
                models.block_library_or_missing(root, names)?
            }
        };
        let resolved: Vec<_> = variants
            .iter()
            .map(|(name, ..)| library.resolve(name))
            .collect();
        // Every texture the models name, decoded side by side before the bake adds them one
        // by one in its own order, as many as the renderer's arrays hold.
        let named: Vec<&str> = resolved
            .iter()
            .flatten()
            .flat_map(|model| model.textures.values())
            .map(|material| material.sprite.as_str())
            .collect();
        sprites.plan_within_capacity(named.iter().copied());
        sprites.prefetch(named);
        let mut baked = Vec::with_capacity(variants.len());
        let mut index = BTreeMap::new();
        for ((name, turns, lock), model) in variants.into_iter().zip(resolved) {
            let model = bake_resolved(sprites, &model?, turns, lock)?;
            index.insert((name.clone(), turns, lock), baked.len() as u32);
            let variant = Variant {
                model: name,
                rotations: turns,
                uv_lock: lock,
            };
            baked.push((variant, model));
        }
        // Every variant of every definition was baked above.
        let choices: Vec<_> = choices.iter().map(|c| c.map(|v| index[&key(v)])).collect();
        let single_models = states
            .iter()
            .map(|state| {
                let (choice, selected) = state.as_ref()?;
                choices[*choice].single_variant(selected).copied()
            })
            .collect();
        Ok(Self {
            choices,
            bounds: OnceLock::new(),
            cull_flags: OnceLock::new(),
            states,
            single_models,
            baked,
            standalone: BTreeMap::new(),
        })
    }
    /// Bakes `names`, models no blockstate needs to name (vanilla's
    /// standalone models, NeoForge's `ModelEvent.RegisterStandalone`), as
    /// they are, unturned, for [`Self::standalone`]. One a block state
    /// already bakes unturned is shared. A model that doesn't read (missing,
    /// or a parent missing) is logged and left out, as a missing blockstate
    /// is; the others still bake.
    pub(crate) fn bake_standalone(
        &mut self,
        root: &crate::resource_source::ResourceSource,
        sprites: &mut Sprites,
        models: &crate::model_library::ParsedModels,
        names: &[String],
    ) -> Result<()> {
        for name in names {
            if self.standalone.contains_key(name) {
                continue;
            }
            if let Some(i) = self
                .baked
                .iter()
                .position(|(v, _)| v.model == *name && v.rotations == [0; 3] && !v.uv_lock)
            {
                self.standalone.insert(name.clone(), i as u32);
                continue;
            }
            let model = models
                .block_library(root, vec![name.clone()])
                .and_then(|library| bake_variant(sprites, &library, name, [0; 3], false));
            match model {
                Ok(model) => {
                    self.standalone
                        .insert(name.clone(), self.baked.len() as u32);
                    let variant = Variant {
                        model: name.clone(),
                        rotations: [0; 3],
                        uv_lock: false,
                    };
                    self.cull_flags.take();
                    self.baked.push((variant, model));
                }
                Err(e) => log::warn!("Horizon: standalone model {name} not loaded: {e}"),
            }
        }
        Ok(())
    }
    /// Bake dependencies atomically into one standalone slot; never publish a partial form.
    pub(crate) fn bake_standalone_forms(
        &mut self,
        root: &crate::resource_source::ResourceSource,
        sprites: &mut Sprites,
        models: &crate::model_library::ParsedModels,
        forms: &[super::StandaloneFormRegistration],
    ) {
        for entry in forms {
            let id = entry.form.id();
            if self.standalone.contains_key(id) {
                continue;
            }
            let result = bake_form(root, sprites, models, &entry.form);
            match result {
                Ok(model) => {
                    self.standalone
                        .insert(id.to_owned(), self.baked.len() as u32);
                    self.cull_flags.take();
                    self.baked.push((
                        Variant {
                            model: id.to_owned(),
                            rotations: [0; 3],
                            uv_lock: false,
                        },
                        model,
                    ));
                }
                Err(error) => log::warn!(
                    "Horizon: mod {} standalone form {id} not loaded: {error}",
                    entry.owner
                ),
            }
        }
    }
    /// Every baked model with the variant it was baked from, to change its faces.
    pub fn models_mut(&mut self) -> impl Iterator<Item = (&Variant, &mut BakedModel)> {
        self.bounds.take();
        self.cull_flags.take();
        self.baked
            .iter_mut()
            .map(|(variant, model)| (&*variant, model))
    }
    /// Precomputed cull flags for every slot in this catalog's baked store,
    /// lazily built and invalidated before mutable access via [`Self::models_mut`].
    pub fn cull_flags(&self) -> &[CullFlags] {
        self.cull_flags
            .get_or_init(|| {
                self.baked
                    .iter()
                    .map(|(_, model)| CullFlags::from_faces(&model.faces))
                    .collect()
            })
            .as_slice()
    }
    /// A model baked by [`Self::bake_standalone`], by id (`mymod:block/shaft_half`).
    pub fn standalone(&self, name: &str) -> Option<&BakedModel> {
        self.model_by_slot(self.standalone_slot(name)?)
    }
    /// Resolve a standalone name once, without copying its baked geometry.
    pub fn standalone_slot(&self, name: &str) -> Option<u32> {
        self.standalone.get(name).copied()
    }
    /// Slots of registered standalone models, including models shared with block states.
    pub fn standalone_slots(&self) -> impl Iterator<Item = u32> + '_ {
        self.standalone.values().copied()
    }
    /// Geometry in this catalog's existing baked store.
    pub fn model_by_slot(&self, slot: u32) -> Option<&BakedModel> {
        self.baked.get(slot as usize).map(|(_, model)| model)
    }
    pub fn collect(
        &self,
        state: BlockStateId,
        position: BlockPos,
    ) -> Result<Vec<(&Variant, &BakedModel)>> {
        // Blocks use their position seed; contained/displayed blocks can supply one.
        self.collect_seeded(state, position_seed(position))
    }
    /// `BlockStateModel.particleMaterial`: the particle sprite layer of the
    /// state's first variant (see `CompiledChoices::particle_variant`).
    pub fn particle(&self, state: BlockStateId) -> Option<u32> {
        let (choice, selected) = self.state(state)?;
        let model = self.choices[*choice].particle_variant(selected)?;
        self.baked[*model as usize].1.particle
    }
    /// Envelope of every weighted/multipart alternative in the state's definition.
    /// Includes transformed resource-pack faces outside the unit cube. Empty or
    /// non-finite geometry has no usable envelope and keeps the original draw path.
    pub fn render_bounds(&self, state: BlockStateId) -> Option<[[f32; 3]; 2]> {
        let (definition, _) = self.state(state)?;
        let bounds = self.bounds.get_or_init(|| {
            self.choices
                .iter()
                .map(|choices| {
                    let mut lo = glam::Vec3::splat(f32::INFINITY);
                    let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
                    let mut valid = true;
                    // `map` visits every choice, without consuming the render RNG.
                    choices.map(|&index| {
                        for face in &self.baked[index as usize].1.faces {
                            for p in face.quad.positions {
                                let p = glam::Vec3::from_array(p);
                                valid &= p.is_finite();
                                lo = lo.min(p);
                                hi = hi.max(p);
                            }
                        }
                    });
                    (valid && lo.is_finite() && hi.is_finite())
                        .then_some([lo.to_array(), hi.to_array()])
                })
                .collect()
        });
        bounds.get(*definition).copied().flatten()
    }

    pub fn collect_seeded(
        &self,
        state: BlockStateId,
        seed: i64,
    ) -> Result<Vec<(&Variant, &BakedModel)>> {
        let mut models = Vec::new();
        self.each_seeded(state, seed, |(variant, model)| {
            models.push((variant, model))
        })?;
        Ok(models)
    }
    /// The models a block draws at `position`, into `out` (cleared first):
    /// `collect` for the mesher, which reuses one list for every block.
    pub fn models_at<'a>(
        &'a self,
        state: BlockStateId,
        position: BlockPos,
        out: &mut Vec<&'a BakedModel>,
    ) -> Result<()> {
        out.clear();
        if let Some(Some(model)) = self.single_models.get(state.0 as usize) {
            out.push(&self.baked[*model as usize].1);
            return Ok(());
        }
        self.each_seeded(state, position_seed(position), |(_, model)| out.push(model))
    }
    /// Like [`Self::models_at`], writing baked-model slot indices into a stack-backed
    /// `SmallVec` so single-variant and small multipart blocks avoid heap allocation
    /// and can inspect [`Self::cull_flags`] before reading [`BakedModel`] geometry.
    pub fn model_slots_at_small<A: smallvec::Array<Item = u32>>(
        &self,
        state: BlockStateId,
        position: BlockPos,
        out: &mut smallvec::SmallVec<A>,
    ) -> Result<()> {
        out.clear();
        if let Some(Some(model)) = self.single_models.get(state.0 as usize) {
            out.push(*model);
            return Ok(());
        }
        self.each_slot_seeded(state, position_seed(position), |model| out.push(model))
    }
    fn each_slot_seeded(
        &self,
        state: BlockStateId,
        seed: i64,
        mut f: impl FnMut(u32),
    ) -> Result<()> {
        let (choice, selected) = self.state(state).ok_or("unbound gallery block state")?;
        let mut random = LegacyRandom::new(seed);
        self.choices[*choice].for_each(selected, &mut random, |&model| {
            f(model);
        })?;
        Ok(())
    }
    fn each_seeded<'a>(
        &'a self,
        state: BlockStateId,
        seed: i64,
        mut f: impl FnMut(&'a (Variant, BakedModel)),
    ) -> Result<()> {
        self.each_slot_seeded(state, seed, |model| f(&self.baked[model as usize]))
    }
    fn state(&self, state: BlockStateId) -> Option<&(usize, SelectedModels)> {
        self.states.get(state.0 as usize)?.as_ref()
    }
}

pub fn descriptor(name: &str, overrides: &[(&str, &str)]) -> BlockState {
    BlockState {
        name: format!("minecraft:{name}"),
        properties: overrides
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_library::ParsedModels;

    /// #7013: a pack's blockstate file that defines only some states (here
    /// a fence gate's `powered=true` ones) keeps vanilla's model for the
    /// rest, as vanilla binds each pack's definition and lets a higher one
    /// replace only what it binds.
    #[test]
    fn a_partial_pack_blockstate_keeps_vanilla_for_the_other_states() {
        let schema: PropertySchema = BTreeMap::from([(
            "powered".to_owned(),
            PropertyValues::Named(BTreeSet::from(["false".to_owned(), "true".to_owned()])),
        )]);
        let states: Vec<Properties> = ["false", "true"]
            .iter()
            .map(|p| BTreeMap::from([("powered".to_owned(), (*p).to_owned())]))
            .collect();
        let stacked = serde_json::json!({ crate::packs::BLOCKSTATE_LAYERS: [
            {"variants":{"":{"model":"minecraft:block/oak_fence_gate"}}},
            {"variants":{"powered=true":{"model":"minecraft:block/stone"}}}
        ]})
        .to_string();
        let layers = definition_layers(&stacked).unwrap();
        assert_eq!(layers.len(), 2);
        let bound = layers
            .iter()
            .map(|text| {
                BlockStateDefinition::from_json(text)
                    .unwrap()
                    .bind(&schema, &states)
                    .unwrap()
            })
            .collect();
        let picked = topmost(bound);
        assert_eq!(picked[0], Some((0, SelectedModels::Variant(0))));
        assert_eq!(picked[1], Some((1, SelectedModels::Variant(0))));
        // A file that isn't stacked is one definition.
        let plain = r#"{"variants":{"":{"model":"minecraft:block/stone"}}}"#;
        assert_eq!(definition_layers(plain).unwrap(), vec![plain.to_owned()]);
    }

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn write(&self, name: &str, json: &str) {
            let (namespace, path) = name.split_once(':').unwrap();
            let path = self
                .0
                .join(format!("assets/{namespace}/models/{path}.json"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, json).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn archive_models_bind_prepared_states_and_load_world_texture_features_in_place() -> Result<()>
    {
        use crate::resource_source::{ResourceRoot, ResourceSource};
        use std::io::Write;
        use {horizon_core::nbt::Tag, horizon_world::DATA_VERSION};
        let fixture = Fixture(
            std::env::temp_dir().join(format!("horizon-world-archive-{}", std::process::id())),
        );
        let prepared = fixture.0.join("prepared");
        fs::create_dir_all(&prepared)?;
        fs::write(
            prepared.join("model-states.json"),
            br#"{"demo:stone":{"schema":{},"states":[{}]}}"#,
        )?;
        let state = BlockState {
            name: "demo:stone".into(),
            properties: Default::default(),
        };
        let registry = BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("states", Tag::List(vec![state.to_nbt()])),
            ("defaults", Tag::compound([("demo:stone", Tag::Int(0))])),
        ]))?;
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()?
                .write_image_data(&[10, 20, 30, 255].repeat(256))?;
        }
        let jar = fixture.0.join("world.jar");
        let mut writer = zip::ZipWriter::new(fs::File::create(&jar)?);
        for (name, bytes) in [
            ("model-states.json", b"this is not the prepared schema".as_slice()),
            ("assets/demo/blockstates/stone.json", br#"{"variants":{"":{"model":"demo:block/child"}}}"#),
            ("assets/demo/models/block/child.json", br#"{"loader":"fusion:model","type":"fusion:connecting","parent":"demo:block/base"}"#),
            ("assets/demo/models/block/base.json", br##"{"textures":{"all":"demo:block/stone","particle":"#all"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"north":{"texture":"#all"}}}]}"##),
            ("assets/demo/textures/block/stone.png.mcmeta", br#"{"fusion":{"type":"connecting","layout":"simple"}}"#),
            ("assets/demo/optifine/ctm/stone.properties", b"matchBlocks=demo:stone\nmethod=fixed\ntiles=./tile\n"),
        ] {
            writer.start_file(name, zip::write::SimpleFileOptions::default())?;
            writer.write_all(bytes)?;
        }
        for (name, color) in [
            ("assets/demo/textures/block/stone.png", [10, 20, 30, 255]),
            ("assets/demo/textures/block/stone_e.png", [30, 40, 50, 255]),
            ("assets/demo/optifine/ctm/tile.png", [50, 60, 70, 255]),
            ("assets/demo/optifine/ctm/tile_e.png", [70, 80, 90, 255]),
            (
                "assets/demo/textures/emissive/glow.png",
                [90, 100, 110, 255],
            ),
        ] {
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, 16, 16);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .write_header()?
                    .write_image_data(&color.repeat(256))?;
            }
            writer.start_file(name, zip::write::SimpleFileOptions::default())?;
            writer.write_all(&bytes)?;
        }
        writer.finish()?;
        let root = ResourceRoot::new(
            prepared.clone(),
            fixture.0.join("cache"),
            ResourceSource::archive(&jar)?,
        );
        let mut sprites = Sprites::with_disk_cache(&root);
        let mut catalog = StateModels::load(&root, &registry, &["demo:stone"], &mut sprites)?;
        let base = sprites.layers["demo:block/stone"];
        let picked = catalog.collect(BlockStateId(0), BlockPos::new(0, 0, 0))?;
        assert_eq!(picked.len(), 1);
        assert_eq!(
            picked[0].1.faces.len(),
            1,
            "parent geometry loaded from the jar"
        );
        assert_eq!(picked[0].1.faces[0].layer, base);
        assert_eq!(picked[0].1.particle, Some(base));
        assert!(
            sprites
                .try_insert_limited("demo:block/stone", png.len() as u64 - 1)
                .is_err(),
            "the byte limit also applies to cached layers"
        );
        assert_eq!(
            sprites.try_insert_limited("demo:block/stone", png.len() as u64)?,
            base
        );
        let _fusion = crate::fusion::FusionTextures::load(
            root.source(),
            &registry,
            &mut sprites,
            &mut catalog,
        );
        assert!(
            catalog.collect(BlockStateId(0), BlockPos::new(0, 0, 0))?[0]
                .1
                .faces[0]
                .fusion
                .is_some()
        );
        let connected = crate::connected::ConnectedTextures::load(
            root.source(),
            horizon_optifine::ctm::load(root.source()),
            &registry,
            &mut sprites,
            |_, _| Some(base),
        );
        assert!(!connected.is_empty());
        crate::native_emissive::load(root.source(), &mut sprites);
        assert!(sprites.layers.contains_key("demo:emissive/glow"));
        let pairs = crate::native_emissive::load_optifine(root.source(), &mut sprites);
        assert!(pairs.contains(&("demo:block/stone".into(), "demo:block/stone_e".into())));
        assert!(pairs.contains(&(
            "assets/demo/optifine/ctm/tile.png".into(),
            "assets/demo/optifine/ctm/tile_e.png".into()
        )));
        sprites.save_disk_cache();
        sprites.finish();
        let offset = base as usize * (sprites.size * sprites.size * 4) as usize;
        assert_eq!(sprites.pixels[offset..offset + 4], [10, 20, 30, 255]);
        assert!(!prepared.join("assets").exists());
        assert!(!fixture.0.join("assets").exists());
        assert_eq!(
            fs::read_dir(&prepared)?.count(),
            1,
            "asset reads did not touch prepared tables"
        );
        Ok(())
    }

    #[test]
    fn standalone_models_bake_once_and_skip_what_does_not_read() -> Result<()> {
        let fixture = Fixture(
            std::env::temp_dir().join(format!("horizon-standalone-{}", std::process::id())),
        );
        // A half shaft: one element, two faces; a child with its parent's elements.
        fixture.write(
            "lanef:block/shaft_half",
            r##"{"textures":{"side":"lanef:block/shaft"},"elements":[{"from":[6,6,0],"to":[10,10,8],"faces":{"north":{"texture":"#side"},"up":{"texture":"#side"}}}]}"##,
        );
        fixture.write(
            "lanef:block/cog",
            r#"{"parent":"lanef:block/shaft_half","textures":{"side":"lanef:block/cog"}}"#,
        );
        fixture.write("lanef:block/orphan", r#"{"parent":"lanef:block/absent"}"#);
        let mut models = StateModels::default();
        let mut sprites = Sprites::new(crate::resource_source::ResourceSource::directory(
            fixture.0.clone(),
        ));
        let names: Vec<String> = [
            "lanef:block/shaft_half",
            "lanef:block/cog",
            "lanef:block/orphan",
            "lanef:block/absent",
            "lanef:block/shaft_half",
        ]
        .map(str::to_owned)
        .into();
        let parsed = ParsedModels::load(&crate::resource_source::ResourceSource::directory(
            fixture.0.clone(),
        ));
        models.bake_standalone(
            &horizon_assets::resource_source::ResourceSource::directory(fixture.0.to_path_buf()),
            &mut sprites,
            &parsed,
            &names,
        )?;
        let shaft = models.standalone("lanef:block/shaft_half").expect("baked");
        assert_eq!(shaft.faces.len(), 2);
        let cog = models.standalone("lanef:block/cog").expect("baked");
        assert_eq!(cog.faces.len(), 2);
        assert_ne!(cog.faces[0].layer, shaft.faces[0].layer, "its own texture");
        assert!(models.standalone("lanef:block/orphan").is_none());
        assert!(models.standalone("lanef:block/absent").is_none());
        assert!(models.standalone("lanef:block/unnamed").is_none());
        // Named twice, baked once.
        assert_eq!(models.baked.len(), 2);
        // Without a parsed library, the files are read directly.
        let mut direct = StateModels::default();
        direct.bake_standalone(
            &horizon_assets::resource_source::ResourceSource::directory(fixture.0.to_path_buf()),
            &mut sprites,
            &ParsedModels::default(),
            &names,
        )?;
        assert_eq!(
            direct.standalone("lanef:block/cog").map(|m| m.faces.len()),
            Some(2)
        );
        Ok(())
    }

    #[test]
    fn a_standalone_model_a_state_bakes_unturned_is_shared() -> Result<()> {
        let mut models = StateModels::default();
        let variant = |rotations| Variant {
            model: "lanef:block/shared".into(),
            rotations,
            uv_lock: false,
        };
        let empty = || BakedModel {
            ambient_occlusion: true,
            particle: None,
            faces: Vec::new(),
            cube: None,
        };
        models.baked.push((variant([0, 90, 0]), empty()));
        models.baked.push((variant([0; 3]), empty()));
        let names = vec!["lanef:block/shared".to_owned()];
        let root = std::env::temp_dir().join("horizon-standalone-none");
        models.bake_standalone(
            &horizon_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
            &mut Sprites::default(),
            &ParsedModels::default(),
            &names,
        )?;
        assert_eq!(models.baked.len(), 2);
        assert_eq!(models.standalone.get("lanef:block/shared"), Some(&1));
        Ok(())
    }

    #[test]
    fn cull_flags_tracks_culled_uncullable_and_rotations() {
        assert_eq!(std::mem::size_of::<CullFlags>(), 1);
        let bake = |json: &str, turns: [u8; 3]| {
            let model = horizon_assets::model::CuboidModel::from_json(json).unwrap();
            let library = horizon_assets::resolve::ModelLibrary::new(BTreeMap::from([(
                "horizon:test".to_owned(),
                model,
            )]))
            .unwrap();
            bake_resolved(
                &mut Sprites::default(),
                &library.resolve("horizon:test").unwrap(),
                turns,
                false,
            )
            .unwrap()
        };

        // All six faces culled, none uncullable.
        let all_culled = serde_json::json!({
            "textures": {"all": "horizon:block/test"},
            "elements": [{
                "from": [0, 0, 0], "to": [16, 16, 16],
                "faces": {
                    "down": {"texture": "#all", "cullface": "down"},
                    "up": {"texture": "#all", "cullface": "up"},
                    "north": {"texture": "#all", "cullface": "north"},
                    "south": {"texture": "#all", "cullface": "south"},
                    "west": {"texture": "#all", "cullface": "west"},
                    "east": {"texture": "#all", "cullface": "east"}
                }
            }]
        })
        .to_string();
        let baked = bake(&all_culled, [0, 0, 0]);
        let flags = CullFlags::from_faces(&baked.faces);
        assert_eq!(flags.cull_mask(), 0b0011_1111);
        assert!(!flags.has_uncullable());

        // Subset of culled faces under rotations: flags track rotated cull directions.
        let subset_culled = serde_json::json!({
            "textures": {"all": "horizon:block/test"},
            "elements": [{
                "from": [0, 0, 0], "to": [16, 16, 16],
                "faces": {
                    "down": {"texture": "#all", "cullface": "down"},
                    "north": {"texture": "#all", "cullface": "north"},
                    "east": {"texture": "#all", "cullface": "east"}
                }
            }]
        })
        .to_string();
        let unrotated = bake(&subset_culled, [0, 0, 0]);
        let unrotated_flags = CullFlags::from_faces(&unrotated.faces);
        assert_eq!(
            unrotated_flags.cull_mask(),
            (1u8 << Direction::Down as u8)
                | (1u8 << Direction::North as u8)
                | (1u8 << Direction::East as u8)
        );
        assert!(!unrotated_flags.has_uncullable());
        for turns in [[0, 90, 0], [90, 0, 0], [90, 180, 0]] {
            let rotated = bake(&subset_culled, turns);
            let rotated_flags = CullFlags::from_faces(&rotated.faces);
            assert!(!rotated_flags.has_uncullable(), "turns={turns:?}");
            let expected_mask = rotated.faces.iter().fold(0u8, |mask, f| {
                mask | f.quad.cull.map_or(0, |d| 1u8 << (d as u8))
            });
            assert_eq!(rotated_flags.cull_mask(), expected_mask, "turns={turns:?}");
        }

        // Uncullable only.
        let uncullable_only = serde_json::json!({
            "textures": {"all": "horizon:block/test"},
            "elements": [{
                "from": [2, 2, 2], "to": [14, 14, 14],
                "faces": {"up": {"texture": "#all"}, "north": {"texture": "#all"}}
            }]
        })
        .to_string();
        let baked = bake(&uncullable_only, [0, 0, 0]);
        let uncullable_flags = CullFlags::from_faces(&baked.faces);
        assert_eq!(uncullable_flags.cull_mask(), 0);
        assert!(uncullable_flags.has_uncullable());

        // Mixed culled and uncullable faces.
        let mixed = serde_json::json!({
            "textures": {"all": "horizon:block/test"},
            "elements": [{
                "from": [0, 0, 0], "to": [16, 8, 16],
                "faces": {
                    "down": {"texture": "#all", "cullface": "down"},
                    "up": {"texture": "#all"}
                }
            }]
        })
        .to_string();
        let baked = bake(&mixed, [0, 0, 0]);
        let mixed_flags = CullFlags::from_faces(&baked.faces);
        assert_eq!(mixed_flags.cull_mask(), 1u8 << Direction::Down as u8);
        assert!(mixed_flags.has_uncullable());

        // Union combines cull masks and uncullable bit.
        let union = unrotated_flags.union(uncullable_flags);
        assert_eq!(union.cull_mask(), unrotated_flags.cull_mask());
        assert!(union.has_uncullable());

        // Empty model (no faces).
        let empty = serde_json::json!({
            "textures": {"all": "horizon:block/test"},
            "elements": []
        })
        .to_string();
        let baked = bake(&empty, [0, 0, 0]);
        assert_eq!(CullFlags::from_faces(&baked.faces), CullFlags::EMPTY);
    }

    #[test]
    fn combined_standalone_model_refreshes_cull_flags() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, value: Value| {
            let path = dir.path().join(format!("assets/merge/models/{name}.json"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, value.to_string()).unwrap();
        };
        write(
            "culled",
            serde_json::json!({
                "textures": {"all": "merge:block/a"},
                "elements": [{
                    "from": [0, 0, 0], "to": [16, 16, 16],
                    "faces": {
                        "north": {"texture": "#all", "cullface": "north"},
                        "west": {"texture": "#all", "cullface": "west"}
                    }
                }]
            }),
        );
        write(
            "uncullable",
            serde_json::json!({
                "textures": {"all": "merge:block/a"},
                "elements": [{
                    "from": [2, 2, 2], "to": [14, 14, 14],
                    "faces": {
                        "up": {"texture": "#all"},
                        "south": {"texture": "#all", "cullface": "south"}
                    }
                }]
            }),
        );
        let source = crate::resource_source::ResourceSource::directory(dir.path().to_path_buf());
        let parsed = ParsedModels::load(&source);
        let mut catalog = StateModels::default();
        // Warm cache before standalone baking to verify bake_standalone_forms invalidates it.
        assert!(catalog.cull_flags().is_empty());
        catalog.bake_standalone_forms(
            &source,
            &mut Sprites::default(),
            &parsed,
            &[crate::models::StandaloneFormRegistration {
                owner: "test",
                form: mod_api::StandaloneModelForm::Combined {
                    id: "merge:combined".into(),
                    parts: vec!["merge:culled".into(), "merge:uncullable".into()],
                },
            }],
        );
        let slot = catalog.standalone_slot("merge:combined").expect("baked");
        let flags = catalog.cull_flags()[slot as usize];
        assert_eq!(
            flags.cull_mask(),
            (1u8 << Direction::North as u8)
                | (1u8 << Direction::West as u8)
                | (1u8 << Direction::South as u8)
        );
        assert!(flags.has_uncullable());
    }

    #[test]
    fn model_slots_at_small_matches_models_at_inline_and_spilled() {
        let model_for = |name: &str, cull: Option<&str>| {
            let mut face = serde_json::json!({"texture": "#all"});
            if let Some(c) = cull {
                face["cullface"] = Value::String(c.to_owned());
            }
            let json = serde_json::json!({
                "textures": {"all": "horizon:block/test"},
                "elements": [{"from": [0, 0, 0], "to": [16, 16, 16], "faces": {"up": face}}]
            })
            .to_string();
            let model = horizon_assets::model::CuboidModel::from_json(&json).unwrap();
            let library = horizon_assets::resolve::ModelLibrary::new(BTreeMap::from([(
                name.to_owned(),
                model,
            )]))
            .unwrap();
            let baked = bake_resolved(
                &mut Sprites::default(),
                &library.resolve(name).unwrap(),
                [0; 3],
                false,
            )
            .unwrap();
            (
                Variant {
                    model: name.to_owned(),
                    rotations: [0; 3],
                    uv_lock: false,
                },
                baked,
            )
        };
        let baked = vec![
            model_for("horizon:a", Some("up")),
            model_for("horizon:b", Some("north")),
            model_for("horizon:c", None),
        ];
        let map_index = |v: &Variant| match v.model.as_str() {
            "horizon:a" => 0u32,
            "horizon:b" => 1u32,
            _ => 2u32,
        };
        let single_def =
            BlockStateDefinition::from_json(r#"{"variants":{"":{"model":"horizon:a"}}}"#).unwrap();
        let weighted_def = BlockStateDefinition::from_json(
            r#"{"variants":{"":[{"model":"horizon:a","weight":1},{"model":"horizon:b","weight":3}]}}"#,
        )
        .unwrap();
        let multi_def = BlockStateDefinition::from_json(
            r#"{"multipart":[{"apply":{"model":"horizon:a"}},{"apply":{"model":"horizon:b"}},{"apply":{"model":"horizon:c"}}]}"#,
        )
        .unwrap();
        let catalog = StateModels {
            choices: vec![
                CompiledChoices::new(&single_def).unwrap().map(map_index),
                CompiledChoices::new(&weighted_def).unwrap().map(map_index),
                CompiledChoices::new(&multi_def).unwrap().map(map_index),
            ],
            bounds: OnceLock::new(),
            cull_flags: OnceLock::new(),
            states: vec![
                Some((0, SelectedModels::Variant(0))),
                Some((1, SelectedModels::Variant(0))),
                Some((2, SelectedModels::Multipart(vec![0, 1, 2]))),
            ],
            single_models: vec![Some(0), None, None],
            baked,
            standalone: BTreeMap::new(),
        };

        let first_flags = catalog.cull_flags();
        let second_flags = catalog.cull_flags();
        assert!(std::ptr::eq(first_flags.as_ptr(), second_flags.as_ptr()));
        assert_eq!(first_flags[0].cull_mask(), 1u8 << Direction::Up as u8);
        assert!(!first_flags[0].has_uncullable());
        assert_eq!(first_flags[1].cull_mask(), 1u8 << Direction::North as u8);
        assert!(!first_flags[1].has_uncullable());
        assert_eq!(first_flags[2].cull_mask(), 0);
        assert!(first_flags[2].has_uncullable());

        let mut vec_buf = Vec::new();
        let mut inline_buf: smallvec::SmallVec<[u32; 8]> = smallvec::SmallVec::new();
        let mut spill_buf: smallvec::SmallVec<[u32; 2]> = smallvec::SmallVec::new();

        for i in 0..32 {
            let pos = BlockPos::new(i * 7 - 5, i % 16, -i * 11);
            for state_id in [0u32, 1, 2] {
                let id = BlockStateId(state_id);
                catalog.models_at(id, pos, &mut vec_buf).unwrap();
                catalog
                    .model_slots_at_small(id, pos, &mut inline_buf)
                    .unwrap();
                catalog
                    .model_slots_at_small(id, pos, &mut spill_buf)
                    .unwrap();

                assert!(
                    !inline_buf.spilled(),
                    "8-slot buffer must stay on the stack"
                );
                if state_id == 2 {
                    assert!(
                        spill_buf.spilled(),
                        "2-slot buffer must spill for 3-part multipart"
                    );
                }
                let expected: Vec<*const BakedModel> =
                    vec_buf.iter().map(|&m| m as *const BakedModel).collect();
                let got_inline: Vec<*const BakedModel> = inline_buf
                    .iter()
                    .map(|&slot| catalog.model_by_slot(slot).unwrap() as *const BakedModel)
                    .collect();
                let got_spill: Vec<*const BakedModel> = spill_buf
                    .iter()
                    .map(|&slot| catalog.model_by_slot(slot).unwrap() as *const BakedModel)
                    .collect();
                assert_eq!(got_inline, expected);
                assert_eq!(got_spill, expected);
            }
        }
    }

    #[test]
    fn models_mut_invalidates_cull_flags_for_face_cull_edits() {
        let json = serde_json::json!({
            "textures": {"all": "horizon:block/test"},
            "elements": [{
                "from": [0, 0, 0], "to": [16, 16, 16],
                "faces": {"north": {"texture": "#all", "cullface": "north"}}
            }]
        })
        .to_string();
        let model = horizon_assets::model::CuboidModel::from_json(&json).unwrap();
        let library = horizon_assets::resolve::ModelLibrary::new(BTreeMap::from([(
            "horizon:panel".to_owned(),
            model,
        )]))
        .unwrap();
        let baked = bake_resolved(
            &mut Sprites::default(),
            &library.resolve("horizon:panel").unwrap(),
            [0; 3],
            false,
        )
        .unwrap();
        let mut catalog = StateModels {
            baked: vec![(
                Variant {
                    model: "horizon:panel".to_owned(),
                    rotations: [0; 3],
                    uv_lock: false,
                },
                baked,
            )],
            ..StateModels::default()
        };

        // Warm cache: single north-culled face.
        let initial = catalog.cull_flags();
        assert_eq!(initial[0].cull_mask(), 1u8 << Direction::North as u8);
        assert!(!initial[0].has_uncullable());
        assert!(std::ptr::eq(
            initial.as_ptr(),
            catalog.cull_flags().as_ptr()
        ));

        // Mutate the yielded `&mut BakedModel` even after the iterator itself has been dropped.
        let (_, model) = catalog.models_mut().next().unwrap();
        model.faces[0].quad.cull = None;
        let uncullable = catalog.cull_flags()[0];
        assert_eq!(uncullable.cull_mask(), 0);
        assert!(uncullable.has_uncullable());

        // Change cull direction from uncullable to south-culled.
        for (_, model) in catalog.models_mut() {
            model.faces[0].quad.cull = Some(Direction::South);
        }
        let south_culled = catalog.cull_flags()[0];
        assert_eq!(south_culled.cull_mask(), 1u8 << Direction::South as u8);
        assert!(!south_culled.has_uncullable());

        // Removing all faces resets flags to empty.
        for (_, model) in catalog.models_mut() {
            model.faces.clear();
        }
        assert_eq!(catalog.cull_flags()[0], CullFlags::EMPTY);
    }
}

#[cfg(test)]
#[path = "catalog/bounds_tests.rs"]
mod bounds_tests;

#[cfg(test)]
#[path = "catalog/standalone_tests.rs"]
mod standalone_tests;
