//! Which structures contain a position (`StructureManager.getStructureAt`
//! and `getStructureWithPieceAt`), from the chunk's structure references and
//! the generator's cached starts.
use super::Game;

impl Game<'_> {
    /// Structures reaching `pos`: (id, inside a piece). `getStructureAt`
    /// tests the start's whole box, `getStructureWithPieceAt` a piece's.
    fn structures_reaching(&self, pos: [i32; 3]) -> Vec<(String, bool)> {
        let Some(chunk) = self.world.chunks.get(&(pos[0] >> 4, pos[2] >> 4)) else {
            return Vec::new();
        };
        if chunk.structure_refs.is_empty() {
            return Vec::new();
        }
        let Some(locator) = self.end.locator() else {
            return Vec::new();
        };
        let mut out: Vec<(String, bool)> = Vec::new();
        for (id, (sx, sz)) in &chunk.structure_refs {
            for start in locator.starts_at(*sx, *sz) {
                if *start.name != **id || !start.bounding_box.is_inside(pos) {
                    continue;
                }
                let piece = start.pieces.iter().any(|p| p.bounding_box().is_inside(pos));
                match out.iter_mut().find(|(n, _)| *n == start.name) {
                    Some(e) => e.1 |= piece,
                    None => out.push((start.name.clone(), piece)),
                }
            }
        }
        out
    }
    /// The structures with a piece containing `pos`.
    pub(super) fn structures_at(&self, pos: [i32; 3]) -> Vec<String> {
        self.structures_reaching(pos)
            .into_iter()
            .filter(|(_, piece)| *piece)
            .map(|(n, _)| n)
            .collect()
    }
    /// `NaturalSpawner.mobsAt`: the fortress bounds on nether bricks, the
    /// structure `spawn_overrides` whose box (or piece) holds `pos`, else the
    /// biome's list.
    pub(super) fn spawn_list(
        &self,
        category: &str,
        pos: [i32; 3],
        biome: &std::collections::HashMap<String, Vec<horizon_client::biome::SpawnEntry>>,
    ) -> Vec<horizon_client::biome::SpawnEntry> {
        use horizon_client::mobs::spawn_selection::{StructureAt, StructureSpawns, mobs_at};
        let structures: Vec<StructureAt> = self
            .structures_reaching(pos)
            .into_iter()
            .map(|(id, in_piece)| StructureAt {
                id,
                in_piece,
                in_bounds: true,
            })
            .collect();
        let below = self.world.block(pos[0], pos[1] - 1, pos[2]);
        let bricks = &*self.res.facts.get(below).name == "minecraft:nether_bricks";
        let spawns = StructureSpawns::shared(&self.options.resources);
        mobs_at(spawns, biome, category, &structures, bricks)
    }
}
