//! A chunk's way to disk and back: the snapshot with its block entities, the chunk's NBT, its
//! bytes and their compression, then the reverse. No file is written or read.
use super::{super::Game, Recorder, world::FLOOR};
use horizon_client::{Result, save};
use horizon_core::{
    nbt::{self, Limits},
    random::{LegacyRandom, RandomSource},
};
use horizon_world::compression::Compression;

const RADIUS: i32 = 2;
const ROUNDS: u64 = 4;

impl Game<'_> {
    pub(super) fn hot_chunks(&mut self, recorder: &mut Recorder) -> Result<()> {
        if !recorder.wants("chunk.") {
            return Ok(());
        }
        // Terrain with something to encode: layers under the floor, seeded ores in them, and the
        // hopper rows as block entities with items.
        self.hot_level(7, RADIUS)?;
        let mut places = LegacyRandom::new(7);
        for cx in -RADIUS..=RADIUS {
            for cz in -RADIUS..=RADIUS {
                let (x, z) = (cx * 16, cz * 16);
                for (low, high, block) in
                    [(0, 39, "deepslate"), (40, 59, "stone"), (60, 62, "dirt")]
                {
                    self.hot_command(&format!(
                        "fill {x} {low} {z} {} {high} {} minecraft:{block}",
                        x + 15,
                        z + 15
                    ))?;
                }
                for _ in 0..96 {
                    let ore = ["coal_ore", "iron_ore", "copper_ore", "gravel"]
                        [places.next_int_bound(4)? as usize];
                    // A place drawn twice with the same ore changes nothing, which the command
                    // reports as a failure.
                    self.run_command_quietly(&format!(
                        "setblock {} {} {} minecraft:{ore}",
                        x + places.next_int_bound(16)?,
                        places.next_int_bound(FLOOR)?,
                        z + places.next_int_bound(16)?
                    ));
                }
            }
        }
        self.hot_hoppers(128, RADIUS)?;
        let registry = self.pack.assets.registry;
        let mut keys: Vec<(i32, i32)> = (-RADIUS..=RADIUS)
            .flat_map(|x| (-RADIUS..=RADIUS).map(move |z| (x, z)))
            .collect();
        keys.sort_unstable();
        let chunks: Vec<_> = keys
            .iter()
            .filter_map(|key| self.world.chunks.get(key).cloned())
            .collect();
        let operations = ROUNDS * chunks.len() as u64;
        let mut saved = Vec::new();
        let mut failed = Ok(());
        recorder.measure("chunk.save", operations, || {
            // The checksum is the size before compression: a chunk's block entities are written
            // in the order of a map seeded per process, which the compressed size follows.
            let mut written = 0;
            for _ in 0..ROUNDS {
                saved.clear();
                for chunk in self.with_entities(&chunks) {
                    let root = save::encode(&chunk, registry, world_biomes!(self));
                    let stored = nbt::write_named(&root, Limits::default())
                        .map_err(|error| error.to_string())
                        .and_then(|bytes| {
                            written += bytes.len();
                            Compression::Zlib
                                .encode(&bytes)
                                .map_err(|error| error.to_string())
                        });
                    match stored {
                        Ok(bytes) => saved.push(bytes),
                        Err(error) => failed = Err(error),
                    }
                }
            }
            written
        })?;
        failed?;
        let mut failed = Ok(());
        recorder.measure("chunk.load", operations, || {
            let mut blocks = 0u64;
            for _ in 0..ROUNDS {
                for bytes in &saved {
                    let loaded = Compression::Zlib
                        .decode(bytes, 64 << 20)
                        .map_err(|error| error.to_string())
                        .and_then(|bytes| {
                            nbt::read_named(&bytes, Limits::default())
                                .map_err(|error| error.to_string())
                        })
                        .and_then(|(root, _)| {
                            save::decode(&root, registry, world_biomes!(self))
                                .ok_or_else(|| "a saved chunk did not load".to_owned())
                        });
                    match loaded {
                        Ok(chunk) => {
                            blocks += u64::from(chunk.get(8, FLOOR, 8))
                                + chunk.block_entities.len() as u64;
                        }
                        Err(error) => failed = Err(error),
                    }
                }
            }
            blocks
        })?;
        failed?;
        Ok(())
    }
}
