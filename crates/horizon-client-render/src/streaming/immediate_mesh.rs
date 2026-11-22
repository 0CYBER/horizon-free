//! Synchronous section meshing for individual block edits (`World::immediate_sections`).
//!
//! Vanilla's `LevelRenderer.compileSections` / `SectionRenderDispatcher.rebuildSectionSync`
//! rebuilds player-edited sections on the render thread before drawing the frame so a placed
//! or broken block appears on the same frame as its selection outline instead of lagging one
//! or more frames behind on worker threads.
use super::{INLINE_RELIGHT_MAX, Streaming, mesh_section};
use crate::{
    fast_hash::{FxHashMap, FxHashSet},
    mesher::SectionMesh,
    section_mask::SectionMask,
    world::{DirtySections, World},
};

/// Maximum number of sections rebuilt synchronously in one frame.
///
/// A single block edit on a chunk/section corner touches at most 8 sections; larger bursts
/// (bulk updates or scripts calling `set_block` in a loop) fall back to the background worker
/// pool via `World::dirty_sections` so they never stall a frame.
pub const IMMEDIATE_MESH_LIMIT: usize = 8;

/// Vanilla `SectionRenderDispatcher` squared distance threshold for `PrioritizeChunkUpdates.NEARBY`.
pub const NEARBY_IMMEDIATE_DIST_SQ: f64 = 768.0;

/// Take ready columns from `immediate` within `IMMEDIATE_MESH_LIMIT`, removing accepted sections
/// from `dirty`. Returns empty when `prioritize_chunk_updates == 0` (Threaded, vanilla default).
fn take_ready_immediate_sections(
    prioritize_chunk_updates: u8,
    immediate: &mut DirtySections,
    dirty: &mut DirtySections,
    lit: &FxHashSet<(i32, i32)>,
    lighting: &FxHashMap<(i32, i32), u64>,
    in_view: impl Fn(i32, i32) -> bool,
    has_chunk: impl Fn(i32, i32) -> bool,
) -> Vec<((i32, i32), SectionMask)> {
    let total = immediate.len();
    if prioritize_chunk_updates == 0 || total == 0 || total > IMMEDIATE_MESH_LIMIT {
        immediate.clear();
        return Vec::new();
    }
    let taken = std::mem::take(immediate);
    let mut ready_columns = Vec::new();
    for ((x, z), sections) in taken.columns() {
        if !has_chunk(x, z) || !in_view(x, z) {
            continue;
        }
        let neighbours_ready = (-1..=1).all(|dz| {
            (-1..=1).all(|dx| {
                let n = (x + dx, z + dz);
                lit.contains(&n) && !lighting.contains_key(&n)
            })
        });
        if !neighbours_ready {
            continue;
        }
        let mut accepted = SectionMask::EMPTY;
        for sy in sections.iter().map(|sy| sy as i32) {
            // Remove from `dirty` if still queued; even if streaming already removed it to
            // dispatch a worker job, accept it so the synchronous rebuild supersedes the
            // in-flight worker job.
            dirty.remove((x, sy, z));
            accepted.insert(sy as usize);
        }
        if !accepted.is_empty() {
            ready_columns.push(((x, z), accepted));
        }
    }
    ready_columns
}

