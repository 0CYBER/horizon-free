//! `SectionOcclusionGraph`: which sections the camera can see through open
//! faces. Breadth-first from the camera section through the faces each
//! section's VisGraph connects, never stepping back towards the camera. As in
//! vanilla's `runUpdates`, a section leads on through whatever any of the faces
//! it was entered through connects to, and beyond 60 blocks a step is taken
//! only if the line from the section towards the camera stays in sections the
//! search has reached.
//!
//! As in vanilla, the full search runs off the render thread: a worker keeps
//! its own copy of every section's connectivity and searches over dense
//! arrays around the camera. Frames use the last finished graph and never wait
//! for the next one. A section meshed in between propagates into the graph the
//! frame has (vanilla's partial update), so what it opens shows at once; what
//! it closes goes at the next full search.
//!
//! GPU culling (gfx/gpu_culling.rs) answers as `contains` does from a copy of
//! the graph (`Reach`): the cells it reached, kept as bits too, sent again
//! whenever the generation moves, which it does whenever an answer may change.
use super::SectionKey;
use crate::fast_hash::{FxHashMap, FxHashSet};
use crate::mesher::ALL_VISIBLE;
use glam::DVec3;
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

/// Vanilla's `Direction` order: down, up, north, south, west, east.
#[cfg(test)]
const STEPS: [(i32, i32, i32); 6] = [
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
    (-1, 0, 0),
    (1, 0, 0),
];
/// In `Graph::directions`: the cell has a node.
const REACHED: u8 = 1 << 7;
/// `MINIMUM_ADVANCED_CULLING_SECTION_DISTANCE`: `blockToSectionCoord(60)`.
const DISTANT_SECTIONS: i32 = 60 >> 4;
/// `MINIMUM_ADVANCED_CULLING_DISTANCE` (60 blocks), squared.
const DISTANT_SQR: f64 = 3600.0;
/// `CEILED_SECTION_DIAGONAL`: `ceil(sqrt(3) * 16)`.
const SECTION_DIAGONAL: f64 = 28.0;
/// In `Graph::rays`: the line from this corner stays in reached sections.
const SEES: u8 = u8::MAX;
/// Edits kept before a full resend.
const EDIT_CAP: usize = 1 << 16;

/// Sentinel slab slot index meaning no column is stored.
const EMPTY_SLOT: u32 = u32::MAX;
/// Initial toroidal grid shift (16x16 = 256 cells, 4 KiB).
const MIN_GRID_SHIFT: u8 = 4;
/// Largest toroidal grid kept across `clear` (128x128 = 16,384 cells, 256 KiB;
/// covers render distance 32+ with zero collisions).
const KEEP_GRID_MAX_LEN: usize = 1 << (7 * 2);

#[derive(Clone, Copy)]
struct GridSlot {
    key_packed: u64,
    /// Slab slot index of the cached resident column, or `EMPTY_SLOT`.
    slot: u32,
    /// Number of other resident columns in `Columns::map` hashing to this
    /// toroidal cell. When 0, this cell is authoritative: a miss proves the
    /// column is absent without probing `map`.
    overflow: u32,
}

impl GridSlot {
    const EMPTY: Self = Self {
        key_packed: 0,
        slot: EMPTY_SLOT,
        overflow: 0,
    };
}

#[inline(always)]
fn pack_key(wx: i32, wz: i32) -> u64 {
    u64::from(wx as u32) | (u64::from(wz as u32) << 32)
}

#[inline(always)]
fn grid_index(wx: i32, wz: i32, mask: i32, shift: u8) -> usize {
    (((wz & mask) as usize) << shift) | ((wx & mask) as usize)
}

#[inline]
fn desired_grid_shift(count: usize) -> u8 {
    if count > 8192 {
        8
    } else if count > 2048 {
        7
    } else if count > 512 {
        6
    } else if count > 128 {
        5
    } else {
        MIN_GRID_SHIFT
    }
}

/// Contiguous slab storing per-column section visibility masks (`rows`) and
/// each column's highest non-air section index (`max_non_air`), eliminating
/// one `Box<[u64]>` allocation per loaded column.
struct ColumnSlab {
    /// Sections per column (`sections.max(0) as usize`).
    stride: usize,
    /// Contiguous `[slot * stride + y]` visibility masks.
    rows: Vec<u64>,
    /// Per-slot highest `y` whose visibility mask is not `ALL_VISIBLE`,
    /// saturated at `u8::MAX` when `stride > 256`.
    max_non_air: Vec<u8>,
    /// Recycled slot indices freed when columns become fully open or unload.
    free_slots: Vec<u32>,
}

impl ColumnSlab {
    fn new(sections: i32) -> Self {
        Self {
            stride: sections.max(0) as usize,
            rows: Vec::new(),
            max_non_air: Vec::new(),
            free_slots: Vec::new(),
        }
    }

    fn alloc(&mut self) -> u32 {
        if let Some(slot) = self.free_slots.pop() {
            let base = slot as usize * self.stride;
            self.rows[base..base + self.stride].fill(ALL_VISIBLE);
            self.max_non_air[slot as usize] = 0;
            slot
        } else {
            let slot =
                u32::try_from(self.max_non_air.len()).expect("occlusion column slab overflow");
            assert!(slot != EMPTY_SLOT, "occlusion column slab overflow");
            self.rows.resize(self.rows.len() + self.stride, ALL_VISIBLE);
            self.max_non_air.push(0);
            slot
        }
    }

    #[inline]
    fn free(&mut self, slot: u32) {
        self.free_slots.push(slot);
    }

    #[inline]
    fn column(&self, slot: u32) -> &[u64] {
        let base = slot as usize * self.stride;
        &self.rows[base..base + self.stride]
    }

    /// Number of bottom sections that may differ from `ALL_VISIBLE`.
    #[inline]
    fn scan_len(&self, slot: u32) -> usize {
        let m = usize::from(self.max_non_air[slot as usize]);
        if m == usize::from(u8::MAX) && self.stride > 256 {
            self.stride
        } else {
            m + 1
        }
    }

    /// Exact highest non-air section `y` for `slot` when `stride > 256` and
    /// `max_non_air[slot]` saturated at `u8::MAX`.
    #[cold]
    fn exact_max_non_air_y(&self, slot: u32) -> i32 {
        self.column(slot)
            .iter()
            .rposition(|&v| v != ALL_VISIBLE)
            .unwrap_or(0) as i32
    }

