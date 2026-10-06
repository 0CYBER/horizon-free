//! Bind prepared registry states to model choices; bake each transformed model
//! once and share its geometry across positions. Resource failure recovery and
//! resource-pack reloads remain separate work.
use crate::{Result, sprites::Sprites};
use horizon_assets::{
    bake::{BlockModelTransform, ModelQuad, bake_model_transformed},
    blockstate::{
        BlockStateDefinition, Variant, VariantSet,
        bind::{Properties, PropertySchema, PropertyValues, SelectedModels},
        choice::CompiledChoices,
    },
    model::CuboidModel,
    resolve::ModelLibrary,
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
    path::Path,
};

pub struct BakedFace {
    pub quad: ModelQuad,
    pub layer: u32,
}
type VariantKey = (String, [u8; 3], bool);
pub struct StateModels {
    choices: Vec<CompiledChoices>,
    states: BTreeMap<BlockStateId, (usize, SelectedModels)>,
    baked: BTreeMap<VariantKey, Vec<BakedFace>>,
}
fn key(variant: &Variant) -> VariantKey {
    (variant.model.clone(), variant.rotations, variant.uv_lock)
}
impl StateModels {
    pub fn load(
        root: &Path,
        registry: &BlockRegistry,
        blocks: &[&str],
        sprites: &mut Sprites,
    ) -> Result<Self> {
        let facts: BTreeMap<String, Value> =
            serde_json::from_reader(fs::File::open(root.join("model-states.json"))?)?;
        let mut choices = Vec::new();
        let mut states = BTreeMap::new();
        let mut variants = BTreeSet::new();
        for block in blocks.iter().copied().collect::<BTreeSet<_>>() {
            let name = format!("minecraft:{block}");
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
            let resource = fs::read_to_string(
                root.join(format!("assets/minecraft/blockstates/{block}.json")),
            )?;
            let definition = BlockStateDefinition::from_json(&resource)?;
            let bound = definition.bind(&schema, &possible)?;
            let native_order: Vec<_> = (0..registry.len())
                .filter_map(|i| {
                    let id = BlockStateId(i as u32);
                    let state = registry.state(id)?;
                    (state.name == name).then_some((id, &state.properties))
                })
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
            for ((id, _), selected) in native_order.into_iter().zip(bound) {
                if let Some(selected) = selected {
                    states.insert(id, (choices.len(), selected));
                }
            }
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
        let mut models = BTreeMap::new();
        let mut pending: Vec<_> = variants.iter().map(|(name, _, _)| name.clone()).collect();
        while let Some(name) = pending.pop() {
            if models.contains_key(&name) {
                continue;
            }
            let (namespace, path) = name.split_once(':').ok_or("invalid model identifier")?;
            let text =
                fs::read_to_string(root.join(format!("assets/{namespace}/models/{path}.json")))?;
            let model = CuboidModel::from_json(&text)?;
            if let Some(parent) = &model.parent {
                pending.push(parent.clone());
            }
            models.insert(name, model);
        }
        let library = ModelLibrary::new(models)?;
        let mut baked = BTreeMap::new();
        for (name, turns, lock) in variants {
            let model = library.resolve(&name)?;
            let faces = bake_model_transformed(&model, BlockModelTransform::new(turns, lock))?.into_iter().map(|quad| {
                if quad.material.force_translucent || quad.tint_index != -1 {
                    return Err("inspection gallery requires untinted opaque or binary-cutout materials".into());
                }
                let layer = sprites.insert(root, &quad.material.sprite)?;
                Ok(BakedFace { quad, layer })
            }).collect::<Result<_>>()?;
            baked.insert((name, turns, lock), faces);
        }
        Ok(Self {
            choices,
            states,
            baked,
        })
    }
    pub fn collect(
        &self,
        state: BlockStateId,
        position: BlockPos,
    ) -> Result<Vec<(&Variant, &[BakedFace])>> {
        let (choice, selected) = self
            .states
            .get(&state)
            .ok_or("unbound gallery block state")?;
        // These inspection blocks use Block's default position seed. Blocks with
        // specialized model seeds (e.g. two-block structures) need their callbacks.
        let mut random = LegacyRandom::new(position_seed(position));
        let mut variants = Vec::new();
        self.choices[*choice].collect(selected, &mut random, &mut variants)?;
        variants
            .into_iter()
            .map(|v| {
                Ok((
                    v,
                    self.baked
                        .get(&key(v))
                        .ok_or("unbaked block model")?
                        .as_slice(),
                ))
            })
            .collect()
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
