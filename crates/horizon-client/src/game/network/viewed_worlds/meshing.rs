//! The viewed levels' sections meshed while a world view names them (arismoko/horizon#4187): `wanted` holds the
//! levels the frame's views name, and each frame `mesh_frame` hands the open levels to
//! `ViewedMeshing`, which meshes them into a set per level after the player's own level's work.
//! No view names one yet, so nothing is meshed: `wanted` stays empty until views of other levels
//! are drawn. A level closed, travelled into or dropped with the others takes its set with it
//! (`ViewedWorlds::remove`, `take`, `clear`).
use super::worlds::ViewedWorlds;
use horizon_client::{
    section_mask::SectionMask,
    streaming::{MeshSource, ViewedLevel, ViewedMeshing},
    world::World,
};
use std::time::Instant;

impl ViewedWorlds {
    /// A frame's meshing of the viewed levels. While no level is wanted and none keeps a set it
    /// is two checks and nothing else: no source on the pool, no job, no allocation.
    pub(in crate::game) fn mesh_frame(
        &mut self,
        source: impl FnOnce() -> MeshSource,
        now: Instant,
    ) {
        if self.wanted.is_empty() && self.meshing.is_none() {
            return;
        }
        let source = source();
        let wanted = &self.wanted;
        let meshing = self.meshing.get_or_insert_with(ViewedMeshing::default);
        let levels = self.worlds.values_mut().map(|(_, viewed)| ViewedLevel {
            id: viewed.id,
            wanted: wanted.contains(&viewed.key),
            nether_shading: viewed.nether_shading,
            world: &mut viewed.world,
        });
        meshing.frame(&source, levels, now);
        if meshing.is_empty() && wanted.is_empty() {
            self.meshing = None;
        }
    }
}

/// Column `key` arrived, went, or changed its light or biomes: it and the columns beside it
/// remesh, as the player's level's do (`Streaming::insert_remote`).
pub(super) fn column_changed(world: &mut World, (x, z): (i32, i32)) {
    let all = SectionMask::below(world.layout().sections());
    for dz in -1..=1 {
        for dx in -1..=1 {
            let column = (x + dx, z + dz);
            if column == (x, z) || world.chunks.contains_key(&column) {
                world.dirty_sections.insert_column_mask(column, all);
            }
        }
    }
}

/// The block at `pos` changed: the sections its neighbourhood reaches remesh
/// (`LevelRenderer.setBlocksDirty` from one block below to one above it on each axis).
pub(super) fn block_changed(world: &mut World, [x, y, z]: [i32; 3]) {
    let Some(min_y) = world.chunks.get(&(x >> 4, z >> 4)).map(|c| c.min_y()) else {
        return;
    };
    let sections = world.layout().sections() as i32;
    for dy in -1..=1 {
        let sy = (y + dy - min_y) >> 4;
        if !(0..sections).contains(&sy) {
            continue;
        }
        for dz in -1..=1 {
            for dx in -1..=1 {
                world
                    .dirty_sections
                    .insert(((x + dx) >> 4, sy, (z + dz) >> 4));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::worlds::ViewedWorld;
    use super::*;
    use crate::game::remote_chunks::ChunkFormat;
    use horizon_client::chunk::Chunk;
    use horizon_protocol::registry::DimensionType;
    use horizon_world::registry::BlockStateId;
    use std::{sync::Arc, time::Duration};

    /// A Nether world with a 3x3 of columns around the origin.
    fn nether() -> ViewedWorld {
        let format = Arc::new(ChunkFormat::new(
            &DimensionType::vanilla("minecraft:the_nether"),
            vec![0],
            1,
        ));
        let mut world = World::new(BlockStateId(0), 1);
        for z in -1..=1 {
            for x in -1..=1 {
                world.chunks.insert((x, z), Arc::new(Chunk::empty(x, z)));
            }
        }
        ViewedWorld::new(
            "minecraft:the_nether".into(),
            format,
            None,
            world,
            horizon_protocol::viewed_levels::MAX_COLUMNS,
        )
    }

    /// A drawn mesh for every section.
    fn source() -> MeshSource {
        MeshSource::custom(|_, _| {
            let mut mesh = horizon_client::mesher::SectionMesh::default();
            mesh.block_entities.push(([0, 0, 0], 1));
            mesh
        })
    }

    /// With no level wanted nothing is kept or queued; a wanted level gets a set, which goes when
    /// the level closes, and with it the meshing's place on the pool.
    #[test]
    fn a_viewed_level_closed_frees_its_set() {
        let mut viewed = ViewedWorlds::default();
        let world = nether();
        let (id, key) = (world.id, world.key);
        viewed.insert(1, 1, world);
        viewed.mesh_frame(|| unreachable!("nothing to mesh"), Instant::now());
        assert!(viewed.meshing.is_none());

        viewed.wanted.insert(key);
        let start = Instant::now();
        while viewed
            .meshing
            .as_ref()
            .and_then(|m| m.sections(id))
            .is_none_or(|mut sections| sections.next().is_none())
        {
            assert!(start.elapsed() < Duration::from_secs(10));
            viewed.mesh_frame(source, Instant::now());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(viewed.remove(1));
        assert!(viewed.meshing.as_ref().unwrap().is_empty());
        viewed.wanted.clear();
        viewed.mesh_frame(source, Instant::now());
        assert!(viewed.meshing.is_none());
    }

    /// A block a server changes on a section's corner dirties the eight sections around it.
    #[test]
    fn a_corner_block_dirties_the_sections_it_touches() {
        let mut world = nether().world;
        let min_y = world.chunks.get(&(0, 0)).unwrap().min_y();
        block_changed(&mut world, [0, min_y + 16, 0]);
        let mut dirty: Vec<_> = world
            .dirty_sections
            .columns()
            .flat_map(|((x, z), mask)| mask.iter().map(move |sy| (x, sy, z)))
            .collect();
        dirty.sort_unstable();
        let mut expected = Vec::new();
        for x in [-1, 0] {
            for sy in [0, 1] {
                for z in [-1, 0] {
                    expected.push((x, sy, z));
                }
            }
        }
        assert_eq!(dirty, expected);
    }
}