    fn clear(&mut self) {
        let shrink = self.max_non_air.capacity() > KEEP_GRID_MAX_LEN;
        self.rows.clear();
        self.max_non_air.clear();
        self.free_slots.clear();
        if shrink {
            self.rows.shrink_to_fit();
            self.max_non_air.shrink_to_fit();
            self.free_slots.shrink_to_fit();
        }
    }
}

/// VisGraph connectivity per column, one entry per section (bottom first).
/// A missing column or section connects every face.
struct Columns {
    /// The lowest block y of section 0 (`Level.getMinY`).
    min_y: i32,
    sections: i32,
    slab: ColumnSlab,
    /// Toroidal power-of-two direct-mapped cache for O(1) hash-free column
    /// lookups (`(wz & grid_mask) << grid_shift | (wx & grid_mask)`).
    grid: Box<[GridSlot]>,
    grid_mask: i32,
    grid_shift: u8,
    /// Authoritative `(wx, wz) -> slab slot` index for toroidal collisions
    /// (`overflow > 0`) and full scans/resyncs.
    map: FxHashMap<(i32, i32), u32>,
}

impl Columns {
    fn new(min_y: i32, sections: i32) -> Self {
        Self {
            min_y,
            sections,
            slab: ColumnSlab::new(sections),
            grid: Box::new([]),
            grid_mask: 0,
            grid_shift: 0,
            map: FxHashMap::default(),
        }
    }
    /// `SectionPos.of`: the section holding `pos`.
    fn section_of(&self, pos: DVec3) -> SectionKey {
        (
            (pos.x.floor() as i32) >> 4,
            (pos.y.floor() as i32 - self.min_y) >> 4,
            (pos.z.floor() as i32) >> 4,
        )
    }
    fn rebuild_grid(&mut self, shift: u8) {
        let side = 1usize << shift;
        let len = side * side;
        if self.grid.len() != len {
            self.grid = vec![GridSlot::EMPTY; len].into_boxed_slice();
        } else {
            self.grid.fill(GridSlot::EMPTY);
        }
        self.grid_shift = shift;
        self.grid_mask = (side - 1) as i32;
        for (&(wx, wz), &slot) in &self.map {
            let idx = grid_index(wx, wz, self.grid_mask, self.grid_shift);
            let cell = &mut self.grid[idx];
            if cell.slot != EMPTY_SLOT {
                cell.overflow += 1;
            }
            cell.key_packed = pack_key(wx, wz);
            cell.slot = slot;
        }
    }
    fn insert_grid_new(&mut self, wx: i32, wz: i32, slot: u32) {
        let target = desired_grid_shift(self.map.len());
        if self.grid_mask == 0 || target > self.grid_shift {
            self.rebuild_grid(target);
            return;
        }
        let idx = grid_index(wx, wz, self.grid_mask, self.grid_shift);
        let cell = &mut self.grid[idx];
        if cell.slot != EMPTY_SLOT {
            cell.overflow += 1;
        }
        cell.key_packed = pack_key(wx, wz);
        cell.slot = slot;
    }
    #[inline(always)]
    fn get_slot(&self, wx: i32, wz: i32) -> u32 {
        let mask = self.grid_mask;
        if mask == 0 {
            debug_assert!(self.map.is_empty());
            return EMPTY_SLOT;
        }
        let cell = self.grid[grid_index(wx, wz, mask, self.grid_shift)];
        if cell.slot != EMPTY_SLOT && cell.key_packed == pack_key(wx, wz) {
            return cell.slot;
        }
        if cell.overflow == 0 {
            return EMPTY_SLOT;
        }
        self.map.get(&(wx, wz)).copied().unwrap_or(EMPTY_SLOT)
    }
    fn detach(&mut self, wx: i32, wz: i32) -> Option<u32> {
        let slot = self.map.remove(&(wx, wz))?;
        if self.grid_mask != 0 {
            let idx = grid_index(wx, wz, self.grid_mask, self.grid_shift);
            let cell = &mut self.grid[idx];
            if cell.slot != EMPTY_SLOT && cell.key_packed == pack_key(wx, wz) {
                cell.slot = EMPTY_SLOT;
            } else {
                debug_assert!(cell.overflow > 0);
                cell.overflow -= 1;
            }
        }
        Some(slot)
    }
    /// Whether the section's connectivity changed.
    fn set(&mut self, key: SectionKey, vis: u64) -> bool {
        if !(0..self.sections).contains(&key.1) {
            return false;
        }
        let y = key.1 as usize;
        let slot = self.get_slot(key.0, key.2);
        if slot == EMPTY_SLOT {
            if vis == ALL_VISIBLE {
                return false;
            }
            let new_slot = self.slab.alloc();
            let base = new_slot as usize * self.slab.stride;
            self.slab.rows[base + y] = vis;
            self.slab.max_non_air[new_slot as usize] = y.min(usize::from(u8::MAX)) as u8;
            self.map.insert((key.0, key.2), new_slot);
            self.insert_grid_new(key.0, key.2, new_slot);
            return true;
        }
        let base = slot as usize * self.slab.stride;
        let entry = &mut self.slab.rows[base + y];
        if *entry == vis {
            return false;
        }
        *entry = vis;
        if vis != ALL_VISIBLE {
            let max_y = &mut self.slab.max_non_air[slot as usize];
            if y > usize::from(*max_y) {
                *max_y = y.min(usize::from(u8::MAX)) as u8;
            }
            // Opportunistically promote an overflowed column into an empty
            // toroidal cell so future lookups avoid `map`.
            if self.grid_mask != 0 {
                let idx = grid_index(key.0, key.2, self.grid_mask, self.grid_shift);
                let cell = &mut self.grid[idx];
                if cell.slot == EMPTY_SLOT {
                    debug_assert!(cell.overflow > 0);
                    cell.key_packed = pack_key(key.0, key.2);
                    cell.slot = slot;
                    cell.overflow -= 1;
                }
            }
            return true;
        }
        let cur_max = usize::from(self.slab.max_non_air[slot as usize]);
        if y < cur_max {
            return true;
        }
        let scan_end = if cur_max == usize::from(u8::MAX) && self.slab.stride > 256 {
            self.slab.stride
        } else {
            y
        };
        match self.slab.rows[base..base + scan_end]
            .iter()
            .rposition(|&v| v != ALL_VISIBLE)
        {
            Some(new_max) => {
                self.slab.max_non_air[slot as usize] = new_max.min(usize::from(u8::MAX)) as u8;
            }
            None => {
                debug_assert!(
                    self.slab.rows[base..base + self.slab.stride]
                        .iter()
                        .all(|&v| v == ALL_VISIBLE)
                );
                let detached = self.detach(key.0, key.2);
                debug_assert_eq!(detached, Some(slot));
                self.slab.free(slot);
            }
        }
        true
    }
    #[inline]
    fn get(&self, key: SectionKey) -> u64 {
        let Ok(y) = usize::try_from(key.1) else {
            return ALL_VISIBLE;
        };
        if y >= self.slab.stride {
            return ALL_VISIBLE;
        }
        let slot = self.get_slot(key.0, key.2);
        if slot == EMPTY_SLOT {
            return ALL_VISIBLE;
        }
        self.slab.rows[slot as usize * self.slab.stride + y]
    }
    fn clear(&mut self) {
        let had_columns = !self.map.is_empty();
        self.map.clear();
        self.slab.clear();
        if self.grid.len() > KEEP_GRID_MAX_LEN {
            self.grid = Box::new([]);
            self.grid_mask = 0;
            self.grid_shift = 0;
        } else if had_columns && self.grid_mask != 0 {
            self.grid.fill(GridSlot::EMPTY);
        }
    }
    fn apply(&mut self, edit: Edit) {
        match edit {
            Edit::Set(key, vis) => {
                self.set(key, vis);
            }
            Edit::Clear => self.clear(),
        }
    }
    #[cfg(test)]
    fn assert_invariants(&self) {
        assert_eq!(
            self.slab.rows.len(),
            self.slab.max_non_air.len() * self.slab.stride,
            "slab rows length matches slot count * stride"
        );
        assert_eq!(
            self.map.len() + self.slab.free_slots.len(),
            self.slab.max_non_air.len(),
            "live + free slots equals allocated slots"
        );
        let free: FxHashSet<u32> = self.slab.free_slots.iter().copied().collect();
        assert_eq!(
            free.len(),
            self.slab.free_slots.len(),
            "no duplicate free slots"
        );
        for (&(wx, wz), &slot) in &self.map {
            assert!((slot as usize) < self.slab.max_non_air.len());
            assert!(!free.contains(&slot), "live slot {slot} in free list");
            let col = self.slab.column(slot);
            let exact_max = col
                .iter()
                .rposition(|&v| v != ALL_VISIBLE)
                .expect("live column has at least one non-air section");
            assert_eq!(
                usize::from(self.slab.max_non_air[slot as usize]),
                exact_max.min(usize::from(u8::MAX)),
                "max_non_air mismatch for ({wx}, {wz})"
            );
            if self.slab.max_non_air[slot as usize] == u8::MAX && self.slab.stride > 256 {
                assert_eq!(self.slab.exact_max_non_air_y(slot), exact_max as i32);
            }
            assert_eq!(self.get_slot(wx, wz), slot);
        }
        if self.grid_mask == 0 {
            assert!(self.map.is_empty(), "empty grid mask with live columns");
        } else {
            let side = 1usize << self.grid_shift;
            assert_eq!(self.grid.len(), side * side);
            assert_eq!(self.grid_mask, (side - 1) as i32);
            let mut counts = vec![0usize; self.grid.len()];
            for &(wx, wz) in self.map.keys() {
                counts[grid_index(wx, wz, self.grid_mask, self.grid_shift)] += 1;
            }
            for (idx, (&cell, &expected)) in self.grid.iter().zip(&counts).enumerate() {
                let present = usize::from(cell.slot != EMPTY_SLOT);
                assert_eq!(
                    present + cell.overflow as usize,
                    expected,
                    "toroidal cell {idx} accounting mismatch"
                );
                if cell.slot != EMPTY_SLOT {
                    let wx = cell.key_packed as u32 as i32;
                    let wz = (cell.key_packed >> 32) as u32 as i32;
                    assert_eq!(
                        grid_index(wx, wz, self.grid_mask, self.grid_shift),
                        idx,
                        "cached key in wrong toroidal cell"
                    );
                    assert_eq!(
                        self.map.get(&(wx, wz)).copied(),
                        Some(cell.slot),
                        "cached slot disagrees with authoritative map"
                    );
                }
            }
        }
    }
}

