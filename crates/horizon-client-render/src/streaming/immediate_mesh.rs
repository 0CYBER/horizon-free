//! Synchronous section meshing for individual block edits (`World::immediate_sections`).
//!
//! Vanilla's `LevelRenderer.compileSections` / `SectionRenderDispatcher.rebuildSectionSync`
//! rebuilds player-edited sections on the render thread before drawing the frame so a placed
//! or broken block appears on the same frame as its selection outline instead of lagging one
//! or more frames behind on worker threads. With `prioritizeChunkUpdates == 2` nearby dirty
//! sections are rebuilt there too, within `NEARBY_MESH_BUDGET`.
use super::{INLINE_RELIGHT_MAX, Streaming, mesh_section};
use crate::{
    fast_hash::{FxHashMap, FxHashSet},
    mesher::{Region, SectionMesh},
    section_mask::SectionMask,
    world::{DirtySections, World},
};
use std::{sync::Arc, time::Duration};

/// Maximum number of sections rebuilt synchronously in one frame, for player edits and for
/// nearby sections each.
///
/// A single block edit on a chunk/section corner touches at most 8 sections; larger bursts
/// (bulk updates or scripts calling `set_block` in a loop) fall back to the background worker
/// pool via `World::dirty_sections` so they never stall a frame.
pub const IMMEDIATE_MESH_LIMIT: usize = 8;

/// Vanilla `SectionRenderDispatcher` squared distance threshold for `PrioritizeChunkUpdates.NEARBY`.
pub const NEARBY_IMMEDIATE_DIST_SQ: f64 = 768.0;

/// Render-thread time per frame after which nearby (`prioritizeChunkUpdates == 2`) sections
/// are left to the worker pool.
///
/// While terrain streams in around the player, up to `IMMEDIATE_MESH_LIMIT` nearby sections
/// stay dirty every frame; meshing all of them on a weak laptop CPU cost several milliseconds
/// per frame. Player edits never wait on it, and the section that crosses it still finishes,
/// so a frame can overshoot it by one section mesh; `NEARBY_COOLDOWN_FRAMES` keeps that
/// overshoot from repeating every frame.
pub const NEARBY_MESH_BUDGET: Duration = Duration::from_millis(2);

/// Frames without synchronous nearby meshing after a call's nearby meshing ended over
/// `NEARBY_MESH_BUDGET`.
///
/// The budget is checked before each section, so one section that alone costs more than the
/// budget (dense terrain on a slow CPU) would otherwise be paid every frame. Eight frames
/// leave the backlog to the workers for about a quarter second at 30 fps (less at higher
/// frame rates), short enough that the setting still visibly favours nearby sections.
pub const NEARBY_COOLDOWN_FRAMES: u8 = 8;

/// Counts frames left before nearby sections are meshed synchronously again.
#[derive(Default)]
pub(super) struct NearbyCooldown {
    frames_left: u8,
}

impl NearbyCooldown {
    /// Called once per `prioritizeChunkUpdates == 2` frame: whether nearby sections may be
    /// meshed synchronously this frame, counting a skipped frame down.
    fn frame_allowed(&mut self) -> bool {
        if self.frames_left == 0 {
            return true;
        }
        self.frames_left -= 1;
        false
    }
    fn arm(&mut self) {
        self.frames_left = NEARBY_COOLDOWN_FRAMES;
    }
}

