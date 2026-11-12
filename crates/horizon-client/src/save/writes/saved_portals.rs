//! The portal blocks of saved chunks an exit portal search read (`SaveDir::saved_portal_blocks`),
//! each kept until its chunk is written again (#15798).
use super::super::*;

/// Saved chunks whose portal blocks are kept: an Overworld exit search reads 19 x 19 of them.
const KEPT: usize = 1 << 16;

/// The cache behind [`SaveDir::saved_portal_blocks`].
#[derive(Default)]
pub(super) struct SavedPortals(Mutex<Kept>);

/// `writes` counts the region writes queued, so a search that read a chunk while it was being
/// written again doesn't keep what it read.
#[derive(Default)]
struct Kept {
    chunks: crate::fast_hash::FxHashMap<(i32, i32), Arc<[BlockPos]>>,
    writes: u64,
}

impl SavedPortals {
    /// The folder queued these chunks to be written: a search reads them again from now on (the
    /// queued records are already where a read finds them).
    pub(super) fn written(&self, folder: Folder, chunks: impl Iterator<Item = (i32, i32)>) {
        if folder != Folder::Region {
            return;
        }
        if let Ok(mut kept) = self.0.lock() {
            kept.writes += 1;
            for key in chunks {
                kept.chunks.remove(&key);
            }
        }
    }
}

impl SaveDir {
    /// The blocks `is_portal` finds in the saved chunk at `x, z` (none without one), read once and
    /// kept until this folder writes the chunk again: an exit portal search
    /// (`PortalForcer.findClosestPortalPosition`) reads up to 19 x 19 chunks that aren't loaded,
    /// for every entity that arrives, and decoding them each time held the tick up for tens of
    /// milliseconds. `is_portal` must be the same test on every call.
    pub fn saved_portal_blocks(
        &self,
        x: i32,
        z: i32,
        registry: &BlockRegistry,
        biomes: &Biomes,
        is_portal: impl Fn(BlockStateId) -> bool,
    ) -> Arc<[BlockPos]> {
        let writes = match self.outstanding.portals.0.lock() {
            Ok(kept) => {
                if let Some(found) = kept.chunks.get(&(x, z)) {
                    return found.clone();
                }
                kept.writes
            }
            Err(_) => u64::MAX,
        };
        let found: Arc<[BlockPos]> = match self.load(x, z, registry, biomes) {
            Some(chunk) => chunk.blocks_where(&is_portal).into(),
            None => Arc::new([]),
        };
        if let Ok(mut kept) = self.outstanding.portals.0.lock()
            && kept.writes == writes
        {
            if kept.chunks.len() >= KEPT {
                kept.chunks.clear();
            }
            kept.chunks.insert((x, z), found.clone());
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_portal_blocks_follow_each_save_of_their_chunk() {
        let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&root).expect("biomes");
        let state = |name: &str| assets.registry.default_state(name).unwrap();
        let (portal, obsidian) = (
            state("minecraft:nether_portal"),
            state("minecraft:obsidian"),
        );
        let is_portal = |s: BlockStateId| s == portal;
        let dir =
            std::env::temp_dir().join(format!("horizon-saved-portals-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |save: &SaveDir, chunk: &Chunk| {
            let record = (
                (chunk.x, chunk.z),
                Some(encode(chunk, assets.registry, &biomes)),
            );
            save.write_records("region", vec![record], true).unwrap();
        };
        let found = |save: &SaveDir| -> Vec<BlockPos> {
            save.saved_portal_blocks(3, -2, assets.registry, &biomes, is_portal)
                .to_vec()
        };
        let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        let mut chunk = Chunk::empty(3, -2);
        chunk.set(1, 70, 2, obsidian.0);
        write(&save, &chunk);
        assert!(found(&save).is_empty());
        // A portal lit and saved after a search: the next search reads the chunk again.
        chunk.set(1, 71, 2, portal.0);
        write(&save, &chunk);
        assert_eq!(found(&save), [BlockPos::new(49, 71, -30)]);
        chunk.set(1, 72, 2, portal.0);
        write(&save, &chunk);
        assert_eq!(
            found(&save),
            [BlockPos::new(49, 71, -30), BlockPos::new(49, 72, -30)]
        );
        // As a new start of the server reads it.
        let reopened = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        assert_eq!(found(&reopened), found(&save));
        // A chunk never saved has none.
        assert!(
            save.saved_portal_blocks(40, 40, assets.registry, &biomes, is_portal)
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
