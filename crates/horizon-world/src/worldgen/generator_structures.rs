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
use std::collections::BTreeMap;
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
    /// Drop the cached starts, so that later chunks build fresh pieces (the
    /// pieces keep per-start placement state such as a shipwreck's height).
    pub(super) fn forget_starts(&self) {
        self.starts.lock().unwrap().clear();
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
    /// ChunkGenerator.findNearestMapStructure: the nearest start of one of
    /// `names` from block `from` (rings: every ring chunk; random spreads:
    /// sectors in growing squares up to `radius`). Returns the structure and
    /// its locate position (the start chunk's corner at y 0).
    pub(super) fn locate(
        &self,
        context: &mut impl super::structure::start::StartContext,
        names: &[String],
        from: [i32; 3],
        radius: i32,
    ) -> Option<(String, [i32; 3])> {
        let seed = context.seed();
        let rings = self
            .rings
            .get_or_init(|| {
                self.registry
                    .ring_positions(&self.possible_sets, seed, &mut |q| context.noise_biome(q))
            })
            .clone();
        let wanted = |s: usize| names.iter().any(|n| *n == self.registry.structures[s].name);
        let dist = |p: [i32; 3]| {
            let (dx, dz) = (f64::from(p[0] - from[0]), f64::from(p[2] - from[2]));
            dx * dx + dz * dz
        };
        let mut best: Option<(f64, String, [i32; 3])> = None;
        let found_at = |this: &Self, context: &mut _, chunk: ChunkPos| -> Option<String> {
            this.starts_at(context, chunk)
                .iter()
                .find(|s| wanted(s.structure))
                .map(|s| s.name.clone())
        };
        for &si in &self.possible_sets {
            let set = &self.registry.sets[si];
            if !set.structures.iter().any(|(s, _)| wanted(*s)) {
                continue;
            }
            match set.spacing() {
                None => {
                    // Rings: nearest by the chunk's (8, 32, 8) block.
                    let Some(chunks) = rings.get(&set.name) else {
                        continue;
                    };
                    let mut nearest: Option<(f64, ChunkPos)> = None;
                    for &c in chunks {
                        let d = dist([c.x * 16 + 8, 32, c.z * 16 + 8]);
                        if nearest.is_none_or(|(n, _)| d < n)
                            && self
                                .registry
                                .is_structure_chunk(set, seed, c.x, c.z, &rings)
                        {
                            nearest = Some((d, c));
                        }
                    }
                    if let Some((_, c)) = nearest
                        && let Some(name) = found_at(self, context, c)
                    {
                        let p = [c.x * 16, 0, c.z * 16];
                        let d = dist(p);
                        if best.as_ref().is_none_or(|(b, ..)| d < *b) {
                            best = Some((d, name, p));
                        }
                    }
                }
                Some(spacing) => {
                    let (cx, cz) = (from[0] >> 4, from[2] >> 4);
                    'search: for r in 0..=radius {
                        for dx in -r..=r {
                            let x_edge = dx == -r || dx == r;
                            for dz in -r..=r {
                                if !(x_edge || dz == -r || dz == r) {
                                    continue;
                                }
                                let (sx, sz) = (cx + spacing * dx, cz + spacing * dz);
                                let Some(c) = StructureRegistry::potential_chunk(set, seed, sx, sz)
                                else {
                                    continue;
                                };
                                if !self
                                    .registry
                                    .is_structure_chunk(set, seed, c.x, c.z, &rings)
                                {
                                    continue;
                                }
                                if let Some(name) = found_at(self, context, c) {
                                    let p = [c.x * 16, 0, c.z * 16];
                                    let d = dist(p);
                                    if best.as_ref().is_none_or(|(b, ..)| d < *b) {
                                        best = Some((d, name, p));
                                    }
                                    break 'search;
                                }
                            }
                        }
                    }
                }
            }
        }
        best.map(|(_, n, p)| (n, p))
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
        // Vanilla keeps each structure's references in a LongOpenHashSet and
        // places the starts in its iteration order; reorder each structure's
        // starts within the slots they already occupy.
        let mut slots: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (i, start) in out.iter().enumerate() {
            slots.entry(start.structure).or_default().push(i);
        }
        let original = out.clone();
        for slots in slots.values().filter(|s| s.len() > 1) {
            let keys: Vec<i64> = slots
                .iter()
                .map(|&i| {
                    let c = original[i].chunk;
                    i64::from(c.x as u32) | (i64::from(c.z as u32) << 32)
                })
                .collect();
            for (&slot, &from) in slots.iter().zip(&super::long_set::iteration_order(&keys)) {
                out[slot] = original[slots[from]].clone();
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
