//! Biome-selected feature scheduling. Structure decoration is a separate stage.
use super::{
    feature_sort::{FeatureIdentity, FeatureSortError, StepFeatureData, build_features_per_step},
    placement::FeaturePlacer,
};
use horizon_core::{
    fast_hash::FxHashMap,
    position::ChunkPos,
    random::{WorldgenRandom, XoroshiroRandom},
};
use std::collections::BTreeSet;

pub use super::biome::BiomeId as DecorationBiome;

/// Transport for resident section palettes and generation progress.
pub trait DecorationWorld {
    type Error;
    fn min_section_y(&mut self) -> Result<i32, Self::Error>;
    /// Return values present in every section's biome palette, including values
    /// outside the generator's possible-biome set. Duplicates are allowed.
    fn chunk_biomes(&mut self, chunk: ChunkPos) -> Result<Vec<DecorationBiome>, Self::Error>;
    fn set_currently_generating(
        &mut self,
        feature: Option<FeatureIdentity>,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug)]
pub enum DecorationError<E> {
    World(E),
    Feature { feature: FeatureIdentity, error: E },
    WorkLimit,
}
impl<E: std::fmt::Display> std::fmt::Display for DecorationError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Feature { feature, error } => write!(f, "feature {}: {error}", feature.0),
            Self::WorkLimit => f.write_str("decoration work limit exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for DecorationError<E> {}

#[derive(Clone, Debug)]
pub struct FeatureDecorationPlan {
    /// Step-local feature indices for each biome and step, pre-resolved at
    /// plan construction so per-chunk decoration needs no `BTreeMap` lookups.
    biome_steps: FxHashMap<DecorationBiome, Vec<Vec<usize>>>,
    steps: Vec<StepFeatureData>,
}
impl FeatureDecorationPlan {
    /// The list must preserve the generator's possible-biome encounter order.
    /// Each biome identity occurs once; feature identity is allocation identity.
    pub fn new(
        biomes: Vec<(DecorationBiome, Vec<Vec<FeatureIdentity>>)>,
    ) -> Result<Self, FeatureSortError> {
        let ordered = biomes
            .iter()
            .map(|(_, steps)| steps.clone())
            .collect::<Vec<_>>();
        let steps = build_features_per_step(&ordered)?;
        let count = biomes.len();
        let mut biome_steps = FxHashMap::default();
        biome_steps.reserve(count);
        for (biome, per_step) in biomes {
            let resolved: Vec<Vec<usize>> = per_step
                .into_iter()
                .enumerate()
                .map(|(step, features)| {
                    let data = &steps[step];
                    features
                        .into_iter()
                        .map(|feature| {
                            data.index(feature)
                                .expect("plan owns complete biome feature lists")
                        })
                        .collect()
                })
                .collect();
            if biome_steps.insert(biome, resolved).is_some() {
                return Err(FeatureSortError::DuplicateBiome);
            }
        }
        Ok(Self { biome_steps, steps })
    }

    /// The step and the step-local index vanilla mixes into `set_feature_seed`.
    pub(super) fn seed(&self, feature: FeatureIdentity) -> Option<(usize, usize)> {
        self.steps
            .iter()
            .enumerate()
            .find_map(|(step, data)| data.index(feature).map(|index| (index, step)))
    }

    /// Execute the feature branch of chunk decoration with structures disabled.
    /// The callback must place the identified root with a biome-check context.
    /// It receives one shared placer and the correctly reseeded random stream;
    /// false results are ignored. Errors stop immediately without clearing the
    /// current-feature marker. A subsequent call starts with fresh placer state.
    pub fn decorate_features<W: DecorationWorld>(
        &self,
        world: &mut W,
        chunk: ChunkPos,
        world_seed: i64,
        place: impl FnMut(
            &mut W,
            &mut FeaturePlacer,
            &mut WorldgenRandom<XoroshiroRandom>,
            FeatureIdentity,
            [i32; 3],
        ) -> Result<bool, W::Error>,
    ) -> Result<(), DecorationError<W::Error>> {
        self.decorate(world, chunk, world_seed, 0, |_, _, _, _| Ok(()), place)
    }

    /// Like `decorate_features`, running `structures` at the start of each of
    /// `max(structure_steps, feature steps)` steps, before that step's
    /// features (vanilla applyBiomeDecoration). It receives the shared random
    /// and decoration seed; it seeds per structure itself.
    pub fn decorate<W: DecorationWorld>(
        &self,
        world: &mut W,
        chunk: ChunkPos,
        world_seed: i64,
        structure_steps: usize,
        mut structures: impl FnMut(
            &mut W,
            &mut WorldgenRandom<XoroshiroRandom>,
            i64,
            usize,
        ) -> Result<(), W::Error>,
        mut place: impl FnMut(
            &mut W,
            &mut FeaturePlacer,
            &mut WorldgenRandom<XoroshiroRandom>,
            FeatureIdentity,
            [i32; 3],
        ) -> Result<bool, W::Error>,
    ) -> Result<(), DecorationError<W::Error>> {
        let origin = [
            chunk.x.wrapping_shl(4),
            world
                .min_section_y()
                .map_err(DecorationError::World)?
                .wrapping_shl(4),
            chunk.z.wrapping_shl(4),
        ];
        let mut random = WorldgenRandom::new(XoroshiroRandom::new(0));
        let seed = random.set_decoration_seed(world_seed, origin[0], origin[2]);
        // ChunkPos.rangeClosed overflows into a huge reversed traversal at the
        // extreme i32 chunk coordinates. Bound that unsupported work explicitly.
        if [chunk.x, chunk.z]
            .iter()
            .any(|v| *v == i32::MIN || *v == i32::MAX)
        {
            return Err(DecorationError::WorkLimit);
        }
        let mut present = Vec::with_capacity(8);
        let mut seen = [0u64; 128];
        let mut seen_spill = BTreeSet::new();
        for z in chunk.z - 1..=chunk.z + 1 {
            for x in chunk.x - 1..=chunk.x + 1 {
                let values = world
                    .chunk_biomes(ChunkPos::new(x, z))
                    .map_err(DecorationError::World)?;
                if values.len() > 65536 {
                    return Err(DecorationError::WorkLimit);
                }
                for id in values {
                    let word = id.0 as usize / 64;
                    if let Some(bits) = seen.get_mut(word) {
                        let mask = 1u64 << (id.0 % 64);
                        if *bits & mask != 0 {
                            continue;
                        }
                        *bits |= mask;
                    } else if !seen_spill.insert(id) {
                        continue;
                    }
                    if let Some(steps) = self.biome_steps.get(&id) {
                        present.push(steps);
                    }
                }
            }
        }
        let mut placer = FeaturePlacer::new();
        for step in 0..structure_steps.max(self.steps.len()) {
            structures(world, &mut random, seed, step).map_err(DecorationError::World)?;
            let Some(data) = self.steps.get(step) else {
                continue;
            };
            let features = data.features();
            if features.len() <= 512 {
                let mut selected = [0u64; 8];
                for biome_steps in &present {
                    if let Some(indices) = biome_steps.get(step) {
                        for &index in indices {
                            selected[index / 64] |= 1u64 << (index % 64);
                        }
                    }
                }
                for (word_idx, &word) in selected.iter().enumerate() {
                    let mut w = word;
                    while w != 0 {
                        let bit = w.trailing_zeros() as usize;
                        w &= w - 1;
                        let index = word_idx * 64 + bit;
                        let feature = features[index];
                        random.set_feature_seed(seed, index as i32, step as i32);
                        world
                            .set_currently_generating(Some(feature))
                            .map_err(|error| DecorationError::Feature { feature, error })?;
                        place(world, &mut placer, &mut random, feature, origin)
                            .map_err(|error| DecorationError::Feature { feature, error })?;
                    }
                }
            } else {
                let mut selected = BTreeSet::new();
                for biome_steps in &present {
                    if let Some(indices) = biome_steps.get(step) {
                        selected.extend(indices.iter().copied());
                    }
                }
                for index in selected {
                    let feature = features[index];
                    random.set_feature_seed(seed, index as i32, step as i32);
                    world
                        .set_currently_generating(Some(feature))
                        .map_err(|error| DecorationError::Feature { feature, error })?;
                    place(world, &mut placer, &mut random, feature, origin)
                        .map_err(|error| DecorationError::Feature { feature, error })?;
                }
            }
        }
        world
            .set_currently_generating(None)
            .map_err(DecorationError::World)
    }
}