/// Whether column `(x, z)` and its eight neighbours are lit and not being relit, so a mesh
/// built now carries final light.
fn neighbours_lit(
    lit: &FxHashSet<(i32, i32)>,
    lighting: &FxHashMap<(i32, i32), u64>,
    x: i32,
    z: i32,
) -> bool {
    (-1..=1).all(|dz| {
        (-1..=1).all(|dx| {
            let n = (x + dx, z + dz);
            lit.contains(&n) && !lighting.contains_key(&n)
        })
    })
}

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
        if !has_chunk(x, z) || !in_view(x, z) || !neighbours_lit(lit, lighting, x, z) {
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

/// Mesh every `player` section, then `nearby` sections in order while `elapsed()` is under
/// `budget`. A nearby section leaves `dirty` only when it is meshed, so the rest stay queued
/// for the worker pool; one already gone from `dirty` (taken as a player edit, or dispatched
/// to a worker) is skipped so an in-flight job for it is not superseded. Player work counts
/// against the budget too; if nearby meshing ends over it, `cooldown` is armed.
fn mesh_within_budget(
    player: &[(i32, i32, i32)],
    nearby: &[(i32, i32, i32)],
    dirty: &mut DirtySections,
    cooldown: &mut NearbyCooldown,
    budget: Duration,
    elapsed: impl Fn() -> Duration,
    mut mesh: impl FnMut((i32, i32, i32)),
) {
    for &key in player {
        mesh(key);
    }
    let mut meshed_nearby = false;
    for &key in nearby {
        if elapsed() >= budget {
            break;
        }
        if dirty.remove(key) {
            mesh(key);
            meshed_nearby = true;
        }
    }
    if meshed_nearby && elapsed() > budget {
        cooldown.arm();
    }
}

impl Streaming {
    /// Called once per `prioritizeChunkUpdates == 2` frame before collecting nearby sections:
    /// false while `NEARBY_COOLDOWN_FRAMES` is counting down, when the caller passes none.
    pub fn nearby_meshing_due(&mut self) -> bool {
        self.nearby_cooldown.frame_allowed()
    }

    /// Synchronously rebuild sections dirtied by individual block edits since `update_world`,
    /// gated by `prioritize_chunk_updates` (`0` = Threaded/none, `1` = Semi-Blocking/player-edited,
    /// `2` = Fully Blocking/nearby), returning meshes ready for immediate GPU upload. `nearby`
    /// (mode `2`, nearest first, from `World::nearby_dirty_sections`) is meshed after the player
    /// edits within `NEARBY_MESH_BUDGET`; an overrun pauses them for `NEARBY_COOLDOWN_FRAMES`.
    pub fn mesh_immediate(
        &mut self,
        world: &mut World,
        prioritize_chunk_updates: u8,
        nearby: &[(i32, i32, i32)],
    ) -> Vec<((i32, i32, i32), SectionMesh)> {
        let start = std::time::Instant::now();
        let nearby: &[_] = if prioritize_chunk_updates >= 2 {
            nearby
        } else {
            &[]
        };
        if prioritize_chunk_updates == 0
            || !self.mesh
            || (world.immediate_sections.is_empty() && nearby.is_empty())
        {
            world.immediate_sections.clear();
            return Vec::new();
        }
        if !world.light_edits.is_empty() {
            if world.light_edits.len() > INLINE_RELIGHT_MAX {
                // Pending lighting exceeds the inline budget: leave all affected sections dirty
                // for the normal relighting and worker meshing path instead of uploading stale light.
                // Nearby sections never left `dirty`.
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
        let mut regions: FxHashMap<(i32, i32), Arc<Region>> = FxHashMap::default();
        let mut player = Vec::new();
        for ((x, z), sections) in ready_columns {
            let Some(region) = Self::region(world, x, z) else {
                // Keep sections dirty for workers if a neighbour column disappeared.
                for sy in sections.iter().map(|sy| sy as i32) {
                    world.dirty_sections.insert((x, sy, z));
                }
                continue;
            };
            regions.insert((x, z), region);
            player.extend(sections.iter().map(|sy| (x, sy as i32, z)));
        }
        let nearby: Vec<_> = {
            let ranges = self.ranges();
            nearby
                .iter()
                .copied()
                .filter(|&(x, _, z)| {
                    ranges.in_view(x, z) && neighbours_lit(&self.lit, &self.lighting, x, z)
                })
                .filter(|&(x, _, z)| {
                    if regions.contains_key(&(x, z)) {
                        return true;
                    }
                    let Some(region) = Self::region(world, x, z) else {
                        return false;
                    };
                    regions.insert((x, z), region);
                    true
                })
                .collect()
        };
        if player.is_empty() && nearby.is_empty() {
            return Vec::new();
        }
        let context = self.shared.context.read().unwrap().clone();
        let settings = self.mesh_settings();
        let mut uploads = Vec::new();
        mesh_within_budget(
            &player,
            &nearby,
            &mut world.dirty_sections,
            &mut self.nearby_cooldown,
            NEARBY_MESH_BUDGET,
            || start.elapsed(),
            |(x, sy, z)| {
                // Invalidate any older in-flight worker job for this section so its stale
                // `Done::Meshed` is discarded when it finishes.
                self.meshing.remove(&(x, sy, z));
                let recycled = self.shared.meshes.lock().unwrap().take();
                let mesh = mesh_section(
                    &context,
                    &settings,
                    &mut self.immediate_faces,
                    &mut self.immediate_scratch,
                    &regions[&(x, z)],
                    sy,
                    recycled,
                );
                self.sequence += 1;
                self.compile_started.insert((x, sy, z), self.sequence);
                self.meshed_total += 1;
                uploads.push(((x, sy, z), mesh));
            },
        );
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
    fn nearby_mode_lists_dirty_sections_within_768_sq_blocks_nearest_first() {
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        world.chunks.insert((1, 0), Arc::new(Chunk::empty(1, 0)));
        world.chunks.insert((5, 0), Arc::new(Chunk::empty(5, 0)));
        world.set(24, 72, 8, BlockStateId(1)).unwrap();
        world.set(8, 72, 8, BlockStateId(1)).unwrap();
        world.set(88, 72, 8, BlockStateId(1)).unwrap();

        let nearby =
            world.nearby_dirty_sections([8, 72, 8], NEARBY_IMMEDIATE_DIST_SQ, IMMEDIATE_MESH_LIMIT);
        let sy = (72 - world.bottom_y()) >> 4;
        assert_eq!(nearby, vec![(0, sy, 0), (1, sy, 0)]);
        // Nearby sections are not player edits: they never enter `immediate_sections`.
        assert!(world.immediate_sections.is_empty());
        assert_eq!(
            world.nearby_dirty_sections([8, 72, 8], NEARBY_IMMEDIATE_DIST_SQ, 1),
            vec![(0, sy, 0)]
        );
    }

    #[test]
    fn nearby_lookup_skips_far_columns_and_keeps_the_in_range_set() {
        let mut world = World::new(BlockStateId(0), 0);
        let sy = (64 - world.bottom_y()) >> 4;
        for (x, z) in [
            (0, 0),
            (1, 0),
            (-2, 0),
            (2, 0),
            (1, 1),
            (100, 100),
            (-100, 3),
        ] {
            world.dirty_sections.insert((x, sy, z));
        }

        // Camera at (0, 64, 0): column centres at x = 24 and -24 are 704 squared blocks
        // away, (2, 0) and the diagonal (1, 1) are past 768.
        assert_eq!(
            world.nearby_dirty_sections([0, 64, 0], NEARBY_IMMEDIATE_DIST_SQ, IMMEDIATE_MESH_LIMIT),
            vec![(0, sy, 0), (-2, sy, 0), (1, sy, 0)]
        );
    }

    /// Runs `mesh_within_budget` with a clock that advances `per_mesh` for every section
    /// meshed, returning the sections meshed in order.
    fn mesh_with_clock(
        player: &[(i32, i32, i32)],
        nearby: &[(i32, i32, i32)],
        dirty: &mut DirtySections,
        budget: Duration,
        per_mesh: Duration,
    ) -> Vec<(i32, i32, i32)> {
        mesh_with_costs(
            player,
            nearby,
            dirty,
            &mut NearbyCooldown::default(),
            budget,
            |_| per_mesh,
        )
    }

    /// `mesh_within_budget` with a clock that advances `cost(key)` for every section meshed.
    fn mesh_with_costs(
        player: &[(i32, i32, i32)],
        nearby: &[(i32, i32, i32)],
        dirty: &mut DirtySections,
        cooldown: &mut NearbyCooldown,
        budget: Duration,
        cost: impl Fn((i32, i32, i32)) -> Duration,
    ) -> Vec<(i32, i32, i32)> {
        let clock = std::cell::Cell::new(Duration::ZERO);
        let mut meshed = Vec::new();
        mesh_within_budget(
            player,
            nearby,
            dirty,
            cooldown,
            budget,
            || clock.get(),
            |key| {
                clock.set(clock.get() + cost(key));
                meshed.push(key);
            },
        );
        meshed
    }

    #[test]
    fn player_edits_are_meshed_with_no_budget_and_nearby_ones_stay_dirty() {
        let mut dirty = DirtySections::default();
        // The player edit was taken out of `dirty` by `take_ready_immediate_sections`.
        let player = [(0, 4, 0)];
        let nearby = [(0, 5, 0), (1, 4, 0)];
        for key in nearby {
            dirty.insert(key);
        }

        let meshed = mesh_with_clock(
            &player,
            &nearby,
            &mut dirty,
            Duration::ZERO,
            Duration::from_millis(1),
        );
        assert_eq!(meshed, vec![(0, 4, 0)]);
        assert!(dirty.contains(&(0, 5, 0)));
        assert!(dirty.contains(&(1, 4, 0)));
        assert_eq!(dirty.len(), 2);
    }

    #[test]
    fn nearby_sections_past_the_budget_go_back_to_the_workers() {
        let mut dirty = DirtySections::default();
        let nearby = [(0, 4, 0), (0, 5, 0), (1, 4, 0), (1, 5, 0)];
        for key in nearby {
            dirty.insert(key);
        }

        let meshed = mesh_with_clock(
            &[],
            &nearby,
            &mut dirty,
            Duration::from_millis(2),
            Duration::from_millis(1),
        );
        assert_eq!(meshed, vec![(0, 4, 0), (0, 5, 0)]);
        assert!(!dirty.contains(&(0, 4, 0)));
        assert!(!dirty.contains(&(0, 5, 0)));
        assert!(dirty.contains(&(1, 4, 0)));
        assert!(dirty.contains(&(1, 5, 0)));
        assert_eq!(dirty.len(), 2);
    }

    #[test]
    fn nearby_sections_within_the_budget_are_all_meshed_once() {
        let mut dirty = DirtySections::default();
        let player = [(0, 4, 0)];
        // (0, 4, 0) is both a player edit and nearby; (2, 4, 0) is nearby but already
        // dispatched to a worker, so it is no longer in `dirty`.
        let nearby = [(0, 4, 0), (0, 5, 0), (1, 4, 0), (2, 4, 0)];
        dirty.insert((0, 5, 0));
        dirty.insert((1, 4, 0));
        dirty.insert((3, 4, 0));

        let meshed = mesh_with_clock(
            &player,
            &nearby,
            &mut dirty,
            NEARBY_MESH_BUDGET,
            Duration::ZERO,
        );
        assert_eq!(meshed, vec![(0, 4, 0), (0, 5, 0), (1, 4, 0)]);
        assert!(dirty.contains(&(3, 4, 0)));
        assert_eq!(dirty.len(), 1);
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

    #[test]
    fn nearby_overrun_counts_player_work_and_arms_the_cooldown() {
        let mut dirty = DirtySections::default();
        let mut cooldown = NearbyCooldown::default();
        let player = [(0, 4, 0)];
        let nearby = [(0, 5, 0), (1, 4, 0)];
        for key in nearby {
            dirty.insert(key);
        }

        // 1 ms of player work leaves 1 ms of budget: the first nearby mesh starts, takes
        // 2 ms and ends the call at 3 ms, over the 2 ms budget.
        let meshed = mesh_with_costs(
            &player,
            &nearby,
            &mut dirty,
            &mut cooldown,
            Duration::from_millis(2),
            |key| Duration::from_millis(if key == (0, 4, 0) { 1 } else { 2 }),
        );
        assert_eq!(meshed, vec![(0, 4, 0), (0, 5, 0)]);
        assert!(dirty.contains(&(1, 4, 0)));
        assert!(!cooldown.frame_allowed());
    }

    #[test]
    fn cooldown_leaves_nearby_sections_to_workers_then_resumes() {
        let mut dirty = DirtySections::default();
        let mut cooldown = NearbyCooldown::default();
        cooldown.arm();
        let nearby = [(1, 4, 0)];
        dirty.insert((1, 4, 0));

        // What `flush_immediate_meshes` does each frame: no nearby list while cooling down.
        let mut frame = |player: &[(i32, i32, i32)], cooldown: &mut NearbyCooldown| {
            let nearby: &[_] = if cooldown.frame_allowed() {
                &nearby
            } else {
                &[]
            };
            mesh_with_costs(
                player,
                nearby,
                &mut dirty,
                cooldown,
                NEARBY_MESH_BUDGET,
                |_| Duration::ZERO,
            )
        };
        for i in 0..NEARBY_COOLDOWN_FRAMES as i32 {
            let player = [(0, i, 0)];
            assert_eq!(frame(&player, &mut cooldown), vec![(0, i, 0)]);
        }
        assert_eq!(frame(&[], &mut cooldown), vec![(1, 4, 0)]);
        assert!(dirty.is_empty());
    }
}