impl Streaming {
    /// Synchronously rebuild sections dirtied by individual block edits since `update_world`,
    /// gated by `prioritize_chunk_updates` (`0` = Threaded/none, `1` = Semi-Blocking/player-edited,
    /// `2` = Fully Blocking/nearby), returning meshes ready for immediate GPU upload.
    pub fn mesh_immediate(
        &mut self,
        world: &mut World,
        prioritize_chunk_updates: u8,
    ) -> Vec<((i32, i32, i32), SectionMesh)> {
        if prioritize_chunk_updates == 0 || !self.mesh || world.immediate_sections.is_empty() {
            world.immediate_sections.clear();
            return Vec::new();
        }
        if !world.light_edits.is_empty() {
            if world.light_edits.len() > INLINE_RELIGHT_MAX {
                // Pending lighting exceeds the inline budget: leave all affected sections dirty
                // for the normal relighting and worker meshing path instead of uploading stale light.
                for ((x, z), sections) in std::mem::take(&mut world.immediate_sections).columns() {
                    for sy in sections.iter().map(|sy| sy as i32) {
                        world.dirty_sections.insert((x, sy, z));
                    }
                }
                return Vec::new();
            }
            self.relight_edits(world);
        }
        let ready_columns = {
            let ranges = self.ranges();
            let chunks = &world.chunks;
            take_ready_immediate_sections(
                prioritize_chunk_updates,
                &mut world.immediate_sections,
                &mut world.dirty_sections,
                &self.lit,
                &self.lighting,
                |x, z| ranges.in_view(x, z),
                |x, z| chunks.contains_key(&(x, z)),
            )
        };
        if ready_columns.is_empty() {
            return Vec::new();
        }
        let context = self.shared.context.read().unwrap().clone();
        let settings = self.mesh_settings();
        let mut uploads = Vec::new();
        for ((x, z), sections) in ready_columns {
            let Some(region) = Self::region(world, x, z) else {
                // Keep sections dirty for workers if a neighbour column disappeared.
                for sy in sections.iter().map(|sy| sy as i32) {
                    world.dirty_sections.insert((x, sy, z));
                }
                continue;
            };
            for sy in sections.iter().map(|sy| sy as i32) {
                // Invalidate any older in-flight worker job for this section so its stale
                // `Done::Meshed` is discarded when it finishes.
                self.meshing.remove(&(x, sy, z));
                let recycled = self.shared.meshes.lock().unwrap().take();
                let mesh = mesh_section(
                    &context,
                    &settings,
                    &mut self.immediate_faces,
                    &mut self.immediate_scratch,
                    &region,
                    sy,
                    recycled,
                );
                self.sequence += 1;
                self.compile_started.insert((x, sy, z), self.sequence);
                self.meshed_total += 1;
                uploads.push(((x, sy, z), mesh));
            }
        }
        uploads
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;
    use horizon_world::registry::BlockStateId;
    use std::sync::Arc;

    #[test]
    fn dirty_sections_remove_clears_single_section_and_prunes_empty_column() {
        let mut dirty = DirtySections::default();
        dirty.insert((1, 4, -2));
        dirty.insert((1, 5, -2));
        assert!(dirty.remove((1, 4, -2)));
        assert!(!dirty.remove((1, 4, -2)));
        assert!(!dirty.contains(&(1, 4, -2)));
        assert!(dirty.contains(&(1, 5, -2)));
        assert_eq!(dirty.len(), 1);
        assert!(dirty.remove((1, 5, -2)));
        assert!(dirty.is_empty());
        assert_eq!(dirty.columns().count(), 0);
    }

    #[test]
    fn set_block_marks_immediate_sections_only_inside_player_edit_scope() {
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        // Ordinary writes (server updates, redstone, crops) must NOT mark immediate_sections.
        world.set(8, 64, 8, BlockStateId(1)).unwrap();
        let sy = (64 - world.bottom_y()) >> 4;
        assert!(world.immediate_sections.is_empty());
        assert!(world.dirty_sections.contains(&(0, sy, 0)));

        // Player-edit scoped writes mark immediate_sections for the touched section and neighbours.
        world.dirty_sections.clear();
        world.begin_player_edit();
        world.set(8, 64, 8, BlockStateId(2)).unwrap();
        world.end_player_edit();
        assert!(world.immediate_sections.contains(&(0, sy, 0)));
        assert!(world.dirty_sections.contains(&(0, sy, 0)));

        // Corner of a section marks up to 8 sections (within IMMEDIATE_MESH_LIMIT).
        world.immediate_sections.clear();
        world.dirty_sections.clear();
        world.begin_player_edit();
        world.set(0, 64, 0, BlockStateId(3)).unwrap();
        world.end_player_edit();
        assert_eq!(world.immediate_sections.len(), 8);
        assert!(world.immediate_sections.len() <= IMMEDIATE_MESH_LIMIT);
    }

    #[test]
    fn threaded_chunk_builder_mode_never_takes_immediate_sections() {
        let mut immediate = DirtySections::default();
        let mut dirty = DirtySections::default();
        let mut lit = FxHashSet::default();
        let lighting = FxHashMap::default();
        for dz in -1..=1 {
            for dx in -1..=1 {
                lit.insert((dx, dz));
            }
        }
        immediate.insert((0, 4, 0));
        dirty.insert((0, 4, 0));

        // prioritizeChunkUpdates == 0 (Threaded, vanilla default): zero main-thread meshing.
        let ready = take_ready_immediate_sections(
            0,
            &mut immediate,
            &mut dirty,
            &lit,
            &lighting,
            |_, _| true,
            |_, _| true,
        );
        assert!(ready.is_empty());
        assert!(immediate.is_empty());
        assert!(dirty.contains(&(0, 4, 0)));
    }

    #[test]
    fn nearby_mode_marks_dirty_sections_within_768_sq_blocks_and_skips_far_ones() {
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        world.chunks.insert((5, 0), Arc::new(Chunk::empty(5, 0)));
        world.set(8, 64, 8, BlockStateId(1)).unwrap();
        world.set(88, 64, 8, BlockStateId(1)).unwrap();
        assert!(world.immediate_sections.is_empty());

        world.mark_nearby_dirty_immediate(
            [8, 64, 8],
            NEARBY_IMMEDIATE_DIST_SQ,
            IMMEDIATE_MESH_LIMIT,
        );
        let sy = (64 - world.bottom_y()) >> 4;
        assert!(world.immediate_sections.contains(&(0, sy, 0)));
        assert!(!world.immediate_sections.contains(&(5, sy, 0)));
    }

    #[test]
    fn take_ready_immediate_clears_dirty_and_keeps_other_sections() {
        let mut immediate = DirtySections::default();
        let mut dirty = DirtySections::default();
        let mut lit = FxHashSet::default();
        let lighting = FxHashMap::default();
        for dz in -1..=1 {
            for dx in -1..=1 {
                lit.insert((dx, dz));
            }
        }
        immediate.insert((0, 4, 0));
        dirty.insert((0, 4, 0));
        dirty.insert((0, 5, 0));

        let ready = take_ready_immediate_sections(
            1,
            &mut immediate,
            &mut dirty,
            &lit,
            &lighting,
            |_, _| true,
            |_, _| true,
        );
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].0, (0, 0));
        assert!(ready[0].1.contains(4));
        assert!(immediate.is_empty());
        assert!(!dirty.contains(&(0, 4, 0)));
        assert!(dirty.contains(&(0, 5, 0)));
    }

    #[test]
    fn take_ready_immediate_falls_back_to_workers_above_limit() {
        let mut immediate = DirtySections::default();
        let mut dirty = DirtySections::default();
        let mut lit = FxHashSet::default();
        let lighting = FxHashMap::default();
        for dz in -1..=1 {
            for dx in -1..=1 {
                lit.insert((dx, dz));
            }
        }
        for sy in 0..=(IMMEDIATE_MESH_LIMIT as i32) {
            immediate.insert((0, sy, 0));
            dirty.insert((0, sy, 0));
        }

        let ready = take_ready_immediate_sections(
            1,
            &mut immediate,
            &mut dirty,
            &lit,
            &lighting,
            |_, _| true,
            |_, _| true,
        );
        assert!(ready.is_empty());
        assert!(immediate.is_empty());
        assert_eq!(dirty.len(), IMMEDIATE_MESH_LIMIT + 1);
    }

    #[test]
    fn take_ready_immediate_skips_columns_with_unlit_or_lighting_neighbours() {
        let mut immediate = DirtySections::default();
        let mut dirty = DirtySections::default();
        let mut lit = FxHashSet::default();
        let mut lighting = FxHashMap::default();
        for dz in -1..=1 {
            for dx in -1..=1 {
                lit.insert((dx, dz));
            }
        }
        lighting.insert((1, 0), 7);
        immediate.insert((0, 4, 0));
        dirty.insert((0, 4, 0));

        let ready = take_ready_immediate_sections(
            1,
            &mut immediate,
            &mut dirty,
            &lit,
            &lighting,
            |_, _| true,
            |_, _| true,
        );
        assert!(ready.is_empty());
        assert!(immediate.is_empty());
        assert!(dirty.contains(&(0, 4, 0)));
    }

    #[test]
    fn take_ready_immediate_accepts_sections_already_dispatched_to_workers() {
        let mut immediate = DirtySections::default();
        let mut dirty = DirtySections::default();
        let mut lit = FxHashSet::default();
        let lighting = FxHashMap::default();
        for dz in -1..=1 {
            for dx in -1..=1 {
                lit.insert((dx, dz));
            }
        }
        // Simulate an edit whose dirty entry was already consumed by streaming worker dispatch.
        immediate.insert((0, 4, 0));

        let ready = take_ready_immediate_sections(
            1,
            &mut immediate,
            &mut dirty,
            &lit,
            &lighting,
            |_, _| true,
            |_, _| true,
        );
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].0, (0, 0));
        assert!(ready[0].1.contains(4));
        assert!(immediate.is_empty());
    }
}