/// Reusable per-search scratch buffers held by the worker thread (and
/// `Occlusion` for render-thread partial updates and fallback searches),
/// avoiding per-search heap allocations.
#[derive(Default)]
struct SearchScratch {
    /// Per `(dz * side + dx)` column in the searched box: slab slot index in
    /// `ColumnSlab::rows`, or `EMPTY_SLOT` when the column is absent.
    col_slots: Vec<u32>,
    /// Reusable buffer for `Graph::seed_outside` when the camera is above or
    /// below the world.
    seeds: Vec<(i64, i32, i32, u8)>,
    /// Reusable BFS frontier queue.
    queue: VecDeque<QueueNode>,
}

#[derive(Clone, Copy)]
enum Edit {
    Set(SectionKey, u64),
    Clear,
}

#[derive(Clone, Copy)]
struct QueueNode {
    index: u32,
    ux: u16,
    uy: u16,
    uz: u16,
}

/// One search's result over the box around the camera: vanilla's
/// `GraphStorage.sectionToNodeMap`, as dense arrays.
#[derive(Default)]
struct Graph {
    from: SectionKey,
    radius: i32,
    /// Precomputed `from.0 - radius` and `from.2 - radius` for branch-minimal indexing.
    origin_x: i32,
    origin_z: i32,
    /// Precomputed `2 * radius + 1` (0 until searched).
    side: u32,
    /// Precomputed `height as u32` (0 until searched).
    height_u: u32,
    /// The world: lowest block y and height in sections.
    min_y: i32,
    sections: i32,
    /// The searched box: lowest section y and its height in sections.
    y0: i32,
    height: i32,
    /// Per cell: `REACHED`, and the directions travelled from the camera to
    /// it (`Node.directions`).
    directions: Vec<u8>,
    /// Per cell: the faces it was entered through, one bit per face
    /// (`Node.sourceDirections`, as the faces rather than the steps); none for
    /// the camera's.
    entered: Vec<u8>,
    /// `REACHED` of each cell again, a bit each at `index` (32 a word, lowest
    /// bit first): the GPU's copy (`Occlusion::reach`), written with it
    /// (`set_reached`).
    bits: Vec<u32>,
    /// Whether any section in the searched top layer was reached. Sections
    /// above a clamped box use this conservative sky-band answer.
    sky_reached: bool,
    /// Per section corner, during a full search: `SEES`, or how far the line
    /// from it towards the camera is known to stay in reached sections (the
    /// first sample that was not). Sections are only ever added during a
    /// search, so a line that stays in reached sections keeps doing so, and
    /// one that didn't need only be checked again from where it failed.
    rays: Vec<u8>,
}

