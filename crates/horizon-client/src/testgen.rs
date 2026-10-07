//! Simple rolling terrain used only until the vanilla overworld generator is
//! bound (`--test-terrain`). Not vanilla generation.
use crate::{chunk::Chunk, streaming::ChunkGenerator};
use horizon_world::registry::BlockRegistry;

pub struct TestTerrain {
    stone: u16,
    dirt: u16,
    grass: u16,
    water: u16,
    sand: u16,
    bedrock: u16,
    log: u16,
    leaves: u16,
    short_grass: u16,
    plains: u16,
}
impl TestTerrain {
    pub fn new(registry: &BlockRegistry, biomes: &crate::biome::Biomes) -> crate::Result<Self> {
        let id = |name: &str| -> crate::Result<u16> {
            Ok(registry
                .default_state(name)
                .ok_or_else(|| format!("missing {name}"))?
                .0 as u16)
        };
        Ok(Self {
            stone: id("minecraft:stone")?,
            dirt: id("minecraft:dirt")?,
            grass: id("minecraft:grass_block")?,
            water: id("minecraft:water")?,
            sand: id("minecraft:sand")?,
            bedrock: id("minecraft:bedrock")?,
            log: id("minecraft:oak_log")?,
            leaves: registry
                .resolve(&horizon_world::chunk::BlockState {
                    name: "minecraft:oak_leaves".into(),
                    properties: [
                        ("distance".to_owned(), "1".to_owned()),
                        ("persistent".to_owned(), "false".to_owned()),
                        ("waterlogged".to_owned(), "false".to_owned()),
                    ]
                    .into(),
                })?
                .0 as u16,
            short_grass: id("minecraft:short_grass")?,
            plains: biomes.id("minecraft:plains"),
        })
    }
    fn height(x: i32, z: i32) -> i32 {
        let (x, z) = (f64::from(x), f64::from(z));
        let h = 68.0
            + 10.0 * (x / 47.0).sin() * (z / 61.0).cos()
            + 5.0 * ((x + z) / 23.0).sin()
            + 3.0 * (x / 11.0).cos() * (z / 13.0).sin();
        h as i32
    }
}
fn hash(x: i32, z: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (z as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ (h >> 12)
}
impl ChunkGenerator for TestTerrain {
    type Worker = ();
    fn worker(&self) {}
    fn generate(&self, _: &mut (), cx: i32, cz: i32) -> Chunk {
        let mut chunk = Chunk::empty(cx, cz);
        chunk.biomes = std::sync::Arc::new(vec![self.plains; chunk.biomes.len()]);
        for z in 0..16 {
            for x in 0..16 {
                let (wx, wz) = (cx * 16 + x, cz * 16 + z);
                let h = Self::height(wx, wz);
                for y in -64..=h.max(62) {
                    let b = if y == -64 {
                        self.bedrock
                    } else if y > h {
                        self.water
                    } else if y < h - 3 {
                        self.stone
                    } else if h <= 63 {
                        self.sand
                    } else if y == h {
                        self.grass
                    } else {
                        self.dirt
                    };
                    chunk.set(x, y, z, b);
                }
                if h > 63 && hash(wx, wz) % 13 == 0 {
                    chunk.set(x, h + 1, z, self.short_grass);
                }
            }
        }
        // Trees fully inside the chunk.
        for z in 2..14 {
            for x in 2..14 {
                let (wx, wz) = (cx * 16 + x, cz * 16 + z);
                let h = Self::height(wx, wz);
                if h <= 64 || hash(wx, wz) % 97 != 0 {
                    continue;
                }
                for y in h + 1..h + 6 {
                    chunk.set(x, y, z, self.log);
                }
                for dy in 3..7 {
                    let r = if dy >= 5 { 1 } else { 2 };
                    for dz in -r..=r {
                        for dx in -r..=r {
                            if chunk.get(x + dx, h + dy, z + dz) == 0 {
                                chunk.set(x + dx, h + dy, z + dz, self.leaves);
                            }
                        }
                    }
                }
            }
        }
        for s in &mut chunk.sections {
            std::sync::Arc::make_mut(s).compact();
        }
        chunk
    }
}
