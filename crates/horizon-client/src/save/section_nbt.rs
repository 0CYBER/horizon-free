//! Lazy canonical section palette caching for vanilla chunk saves.
use super::{Biomes, BlockRegistry, BlockStateId, SaveTables, Section, Tag, string};
use horizon_core::paletted::SavedPaletteBudget;
use std::sync::Arc;

/// Default byte cap for retained canonical section palettes: one per world,
/// shared by its levels' `SaveDir`s ([`SaveDir::share_section_palettes`]).
pub(super) const DEFAULT_PALETTE_CACHE_BYTES: usize = 128 * 1024 * 1024;

pub(super) fn new_budget() -> Arc<SavedPaletteBudget> {
    Arc::new(SavedPaletteBudget::new(DEFAULT_PALETTE_CACHE_BYTES))
}

fn block_state_nbt(id: u32, registry: &BlockRegistry, tables: Option<&SaveTables>) -> Tag {
    match registry.state(BlockStateId(id)) {
        // BlockState.CODEC: the block name for default states.
        Some(state)
            if tables
                .is_some_and(|tables| tables.default_state.get(id as usize) == Some(&true)) =>
        {
            string(&state.name)
        }
        Some(state) => state.to_nbt(),
        None => string("minecraft:air"),
    }
}

pub(super) fn palettes(
    section: &Section,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    budget: Option<&Arc<SavedPaletteBudget>>,
) -> (Tag, Tag) {
    let blocks = section
        .blocks
        .pack_nbt(budget, |id| block_state_nbt(id, registry, tables));
    let biomes = section
        .biomes
        .pack_nbt(budget, |id| string(&biomes.saved_name(id as u16)));
    (blocks, biomes)
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::DEFAULT_PALETTE_CACHE_BYTES;
    use horizon_core::nbt::Limits;
    use horizon_world::{DATA_VERSION, region::RegionFile};
    use std::{fs, sync::OnceLock};

    fn test_registry(names: &[&str], with_props: bool) -> BlockRegistry {
        let mut states: Vec<Tag> = names.iter().map(|n| Tag::String((*n).into())).collect();
        if with_props {
            states.push(Tag::compound([
                ("id", Tag::String(names[1].into())),
                (
                    "properties",
                    Tag::compound([("axis", Tag::String("x".into()))]),
                ),
            ]));
        }
        BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("states", Tag::List(states)),
            (
                "defaults",
                Tag::compound(
                    names
                        .iter()
                        .enumerate()
                        .map(|(i, n)| (*n, Tag::Int(i as i32))),
                ),
            ),
        ]))
        .expect("build synthetic registry")
    }

    /// The registry for saves that queue an `Encoding`, which holds it as
    /// `&'static`.
    fn static_registry() -> &'static BlockRegistry {
        static REGISTRY: OnceLock<BlockRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            test_registry(
                &[
                    "minecraft:air",
                    "minecraft:stone",
                    "minecraft:dirt",
                    "minecraft:sand",
                    "minecraft:gravel",
                    "minecraft:oak_log",
                ],
                true,
            )
        })
    }

    fn test_biomes(names: &[&str]) -> Arc<Biomes> {
        let dir = tempfile::tempdir().expect("tempdir");
        let textures = dir.path().join("assets/minecraft/textures/colormap");
        let biome_dir = dir.path().join("data/minecraft/worldgen/biome");
        fs::create_dir_all(&textures).unwrap();
        fs::create_dir_all(&biome_dir).unwrap();
        for name in ["grass", "foliage", "dry_foliage"] {
            let mut encoder = png::Encoder::new(
                fs::File::create(textures.join(format!("{name}.png"))).unwrap(),
                1,
                1,
            );
            encoder.set_color(png::ColorType::Rgb);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[0, 128, 0])
                .unwrap();
        }
        for name in names {
            fs::write(biome_dir.join(format!("{name}.json")), b"{}").unwrap();
        }
        Arc::new(
            Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
                dir.path().to_path_buf(),
            ))
            .expect("load synthetic biomes"),
        )
    }

    fn sample_chunk(cx: i32, cz: i32, state_count: u32, biome_count: u32) -> Chunk {
        let mut chunk = Chunk::empty(cx, cz);
        chunk.lit = true;
        for si in 0..4usize {
            let mut blocks = [0u32; 4096];
            for (i, slot) in blocks.iter_mut().enumerate() {
                *slot = ((i + si * 7 + (cx.unsigned_abs() as usize) * 13) % state_count as usize)
                    as u32;
            }
            let mut biomes = [0u32; 64];
            for (i, slot) in biomes.iter_mut().enumerate() {
                *slot = ((i + si + cz.unsigned_abs() as usize) % biome_count as usize) as u32;
            }
            let sec = Arc::make_mut(&mut chunk.sections[si]);
            sec.blocks = PalettedContainer::from_values(block_strategy(), &blocks);
            sec.biomes = PalettedContainer::from_values(BIOME_STRATEGY, &biomes);
            sec.light = Light::Uniform(0xF0);
        }
        chunk
    }

    #[test]
    fn lazy_cache_fills_on_save_not_load_and_releases_on_drop() {
        let registry = static_registry();
        let biomes = test_biomes(&["plains", "forest", "desert"]);
        let dir = tempfile::tempdir().unwrap();
        let save = SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD);

        let chunk = Arc::new(sample_chunk(0, 0, 5, 3));
        assert_eq!(save.section_palettes.used(), 0);

        save.save(&[Arc::clone(&chunk)], registry, &biomes).unwrap();
        let charged = save.section_palettes.used();
        assert!(charged > 0, "first save should populate lazy palette cache");

        // Loading into a fresh SaveDir must NOT eagerly seed the cache.
        let reloaded_dir = SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD);
        let mut loaded = Arc::new(
            reloaded_dir
                .load(0, 0, registry, &biomes)
                .expect("chunk reloads"),
        );
        assert_eq!(
            reloaded_dir.section_palettes.used(),
            0,
            "load must not eagerly populate palette cache"
        );

        // First async save fills the new folder's budget.
        reloaded_dir
            .save_later(&[Arc::clone(&loaded)], registry, &biomes)
            .unwrap();
        reloaded_dir.flush().unwrap();
        // `flush` can return while the writer still holds the queued snapshot;
        // closing the folder joins the writer, so `loaded` is the only owner.
        let reloaded_budget = Arc::clone(&reloaded_dir.section_palettes);
        drop(reloaded_dir);
        let reloaded_charged = reloaded_budget.used();
        assert_eq!(reloaded_charged, charged);

        // Mutating one section drops only that section's cached entry.
        Arc::make_mut(&mut Arc::make_mut(&mut loaded).sections[0])
            .blocks
            .set(0, 4);
        assert!(reloaded_budget.used() < reloaded_charged);
        assert!(reloaded_budget.used() > 0);

        drop(loaded);
        assert_eq!(reloaded_budget.used(), 0);
        drop(chunk);
        assert_eq!(save.section_palettes.used(), 0);
    }

    #[test]
    fn levels_sharing_a_budget_charge_one_cap() {
        let registry = static_registry();
        let biomes = test_biomes(&["plains", "forest", "desert"]);
        let dir = tempfile::tempdir().unwrap();
        let overworld = SaveDir::new(dir.path().join("overworld"), LevelShape::OVERWORLD);
        let mut other = SaveDir::new(dir.path().join("other"), LevelShape::OVERWORLD);
        other.share_section_palettes(&overworld);
        let apart = SaveDir::new(dir.path().join("apart"), LevelShape::OVERWORLD);

        let first = Arc::new(sample_chunk(0, 0, 5, 3));
        overworld
            .save(&[Arc::clone(&first)], registry, &biomes)
            .unwrap();
        let one = overworld.section_palettes.used();
        assert!(one > 0);
        let second = Arc::new(sample_chunk(1, 0, 5, 3));
        other
            .save(&[Arc::clone(&second)], registry, &biomes)
            .unwrap();
        // Both folders' saves count against the one budget.
        assert!(overworld.section_palettes.used() > one);
        assert_eq!(
            other.section_palettes.used(),
            overworld.section_palettes.used()
        );
        // A folder that doesn't share keeps its own.
        assert_eq!(apart.section_palettes.used(), 0);

        drop((first, second));
        assert_eq!(overworld.section_palettes.used(), 0);
    }

    #[test]
    fn cached_saves_produce_byte_identical_region_files() {
        let registry = static_registry();
        let biomes = test_biomes(&["plains", "forest", "desert", "taiga"]);
        let budget = Arc::new(SavedPaletteBudget::new(DEFAULT_PALETTE_CACHE_BYTES));
        let mut chunks: Vec<Arc<Chunk>> = (0..8)
            .map(|i| Arc::new(sample_chunk(i % 4, i / 4, 7, 4)))
            .collect();

        let write_region =
            |path: &std::path::Path,
             chunks: &[Arc<Chunk>],
             palette_cache: Option<Arc<SavedPaletteBudget>>| {
                let encoding = Arc::new(Encoding {
                    registry,
                    biomes: Arc::clone(&biomes),
                    tables: None,
                    shape: LevelShape::OVERWORLD,
                    section_palettes: palette_cache,
                });
                let records: Vec<((i32, i32), Record)> = chunks
                    .iter()
                    .map(|c| {
                        (
                            (c.x, c.z),
                            Record::Chunk(Arc::clone(c), Arc::clone(&encoding), None),
                        )
                    })
                    .collect();
                let refs: Vec<((i32, i32), &Record)> =
                    records.iter().map(|(k, r)| (*k, r)).collect();
                let compressed = compress_records(&refs, 1_700_000_000);
                fs::create_dir_all(path).unwrap();
                let file = path.join("r.0.0.mca");
                let mut region = RegionFile::open_writable(&file, true).unwrap();
                for ((cx, cz), rec) in records.iter().map(|(k, _)| *k).zip(compressed) {
                    let rec = rec.unwrap().expect("chunk record");
                    region.write(ChunkPos::new(cx, cz), &rec).unwrap();
                }
                region.sync().unwrap();
                fs::read(&file).unwrap()
            };

        let dir = tempfile::tempdir().unwrap();
        let _codec = compression_pool::holding_selected_codec();
        let uncached_bytes = write_region(&dir.path().join("uncached"), &chunks, None);
        let first_cached = write_region(
            &dir.path().join("first_cached"),
            &chunks,
            Some(Arc::clone(&budget)),
        );
        assert!(budget.used() > 0);
        let second_cached = write_region(
            &dir.path().join("second_cached"),
            &chunks,
            Some(Arc::clone(&budget)),
        );
        assert_eq!(
            first_cached, uncached_bytes,
            "first save region bytes match"
        );
        assert_eq!(
            second_cached, uncached_bytes,
            "second (cache-hit) save region bytes match"
        );

        // Mutate one block in one section of every chunk and verify edited region bytes match too.
        for chunk in &mut chunks {
            Arc::make_mut(&mut Arc::make_mut(chunk).sections[1])
                .blocks
                .set(42, 6);
        }
        let uncached_edited = write_region(&dir.path().join("uncached_edit"), &chunks, None);
        let cached_edited = write_region(
            &dir.path().join("cached_edit"),
            &chunks,
            Some(Arc::clone(&budget)),
        );
        assert_eq!(
            cached_edited, uncached_edited,
            "post-edit cached save region bytes match"
        );
        drop(chunks);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn cached_palette_reresolves_registry_and_biome_names_at_save_time() {
        let reg_a = test_registry(
            &[
                "minecraft:air",
                "minecraft:stone",
                "minecraft:dirt",
                "minecraft:oak_log",
            ],
            true,
        );
        let reg_b = test_registry(
            &[
                "minecraft:air",
                "minecraft:deepslate",
                "minecraft:coarse_dirt",
                "minecraft:birch_log",
            ],
            true,
        );
        let biomes_a = test_biomes(&["plains", "forest"]);
        let biomes_b = test_biomes(&["desert", "taiga"]);
        let budget = Arc::new(SavedPaletteBudget::new(DEFAULT_PALETTE_CACHE_BYTES));
        let chunk = sample_chunk(0, 0, 5, 2);

        // First save populates the cache under reg_a / biomes_a.
        let first = encode_saved(
            &chunk,
            &reg_a,
            &biomes_a,
            None,
            LevelShape::OVERWORLD,
            None,
            Some(&budget),
        );
        assert_eq!(
            first,
            encode_with(&chunk, &reg_a, &biomes_a, None, LevelShape::OVERWORLD)
        );
        assert!(budget.used() > 0);

        // Cache hit under reg_b / biomes_b must re-resolve names from the new registry and biomes.
        let second = encode_saved(
            &chunk,
            &reg_b,
            &biomes_b,
            None,
            LevelShape::OVERWORLD,
            None,
            Some(&budget),
        );
        let expected_b = encode_with(&chunk, &reg_b, &biomes_b, None, LevelShape::OVERWORLD);
        assert_eq!(second, expected_b);
        assert_ne!(second, first);
    }

    #[test]
    fn budget_cap_enforced_and_over_cap_chunks_encode_identically() {
        let registry = test_registry(
            &["minecraft:air", "minecraft:stone", "minecraft:dirt"],
            false,
        );
        let biomes = test_biomes(&["plains", "forest"]);
        let dir = tempfile::tempdir().unwrap();
        let mut save = SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD);
        // Tight cap: admits at most one block section (~2.1 KiB), rejecting the rest.
        save.section_palettes = Arc::new(SavedPaletteBudget::new(3000));

        let chunks: Vec<Arc<Chunk>> = (0..4).map(|x| Arc::new(sample_chunk(x, 0, 3, 2))).collect();
        save.save(&chunks, &registry, &biomes).unwrap();
        assert!(save.section_palettes.used() > 0);
        assert!(save.section_palettes.used() <= save.section_palettes.limit());

        for chunk in &chunks {
            let expected = nbt::write_named(
                &encode_with(chunk, &registry, &biomes, None, LevelShape::OVERWORLD),
                Limits::saved(),
            )
            .unwrap();
            let actual = nbt::write_named(
                &encode_saved(
                    chunk,
                    &registry,
                    &biomes,
                    None,
                    LevelShape::OVERWORLD,
                    None,
                    Some(&save.section_palettes),
                ),
                Limits::saved(),
            )
            .unwrap();
            assert_eq!(actual, expected);
        }

        drop(chunks);
        assert_eq!(save.section_palettes.used(), 0);
    }
}