impl Graph {
    #[inline]
    fn node(&self, key: SectionKey) -> Option<QueueNode> {
        let ux = key.0.wrapping_sub(self.origin_x) as u32;
        let uz = key.2.wrapping_sub(self.origin_z) as u32;
        let uy = key.1.wrapping_sub(self.y0) as u32;
        if ux >= self.side || uz >= self.side || uy >= self.height_u {
            return None;
        }
        let index = (uz * self.side + ux) * self.height_u + uy;
        Some(QueueNode {
            index,
            ux: ux as u16,
            uy: uy as u16,
            uz: uz as u16,
        })
    }
    #[cfg(test)]
    #[inline]
    fn index(&self, key: SectionKey) -> Option<usize> {
        self.node(key).map(|n| n.index as usize)
    }
    /// Whether cell `i` was reached, reading the 8x denser L1-resident `bits`
    /// array kept in lockstep with `directions` by `set_reached`.
    #[inline(always)]
    fn is_reached(&self, i: usize) -> bool {
        (self.bits[i >> 5] >> (i & 31)) & 1 != 0
    }
    /// Whether `key` is reached by this graph, reading the 8x denser
    /// L1-resident `bits` array kept in lockstep with `directions` by
    /// `set_reached`.
    #[inline]
    fn contains_bit(&self, key: SectionKey) -> bool {
        let ux = key.0.wrapping_sub(self.origin_x) as u32;
        let uz = key.2.wrapping_sub(self.origin_z) as u32;
        if ux >= self.side || uz >= self.side {
            return false;
        }
        let uy = key.1.wrapping_sub(self.y0) as u32;
        if uy < self.height_u {
            let i = ((uz * self.side + ux) * self.height_u + uy) as usize;
            self.is_reached(i)
        } else {
            // Everything above the highest non-air section is open sky. A
            // full search can reach that layer from a neighbouring column,
            // so checking only this column's top cell can hide an upper
            // section that is visible through the open layer. Once any top
            // cell is reached, conservatively keep the whole sky band visible.
            self.sky_reached
                && key.1 >= self.y0 + self.height
                && (0..=self.sections).contains(&key.1)
        }
    }
    /// Cell `i` is reached, having travelled `directions` (`REACHED` among
    /// them): the one writer of `REACHED`, so `bits` say what `directions` do.
    #[inline(always)]
    fn set_reached(&mut self, i: usize, directions: u8) {
        debug_assert!(directions & REACHED != 0);
        self.directions[i] = directions;
        self.bits[i >> 5] |= 1 << (i & 31);
    }
    /// The full search from the camera (vanilla's `scheduleFullUpdate`).
    fn search(
        &mut self,
        columns: &Columns,
        camera: DVec3,
        radius: i32,
        scratch: &mut SearchScratch,
    ) {
        let from = columns.section_of(camera);
        let origin_x = from.0 - radius;
        let origin_z = from.2 - radius;
        let side = (2 * radius + 1) as usize;
        self.from = from;
        self.radius = radius;
        self.origin_x = origin_x;
        self.origin_z = origin_z;
        self.side = side as u32;
        self.min_y = columns.min_y;
        self.sections = columns.sections;
        self.y0 = -1;

        let area = side * side;
        scratch.col_slots.resize(area, EMPTY_SLOT);
        let compute_clamp = from.1 < columns.sections;
        let mut max_non_air_y = 0;
        if columns.map.is_empty() {
            scratch.col_slots.fill(EMPTY_SLOT);
        } else {
            debug_assert_ne!(columns.grid_mask, 0);
            let grid = &columns.grid;
            let mask = columns.grid_mask;
            let shift = columns.grid_shift;
            let wide_sections = columns.sections > 256;
            for dz in 0..side {
                let wz = origin_z + dz as i32;
                let z_base = ((wz & mask) as usize) << shift;
                let wz_hi = u64::from(wz as u32) << 32;
                let row_out = &mut scratch.col_slots[dz * side..(dz + 1) * side];
                for (dx, slot_out) in row_out.iter_mut().enumerate() {
                    let wx = origin_x + dx as i32;
                    let cell = grid[z_base | ((wx & mask) as usize)];
                    let slot = if cell.slot != EMPTY_SLOT
                        && cell.key_packed == (u64::from(wx as u32) | wz_hi)
                    {
                        cell.slot
                    } else if cell.overflow == 0 {
                        EMPTY_SLOT
                    } else {
                        columns.map.get(&(wx, wz)).copied().unwrap_or(EMPTY_SLOT)
                    };
                    *slot_out = slot;
                    if compute_clamp && slot != EMPTY_SLOT {
                        let m = columns.slab.max_non_air[slot as usize];
                        let col_max = if wide_sections && m == u8::MAX {
                            columns.slab.exact_max_non_air_y(slot)
                        } else {
                            i32::from(m)
                        };
                        if col_max > max_non_air_y {
                            max_non_air_y = col_max;
                        }
                    }
                }
            }
        }
        if compute_clamp {
            let top_y = max_non_air_y.max(from.1);
            self.height = (top_y + 3).max(3).min(columns.sections + 2);
        } else {
            self.height = columns.sections + 2;
        }
        self.height_u = self.height as u32;
        let h = self.height as usize;
        let size = side * side * h;
        self.directions.clear();
        self.directions.resize(size, 0);
        self.bits.clear();
        self.bits.resize(size.div_ceil(32), 0);
        self.entered.clear();
        self.entered.resize(size, 0);
        self.sky_reached = false;
        self.rays.clear();
        self.rays
            .resize((side + 1) * (side + 1) * (self.height + 1) as usize, 0);
        scratch.queue.clear();
        if (0..columns.sections).contains(&from.1) {
            let start = self.node(from).expect("camera inside the searched box");
            self.set_reached(start.index as usize, REACHED);
            scratch.queue.push_back(start);
        } else {
            self.seed_outside(camera, scratch);
        }
        let col_slots = &scratch.col_slots;
        let rows = &columns.slab.rows;
        let stride = columns.slab.stride;
        self.propagate(&mut scratch.queue, camera, true, |node, key| {
            let slot = col_slots[usize::from(node.uz) * side + usize::from(node.ux)];
            if slot == EMPTY_SLOT {
                return ALL_VISIBLE;
            }
            let Ok(y) = usize::try_from(key.1) else {
                return ALL_VISIBLE;
            };
            if y >= stride {
                return ALL_VISIBLE;
            }
            rows[slot as usize * stride + y]
        });
    }
    /// `initializeQueueForFullUpdate` with the camera above or below the
    /// world (no render section there): every section of the world's top
    /// layer (bottom layer from below) is entered through its outer face, its
    /// travelled directions pointing away from the camera's column, sorted
    /// by the squared distance from the camera's block to the section's
    /// centre (a stable sort over vanilla's x-then-z loop). Vanilla seeds only
    /// columns in view distance; this search covers its whole box, as it does
    /// from inside the world.
    fn seed_outside(&mut self, camera: DVec3, scratch: &mut SearchScratch) {
        // Direction order: down, up, north, south, west, east.
        let (y, travel) = if self.from.1 < 0 {
            (0, 1)
        } else {
            (self.sections - 1, 0)
        };
        let block = (camera.x.floor() as i64, camera.z.floor() as i64);
        let side = (2 * self.radius + 1) as usize;
        scratch.seeds.clear();
        scratch.seeds.reserve(side * side);
        for dx in -self.radius..=self.radius {
            for dz in -self.radius..=self.radius {
                let mut directions = REACHED | 1 << travel;
                directions |= match dx.signum() {
                    1 => 1 << 5,
                    -1 => 1 << 4,
                    _ => 0,
                };
                directions |= match dz.signum() {
                    1 => 1 << 3,
                    -1 => 1 << 2,
                    _ => 0,
                };
                // The y offset is the same for every seed.
                let centre = |section: i32| i64::from(section) * 16 + 8;
                let (ox, oz) = (
                    block.0 - centre(self.from.0 + dx),
                    block.1 - centre(self.from.2 + dz),
                );
                scratch.seeds.push((ox * ox + oz * oz, dx, dz, directions));
            }
        }
        // `(dx, dz)` increases strictly in the outer/inner loops above, so
        // sorting by `(dist_sq, dx, dz)` is an exact total order matching
        // vanilla's stable sort by `dist_sq` without heap scratch allocation.
        scratch
            .seeds
            .sort_unstable_by_key(|&(dist, dx, dz, _)| (dist, dx, dz));
        for &(_, dx, dz, directions) in &scratch.seeds {
            let node = self
                .node((self.from.0 + dx, y, self.from.2 + dz))
                .expect("seed inside the searched box");
            let i = node.index as usize;
            self.set_reached(i, directions);
            // Entered through the face opposite the travel (`sourceDirection`).
            self.entered[i] = 1 << (travel ^ 1);
            scratch.queue.push_back(node);
        }
    }
    /// Vanilla's `runUpdates`: take nodes from the queue and step through
    /// every face their entry faces connect to, away from the camera. A step
    /// into a section already reached adds the face it enters through.
    /// `full`: the search from `camera` (`rays` holds its lines), not a
    /// partial update. Whether it reached a cell it had not.
    fn propagate(
        &mut self,
        queue: &mut VecDeque<QueueNode>,
        camera: DVec3,
        full: bool,
        vis: impl Fn(QueueNode, SectionKey) -> u64,
    ) -> bool {
        let mut grew = false;
        let eye = (
            (camera.x.floor() as i32) >> 4,
            (camera.y.floor() as i32 - self.min_y) >> 4,
            (camera.z.floor() as i32) >> 4,
        );
        // `SectionPos.center` of the camera's section.
        let centre = [eye.0 * 16 + 8, eye.1 * 16 + self.min_y + 8, eye.2 * 16 + 8];
        let stride_x = self.height_u as usize;
        let stride_z = (self.side * self.height_u) as usize;
        let max_u = self.side.saturating_sub(1) as u16;
        let top_uy = self.height_u.saturating_sub(1) as u16;
        let reuse = eye == self.from;

        while let Some(node) = queue.pop_front() {
            let i = node.index as usize;
            let (ux, uy, uz) = (node.ux, node.uy, node.uz);
            if uy == top_uy {
                self.sky_reached = true;
            }
            let key = (
                self.origin_x + i32::from(ux),
                self.y0 + i32::from(uy),
                self.origin_z + i32::from(uz),
            );
            let entered = self.entered[i];
            let exits = if entered == 0 {
                0x3f
            } else {
                let v = vis(node, key);
                if v == ALL_VISIBLE {
                    0x3f
                } else if v == 0 {
                    continue;
                } else {
                    let mut m = entered;
                    let mut out = 0u8;
                    while m != 0 {
                        let face = m.trailing_zeros();
                        out |= (v >> (face * 6)) as u8;
                        m &= m - 1;
                    }
                    out & 0x3f
                }
            };
            if exits == 0 {
                continue;
            }
            let directions = self.directions[i];
            // Block steps opposite any already-travelled direction (`d ^ 1`).
            let blocked = ((directions & 0x15) << 1) | ((directions & 0x2a) >> 1);
            let mut mask = exits & !blocked;
            if mask == 0 {
                continue;
            }
            let distant = (key.0 - eye.0).abs() > DISTANT_SECTIONS
                || (key.1 - eye.1).abs() > DISTANT_SECTIONS
                || (key.2 - eye.2).abs() > DISTANT_SECTIONS;
            // The distant test once per axis: both steps along an axis start
            // from the same corner. While the camera is in the section the
            // graph was searched from, every step leads away from it or
            // sideways, so no point of the line falls in a neighbour this loop
            // adds. A partial update after the camera moved tests every step.
            let mut seen = [None; 3];
            while mask != 0 {
                let d = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                let (n, nux, nuy, nuz) = match d {
                    0 => {
                        if uy == 0 {
                            continue;
                        }
                        (i - 1, ux, uy - 1, uz)
                    }
                    1 => {
                        if uy == top_uy {
                            continue;
                        }
                        (i + 1, ux, uy + 1, uz)
                    }
                    2 => {
                        if uz == 0 {
                            continue;
                        }
                        (i - stride_z, ux, uy, uz - 1)
                    }
                    3 => {
                        if uz == max_u {
                            continue;
                        }
                        (i + stride_z, ux, uy, uz + 1)
                    }
                    4 => {
                        if ux == 0 {
                            continue;
                        }
                        (i - stride_x, ux - 1, uy, uz)
                    }
                    _ => {
                        if ux == max_u {
                            continue;
                        }
                        (i + stride_x, ux + 1, uy, uz)
                    }
                };
                if distant {
                    let sees = match seen[d / 2] {
                        Some(sees) if reuse => sees,
                        _ => self.sees_camera(key, d, centre, camera, full),
                    };
                    seen[d / 2] = Some(sees);
                    if !sees {
                        continue;
                    }
                }
                if !self.is_reached(n) {
                    self.set_reached(n, directions | 1 << d);
                    self.entered[n] = 1 << (d ^ 1);
                    queue.push_back(QueueNode {
                        index: n as u32,
                        ux: nux,
                        uy: nuy,
                        uz: nuz,
                    });
                    grew = true;
                } else {
                    self.entered[n] |= 1 << (d ^ 1);
                }
            }
        }
        grew
    }
    /// Vanilla's distant step test: from the corner of `key` that the step `d`
    /// picks, walk towards the camera in section diagonals while farther than
    /// 60 blocks. Every point inside the world has to fall in a section the
    /// search has reached. Same arithmetic as JOML's `Vector3d` (no FMA).
    /// `full`: remember the answer per corner in `rays`.
    fn sees_camera(
        &mut self,
        key: SectionKey,
        d: usize,
        centre: [i32; 3],
        camera: DVec3,
        full: bool,
    ) -> bool {
        let origin = [key.0 * 16, key.1 * 16 + self.min_y, key.2 * 16];
        // The step's axis in x, y, z order: down/up, north/south, west/east.
        let axis = [1, 2, 0][d / 2];
        let far = |c: usize| {
            i32::from(if c == axis {
                centre[c] > origin[c]
            } else {
                centre[c] < origin[c]
            })
        };
        let corner = (key.0 + far(0), key.1 + far(1), key.2 + far(2));
        let slot = full.then(|| {
            let side = 2 * self.radius + 2;
            let (x, z) = (
                corner.0 - self.from.0 + self.radius,
                corner.2 - self.from.2 + self.radius,
            );
            ((z * side + x) * (self.height + 1) + corner.1 - self.y0) as usize
        });
        let known = slot.map_or(0, |slot| self.rays[slot]);
        if known == SEES {
            return true;
        }
        let failed = self.walk(corner, camera, known);
        if let Some(slot) = slot {
            self.rays[slot] = failed.unwrap_or(SEES);
        }
        failed.is_none()
    }
    /// The line from the section corner `corner` towards the camera; `None`
    /// if every sample is in a reached section, else the first one that is
    /// not (counted from 1). The first `known - 1` samples are known to be
    /// reached: they are stepped over (the same additions, so the same
    /// points) without looking them up.
    fn walk(&self, corner: SectionKey, camera: DVec3, known: u8) -> Option<u8> {
        let mut check = [
            f64::from(corner.0 * 16),
            f64::from(corner.1 * 16 + self.min_y),
            f64::from(corner.2 * 16),
        ];
        let camera = [camera.x, camera.y, camera.z];
        let to = [
            camera[0] - check[0],
            camera[1] - check[1],
            camera[2] - check[2],
        ];
        let inverse = 1.0 / (to[0] * to[0] + (to[1] * to[1] + to[2] * to[2])).sqrt();
        let step = to.map(|t| t * inverse * SECTION_DIAGONAL);
        let (min_y, max_y) = (
            f64::from(self.min_y),
            f64::from(self.min_y + self.sections * 16 - 1),
        );
        let far = |p: &[f64; 3]| {
            let (x, y, z) = (p[0] - camera[0], p[1] - camera[1], p[2] - camera[2]);
            x * x + (y * y + z * z) > DISTANT_SQR
        };
        let mut sample = 0u8;
        while sample + 1 < known {
            for (c, s) in check.iter_mut().zip(step) {
                *c += s;
            }
            sample += 1;
        }
        while far(&check) {
            for (c, s) in check.iter_mut().zip(step) {
                *c += s;
            }
            sample = sample.saturating_add(1);
            if check[1] > max_y || check[1] < min_y {
                break;
            }
            let section = (
                floor(check[0]) >> 4,
                (floor(check[1]) - self.min_y) >> 4,
                floor(check[2]) >> 4,
            );
            if !self.contains_bit(section) {
                // A count too long to keep (`SEES`) starts over next time.
                return Some(if sample == SEES { 0 } else { sample });
            }
        }
        None
    }
}

