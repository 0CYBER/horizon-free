//! Which structures contain a position (`StructureManager.getStructureAt`
//! and `getStructureWithPieceAt`), from the chunk's structure references and
//! the generator's cached starts.
use super::Game;
use horizon_world::worldgen::structure::start::StructureStart;
use std::sync::Arc;

/// A structure a position is in, by either of vanilla's two reads: `getStructureAt` tests its
/// start's box, `getStructureWithPieceAt` its pieces as placed, with no start box gate. A
/// piece that settled below the box it was made in (a shipwreck) holds blocks outside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Reach {
    pub name: String,
    /// The start's box holds the position.
    pub in_box: bool,
    /// One of its pieces, as placed, holds the position.
    pub in_piece: bool,
}

impl Game<'_> {
    /// Structures reaching `pos` by either read ([`Reach`]): (id, inside a piece as placed).
    pub(super) fn structures_reaching(&self, pos: [i32; 3]) -> Vec<(String, bool)> {
        self.structures_reaching_where(pos, &|_| true)
    }
    /// The same, for the structures whose id `wanted` takes: the others'
    /// starts aren't looked up.
    pub(super) fn structures_reaching_where(
        &self,
        pos: [i32; 3],
        wanted: &dyn Fn(&str) -> bool,
    ) -> Vec<(String, bool)> {
        structures_reaching_in(&self.world, &|| self.end.locator(), pos, wanted)
    }
    /// The structures with a piece containing `pos` (`getStructureWithPieceAt`).
    pub(super) fn structures_at(&self, pos: [i32; 3]) -> Vec<String> {
        self.structures_reaching(pos)
            .into_iter()
            .filter(|(_, piece)| *piece)
            .map(|(n, _)| n)
            .collect()
    }
}

type Locator<'a> = &'a dyn Fn() -> Option<Arc<horizon_client::vanillagen::VanillaTerrain>>;

/// [`Game::structures_reaching_where`] over what it reads: [`reaches_in`] as (id, inside a
/// piece as placed).
pub(super) fn structures_reaching_in(
    world: &horizon_client::world::World,
    locator: Locator<'_>,
    pos: [i32; 3],
    wanted: &dyn Fn(&str) -> bool,
) -> Vec<(String, bool)> {
    reaches_in(world, locator, pos, wanted)
        .into_iter()
        .map(|reach| (reach.name, reach.in_piece))
        .collect()
}

/// The structures reaching `pos` by either read, from the world's chunks and, asked only when a
/// chunk refers to a structure, the terrain that locates the starts. A start the generator no
/// longer holds is built again, as vanilla's `getChunk(STRUCTURE_STARTS)` loads it.
pub(super) fn reaches_in(
    world: &horizon_client::world::World,
    locator: Locator<'_>,
    pos: [i32; 3],
    wanted: &dyn Fn(&str) -> bool,
) -> Vec<Reach> {
    let Some(chunk) = world.chunks.get(&(pos[0] >> 4, pos[2] >> 4)) else {
        return Vec::new();
    };
    if chunk.structure_refs.is_empty() {
        return Vec::new();
    }
    let Some(locator) = locator() else {
        return Vec::new();
    };
    reaching(chunk, pos, wanted, &|x, z| Some(locator.starts_at(x, z))).unwrap_or_default()
}

/// [`structures_reaching_in`] for a read that loads and builds nothing: from the starts the
/// generator holds already. None for an unloaded chunk, a level without the terrain that
/// locates its references, and a reference whose starts aren't held.
pub(super) fn cached_structures_reaching_in(
    world: &horizon_client::world::World,
    locator: Locator<'_>,
    pos: [i32; 3],
) -> Option<Vec<Reach>> {
    let chunk = world.chunks.get(&(pos[0] >> 4, pos[2] >> 4))?;
    if chunk.structure_refs.is_empty() {
        return Some(Vec::new());
    }
    let locator = locator()?;
    reaching(chunk, pos, &|_| true, &|x, z| locator.cached_starts_at(x, z))
}

