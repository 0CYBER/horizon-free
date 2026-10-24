use crate::support::decoration_storage as storage;
use horizon_core::{
    biome::obfuscate_seed,
    position::{BlockPos, ChunkPos},
};
use horizon_world::{
    heightmap::HeightmapType,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        anchor::WorldGenerationContext,
        biome_chunk::{BiomeChunk, BiomeRegion},
        biome_predicate::BiomePredicateBindings,
        block_feature::FeatureBlockBindings,
        decoration::{DecorationBiome, DecorationWorld, FeatureDecorationPlan},
        feature::{FeatureBlocks, FeatureWorld},
        feature_graph::FeatureDefinition,
        feature_region::TerrainRegion,
        feature_sort::FeatureIdentity,
        placement::{LayerBlock, PlacementContext, PlacementModifier, PlacementQueryError},
        terrain::{PreLightChunkStage, TerrainChunk},
    },
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
};
use storage::{Properties, State};
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
struct World<'a, W> {
    out: &'a mut W,
    f: &'a Value,
    region: TerrainRegion<'a, Properties<'a>>,
    biomes: BiomeRegion<DecorationBiome>,
    settings: Vec<Vec<Vec<FeatureIdentity>>>,
    writes: i32,
}
impl<W: Write> DecorationWorld for World<'_, W> {
    type Error = ();
    fn min_section_y(&mut self) -> Result<i32, ()> {
        Ok(n(&self.f["min"]) >> 4)
    }
    fn chunk_biomes(&mut self, p: ChunkPos) -> Result<Vec<DecorationBiome>, ()> {
        writeln!(self.out, "chunk {} {}", p.x, p.z).map_err(|_| ())?;
        self.biomes.present_in_chunk(p).map_err(|_| ())
    }
    fn set_currently_generating(&mut self, feature: Option<FeatureIdentity>) -> Result<(), ()> {
        match feature {
            Some(id) => writeln!(
                self.out,
                "generating ResourceKey[minecraft:worldgen/placed_feature / horizon:f{}]",
                id.0
            ),
            None => writeln!(self.out, "generating none"),
        }
        .map_err(|_| ())
    }
}
impl<W: Write> FeatureWorld for World<'_, W> {
    type Error = ();
    fn block(&mut self, p: [i32; 3]) -> Result<BlockStateId, ()> {
        self.region.block(p).map_err(|_| ())
    }
    fn set_block(&mut self, p: [i32; 3], state: BlockStateId, flags: i32) -> Result<bool, ()> {
        writeln!(
            self.out,
            "write {} {} {} {} {flags}",
            p[0], p[1], p[2], state.0
        )
        .map_err(|_| ())?;
        let call = self.writes;
        self.writes += 1;
        if call == n(&self.f["write_fail"]) {
            return Err(());
        }
        let reject = n(&self.f["reject"]);
        let result = reject == 0 || call % reject != 0;
        if result {
            self.region.set_block_worldgen(p, state).map_err(|_| ())?;
        }
        writeln!(self.out, "written {result}").map_err(|_| ())?;
        Ok(result)
    }
}
impl<W: Write> PlacementContext for World<'_, W> {
    fn biome_identity(&mut self, p: [i32; 3]) -> Result<DecorationBiome, PlacementQueryError> {
        let id = *self
            .region
            .biome_at(&self.biomes, BlockPos::new(p[0], p[1], p[2]))
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        writeln!(self.out, "biome {} {} {} {}", p[0], p[1], p[2], id.0)
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        Ok(id)
    }
    fn generation(&self) -> WorldGenerationContext {
        self.region.generation()
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.min_y()
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.max_y()
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32, PlacementQueryError> {
        self.region.height(kind, x, z)
    }
    fn layer_block(&mut self, p: [i32; 3]) -> Result<LayerBlock, PlacementQueryError> {
        self.region.layer_block(p)
    }
    fn biome_allows_feature(
        &mut self,
        p: [i32; 3],
        feature: FeatureIdentity,
    ) -> Result<bool, PlacementQueryError> {
        let id = self.biome_identity(p)?;
        Ok(self
            .settings
            .get(id.0 as usize)
            .is_some_and(|b| b.iter().flatten().any(|v| *v == feature)))
    }
}
fn fixture(
    f: &Value,
    registry: &BlockRegistry,
    states: &[State],
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = f["biomes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            b.as_array()
                .unwrap()
                .iter()
                .map(|s| {
                    s.as_array()
                        .unwrap()
                        .iter()
                        .map(|id| FeatureIdentity(n(id) as u32))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let plan = FeatureDecorationPlan::new(
        settings
            .iter()
            .enumerate()
            .map(|(i, b)| (DecorationBiome(i as u32), b.clone()))
            .collect(),
    )?;
    writeln!(out, "plan ok")?;
    let names = (0..=settings.len())
        .map(|i| (format!("horizon:b{i}"), DecorationBiome(i as u32)))
        .collect::<BTreeMap<_, _>>();
    let biomes = Arc::new(BiomePredicateBindings::new(&names, &BTreeMap::new())?);
    let bindings = FeatureBlockBindings::new(registry, BTreeSet::new())?.with_biomes(biomes);
    let features = f["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| FeatureDefinition::from_json_with_blocks(&v.to_string(), &bindings))
        .collect::<Result<Vec<_>, _>>()?;
    let placements = f["placements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            p.as_array()
                .unwrap()
                .iter()
                .map(|m| PlacementModifier::from_json_with_blocks(&m.to_string(), &bindings))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let blocks = FeatureBlocks::from_registry(registry)?;
    let min = n(&f["min"]);
    let height = n(&f["height"]);
    let cp = ChunkPos::new(n(&f["cx"]), n(&f["cz"]));
    let stage = PreLightChunkStage::ALL
        .into_iter()
        .find(|s| s.name() == f["stage"].as_str().unwrap())
        .unwrap();
    let mut chunks = Vec::new();
    let mut biome_chunks = Vec::new();
    for dz in -1..=1 {
        for dx in -1..=1 {
            let p = ChunkPos::new(cp.x + dx, cp.z + dz);
            let mut chunk = TerrainChunk::new(p, min, height, registry, Properties(states))?;
            chunk.set_generation_stage(stage);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set_block_worldgen(
                        (p.x << 4) + x,
                        -56,
                        (p.z << 4) + z,
                        registry.default_state("minecraft:stone").unwrap(),
                    )?;
                    if (x + z) % 3 != 0 {
                        chunk.set_block_worldgen(
                            (p.x << 4) + x,
                            -48,
                            (p.z << 4) + z,
                            registry.default_state("minecraft:dirt").unwrap(),
                        )?;
                    }
                }
            }
            chunks.push(chunk);
            let values = f["palettes"][((dz + 1) * 3 + dx + 1) as usize]
                .as_array()
                .unwrap();
            let mut biomes =
                BiomeChunk::new(p, min, height, DecorationBiome(settings.len() as u32))?;
            biomes.fill(min, min + height - 1, |[x, y, z]| {
                let index = ((y - (min >> 2)) * 16 + (z & 3) * 4 + (x & 3))
                    .rem_euclid(values.len() as i32) as usize;
                Ok::<_, std::convert::Infallible>(DecorationBiome(n(&values[index]) as u32))
            })?;
            biome_chunks.push(biomes);
        }
    }
    let mut world = World {
        out,
        f,
        region: TerrainRegion::new(chunks, WorldGenerationContext::new(-64, 384, 63))?,
        biomes: BiomeRegion::new(biome_chunks, obfuscate_seed(f["seed"].as_i64().unwrap()))?,
        settings,
        writes: 0,
    };
    for _ in 0..2 {
        world.writes = 0;
        let result = plan.decorate_features(
            &mut world,
            cp,
            f["seed"].as_i64().unwrap(),
            |w, placer, rng, id, origin| {
                placer
                    .place_with_biome_check(
                        &placements[id.0 as usize],
                        w,
                        rng,
                        origin,
                        id,
                        |w, r, p| features[id.0 as usize].place(w, r, p, blocks),
                    )
                    .map_err(|_| ())
            },
        );
        writeln!(
            world.out,
            "decorate {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        for [x, z] in [[0, 0], [15, 15], [16, 16], [-1, -1]] {
            for kind in HeightmapType::ALL {
                writeln!(
                    world.out,
                    "height {} {x} {z} {}",
                    kind.name(),
                    world.region.height(kind, x, z)?
                )?;
            }
        }
    }
    for chunk in world.region.chunks() {
        let p = chunk.position();
        writeln!(world.out, "stored {} {}", p.x, p.z)?;
        for kind in HeightmapType::ALL {
            if let Some(map) = chunk.heightmap(kind) {
                writeln!(world.out, "map {}", kind.name())?;
                for word in map.raw_data() {
                    writeln!(world.out, "{word:x}")?;
                }
            } else {
                writeln!(world.out, "missing {}", kind.name())?;
            }
        }
        for i in 0..height as usize / 16 {
            writeln!(world.out, "section {i}")?;
            for index in 0..4096 {
                writeln!(world.out, "{}", chunk.section(i).unwrap().get(index)?.0)?;
            }
        }
    }
    Ok(())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let (registry, states) = storage::load(dir)?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        fixture(&serde_json::from_str(line)?, &registry, &states, &mut out)?;
    }
    Ok(())
}