/// The graph `Occlusion::contains` reads, for GPU culling's copy
/// (gfx/gpu_culling.rs): its searched box (`Graph::index`), the level's height
/// in sections, whether the search reached the box's top layer (the sky band
/// above a clamped box), and its cells' `REACHED` as bits.
#[derive(Clone, Copy)]
pub struct Reach<'a> {
    /// `Occlusion::generation` when it was taken.
    pub generation: u64,
    pub from_x: i32,
    pub from_z: i32,
    pub radius: i32,
    pub y0: i32,
    pub height: i32,
    pub sections: i32,
    pub sky_reached: bool,
    pub bits: &'a [u32],
}

#[cfg(test)]
impl Reach<'_> {
    /// gpu_culling.wgsl's `reached`, statement for statement: what
    /// `Graph::contains_bit` answers.
    #[inline]
    pub fn contains(&self, key: SectionKey) -> bool {
        let r = self.radius;
        if r < 0 {
            return false;
        }
        let side = (2 * r + 1) as u32;
        let ux = key.0.wrapping_sub(self.from_x - r) as u32;
        let uz = key.2.wrapping_sub(self.from_z - r) as u32;
        if ux >= side || uz >= side {
            return false;
        }
        let uy = key.1.wrapping_sub(self.y0) as u32;
        let h = self.height.max(0) as u32;
        if uy < h {
            let i = ((uz * side + ux) * h + uy) as usize;
            (self.bits[i >> 5] >> (i & 31)) & 1 != 0
        } else {
            self.sky_reached
                && key.1 >= self.y0 + self.height
                && (0..=self.sections).contains(&key.1)
        }
    }
}