/// The structures `chunk` refers to that hold `pos` by either read ([`Reach`]); `starts`
/// gives a chunk's starts. None when it can't give those of a chunk a wanted reference names.
fn reaching(
    chunk: &horizon_client::chunk::Chunk,
    pos: [i32; 3],
    wanted: &dyn Fn(&str) -> bool,
    starts: &dyn Fn(i32, i32) -> Option<Vec<Arc<StructureStart>>>,
) -> Option<Vec<Reach>> {
    let mut out: Vec<Reach> = Vec::new();
    for (id, (sx, sz)) in &chunk.structure_refs {
        if !wanted(id) {
            continue;
        }
        for start in starts(*sx, *sz)? {
            if *start.name != **id {
                continue;
            }
            let in_box = start.bounding_box.is_inside(pos);
            // `structureHasPieceAt`: the placed pieces, whatever the start's box says.
            let in_piece = start
                .pieces
                .iter()
                .any(|p| p.placed_bounding_box().is_inside(pos));
            if !in_box && !in_piece {
                continue;
            }
            match out.iter_mut().find(|r| r.name == start.name) {
                Some(r) => {
                    r.in_box |= in_box;
                    r.in_piece |= in_piece;
                }
                None => out.push(Reach {
                    name: start.name.clone(),
                    in_box,
                    in_piece,
                }),
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::chunk::Chunk;
    use horizon_core::{density::TerrainAdjustment, position::ChunkPos};
    use horizon_world::worldgen::structure::{
        Mirror, Rotation, StructureTemplate,
        legacy::{TemplateHandler, TemplatePiece},
        processor::ProcessorList,
        start::{Piece, StructureDef, StructureKind},
    };

    /// `getStructureWithPieceAt` reads a piece where it was placed, with no start box gate: a
    /// shipwreck that settled below the box it was made in holds the blocks of its hull there,
    /// and none of the empty box above, which only `getStructureAt` still reads.
    #[test]
    fn a_settled_shipwreck_is_found_in_its_hull_and_not_in_the_box_it_left() {
        let mut template = StructureTemplate::empty();
        template.size = [9, 7, 13];
        let wreck = TemplatePiece::new(
            "minecraft:shipwreck",
            "minecraft:shipwreck/with_mast",
            Arc::new(template),
            [100, 90, 200],
            Rotation::None,
            Mirror::None,
            [4, 0, 15],
            Arc::new(ProcessorList::default()),
            true,
            Vec::new(),
            TemplateHandler::Shipwreck { beached: false },
        );
        *wreck.state.lock().unwrap() = Some(60);
        let def = StructureDef {
            name: "minecraft:shipwreck".into(),
            kind: StructureKind::Unsupported("test".into()),
            biomes: Default::default(),
            step: 0,
            terrain: TerrainAdjustment::None,
            enhanced_terrain: None,
        };
        let home = ChunkPos::new(100 >> 4, 200 >> 4);
        let start = Arc::new(StructureStart::new(
            0,
            &def,
            home,
            vec![Piece::Legacy(Box::new(wreck))],
        ));
        let mut chunk = Chunk::empty(home.x, home.z);
        chunk.structure_refs = vec![(Arc::from("minecraft:shipwreck"), (home.x, home.z))];
        let at = |pos| reaching(&chunk, pos, &|_| true, &|_, _| Some(vec![start.clone()])).unwrap();
        let reach = |in_box, in_piece| {
            vec![Reach {
                name: "minecraft:shipwreck".into(),
                in_box,
                in_piece,
            }]
        };
        assert_eq!(at([100, 60, 200]), reach(false, true), "in the settled hull");
        assert_eq!(at([100, 90, 200]), reach(true, false), "in the box it left, in no piece");
        assert_eq!(at([100, 80, 200]), Vec::new(), "between the two");
    }
}
