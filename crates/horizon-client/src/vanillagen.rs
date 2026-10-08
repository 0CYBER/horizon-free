//! Binds `horizon_world`'s vanilla overworld generator (noise terrain,
//! aquifers, surface rules, carvers and placed features) to chunk streaming.
use crate::{
    biome::Biomes,
    chunk::{Chunk, HEIGHT, MIN_Y},
    streaming::{ChunkGenerator, Proto},
};
use horizon_core::position::ChunkPos;
use horizon_world::{
    registry::BlockRegistry,
    worldgen::generator::{GeneratorData, GeneratorWorker, OverworldGenerator, ProtoChunk},
};
use std::{path::Path, sync::Arc};

pub struct VanillaTerrain {
    generator: OverworldGenerator,
    /// Generator biome index -> client biome table index.
    biome_map: Vec<u16>,
}
/// A proto chunk, or the position of one whose generation failed.
enum VanillaProto {
    Ready(Box<ProtoChunk>),
    Failed(i32, i32),
}
impl VanillaTerrain {
    pub fn new(dir: &Path, registry: &'static BlockRegistry, biomes: &Biomes, seed: i64) -> crate::Result<Self> {
        Self::for_dimension(dir, registry, biomes, seed, "minecraft:overworld")
    }
    pub fn for_dimension(dir: &Path, registry: &'static BlockRegistry, biomes: &Biomes, seed: i64, dimension: &str) -> crate::Result<Self> {
        let data: Arc<GeneratorData> = GeneratorData::load(dir, registry)?;
        let biome_map = data.biome_names().iter().map(|name| biomes.id(name)).collect();
        let generator = OverworldGenerator::for_dimension(data, seed, "minecraft:normal", dimension)?;
        let unsupported = generator.unsupported_features();
        if !unsupported.is_empty() {
            eprintln!(
                "Horizon: {} placed features cannot run and will be skipped",
                unsupported.len()
            );
            if !dir.join("blocks/block-properties.nbt").exists() {
                eprintln!(
                    "Horizon: the worldgen bundle lacks the block behavior exports (worldgen/blocks/*.nbt); \
                     run python3 tools/prepare_native_client.py to add them"
                );
            }
        }
        Ok(Self { generator, biome_map })
    }
}
impl ChunkGenerator for VanillaTerrain {
    type Worker = GeneratorWorker;
    fn worker(&self) -> GeneratorWorker {
        self.generator.worker()
    }
    fn proto(&self, worker: &mut GeneratorWorker, x: i32, z: i32) -> Proto {
        match worker.generate_proto(ChunkPos::new(x, z)) {
            Ok(chunk) => Box::new(VanillaProto::Ready(Box::new(chunk))),
            Err(e) => {
                eprintln!("Horizon: generating chunk {x} {z} failed: {e}");
                Box::new(VanillaProto::Failed(x, z))
            }
        }
    }
    fn decorates(&self) -> bool {
        true
    }
    fn decorate(&self, worker: &mut GeneratorWorker, region: &mut [Proto; 9]) {
        let taken: Vec<Box<VanillaProto>> = region
            .iter_mut()
            .map(|p| {
                std::mem::replace(p, Box::new(()))
                    .downcast::<VanillaProto>()
                    .expect("vanilla proto")
            })
            .collect();
        let all_ready = taken.iter().all(|p| matches!(**p, VanillaProto::Ready(_)));
        if all_ready {
            let chunks: Vec<ProtoChunk> = taken
                .into_iter()
                .map(|p| match *p {
                    VanillaProto::Ready(c) => *c,
                    VanillaProto::Failed(..) => unreachable!(),
                })
                .collect();
            let mut array: [ProtoChunk; 9] = match chunks.try_into() {
                Ok(a) => a,
                Err(_) => unreachable!(),
            };
            if let Err(e) = worker.decorate(&mut array) {
                eprintln!("Horizon: decorating chunk failed: {e}");
            }
            for (slot, chunk) in region.iter_mut().zip(array) {
                *slot = Box::new(VanillaProto::Ready(Box::new(chunk)));
            }
        } else {
            for (slot, p) in region.iter_mut().zip(taken) {
                *slot = p;
            }
        }
    }
    fn finish(&self, proto: Proto) -> Chunk {
        let proto = proto.downcast::<VanillaProto>().expect("vanilla proto");
        let chunk = match *proto {
            VanillaProto::Ready(c) => c,
            VanillaProto::Failed(x, z) => return Chunk::empty(x, z),
        };
        let pos = chunk.position();
        let mut out = Chunk::empty(pos.x, pos.z);
        let (bx, bz) = (pos.x * 16, pos.z * 16);
        // Only the generated range (the Nether is 0..256) is copied.
        let (lo, hi) = (chunk.terrain.min_y().max(MIN_Y), (chunk.terrain.min_y() + chunk.terrain.height()).min(MIN_Y + HEIGHT));
        for y in lo..hi {
            for z in 0..16 {
                for x in 0..16 {
                    let id = chunk.block(bx + x, y, bz + z).0 as u16;
                    if id != 0 {
                        out.set(x, y, z, id);
                    }
                }
            }
        }
        let mut biomes = vec![0u16; 16 * (HEIGHT as usize / 4)];
        for qy in 0..HEIGHT / 4 {
            for qz in 0..4 {
                for qx in 0..4 {
                    let y = (MIN_Y + qy * 4).clamp(lo, hi - 1);
                    let id = chunk.biome_id_at(bx + qx * 4, y, bz + qz * 4);
                    let index = id.0 as usize;
                    biomes[(qy * 16 + qz * 4 + qx) as usize] = self.biome_map.get(index).copied().unwrap_or(0);
                }
            }
        }
        out.biomes = Arc::new(biomes);
        for s in &mut out.sections {
            Arc::make_mut(s).compact();
        }
        out.fresh = true;
        out
    }
}