/// `Mth.floor` for coordinates inside the world, without a libm call (the
/// x86-64 baseline has no rounding instruction).
#[inline(always)]
fn floor(x: f64) -> i32 {
    let i = x as i32;
    i - i32::from(f64::from(i) > x)
}

struct Job {
    edits: Vec<Edit>,
    camera: DVec3,
    radius: i32,
    /// Buffers to reuse.
    graph: Graph,
}

/// The thread that runs full searches over its own copy of the columns.
struct Worker {
    jobs: Sender<Job>,
    done: Receiver<(Graph, Vec<Edit>)>,
}

impl Worker {
    fn spawn(min_y: i32, sections: i32) -> Option<Self> {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("occlusion".into())
            .spawn(move || {
                let mut columns = Columns::new(min_y, sections);
                let mut scratch = SearchScratch::default();
                while let Ok(mut job) = job_rx.recv() {
                    for edit in job.edits.drain(..) {
                        columns.apply(edit);
                    }
                    job.graph
                        .search(&columns, job.camera, job.radius, &mut scratch);
                    if done_tx.send((job.graph, job.edits)).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self { jobs, done })
    }
}

/// A new value for `Occlusion::generation`, unique in the process: a copy made
/// from an older `Occlusion` (another level's) never matches. As gfx.rs
/// `next_block_entity_revision`.
fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub struct Occlusion {
    /// The render thread's copy, for partial updates (and searches when the
    /// worker is gone).
    columns: Columns,
    /// Edits the worker has not been sent yet.
    edits: Vec<Edit>,
    /// A full search is due: sections changed since the last one started.
    dirty: bool,
    /// The last finished search; `None` until one finishes, or after the
    /// columns were cleared.
    graph: Option<Graph>,
    spare: Graph,
    worker: Option<Worker>,
    /// The camera section and radius of the running search.
    running: Option<(SectionKey, i32)>,
    /// The running search started before the columns were cleared: drop it.
    discard: bool,
    /// The camera section and radius `graph`'s search started from (or is
    /// starting from).
    searched: Option<(SectionKey, i32)>,
    /// Sections changed since `graph`'s search started, to propagate from.
    changed: Vec<SectionKey>,
    /// Sections changed since the running search started: they propagate into
    /// its graph when it lands.
    changed_late: Vec<SectionKey>,
    scratch: SearchScratch,
    /// Changes whenever `contains` may answer differently (`generation`).
    generation: u64,
    /// Set when edits were dropped; the next search resends every column.
    resync: bool,
}

impl Occlusion {
    /// The world's lowest block y and its height in sections.
    pub fn new(min_y: i32, sections: i32) -> Self {
        Self {
            columns: Columns::new(min_y, sections),
            edits: Vec::new(),
            dirty: true,
            graph: None,
            spare: Graph::default(),
            worker: Worker::spawn(min_y, sections),
            running: None,
            discard: false,
            searched: None,
            changed: Vec::new(),
            changed_late: Vec::new(),
            scratch: SearchScratch::default(),
            generation: next_generation(),
            resync: false,
        }
    }
    pub fn set(&mut self, key: SectionKey, vis: u64) {
        if self.columns.set(key, vis) {
            self.changed(key, vis);
        }
    }
    fn changed(&mut self, key: SectionKey, vis: u64) {
        self.dirty = true;
        if self.edits.len() >= EDIT_CAP || self.changed.len() >= EDIT_CAP {
            // nobody drains these with cave culling off
            self.resync = true;
            self.edits.clear();
            self.changed.clear();
            self.changed_late.clear();
        }
        if self.resync {
            return;
        }
        self.edits.push(Edit::Set(key, vis));
        self.changed.push(key);
        if self.running.is_some() {
            self.changed_late.push(key);
        }
    }
    fn resync_edits(&mut self) {
        self.resync = false;
        self.edits.clear();
        self.edits.push(Edit::Clear);
        for (&(x, z), &slot) in &self.columns.map {
            let col = self.columns.slab.column(slot);
            let top = self.columns.slab.scan_len(slot);
            for (y, &vis) in col[..top].iter().enumerate() {
                if vis != ALL_VISIBLE {
                    self.edits.push(Edit::Set((x, y as i32, z), vis));
                }
            }
        }
    }
    /// Every section of the column `x`, `z` goes (sees through every face).
    pub fn remove_column(&mut self, x: i32, z: i32) {
        let Some(slot) = self.columns.detach(x, z) else {
            return;
        };
        let base = slot as usize * self.columns.slab.stride;
        let top = self.columns.slab.scan_len(slot);
        for y in 0..top {
            if self.columns.slab.rows[base + y] != ALL_VISIBLE {
                self.changed((x, y as i32, z), ALL_VISIBLE);
            }
        }
        self.columns.slab.free(slot);
    }
    /// `remove_column` for each of `gone`. When they are every column it holds (a
    /// teleport drops the whole view), the old graph says nothing about what arrives
    /// next: it starts over as `clear` does instead of seeing through each removed
    /// section one edit at a time (some 90 000 at render distance 32).
    pub fn remove_columns(&mut self, gone: &FxHashSet<(i32, i32)>) {
        if self.columns.map.keys().all(|column| gone.contains(column)) {
            self.clear();
            return;
        }
        for &(x, z) in gone {
            self.remove_column(x, z);
        }
    }
    pub fn clear(&mut self) {
        self.columns.clear();
        self.edits.clear();
        self.edits.push(Edit::Clear);
        self.resync = false;
        self.dirty = true;
        // Another world: the old graph says nothing about it. Without one, every
        // section was visible already, and stays so.
        if let Some(graph) = self.graph.take() {
            self.spare = graph;
            self.generation = next_generation();
        }
        self.searched = None;
        self.discard = self.running.is_some();
        self.changed.clear();
        self.changed_late.clear();
    }
    /// Whether `key` is visible from the camera. Without a usable graph
    /// (before the first search lands, or with the camera somewhere the graph
    /// never reached) every section is.
    #[inline]
    pub fn contains(&self, key: SectionKey) -> bool {
        self.graph.as_ref().is_none_or(|g| g.contains_bit(key))
    }
    /// Whether the current graph knows that `key` is visible from
    /// `camera_section`. Unknown is returned when the graph is absent, was
    /// searched from another section, or does not cover the queried section.
    pub fn visible_for_camera(&self, key: SectionKey, camera_section: SectionKey) -> Option<bool> {
        let graph = self.graph.as_ref()?;
        if graph.from != camera_section {
            return None;
        }
        let dx = i64::from(key.0) - i64::from(graph.from.0);
        let dz = i64::from(key.2) - i64::from(graph.from.2);
        if dx.abs() > i64::from(graph.radius)
            || dz.abs() > i64::from(graph.radius)
            || key.1 < graph.y0
            || key.1 > graph.sections
        {
            return None;
        }
        Some(graph.contains_bit(key))
    }
    /// Changes whenever `contains` may answer differently: a search landed, a
    /// partial update reached more sections, or the graph went.
    #[cfg(any(test, debug_assertions))]
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// The graph `contains` reads; `None` while every section counts as
    /// visible.
    pub fn reach(&self) -> Option<Reach<'_>> {
        self.graph.as_ref().map(|g| Reach {
            generation: self.generation,
            from_x: g.from.0,
            from_z: g.from.2,
            radius: g.radius,
            y0: g.y0,
            height: g.height,
            sections: g.sections,
            sky_reached: g.sky_reached,
            bits: &g.bits,
        })
    }
    /// The bits `reach` exports are the cells' `REACHED`, cell for cell.
    #[cfg(any(test, debug_assertions))]
    pub fn reach_is_exact(&self) -> bool {
        self.graph.as_ref().is_none_or(|g| {
            g.bits.len() == g.directions.len().div_ceil(32)
                && g.directions
                    .iter()
                    .enumerate()
                    .all(|(i, &d)| (d & REACHED != 0) == ((g.bits[i / 32] >> (i % 32)) & 1 != 0))
        })
    }
    /// Take a finished search, propagate changed sections into the graph,
    /// and start a search when sections changed or the camera moved to
    /// another section. Never waits for the worker.
    pub fn update(&mut self, camera: DVec3, radius: i32) {
        let from = self.columns.section_of(camera);
        self.collect(false);
        self.propagate_changes(camera);
        if self.running.is_none() && (self.dirty || self.searched != Some((from, radius))) {
            self.start(camera, radius);
        }
        // A graph that never reached the camera's section hides everything
        // around it: until the new one lands, cull by the frustum alone.
        if self
            .graph
            .as_ref()
            .is_some_and(|g| g.from != from && !g.contains_bit(from))
            && let Some(graph) = self.graph.take()
        {
            self.spare = graph;
            self.generation = next_generation();
        }
    }
    fn start(&mut self, camera: DVec3, radius: i32) {
        let from = self.columns.section_of(camera);
        self.dirty = false;
        self.searched = Some((from, radius));
        if self.resync {
            self.resync_edits();
        }
        let mut graph = std::mem::take(&mut self.spare);
        if let Some(worker) = &self.worker {
            let job = Job {
                edits: std::mem::take(&mut self.edits),
                camera,
                radius,
                graph,
            };
            match worker.jobs.send(job) {
                Ok(()) => {
                    self.running = Some((from, radius));
                    self.changed_late.clear();
                    return;
                }
                Err(mpsc::SendError(job)) => {
                    graph = job.graph;
                    log::warn!(
                        "Horizon: the occlusion worker stopped; culling on the render thread"
                    );
                    self.worker = None;
                }
            }
        }
        self.edits.clear();
        graph.search(&self.columns, camera, radius, &mut self.scratch);
        self.land(graph);
    }
    fn collect(&mut self, wait: bool) {
        let Some(worker) = &self.worker else { return };
        if self.running.is_none() {
            return;
        }
        let result = if wait {
            worker.done.recv().map_err(|_| TryRecvError::Disconnected)
        } else {
            worker.done.try_recv()
        };
        match result {
            Ok((graph, edits)) => {
                self.running = None;
                if self.edits.is_empty() {
                    self.edits = edits;
                }
                if std::mem::take(&mut self.discard) {
                    self.spare = graph;
                    return;
                }
                self.land(graph);
                self.changed.clear();
                self.changed.append(&mut self.changed_late);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                // The edits it held are lost with it: search here from now on.
                self.worker = None;
                self.running = None;
                self.discard = false;
                self.dirty = true;
            }
        }
    }
    fn land(&mut self, graph: Graph) {
        if let Some(old) = self.graph.replace(graph) {
            self.spare = old;
        }
        self.generation = next_generation();
    }
    /// Vanilla's `runPartialUpdate`: what changed sections now connect to.
    fn propagate_changes(&mut self, camera: DVec3) {
        let Some(graph) = &mut self.graph else {
            self.changed.clear();
            return;
        };
        if self.changed.is_empty() {
            return;
        }
        self.scratch.queue.clear();
        for key in self.changed.drain(..) {
            if let Some(node) = graph.node(key)
                && graph.is_reached(node.index as usize)
            {
                self.scratch.queue.push_back(node);
            }
        }
        if self.scratch.queue.is_empty() {
            return;
        }
        let columns = &self.columns;
        if graph.propagate(&mut self.scratch.queue, camera, false, |_, key| {
            columns.get(key)
        }) {
            self.generation = next_generation();
        }
    }
    /// Wait for the running search: tests, and A/B timing of the old
    /// blocking search.
    #[cfg(test)]
    fn settle(&mut self, camera: DVec3, radius: i32) {
        self.update(camera, radius);
        self.collect(true);
        self.propagate_changes(camera);
    }
}

#[cfg(test)]
mod tests;
