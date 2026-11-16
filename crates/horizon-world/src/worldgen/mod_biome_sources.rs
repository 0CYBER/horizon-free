//! Biome sources a mod registers (vanilla's `BiomeSource`), handed over once
//! at startup and looked up when a level stem's `biome_source` names one.
//! Like a mod's chunk generator it runs on the worldgen worker threads, so it
//! keeps no mutable state and has no `Host`.
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

/// A biome source's engine side, as the executable's adapter sees it.
pub trait ModBiomeSource: Send + Sync {
    /// `BiomeSource.possibleBiomes`: the biome ids `noise_biome` indexes, as
    /// the mod listed them once at startup.
    fn possible_biomes(&self) -> &[String];
    /// `BiomeResolver.getNoiseBiome` at quart cell `quart` (block >> 2) of a
    /// world with `seed`: an index into [`possible_biomes`](Self::possible_biomes).
    fn noise_biome(&self, quart: [i32; 3], seed: i64) -> u16;
}

/// The installed biome sources, `modid:chaos` → source. Set once, before any
/// world opens (the executable hands the mods' registrations over).
pub type ModBiomeSources = BTreeMap<String, Arc<dyn ModBiomeSource>>;

static MOD_BIOME_SOURCES: OnceLock<ModBiomeSources> = OnceLock::new();
static EMPTY: ModBiomeSources = BTreeMap::new();

/// Install the process-wide table once; a second installation keeps the first.
pub fn install_mod_biome_sources(sources: ModBiomeSources) -> Result<(), ModBiomeSources> {
    MOD_BIOME_SOURCES.set(sources)
}

/// The installed biome sources: empty until the executable installs the mods'.
pub fn mod_biome_sources() -> &'static ModBiomeSources {
    MOD_BIOME_SOURCES.get().unwrap_or(&EMPTY)
}

/// A registered biome source bound to one level: its possible biomes resolved
/// to the level's `T` (a biome id or name) once, when the level opens.
pub struct LevelBiomeSource<T> {
    source: Arc<dyn ModBiomeSource>,
    biomes: Vec<T>,
    seed: i64,
}

impl<T> LevelBiomeSource<T> {
    /// Source `id` of `sources` over a world with `seed`, each of its possible
    /// biomes resolved by `biome`. Refused, naming the source, when nobody
    /// registered `id` (None), when it lists no biome, or naming the biome too
    /// when `biome` doesn't know one of them.
    pub fn resolve(
        id: &str,
        sources: &ModBiomeSources,
        seed: i64,
        mut biome: impl FnMut(&str) -> Option<T>,
    ) -> Option<Result<Self, String>> {
        let source = sources.get(id)?.clone();
        let names = source.possible_biomes();
        if names.is_empty() {
            return Some(Err(format!("biome source {id} lists no biome")));
        }
        let biomes = names
            .iter()
            .map(|name| {
                biome(name)
                    .ok_or_else(|| format!("biome source {id} names the unknown biome {name}"))
            })
            .collect::<Result<_, _>>();
        Some(biomes.map(|biomes| Self {
            source,
            biomes,
            seed,
        }))
    }
    /// The biome at quart cell `quart`. An index past the list wraps around it.
    pub fn noise_biome(&self, quart: [i32; 3]) -> &T {
        let index = usize::from(self.source.noise_biome(quart, self.seed));
        &self.biomes[index % self.biomes.len()]
    }
    /// The possible biomes, in the order the mod listed them.
    pub fn biomes(&self) -> &[T] {
        &self.biomes
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for LevelBiomeSource<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LevelBiomeSource")
            .field("biomes", &self.biomes)
            .field("seed", &self.seed)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The biome of each quart cell is its x plus z, past the list's end too.
    struct Stripes(Vec<String>);
    impl ModBiomeSource for Stripes {
        fn possible_biomes(&self) -> &[String] {
            &self.0
        }
        fn noise_biome(&self, [x, _, z]: [i32; 3], seed: i64) -> u16 {
            (x + z + seed as i32) as u16
        }
    }

    fn sources(biomes: &[&str]) -> ModBiomeSources {
        let source = Stripes(biomes.iter().map(|b| (*b).to_owned()).collect());
        ModBiomeSources::from([("test:stripes".to_owned(), Arc::new(source) as _)])
    }

    /// Plains, desert, plains, desert along x: the seed shifts the stripes, and
    /// the indices 2 and 3 past the two biomes wrap around them.
    #[test]
    fn an_index_past_the_list_wraps_around_it() {
        let known = |name: &str| {
            ["minecraft:desert", "minecraft:plains"]
                .iter()
                .position(|b| *b == name)
        };
        let biomes = sources(&["minecraft:plains", "minecraft:desert"]);
        let level = LevelBiomeSource::resolve("test:stripes", &biomes, 1, known)
            .unwrap()
            .unwrap();
        assert_eq!(level.biomes(), [1, 0]);
        let row: Vec<usize> = (-1..3).map(|x| *level.noise_biome([x, 7, 0])).collect();
        assert_eq!(row, [1, 0, 1, 0]);
    }
}
