//! STRUCTURE_STARTS / STRUCTURE_REFERENCES for the chunk generator.
use super::{
    biome::BiomeId,
    block_feature::FeatureBlockBindings,
    generator::{GeneratorData, GeneratorError},
    structure::{
        StructureResources,
        start::{Rings, StructureRegistry, StructureStart, beardifier, create_starts},
    },
};
use horizon_core::{density::Beardifier, position::ChunkPos};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex, OnceLock},
};

pub(super) type Starts = Arc<Vec<Arc<StructureStart>>>;
pub(super) struct StructureState {
    pub(super) registry: StructureRegistry,
    pub(super) resources: Arc<StructureResources>,
    pub(super) bindings: FeatureBlockBindings<'static>,
    possible_sets: Vec<usize>,
    /// Structure indices of each decoration step, in registry order.
    pub(super) by_step: Vec<Vec<usize>>,
    starts: Mutex<HashMap<(i32, i32), Starts>>,
    /// Concentric ring positions, computed on first use.
    rings: OnceLock<Rings>,
}
const START_CACHE_LIMIT: usize = 65536;

fn biome_set(data: &GeneratorData, value: &Value) -> Result<BTreeSet<BiomeId>, super::DecodeError> {
    let names: Vec<&str> = match value {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let mut out = BTreeSet::new();
    for name in names {
        match name.strip_prefix('#') {
            Some(tag) => {
                for biome in data.biome_tags.get(tag).into_iter().flatten() {
                    out.extend(data.biome_id(biome));
                }
            }
            None => out.extend(data.biome_id(name)),
        }
    }
    Ok(out)
}
impl StructureState {
    pub(super) fn new(
        data: &Arc<GeneratorData>,
        possible_biomes: &BTreeSet<BiomeId>,
    ) -> Result<Option<Self>, GeneratorError> {
        let (Some(resources), Some(raw)) = (&data.structures, &data.structure_resources) else {
            return Ok(None);
        };
        let registry =
            StructureRegistry::new(&raw.structures, &raw.sets, raw.pools.clone(), &|value| {
                biome_set(data, value)
            })?;
        let possible_sets = registry
            .sets
            .iter()
            .enumerate()
            .filter(|(_, set)| {
                set.structures
                    .iter()
                    .any(|(s, _)| !registry.structures[*s].biomes.is_disjoint(possible_biomes))
            })
            .map(|(i, _)| i)
            .collect();
        let mut by_step = vec![Vec::new(); super::structure::start::STEPS.len()];
        for (i, def) in registry.structures.iter().enumerate() {
            by_step[def.step].push(i);
        }
        let bindings = FeatureBlockBindings::with_tags(
            data.registry(),
            data.integers().clone(),
            &data.block_tags,
        )?;
        Ok(Some(Self {
            registry,
            resources: resources.clone(),
            bindings,
            possible_sets,
            by_step,
            starts: Mutex::new(HashMap::new()),
            rings: OnceLock::new(),
        }))
    }
    /// Concentric ring positions of a structure set, once the first start
    /// lookup has computed them.
    pub(super) fn cached_rings(&self, set: &str) -> Option<Vec<ChunkPos>> {
        self.rings.get().and_then(|rings| rings.get(set).cloned())
    }
    /// The (cached) structure starts owned by a chunk.
    pub(super) fn starts_at(
        &self,
        context: &mut impl super::structure::start::StartContext,
        chunk: ChunkPos,
    ) -> Starts {
        if let Some(found) = self.starts.lock().unwrap().get(&(chunk.x, chunk.z)) {
            return found.clone();
        }
        let rings = self.rings.get_or_init(|| {
            let seed = context.seed();
            self.registry
                .ring_positions(&self.possible_sets, seed, &mut |q| context.noise_biome(q))
        });
        let starts: Starts = Arc::new(
            create_starts(
                &self.registry,
                &self.resources,
                &self.bindings,
                context,
                &self.possible_sets,
                rings,
                chunk,
            )
            .into_iter()
            .map(Arc::new)
            .collect(),
        );
        let mut cache = self.starts.lock().unwrap();
        if cache.len() >= START_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert((chunk.x, chunk.z), starts.clone());
        starts
    }
    /// ChunkGenerator.createReferences: starts within 8 chunks whose box
    /// intersects this chunk's column.
    pub(super) fn references(
        &self,
        context: &mut impl super::structure::start::StartContext,
        chunk: ChunkPos,
    ) -> Vec<Arc<StructureStart>> {
        let (bx, bz) = (chunk.x * 16, chunk.z * 16);
        let mut out = Vec::new();
        for sx in chunk.x - 8..=chunk.x + 8 {
            for sz in chunk.z - 8..=chunk.z + 8 {
                for start in self.starts_at(context, ChunkPos::new(sx, sz)).iter() {
                    let b = start.bounding_box;
                    if b.max[0] >= bx
                        && b.min[0] <= bx + 15
                        && b.max[2] >= bz
                        && b.min[2] <= bz + 15
                    {
                        out.push(start.clone());
                    }
                }
            }
        }
        out
    }
    pub(super) fn beardifier(
        &self,
        references: &[Arc<StructureStart>],
        chunk: ChunkPos,
    ) -> Option<Beardifier> {
        beardifier(&self.registry, references, chunk)
    }
}
