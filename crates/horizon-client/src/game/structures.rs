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
    /// `ChunkGenerator.getMobsAt` / `NaturalSpawner.mobsAt`: a structure's
    /// `spawn_overrides` for the category replace the biome's list where its
    /// box (or piece) holds `pos`; fortress spawns on nether bricks.
    pub(super) fn structure_spawns(
        &mut self,
        category: &str,
        pos: [i32; 3],
    ) -> Option<Vec<horizon_client::biome::SpawnEntry>> {
        let reaching = self.structures_reaching(pos);
        if reaching.is_empty() {
            return None;
        }
        let below = self.world.block(pos[0], pos[1] - 1, pos[2]);
        let bricks = &*self.res.facts.get(below).name == "minecraft:nether_bricks";
        for (id, piece) in &reaching {
            let Some(o) = self.spawn_override(id, category) else {
                continue;
            };
            if !o.0 || *piece {
                return Some(o.1);
            }
        }
        if category == "monster"
            && bricks
            && reaching.iter().any(|(n, _)| n == "minecraft:fortress")
        {
            return self
                .spawn_override("minecraft:fortress", "monster")
                .map(|o| o.1);
        }
        None
    }
    /// A structure's override for a category: (piece-only, spawns).
    fn spawn_override(
        &mut self,
        structure: &str,
        category: &str,
    ) -> Option<(bool, Vec<horizon_client::biome::SpawnEntry>)> {
        if !self.spawn_overrides.contains_key(structure) {
            let (ns, path) = structure
                .split_once(':')
                .unwrap_or(("minecraft", structure));
            let file = self
                .options
                .resources
                .join(format!("data/{ns}/worldgen/structure/{path}.json"));
            let v: serde_json::Value = std::fs::read(file)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            let mut map = std::collections::HashMap::new();
            for (cat, o) in v["spawn_overrides"].as_object().into_iter().flatten() {
                let piece = o["bounding_box"].as_str() == Some("piece");
                let spawns = o["spawns"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|e| {
                        let count = &e["count"];
                        horizon_client::biome::SpawnEntry {
                            entity: e["type"].as_str().unwrap_or_default().to_owned(),
                            weight: e["weight"].as_u64().unwrap_or(1) as u32,
                            min: count["min_inclusive"]
                                .as_u64()
                                .or_else(|| e["minCount"].as_u64())
                                .unwrap_or(1) as u32,
                            max: count["max_inclusive"]
                                .as_u64()
                                .or_else(|| e["maxCount"].as_u64())
                                .unwrap_or(1) as u32,
                        }
                    })
                    .collect();
                map.insert(cat.clone(), (piece, spawns));
            }
            self.spawn_overrides.insert(structure.to_owned(), map);
        }
        self.spawn_overrides.get(structure)?.get(category).cloned()
    }
}
