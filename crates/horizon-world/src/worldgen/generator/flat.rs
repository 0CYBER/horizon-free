//! A superflat world (`FlatLevelSource`): its layers where the noise terrain would be, moved out
//! of `generator.rs` unchanged.
use super::*;

/// `FlatLevelGeneratorSettings.getLayers` as `FlatLevelSource` reads them: the states from the
/// level's minimum Y up, with the layers `adjustGenerationSettings` replaced by `null`
/// (not opaque to `MOTION_BLOCKING`: a feature fills them later) marked.
pub(super) struct FlatLayers {
    pub(super) min_y: i32,
    pub(super) states: Vec<BlockStateId>,
    /// `layers.get(i) == null`: `getBaseHeight` skips it, `getBaseColumn` reads it as air, and
    /// `buildTerrain` leaves it for the `FillLayerFeature` that runs after the structures.
    pub(super) null: Vec<bool>,
}
/// A superflat world's generator settings (`FlatLevelGeneratorSettings`).
#[derive(Clone, Debug)]
pub struct FlatSettings {
    /// The layers, bottom up: block id and height.
    pub layers: Vec<(String, i32)>,
    /// The biome the whole world is.
    pub biome: String,
    /// `structure_overrides`: the structure sets that may generate (all of them when `None`).
    pub structure_overrides: Option<Vec<String>>,
    /// `features`: the biome's own placed features decorate the world.
    pub features: bool,
    /// `lakes`: the lava lakes (`createLakesList`) take the biome's lakes step.
    pub lakes: bool,
}
impl OverworldGenerator {
    /// A superflat overworld (`FlatLevelSource`): the layers where the terrain would be, a
    /// fixed biome, and the structure sets `structure_overrides` allows (every one when it
    /// names none) generating on the flat ground, with or without structures.
    pub fn flat(
        data: Arc<GeneratorData>,
        seed: i64,
        flat: &FlatSettings,
        structures: bool,
    ) -> Result<Self, GeneratorError> {
        let registry = data.registry;
        let (level_min_y, level_height) = overrides::LevelHeight::of_type(
            &Value::from("minecraft:overworld"),
            &data.dimension_types,
        )
        .map_or((-64, 384), |h| (h.min_y, h.height));
        // The overworld's own noise settings give the level its sea level and the rest of
        // the generator its data; no noise is ever sampled.
        let settings_json: Value = serde_json::from_str(
            data.settings
                .get("minecraft:overworld")
                .ok_or_else(|| resource("missing noise settings minecraft:overworld"))?,
        )?;
        // `Biome.CODEC.lenientOptionalFieldOf("biome")`: an unknown biome is Plains.
        let biome = if data.biome_id(&flat.biome).is_some() {
            flat.biome.as_str()
        } else {
            "minecraft:plains"
        };
        let generator = serde_json::json!({
            "type": "minecraft:noise",
            "settings": "minecraft:overworld",
            "biome_source": { "type": "minecraft:fixed", "biome": biome },
            "structure_overrides": flat.structure_overrides,
        });
        let mut pipeline = Pipeline::new(
            data.clone(),
            seed,
            XoroshiroRandom::new(seed),
            &generator,
            &settings_json,
            structures,
            Some((level_min_y, level_height)),
            None,
        )?;
        let mut states = Vec::new();
        let mut null = Vec::new();
        for (block, height) in &flat.layers {
            let id = registry
                .default_state(block)
                .ok_or_else(|| resource(format!("missing block {block}")))?;
            // `adjustGenerationSettings`: a layer that is not opaque to MOTION_BLOCKING is
            // `null` in the layers `FlatLevelSource` reads.
            let hole = !HeightmapType::MotionBlocking.is_opaque(data.states.get(id));
            for _ in 0..(*height).max(0) {
                states.push(id);
                null.push(hole);
            }
        }
        // `updateLayers`: `voidGen` when every layer is air (no layers at all, too).
        let air = registry.default_state("minecraft:air");
        let void_gen = states.iter().all(|&id| Some(id) == air);
        states.truncate(level_height.max(0) as usize);
        null.truncate(states.len());
        // `FlatLevelSource.getSeaLevel`; its minimum Y and depth are in `generation`.
        pipeline.settings.sea_level = -63;
        let top_layers: Vec<(i32, BlockStateId)> = null
            .iter()
            .zip(&states)
            .enumerate()
            .filter(|(_, (hole, _))| **hole)
            .map(|(i, (_, &state))| (level_min_y + i as i32, state))
            .collect();
        pipeline.flat = Some(FlatLayers {
            min_y: level_min_y,
            states,
            null,
        });
        // `FlatLevelSource` decorates with `adjustGenerationSettings` of the biome.
        let adjust = super::super::generator_decoration::FlatDecoration::new(
            flat.features,
            flat.lakes,
            void_gen,
            biome == "minecraft:the_void",
        );
        pipeline.decoration = super::super::generator_decoration::Decoration::new_with(
            &data,
            &pipeline.source,
            Some(adjust),
        )?
        .map(|mut decoration| {
            decoration.top_layers = top_layers;
            Arc::new(decoration)
        });
        if let (Some(state), Some(only)) = (pipeline.structures.as_mut(), &flat.structure_overrides)
        {
            state.restrict_to(only);
        }
        // `ChunkGeneratorStructureState.createForFlat`: the stronghold rings use seed 0.
        if let Some(state) = pipeline.structures.as_mut() {
            state.ring_seed = Some(0);
        }
        Ok(Self {
            pipeline: Arc::new(pipeline),
            noise_clamped: None,
        })
    }
}
