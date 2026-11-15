//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;
use horizon_core::level_id::valid_level_id;

impl PartRegistrar {
    /// Refuses a dimension whose generator or mod biome source no one provides.
    /// After every mod has registered, as either may come from a later mod's
    /// registration.
    pub(crate) fn refuse_unknown_generators_and_biome_sources(&mut self) {
        for dimension in &self.dimensions {
            let generator = &dimension.settings.generator;
            if generator != mod_vocab::__private::NOISE_GENERATOR
                && !self.chunk_generators.iter().any(|g| *g.id == **generator)
            {
                self.refused.push(format!(
                    "{} registered dimension {} with the unknown generator {generator:?}",
                    dimension.mod_id, dimension.id
                ));
            }
            if let Some(source) = &dimension.settings.mod_biome_source
                && !self.biome_sources.iter().any(|s| *s.id == **source)
            {
                self.refused.push(format!(
                    "{} registered dimension {} with the unknown biome source {source:?}",
                    dimension.mod_id, dimension.id
                ));
            }
        }
    }
}

impl DimensionsRegistrar for PartRegistrar {
    fn add_dimension(&mut self, name: &str, settings: crate::DimensionSettings) {
        let id = match self.owned_declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        let refusal = if !valid_level_id(&id) {
            Some(format!(
                "{} registered dimension {id:?}, whose name must be lowercase letters, digits and \
                 _ . - with / between segments, none empty, `.` or `..`: it names a folder of the world",
                self.mod_id
            ))
        } else if VANILLA_DIMENSIONS.contains(&id.as_str()) {
            Some(format!(
                "{} registered dimension {id}, which is a vanilla level",
                self.mod_id
            ))
        } else if self.dimensions.iter().any(|d| *d.id == *id) {
            Some(format!("{} registered dimension {id} twice", self.mod_id))
        } else if !(self.dimension_shape)(&settings) {
            Some(format!(
                "{} registered dimension {id} with a build height the game refuses: min_y {}, \
                 height {}, logical_height {}",
                self.mod_id, settings.min_y, settings.height, settings.logical_height
            ))
        } else if let Some(why) = settings
            .invalid_biome_source()
            .or_else(|| settings.invalid_field())
        {
            Some(format!(
                "{} registered dimension {id} with a dimension type the game refuses: {why}",
                self.mod_id
            ))
        } else {
            None
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        self.dimensions.push(DimensionInfo {
            mod_id: self.mod_id,
            id: id.into(),
            settings,
        });
    }
    fn add_chunk_generator(&mut self, name: &str, generator: Arc<dyn crate::ChunkGenerator>) {
        let id = match self.owned_declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        let refusal = if !valid_level_id(&id) {
            Some(format!(
                "{} registered chunk generator {id:?}, whose name must be lowercase letters, \
                 digits and _ . - with / between segments, none empty, `.` or `..`",
                self.mod_id
            ))
        } else if id == mod_vocab::__private::NOISE_GENERATOR {
            Some(format!(
                "{} registered chunk generator {id}, which is the game's own",
                self.mod_id
            ))
        } else {
            self.chunk_generators
                .iter()
                .any(|g| *g.id == *id)
                .then(|| format!("{} registered chunk generator {id} twice", self.mod_id))
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        self.chunk_generators.push(ChunkGeneratorInfo {
            id: id.into(),
            // ChunkGenerator.vanilla_stages, asked once.
            stages: generator.vanilla_stages(),
            generator,
        });
    }
    fn add_biome_source(&mut self, name: &str, source: Arc<dyn crate::BiomeSource>) {
        let id = match self.owned_declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        // BiomeSource.possibleBiomes, asked once.
        let biomes = source.possible_biomes();
        let refusal = if !valid_level_id(&id) {
            Some(format!(
                "{} registered biome source {id:?}, whose name must be lowercase letters, \
                 digits and _ . - with / between segments, none empty, `.` or `..`",
                self.mod_id
            ))
        } else if self.biome_sources.iter().any(|s| *s.id == *id) {
            Some(format!(
                "{} registered biome source {id} twice",
                self.mod_id
            ))
        } else if biomes.is_empty() {
            Some(format!(
                "{} registered biome source {id}, whose possible_biomes lists no biome",
                self.mod_id
            ))
        } else {
            None
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        self.biome_sources.push(BiomeSourceInfo {
            id: id.into(),
            biomes,
            source,
        });
    }
}

/// The levels every world has, which no mod's can be.
const VANILLA_DIMENSIONS: [&str; 3] = [
    "minecraft:overworld",
    "minecraft:the_nether",
    "minecraft:the_end",
];
