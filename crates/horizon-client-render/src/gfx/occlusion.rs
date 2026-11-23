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
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The centre of a section, in a world whose section 0 starts at y 0.
    fn at(key: SectionKey) -> DVec3 {
        DVec3::new(
            f64::from(key.0) * 16.0 + 8.0,
            f64::from(key.1) * 16.0 + 8.0,
            f64::from(key.2) * 16.0 + 8.0,
        )
    }

    #[test]
    fn constants_are_vanillas() {
        assert_eq!(SECTION_DIAGONAL, (3f64.sqrt() * 16.0).ceil());
        assert_eq!(DISTANT_SECTIONS, 3);
    }

    #[test]
    fn floor_is_std_floor() {
        let mut x = -40_000.0_f64;
        while x < 40_000.0 {
            for v in [x, x + 0.5, x - 1e-9, x + 1e-9, -x] {
                assert_eq!(floor(v), v.floor() as i32, "{v}");
            }
            x += 0.37;
        }
    }

    #[test]
    fn open_space_reaches_the_whole_box() {
        let mut o = Occlusion::new(0, 4);
        o.settle(at((0, 1, 0)), 2);
        for x in -2..=2 {
            for z in -2..=2 {
                for y in -1..=4 {
                    assert!(o.contains((x, y, z)), "{x} {y} {z}");
                }
            }
        }
        assert!(!o.contains((3, 1, 0)));
    }

    #[test]
    fn closed_neighbours_hide_what_is_behind_them() {
        let mut o = Occlusion::new(0, 4);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle(at((0, 1, 0)), 3);
        assert!(o.contains((1, 1, 0)), "a neighbour is seen");
        assert!(!o.contains((2, 1, 0)), "nothing shows through it");
        o.set((1, 1, 0), ALL_VISIBLE);
        o.settle(at((0, 1, 0)), 3);
        assert!(o.contains((2, 1, 0)));
    }

    /// `visible_for_camera` answers only for the box of the graph it holds, and
    /// only for the camera that search started from. Everywhere else it says
    /// unknown, so the caller keeps the geometry.
    #[test]
    fn visible_for_camera_is_unknown_outside_its_box_and_its_camera() {
        // No graph yet: nothing to answer with.
        assert_eq!(
            Occlusion::new(0, 4).visible_for_camera((0, 1, 0), (0, 1, 0)),
            None
        );
        let mut o = Occlusion::new(0, 4);
        o.settle(at((0, 1, 0)), 2);
        let from = o.columns.section_of(at((0, 1, 0)));
        assert_eq!(from, (0, 1, 0), "the camera section searched from");
        // Inside the searched box the answer is `contains`, its edges too.
        for x in -2..=2 {
            for z in -2..=2 {
                for y in -1..=4 {
                    assert_eq!(
                        o.visible_for_camera((x, y, z), from),
                        Some(o.contains((x, y, z))),
                        "{x} {y} {z}"
                    );
                }
            }
        }
        assert_eq!(
            o.visible_for_camera((2, 1, 0), from),
            Some(true),
            "the box's edge"
        );
        // Off the box's sides, below it, or above the level: unknown.
        for key in [
            (3, 1, 0),
            (-3, 1, 0),
            (0, 1, 3),
            (0, 1, -3),
            (0, -2, 0),
            (0, 5, 0),
        ] {
            assert_eq!(o.visible_for_camera(key, from), None, "{key:?}");
        }
        // Another camera section: this graph is not about it.
        assert_eq!(o.visible_for_camera((0, 1, 0), (1, 1, 0)), None);
    }

    /// The answer follows the graph: a closed wall hides the section behind it,
    /// and opening the wall shows it again.
    #[test]
    fn visible_for_camera_reports_what_the_graph_hides() {
        let mut o = Occlusion::new(0, 4);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle(at((0, 1, 0)), 3);
        let from = (0, 1, 0);
        assert_eq!(
            o.visible_for_camera((1, 1, 0), from),
            Some(true),
            "the wall itself"
        );
        assert_eq!(
            o.visible_for_camera((2, 1, 0), from),
            Some(false),
            "hidden behind it"
        );
        o.set((1, 1, 0), ALL_VISIBLE);
        o.settle(at((0, 1, 0)), 3);
        assert_eq!(o.visible_for_camera((2, 1, 0), from), Some(true));
    }

    /// Faces `a` and `b` of a section see each other, nothing else does.
    fn connects(a: usize, b: usize) -> u64 {
        1 << (a * 6 + b) | 1 << (b * 6 + a)
    }

    /// #629 / #782 rule 1: a section first entered through a face that leads
    /// nowhere still leads on through the face it is entered through later.
    #[test]
    fn a_section_leads_on_through_every_face_it_is_entered_through() {
        let mut o = Occlusion::new(0, 4);
        // (1, 1, 1) is reached through its west face first (from (0, 1, 1),
        // queued before (1, 1, 0)), then through its north face. Only north
        // leads on, to the east.
        o.set((1, 1, 1), connects(2, 5));
        // Every other way into (2, 1, 1) is closed.
        for key in [(2, 1, 0), (2, 0, 1), (2, 2, 1)] {
            o.set(key, 0);
        }
        o.settle(at((0, 1, 0)), 3);
        let graph = o.graph.as_ref().expect("searched");
        let i = graph.index((1, 1, 1)).unwrap();
        assert_eq!(
            graph.entered[i],
            1 << 2 | 1 << 4,
            "entered from north and west"
        );
        assert!(o.contains((2, 1, 1)), "seen through the north face");
    }

    #[test]
    fn frames_never_wait_and_partial_updates_open_at_once() {
        let mut o = Occlusion::new(0, 4);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        let camera = at((0, 1, 0));
        o.settle(camera, 3);
        assert!(!o.contains((2, 1, 0)));
        // Opening the wall shows what is behind it in the same frame, from the
        // graph the frame has, whether or not the full search has finished.
        o.set((1, 1, 0), ALL_VISIBLE);
        o.update(camera, 3);
        assert!(o.contains((2, 1, 0)));
        assert!(o.contains((3, 1, 0)));
        // Closing it again lasts until the next full search lands.
        o.set((1, 1, 0), 0);
        o.settle(camera, 3);
        o.settle(camera, 3);
        assert!(!o.contains((2, 1, 0)));
    }

    #[test]
    fn a_graph_that_lost_the_camera_culls_nothing() {
        let mut o = Occlusion::new(0, 4);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle(at((0, 1, 0)), 3);
        assert!(!o.contains((3, 1, 0)));
        // The camera jumped past the wall; the next search is still running.
        let o = &mut o;
        o.update(at((3, 1, 0)), 3);
        if o.running.is_some() {
            assert!(o.contains((3, 1, 0)));
        }
        o.settle(at((3, 1, 0)), 3);
        assert!(o.contains((3, 1, 0)));
        assert!(
            !o.contains((0, 1, 0)),
            "the old camera section is walled in"
        );
    }

    /// #1037: from far above or below the world, the search starts from the
    /// world's top or bottom layer (`initializeQueueForFullUpdate`), so the
    /// ground stays drawn at any height.
    #[test]
    fn a_camera_outside_the_world_sees_it_from_its_top_or_bottom() {
        for from in [(0, 6, 0), (0, 9, 0), (2, 40, -1), (0, -1, 0), (1, -7, 0)] {
            let mut o = Occlusion::new(0, 6);
            o.settle(at(from), 3);
            for x in -1..=5 {
                for z in -4..=2 {
                    for y in 0..6 {
                        let inside = (x - from.0).abs() <= 3 && (z - from.2).abs() <= 3;
                        assert_eq!(o.contains((x, y, z)), inside, "{from:?}: {x} {y} {z}");
                    }
                }
            }
            assert!(
                !o.contains(from),
                "{from:?}: the camera's own section is not in the world"
            );
        }
    }

    /// Seeds enter through the outer face and never turn back up (down)
    /// or towards the camera's column.
    #[test]
    fn a_closed_top_layer_hides_the_world_from_above() {
        let mut o = Occlusion::new(0, 6);
        for x in -3..=3 {
            for z in -3..=3 {
                o.set((x, 5, z), 0);
            }
        }
        o.settle(at((0, 30, 0)), 3);
        assert!(
            o.contains((0, 5, 0)) && o.contains((3, 5, -3)),
            "the top is seen"
        );
        assert!(
            !o.contains((0, 4, 0)) && !o.contains((2, 1, 2)),
            "nothing under it"
        );
        assert!(!o.contains((0, 6, 0)), "seeds don't step back up");
        // From below, the open bottom layer shows everything up to the lid.
        o.settle(at((0, -9, 0)), 3);
        assert!(o.contains((0, 0, 0)) && o.contains((1, 4, 1)) && o.contains((1, 5, 1)));
        assert!(!o.contains((0, -1, 0)), "seeds don't step back down");
    }

    /// Vanilla's `runUpdates` as written (a map of nodes, a FIFO queue,
    /// `Vector3d` maths), over the same box: whether each cell is reached.
    /// `rays: false` leaves out the distant step test.
    fn reference(columns: &Columns, camera: DVec3, radius: i32, rays: bool) -> Vec<bool> {
        struct Node {
            directions: u8,
            sources: u8,
        }
        let from = columns.section_of(camera);
        let y0 = -1;
        let height = columns.sections + 2;
        let side = 2 * radius + 1;
        let index = |k: SectionKey| {
            let (dx, dz, dy) = (k.0 - from.0, k.2 - from.2, k.1 - y0);
            (dx.abs() <= radius && dz.abs() <= radius && (0..height).contains(&dy))
                .then(|| (((dz + radius) * side + dx + radius) * height + dy) as usize)
        };
        let block = |k: SectionKey| [k.0 * 16, k.1 * 16 + columns.min_y, k.2 * 16];
        // Direction.getAxis in x, y, z order.
        let axis = |d: usize| [1, 2, 0][d / 2];
        let centre = block(from).map(|c| c + 8);
        let mut nodes: HashMap<SectionKey, Node> = HashMap::new();
        let mut queue = VecDeque::new();
        if (0..columns.sections).contains(&from.1) {
            nodes.insert(
                from,
                Node {
                    directions: 0,
                    sources: 0,
                },
            );
            queue.push_back(from);
        } else {
            // initializeQueueForFullUpdate from outside the world.
            let above = from.1 >= columns.sections;
            let (y, travel) = if above {
                (columns.sections - 1, 0)
            } else {
                (0, 1)
            };
            let camera_block = [camera.x.floor() as i64, camera.z.floor() as i64];
            let mut seeds = Vec::new();
            for x in -radius..=radius {
                for z in -radius..=radius {
                    let key = (from.0 + x, y, from.2 + z);
                    let mut directions = 1 << travel;
                    if x > 0 {
                        directions |= 1 << 5;
                    } else if x < 0 {
                        directions |= 1 << 4;
                    }
                    if z > 0 {
                        directions |= 1 << 3;
                    } else if z < 0 {
                        directions |= 1 << 2;
                    }
                    let origin = block(key);
                    let dx = camera_block[0] - i64::from(origin[0] + 8);
                    let dz = camera_block[1] - i64::from(origin[2] + 8);
                    seeds.push((dx * dx + dz * dz, key, directions));
                }
            }
            seeds.sort_by_key(|seed| seed.0);
            for (_, key, directions) in seeds {
                nodes.insert(
                    key,
                    Node {
                        directions,
                        sources: 1 << travel,
                    },
                );
                queue.push_back(key);
            }
        }
        while let Some(key) = queue.pop_front() {
            let node = &nodes[&key];
            let (directions, sources) = (node.directions, node.sources);
            let vis = columns.get(key);
            let distant = (key.0 - from.0).abs() > 3
                || (key.1 - from.1).abs() > 3
                || (key.2 - from.2).abs() > 3;
            for (d, step) in STEPS.iter().enumerate() {
                let next = (key.0 + step.0, key.1 + step.1, key.2 + step.2);
                if next.1 < -1 || next.1 > columns.sections || index(next).is_none() {
                    continue;
                }
                if directions & (1 << (d ^ 1)) != 0 {
                    continue;
                }
                // facesCanSeeEachother(source.getOpposite(), direction).
                if sources != 0
                    && !(0..6)
                        .any(|s| sources & (1 << s) != 0 && vis & (1 << ((s ^ 1) * 6 + d)) != 0)
                {
                    continue;
                }
                if rays && distant {
                    let origin = block(key);
                    let max = |c: usize| {
                        if c == axis(d) {
                            centre[c] > origin[c]
                        } else {
                            centre[c] < origin[c]
                        }
                    };
                    let mut check = DVec3::new(
                        f64::from(origin[0] + if max(0) { 16 } else { 0 }),
                        f64::from(origin[1] + if max(1) { 16 } else { 0 }),
                        f64::from(origin[2] + if max(2) { 16 } else { 0 }),
                    );
                    let length = |v: DVec3| (v.x * v.x + (v.y * v.y + v.z * v.z)).sqrt();
                    let to = camera - check;
                    let step = to * (1.0 / length(to)) * 28.0;
                    let mut visible = true;
                    while length(check - camera).powi(2) > 3600.0 {
                        check += step;
                        let max_y = columns.min_y + columns.sections * 16 - 1;
                        if check.y > f64::from(max_y) || check.y < f64::from(columns.min_y) {
                            break;
                        }
                        let section = columns.section_of(check);
                        if !nodes.contains_key(&section) {
                            visible = false;
                            break;
                        }
                    }
                    if !visible {
                        continue;
                    }
                }
                match nodes.get_mut(&next) {
                    Some(existing) => existing.sources |= 1 << d,
                    None => {
                        nodes.insert(
                            next,
                            Node {
                                directions: directions | 1 << d,
                                sources: 1 << d,
                            },
                        );
                        queue.push_back(next);
                    }
                }
            }
        }
        let mut reached = vec![false; (side * side * height) as usize];
        for key in nodes.keys() {
            reached[index(*key).unwrap()] = true;
        }
        reached
    }

    fn random_world(o: &mut Occlusion, seed: u64, count: usize, spread: i32) {
        let mut seed = seed;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let span = (2 * spread + 1) as u64;
        for _ in 0..count {
            let r = next();
            let key = (
                (r % span) as i32 - spread,
                ((r >> 8) % 6) as i32,
                ((r >> 16) % span) as i32 - spread,
            );
            // Open, half open or nearly closed sections.
            let vis = match r >> 40 & 3 {
                0 => next() & next() & next(),
                1 => next(),
                _ => next() | next(),
            };
            o.set(key, vis & ALL_VISIBLE);
        }
    }

    fn reached(o: &Occlusion) -> Vec<bool> {
        let graph = o.graph.as_ref().expect("searched");
        graph.directions.iter().map(|&d| d & REACHED != 0).collect()
    }

    /// The graph is vanilla's, cell for cell, near the camera and beyond the
    /// distant step test's 60 blocks.
    #[test]
    #[ignore = "daily"]
    fn same_sections_as_vanilla() {
        let (mut hidden, mut rayed) = (0, 0);
        for seed in [0x9e37_79b9, 7, 0xdead_beef] {
            for (spread, count, radius) in [(6, 900, 6), (12, 2600, 12)] {
                let mut o = Occlusion::new(0, 6);
                random_world(&mut o, seed, count, spread);
                for from in [(0, 2, 0), (3, 0, -2), (-5, 5, 4), (1, -1, 1), (0, 7, 0)] {
                    // Off-centre, so the rays are not axis-aligned.
                    let camera = at(from) + DVec3::new(3.3, -5.7, 1.9);
                    o.settle(camera, radius);
                    o.settle(camera, radius);
                    let reached = reached(&o);
                    assert_eq!(
                        reached,
                        reference(&o.columns, camera, radius, true),
                        "{seed} {from:?} {radius}"
                    );
                    let no_rays = reference(&o.columns, camera, radius, false);
                    rayed += no_rays
                        .iter()
                        .zip(&reached)
                        .filter(|(a, b)| **a && !**b)
                        .count();
                    hidden += reached.iter().filter(|&&r| !r).count();
                }
            }
        }
        assert!(hidden > 1000, "the worlds hide sections ({hidden})");
        assert!(
            rayed > 100,
            "the distant step test culls sections ({rayed})"
        );
    }

    /// A clamped search may conservatively show sections above its searched
    /// box, but it must never hide one that the full-height search reaches.
    #[test]
    #[ignore = "daily"]
    fn vertical_clamping_only_adds_overdraw() {
        let mut compared = 0;
        let mut overdraw = 0;
        let mut clamped = 0;
        for seed in [0x9e37_79b9, 7, 0xdead_beef, 0x2545_f491] {
            for (spread, count, radius) in [(6, 900, 6), (12, 2600, 12)] {
                let mut o = Occlusion::new(0, 24);
                random_world(&mut o, seed, count, spread);
                let camera = at((0, 2, 0)) + DVec3::new(3.3, -5.7, 1.9);
                o.settle(camera, radius);
                let graph = o.graph.as_ref().expect("searched");
                clamped += usize::from(graph.height < o.columns.sections + 2);
                let full = reference(&o.columns, camera, radius, true);
                let from = o.columns.section_of(camera);
                let side = 2 * radius + 1;
                let height = o.columns.sections + 2;
                let full_contains = |key: SectionKey| {
                    let (dx, dz, dy) = (key.0 - from.0, key.2 - from.2, key.1 + 1);
                    if dx.abs() > radius || dz.abs() > radius || !(0..height).contains(&dy) {
                        return false;
                    }
                    full[(((dz + radius) * side + dx + radius) * height + dy) as usize]
                };
                for x in from.0 - radius..=from.0 + radius {
                    for z in from.2 - radius..=from.2 + radius {
                        for y in -1..=o.columns.sections {
                            let key = (x, y, z);
                            let expected = full_contains(key);
                            let actual = o.contains(key);
                            compared += 1;
                            assert!(
                                !expected || actual,
                                "clamped search hid {key:?} for seed {seed:#x}, spread {spread}, radius {radius}; from={from:?}, y0={}, height={}, expected={expected}, actual={actual}",
                                graph.y0,
                                graph.height
                            );
                            overdraw += usize::from(actual && !expected);
                            if actual && !expected {
                                assert!(
                                    y >= graph.y0 + graph.height,
                                    "overdraw must be above the clamped box: {key:?}, y0={}, height={}",
                                    graph.y0,
                                    graph.height
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(clamped > 0, "the comparison exercised a clamped search");
        assert!(compared > 50_000, "compared {compared} sections");
        assert!(overdraw > 0, "the conservative top-cell rule was exercised");
    }

    /// A line checked again from the sample it failed at, after more
    /// sections were reached, ends where checking it from its corner does.
    #[test]
    fn a_line_checked_again_resumes_where_it_failed() {
        let mut seed = 0x2545_f491_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let columns = Columns::new(-64, 24);
        let mut graph = Graph::default();
        let mut scratch = SearchScratch::default();
        let (mut resumed, mut seen) = (0, 0);
        for _ in 0..40 {
            let camera = DVec3::new(
                (next() % 3000) as f64 / 100.0 - 15.0,
                (next() % 30000) as f64 / 100.0 - 60.0,
                (next() % 3000) as f64 / 100.0 - 15.0,
            );
            // Open air: every section reached; then holes along the lines.
            graph.search(&columns, camera, 12, &mut scratch);
            for (i, cell) in graph.directions.iter_mut().enumerate() {
                if next().is_multiple_of(4) {
                    *cell = 0;
                    graph.bits[i >> 5] &= !(1 << (i & 31));
                }
            }
            let from = graph.from;
            for _ in 0..25 {
                let corner = (
                    from.0 + (next() % 25) as i32 - 12,
                    graph.y0 + (next() % graph.height as u64) as i32,
                    from.2 + (next() % 25) as i32 - 12,
                );
                let mut known = 0;
                while let Some(failed) = graph.walk(corner, camera, known) {
                    for earlier in 0..=failed {
                        assert_eq!(graph.walk(corner, camera, earlier), Some(failed));
                    }
                    known = failed;
                    resumed += 1;
                    // More sections reached, as the search goes on.
                    for (i, cell) in graph.directions.iter_mut().enumerate() {
                        if next().is_multiple_of(3) {
                            *cell |= REACHED;
                            graph.bits[i >> 5] |= 1 << (i & 31);
                        }
                    }
                    assert_eq!(
                        graph.walk(corner, camera, known),
                        graph.walk(corner, camera, 0)
                    );
                }
                seen += usize::from(known > 1);
            }
        }
        assert!(resumed > 200 && seen > 50, "{resumed} {seen}");
    }

    /// #782 rule 2: behind a wall, a section the search reaches around its
    /// edge is culled when the line to the camera crosses the wall's shadow.
    #[test]
    fn distant_sections_behind_a_wall_are_culled() {
        let mut o = Occlusion::new(0, 4);
        // A closed wall at x 2, z -2..=2, the whole height.
        for z in -2..=2 {
            for y in -1..=4 {
                o.set((2, y, z), 0);
            }
        }
        let camera = at((0, 1, 0));
        o.settle(camera, 12);
        let no_rays = reference(&o.columns, camera, 12, false);
        let graph = o.graph.as_ref().expect("searched");
        let behind = graph.index((9, 1, 3)).unwrap();
        assert!(no_rays[behind], "reached around the wall's edge");
        assert!(
            !o.contains((9, 1, 3)),
            "but its line to the camera is hidden"
        );
        assert!(o.contains((9, 1, 9)), "a section off to the side is seen");
    }

    /// `cargo test --profile fast -p horizon-client --lib occlusion::tests::search_cost -- --ignored --nocapture`:
    /// full searches at render distance 32 over a world with open sky above
    /// y 64 and caves below it.
    #[test]
    #[ignore]
    fn search_cost() {
        let mut o = Occlusion::new(-64, 24);
        let mut seed = 0x9e37_79b9_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for x in -40..=40 {
            for z in -40..=40 {
                for y in 0..24 {
                    let vis = if y >= 8 {
                        ALL_VISIBLE
                    } else if next().is_multiple_of(5) {
                        next() & ALL_VISIBLE
                    } else {
                        0
                    };
                    o.set((x, y, z), vis);
                }
            }
        }
        let camera = DVec3::new(3.3, 70.2, -5.6);
        let mut scratch = SearchScratch::default();
        let mut graph = Graph::default();
        let mut best = f64::MAX;
        for _ in 0..12 {
            let started = std::time::Instant::now();
            graph.search(&o.columns, camera, 34, &mut scratch);
            best = best.min(started.elapsed().as_secs_f64() * 1000.0);
        }
        let reached = graph
            .directions
            .iter()
            .filter(|&&d| d & REACHED != 0)
            .count();
        println!("search_cost: best {best:.2} ms over 12, {reached} reached");
    }

    /// Removed columns see through every face: the graph is the one of a world that
    /// never had them, whether some columns go or all of them (a teleport).
    #[test]
    fn removed_columns_leave_the_graph_of_a_world_without_them() {
        fn without(o: &Occlusion, gone: &FxHashSet<(i32, i32)>) -> Occlusion {
            let mut fresh = Occlusion::new(0, 6);
            for (&(x, z), &slot) in &o.columns.map {
                if !gone.contains(&(x, z)) {
                    for (y, &vis) in o.columns.slab.column(slot).iter().enumerate() {
                        fresh.set((x, y as i32, z), vis);
                    }
                }
            }
            fresh
        }
        let camera = at((0, 2, 0));
        let mut o = Occlusion::new(0, 6);
        random_world(&mut o, 0x51ed_2701, 900, 6);
        o.settle(camera, 6);
        let some: FxHashSet<(i32, i32)> = (-6..=6)
            .flat_map(|x| (-1..=1).map(move |z| (x, z)))
            .collect();
        let mut expected = without(&o, &some);
        o.remove_columns(&some);
        o.settle(camera, 6);
        expected.settle(camera, 6);
        let graph = |o: &Occlusion| o.graph.as_ref().expect("searched").directions.clone();
        assert_eq!(graph(&o), graph(&expected));

        let all: FxHashSet<(i32, i32)> = o.columns.map.keys().copied().collect();
        o.remove_columns(&all);
        assert!(o.contains((40, 3, 40)), "no graph from the old view culls");
        random_world(&mut o, 0x2545_f491, 900, 6);
        let mut arrived = Occlusion::new(0, 6);
        random_world(&mut arrived, 0x2545_f491, 900, 6);
        o.settle(camera, 6);
        arrived.settle(camera, 6);
        assert_eq!(graph(&o), graph(&arrived));
    }

    /// The worker's graph is the one the render thread would compute.
    #[test]
    fn the_worker_matches_a_search_on_the_render_thread() {
        let mut o = Occlusion::new(0, 6);
        random_world(&mut o, 0x9e37_79b9, 800, 6);
        for from in [(0, 2, 0), (3, 0, -2), (-5, 5, 4)] {
            let camera = at(from);
            o.settle(camera, 6);
            o.settle(camera, 6);
            let mut local = Graph::default();
            local.search(&o.columns, camera, 6, &mut SearchScratch::default());
            let graph = o.graph.as_ref().expect("searched");
            assert_eq!(graph.directions, local.directions);
            assert_eq!(graph.entered, local.entered);
        }
    }

    #[test]
    fn vertical_clamping_bounds_height_and_reaches_open_sky() {
        let mut o = Occlusion::new(0, 24);
        o.set((0, 5, 0), 0); // Wall at section 5
        o.settle(at((0, 2, 0)), 4);
        let graph = o.graph.as_ref().expect("searched");
        assert_eq!(graph.height, 5 + 3);
        assert!(o.contains((0, 2, 0)), "camera section is reached");
        assert!(o.contains((1, 20, 1)), "open air column is reached");

        // An enclosed cave cannot reach the sky
        let mut cave = Occlusion::new(0, 24);
        for (dx, dy, dz) in STEPS {
            cave.set((dx, 1 + dy, dz), 0);
        }
        cave.settle(at((0, 1, 0)), 4);
        assert!(
            !cave.contains((0, 10, 0)),
            "enclosed cave does not see the sky"
        );
    }

    /// `contains` over the columns `radius` + 3 around `from`, from two
    /// sections below the level to two above it, checked against the graph
    /// `reach` exports: `None` exactly without a graph, the same answer for
    /// every key, nothing past its box, its bits its cells' `REACHED`, and its
    /// sky band reached exactly when a bit of its top layer is.
    fn answers(o: &Occlusion, from: SectionKey, radius: i32) -> Vec<bool> {
        let reach = o.reach();
        assert_eq!(reach.is_some(), o.graph.is_some(), "a graph exported");
        assert!(o.reach_is_exact(), "the bits are the cells'");
        if let Some(g) = &o.graph {
            let height = g.height as usize;
            let mut top = (height - 1..g.directions.len()).step_by(height);
            let sky = top.any(|i| (g.bits[i / 32] >> (i % 32)) & 1 != 0);
            assert_eq!(g.sky_reached, sky, "the sky band");
        }
        let past = |key: SectionKey| {
            o.graph.as_ref().is_some_and(|g| {
                (key.0 - g.from.0).abs() > g.radius
                    || (key.2 - g.from.2).abs() > g.radius
                    || !(g.y0..=g.sections).contains(&key.1)
            })
        };
        let mut answers = Vec::new();
        for x in from.0 - radius - 3..=from.0 + radius + 3 {
            for z in from.2 - radius - 3..=from.2 + radius + 3 {
                for y in -2..=o.columns.sections + 1 {
                    let key = (x, y, z);
                    let contains = o.contains(key);
                    assert_eq!(reach.is_none_or(|r| r.contains(key)), contains, "{key:?}");
                    assert!(!(contains && past(key)), "{key:?} past the box");
                    answers.push(contains);
                }
            }
        }
        answers
    }

    /// The graph `reach` exports answers as `contains` (`answers`): walled in,
    /// opened by a partial update, cleared; over clamped boxes, the sky band
    /// above them reached or not (a closed cave); from above the world. The
    /// search, the partial update and `clear` each change the generation, and
    /// another level's graph never repeats one.
    #[test]
    fn the_exported_graph_answers_as_contains() {
        let mut o = Occlusion::new(0, 4);
        let first = o.generation();
        answers(&o, (0, 1, 0), 3);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle(at((0, 1, 0)), 3);
        assert_ne!(o.generation(), first, "a search landed");
        let walled = answers(&o, (0, 1, 0), 3);
        let searched = o.generation();
        o.set((0, 1, 1), ALL_VISIBLE);
        o.update(at((0, 1, 0)), 3);
        assert_ne!(o.generation(), searched, "a partial update reached more");
        assert_ne!(answers(&o, (0, 1, 0), 3), walled);
        let opened = o.generation();
        o.clear();
        assert_ne!(o.generation(), opened, "the graph went");
        answers(&o, (0, 1, 0), 3);
        // Another level's graph never takes one of this one's generations, or
        // the pass would keep its copy of this one.
        let mut other = Occlusion::new(0, 4);
        other.settle(at((0, 1, 0)), 3);
        assert_ne!(other.generation(), searched, "another level's graph");

        let (mut clamped, mut band) = (0, [0; 2]);
        let mut check = |mut o: Occlusion, camera: DVec3, radius: i32| {
            o.settle(camera, radius);
            answers(&o, o.columns.section_of(camera), radius);
            let g = o.graph.as_ref().expect("searched");
            if g.height < o.columns.sections + 2 {
                clamped += 1;
                // The first section above the box, over the camera's column.
                band[usize::from(o.contains((g.from.0, g.y0 + g.height, g.from.2)))] += 1;
            }
        };
        for seed in [0x9e37_79b9, 7, 0xdead_beef, 0x2545_f491] {
            for (spread, count, radius) in [(6, 900, 6), (12, 2600, 12)] {
                let mut o = Occlusion::new(0, 24);
                random_world(&mut o, seed, count, spread);
                check(o, at((0, 2, 0)) + DVec3::new(3.3, -5.7, 1.9), radius);
            }
        }
        let mut cave = Occlusion::new(0, 24);
        for (dx, dy, dz) in STEPS {
            cave.set((dx, 1 + dy, dz), 0);
        }
        check(cave, at((0, 1, 0)), 4);
        let mut above = Occlusion::new(0, 24);
        random_world(&mut above, 7, 900, 6);
        check(above, DVec3::new(3.3, 400.0, -5.6), 6);
        assert!(
            clamped > 0 && band[0] > 0 && band[1] > 0,
            "{clamped} clamped, the band above hidden and seen {band:?}"
        );
    }

    /// Whenever an answer of `contains` changes, so does the generation: over
    /// 300 random steps, each a section set (open, closed or anything between),
    /// a column removed, the camera moved to another section (now and then
    /// above or below the world), or now and then another world, after a
    /// `clear` or a teleport that drops every column (`remove_columns`), each
    /// followed by a frame's `update` or a settled search; then an update from
    /// a section the graph never reached. The exported graph answers as
    /// `contains` after every step.
    #[test]
    fn every_change_of_the_answers_changes_the_generation() {
        let mut seed = 0x0bad_5eed_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let (spread, radius, around) = (5, 3, (0, 0, 0));
        let offset = DVec3::new(3.3, -5.7, 1.9);
        let mut o = Occlusion::new(0, 24);
        random_world(&mut o, 0x2545_f491, 400, spread);
        let mut camera = at((0, 2, 0)) + offset;
        o.settle(camera, radius);
        let mut last = (o.generation(), answers(&o, around, spread + radius));
        let mut changed = 0;
        for step in 0..300 {
            let r = next();
            let key = (
                (r % 11) as i32 - spread,
                ((r >> 8) % 7) as i32,
                ((r >> 16) % 11) as i32 - spread,
            );
            match r >> 24 & 15 {
                0..=7 => {
                    let vis = match r >> 28 & 3 {
                        0 => ALL_VISIBLE,
                        1 => 0,
                        _ => next() & ALL_VISIBLE,
                    };
                    o.set(key, vis);
                }
                8 | 9 => o.remove_column(key.0, key.2),
                10..=13 => {
                    let y = match r >> 32 & 7 {
                        0 => 30,
                        1 => -3,
                        _ => key.1,
                    };
                    camera = at((key.0, y, key.2)) + offset;
                }
                _ if r >> 36 & 7 == 0 => {
                    o.clear();
                    random_world(&mut o, next(), 400, spread);
                }
                _ if r >> 36 & 7 == 1 => {
                    // A teleport: every column goes at once.
                    let all: FxHashSet<_> = o.columns.map.keys().copied().collect();
                    o.remove_columns(&all);
                    random_world(&mut o, next(), 400, spread);
                }
                _ => {}
            }
            if r >> 40 & 1 == 0 {
                o.update(camera, radius);
            } else {
                o.settle(camera, radius);
            }
            let now = (o.generation(), answers(&o, around, spread + radius));
            if now.1 != last.1 {
                assert_ne!(
                    now.0, last.0,
                    "step {step}: answers changed, the generation not"
                );
                changed += 1;
            }
            last = now;
        }
        assert!(changed >= 50, "{changed} steps changed answers");
        // Walled in, then past the wall: the graph never reached the camera's
        // section, and goes until the next search lands. (The first settle may
        // land a search that started before the wall.)
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle(at((0, 1, 0)), radius);
        o.settle(at((0, 1, 0)), radius);
        let walled = (o.generation(), answers(&o, around, spread + radius));
        assert!(!o.contains((3, 1, 0)), "past the wall");
        o.update(at((3, 1, 0)), radius);
        assert_ne!(answers(&o, around, spread + radius), walled.1);
        assert_ne!(o.generation(), walled.0, "the graph went");
    }

    #[test]
    fn edit_queue_stays_bounded_without_searches() {
        let mut o = Occlusion::new(0, 4);
        // Fill the queue, trigger the resend, then edit once while it is pending.
        // Further edits repeat that last branch without exercising a new boundary.
        let total = EDIT_CAP + 2;
        for i in 0..total {
            o.set(((i % 700) as i32, 1, (i / 700) as i32), 1 + (i as u64 % 5));
        }
        assert!(o.edits.len() <= 1 << 16, "{} edits queued", o.edits.len());
        assert!(
            o.changed.len() <= 1 << 16,
            "{} changed queued",
            o.changed.len()
        );

        let mut fresh = Occlusion::new(0, 4);
        for (&(x, z), &slot) in &o.columns.map {
            for (y, &vis) in o.columns.slab.column(slot).iter().enumerate() {
                // Missing sections in the fresh world already connect every face.
                if vis != ALL_VISIBLE {
                    fresh.set((x, y as i32, z), vis);
                }
            }
        }
        let camera = at((0, 1, 0));
        for _ in 0..2 {
            o.settle(camera, 6);
            fresh.settle(camera, 6);
        }
        assert_eq!(
            reached(&o),
            reached(&fresh),
            "the worker saw the same world"
        );
    }

    /// Exercises `ColumnSlab` and the toroidal `GridSlot` direct-mapped cache
    /// across negative coordinates, extreme `i32` bounds, deliberate toroidal
    /// collisions, grid growth (`shift` 4 -> 7), `max_non_air` saturation
    /// (`stride` 0, 4, 24, 256, 270), slot recycling, promotion, and `clear`.
    #[test]
    fn column_slab_toroidal_invariants_and_collisions() {
        for sections in [0, 4, 24, 256, 270] {
            let mut cols = Columns::new(-64, sections);
            let mut oracle: HashMap<(i32, i32), Vec<u64>> = HashMap::new();
            let mut seed = 0x517c_c1b7_2722_0a95_u64 ^ (sections as u64);
            let mut next = || {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed
            };
            // Deliberately mix negative coords, i32 extremes, toroidal
            // multiples (16, 32, 64, 128, 256), and a dense 20x20 patch to
            // exercise shift 4 -> 5 -> 6 growth and overflow chains.
            let mut coords = Vec::new();
            for bx in [
                i32::MIN,
                -256,
                -128,
                -64,
                -32,
                -16,
                -1,
                0,
                1,
                16,
                32,
                64,
                128,
                256,
                i32::MAX,
            ] {
                for bz in [-128, -16, -1, 0, 1, 16, 128] {
                    coords.push((bx, bz));
                }
            }
            for x in -10..10 {
                for z in -10..10 {
                    coords.push((x, z));
                }
            }

            let oracle_set =
                |oracle: &mut HashMap<(i32, i32), Vec<u64>>, key: SectionKey, vis: u64| -> bool {
                    if !(0..sections).contains(&key.1) {
                        return false;
                    }
                    let col = match oracle.get_mut(&(key.0, key.2)) {
                        Some(col) => col,
                        None if vis == ALL_VISIBLE => return false,
                        None => oracle
                            .entry((key.0, key.2))
                            .or_insert_with(|| vec![ALL_VISIBLE; sections as usize]),
                    };
                    let slot = &mut col[key.1 as usize];
                    if *slot == vis {
                        return false;
                    }
                    *slot = vis;
                    if vis == ALL_VISIBLE && col.iter().all(|&v| v == ALL_VISIBLE) {
                        oracle.remove(&(key.0, key.2));
                    }
                    true
                };
            let oracle_get = |oracle: &HashMap<(i32, i32), Vec<u64>>, key: SectionKey| -> u64 {
                if !(0..sections).contains(&key.1) {
                    return ALL_VISIBLE;
                }
                oracle
                    .get(&(key.0, key.2))
                    .map_or(ALL_VISIBLE, |col| col[key.1 as usize])
            };

            let ops: usize = if sections <= 24 { 2500 } else { 1200 };
            for step in 0..ops {
                let r = next();
                let (wx, wz) = coords[(r as usize) % coords.len()];
                match (r >> 16) % 20 {
                    0..=13 => {
                        let y = match (r >> 24) % 8 {
                            0 => -1,
                            1 => sections,
                            2 => 0,
                            3 if sections > 0 => sections - 1,
                            4 if sections > 256 => 255,
                            5 if sections > 256 => 256,
                            _ if sections > 0 => ((r >> 32) % sections as u64) as i32,
                            _ => 0,
                        };
                        let vis = match (r >> 40) % 4 {
                            0 => ALL_VISIBLE,
                            1 => 0,
                            2 => 1,
                            _ => next() & ALL_VISIBLE,
                        };
                        let expected = oracle_set(&mut oracle, (wx, y, wz), vis);
                        let actual = cols.set((wx, y, wz), vis);
                        assert_eq!(actual, expected, "sections={sections} step={step}");
                    }
                    14..=16 => {
                        let detached = cols.detach(wx, wz);
                        let expected = oracle.remove(&(wx, wz));
                        assert_eq!(detached.is_some(), expected.is_some());
                        if let Some(slot) = detached {
                            assert_eq!(cols.slab.column(slot), expected.unwrap().as_slice());
                            cols.slab.free(slot);
                        }
                    }
                    17 | 18 => {
                        let y = if sections > 0 {
                            ((r >> 24) % (sections as u64 + 2)) as i32 - 1
                        } else {
                            0
                        };
                        assert_eq!(cols.get((wx, y, wz)), oracle_get(&oracle, (wx, y, wz)));
                    }
                    _ => {
                        cols.clear();
                        oracle.clear();
                        // Second clear on an already-empty slab must remain valid.
                        cols.clear();
                    }
                }
                if step.is_multiple_of(200) || step + 1 == ops {
                    cols.assert_invariants();
                    assert_eq!(cols.map.len(), oracle.len());
                    for (&(cx, cz), exp_col) in &oracle {
                        let slot = cols.get_slot(cx, cz);
                        assert_ne!(slot, EMPTY_SLOT);
                        assert_eq!(cols.slab.column(slot), exp_col.as_slice());
                    }
                }
            }
        }
    }

    /// Differential check of `Graph::search` + `ColumnSlab` + `col_slots` +
    /// `seed_outside` zero-alloc sort against `reference()` across negative
    /// coordinates, sparse and dense worlds, inside/above/below-world cameras,
    /// and clamped vs unclamped vertical boxes.
    #[test]
    fn differential_search_negative_coords_sparse_and_dense() {
        let mut seed = 0xa5a5_5a5a_dead_beef_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut scratch = SearchScratch::default();
        let mut graph = Graph::default();

        // Unclamped (sections = 6) exact cell-for-cell differential across
        // negative coordinates, sparse and dense layouts, and colliding keys.
        for &(origin_x, origin_z, sparse) in &[
            (-1024, -2048, true),
            (-1024, -2048, false),
            (-65, 127, true),
            (-130, -130, false),
        ] {
            let mut o = Occlusion::new(-64, 6);
            let radius = 6;
            for dx in -radius - 1..=radius + 1 {
                for dz in -radius - 1..=radius + 1 {
                    if sparse && !next().is_multiple_of(9) {
                        continue;
                    }
                    for y in 0..6 {
                        let vis = match next() & 3 {
                            0 => 0,
                            1 => next() & ALL_VISIBLE,
                            2 => (next() | next()) & ALL_VISIBLE,
                            _ => ALL_VISIBLE,
                        };
                        o.set((origin_x + dx, y, origin_z + dz), vis);
                    }
                }
            }
            // Also plant distant toroidal-colliding columns (offset by +/- 16/32/64).
            for offset in [16, -32, 64] {
                o.set((origin_x + offset, 2, origin_z), 0);
                o.set((origin_x, 3, origin_z + offset), connects(0, 1));
            }
            o.columns.assert_invariants();

            for &(dy, off) in &[
                (2, DVec3::new(3.3, -5.7, 1.9)),
                (0, DVec3::new(11.2, 2.4, 14.8)),
                (5, DVec3::new(0.5, 15.1, 0.2)),
                // Above the world -> exercises `seed_outside` (top layer).
                (9, DVec3::new(7.4, 4.0, 12.1)),
                // Below the world -> exercises `seed_outside` (bottom layer).
                (-3, DVec3::new(2.1, -1.5, 9.6)),
            ] {
                let cam_sec = (origin_x, dy, origin_z);
                let camera = DVec3::new(
                    f64::from(cam_sec.0 * 16) + off.x,
                    f64::from(cam_sec.1 * 16 - 64) + off.y,
                    f64::from(cam_sec.2 * 16) + off.z,
                );
                // Settle twice in case first settle finishes prior edits.
                o.settle(camera, radius);
                o.settle(camera, radius);
                assert!(o.reach_is_exact());
                let g = o.graph.as_ref().expect("searched");
                graph.search(&o.columns, camera, radius, &mut scratch);
                assert_eq!(g.directions, graph.directions);
                assert_eq!(g.entered, graph.entered);
                assert_eq!(g.bits, graph.bits);
                assert_eq!(g.sky_reached, graph.sky_reached);

                let full = reference(&o.columns, camera, radius, true);
                let from = o.columns.section_of(camera);
                let side = 2 * radius + 1;
                let full_h = o.columns.sections + 2;
                for x in from.0 - radius..=from.0 + radius {
                    for z in from.2 - radius..=from.2 + radius {
                        for y in -1..=o.columns.sections {
                            let expected =
                                full[(((z - from.2 + radius) * side + x - from.0 + radius) * full_h
                                    + y
                                    + 1) as usize];
                            let actual = o.contains((x, y, z));
                            if g.height == full_h {
                                assert_eq!(
                                    actual, expected,
                                    "origin=({origin_x},{origin_z}) sparse={sparse} dy={dy} key=({x},{y},{z})"
                                );
                            } else {
                                assert!(
                                    !expected || actual,
                                    "clamped search hid reachable ({x},{y},{z})"
                                );
                                if actual && !expected {
                                    assert!(y >= g.y0 + g.height);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Clamped 24-section world at negative coords: verify max_non_air clamp
        // matches full reference conservatively only in the sky band.
        let mut o = Occlusion::new(-64, 24);
        for x in -206..=-194 {
            for z in -106..=-94 {
                for y in 0..5 {
                    let vis = if next().is_multiple_of(3) {
                        0
                    } else {
                        next() & ALL_VISIBLE
                    };
                    o.set((x, y, z), vis);
                }
            }
        }
        o.columns.assert_invariants();
        let camera = DVec3::new(
            f64::from(-200 * 16) + 5.2,
            f64::from(2 * 16 - 64) + 6.4,
            f64::from(-100 * 16) + 11.7,
        );
        o.settle(camera, 6);
        let g = o.graph.as_ref().expect("searched");
        assert!(g.height < o.columns.sections + 2, "expected vertical clamp");
        assert!(o.reach_is_exact());
        let full = reference(&o.columns, camera, 6, true);
        let from = o.columns.section_of(camera);
        let side = 13;
        let full_h = 26;
        for x in from.0 - 6..=from.0 + 6 {
            for z in from.2 - 6..=from.2 + 6 {
                for y in -1..=24 {
                    let expected = full
                        [(((z - from.2 + 6) * side + x - from.0 + 6) * full_h + y + 1) as usize];
                    let actual = o.contains((x, y, z));
                    assert!(!expected || actual, "hid ({x},{y},{z})");
                    if actual && !expected {
                        assert!(y >= g.y0 + g.height);
                    }
                    assert_eq!(o.reach().unwrap().contains((x, y, z)), actual);
                }
            }
        }
    }

    /// Differential verification of render-thread `propagate_changes` (partial
    /// updates) at negative coordinates, near and distant (>60 blocks) rays,
    /// toroidal collisions, and column removals against an independent
    /// `HashMap`-based partial BFS.
    #[test]
    fn differential_propagate_changes_matches_reference_partial_update() {
        #[derive(Clone, Copy)]
        struct RefNode {
            directions: u8,
            entered: u8,
        }
        #[derive(Clone, Copy)]
        struct RefBox {
            from: SectionKey,
            radius: i32,
            y0: i32,
            height: i32,
            min_y: i32,
            sections: i32,
        }

        fn set_both(
            o: &mut Occlusion,
            vis_map: &mut HashMap<SectionKey, u64>,
            k: SectionKey,
            v: u64,
        ) {
            o.set(k, v);
            if v == ALL_VISIBLE {
                vis_map.remove(&k);
            } else {
                vis_map.insert(k, v);
            }
        }

        fn ref_partial(
            nodes: &mut HashMap<SectionKey, RefNode>,
            sky_reached: &mut bool,
            vis_map: &HashMap<SectionKey, u64>,
            b: RefBox,
            camera: DVec3,
            seeds: &[SectionKey],
        ) -> bool {
            let in_box = |k: SectionKey| {
                (k.0 - b.from.0).abs() <= b.radius
                    && (k.2 - b.from.2).abs() <= b.radius
                    && (b.y0..b.y0 + b.height).contains(&k.1)
            };
            let eye = (
                (camera.x.floor() as i32) >> 4,
                (camera.y.floor() as i32 - b.min_y) >> 4,
                (camera.z.floor() as i32) >> 4,
            );
            let centre = [eye.0 * 16 + 8, eye.1 * 16 + b.min_y + 8, eye.2 * 16 + 8];
            let axis = |d: usize| [1, 2, 0][d / 2];
            let mut queue = VecDeque::new();
            for &k in seeds {
                if in_box(k) && nodes.contains_key(&k) {
                    queue.push_back(k);
                }
            }
            let mut grew = false;
            while let Some(key) = queue.pop_front() {
                if key.1 == b.y0 + b.height - 1 {
                    *sky_reached = true;
                }
                let node = nodes[&key];
                let vis = vis_map.get(&key).copied().unwrap_or(ALL_VISIBLE);
                let exits = if node.entered == 0 || vis == ALL_VISIBLE {
                    0x3f
                } else {
                    let mut out = 0u8;
                    for f in 0..6 {
                        if node.entered & (1 << f) != 0 {
                            out |= (vis >> (f * 6)) as u8;
                        }
                    }
                    out & 0x3f
                };
                let distant = (key.0 - eye.0).abs() > DISTANT_SECTIONS
                    || (key.1 - eye.1).abs() > DISTANT_SECTIONS
                    || (key.2 - eye.2).abs() > DISTANT_SECTIONS;
                for (d, step) in STEPS.iter().enumerate() {
                    if exits & (1 << d) == 0 || node.directions & (1 << (d ^ 1)) != 0 {
                        continue;
                    }
                    let next = (key.0 + step.0, key.1 + step.1, key.2 + step.2);
                    if !in_box(next) {
                        continue;
                    }
                    if distant {
                        let origin = [key.0 * 16, key.1 * 16 + b.min_y, key.2 * 16];
                        let far = |c: usize| {
                            if c == axis(d) {
                                centre[c] > origin[c]
                            } else {
                                centre[c] < origin[c]
                            }
                        };
                        let mut check = DVec3::new(
                            f64::from(origin[0] + if far(0) { 16 } else { 0 }),
                            f64::from(origin[1] + if far(1) { 16 } else { 0 }),
                            f64::from(origin[2] + if far(2) { 16 } else { 0 }),
                        );
                        let len = |v: DVec3| (v.x * v.x + (v.y * v.y + v.z * v.z)).sqrt();
                        let to = camera - check;
                        let step_v = to * (1.0 / len(to)) * SECTION_DIAGONAL;
                        let mut visible = true;
                        let max_y = f64::from(b.min_y + b.sections * 16 - 1);
                        let min_yf = f64::from(b.min_y);
                        while {
                            let d = check - camera;
                            d.x * d.x + (d.y * d.y + d.z * d.z) > DISTANT_SQR
                        } {
                            check += step_v;
                            if check.y > max_y || check.y < min_yf {
                                break;
                            }
                            let sec = (
                                floor(check.x) >> 4,
                                (floor(check.y) - b.min_y) >> 4,
                                floor(check.z) >> 4,
                            );
                            let hit = if in_box(sec) {
                                nodes.contains_key(&sec)
                            } else {
                                *sky_reached
                                    && (sec.0 - b.from.0).abs() <= b.radius
                                    && (sec.2 - b.from.2).abs() <= b.radius
                                    && sec.1 >= b.y0 + b.height
                                    && (0..=b.sections).contains(&sec.1)
                            };
                            if !hit {
                                visible = false;
                                break;
                            }
                        }
                        if !visible {
                            continue;
                        }
                    }
                    if let Some(existing) = nodes.get_mut(&next) {
                        existing.entered |= 1 << (d ^ 1);
                    } else {
                        nodes.insert(
                            next,
                            RefNode {
                                directions: node.directions | (1 << d),
                                entered: 1 << (d ^ 1),
                            },
                        );
                        queue.push_back(next);
                        grew = true;
                    }
                }
            }
            grew
        }

        let (ox, oz) = (-512, -256);
        let radius = 8;
        let mut o = Occlusion::new(-64, 6);
        let mut vis_map: HashMap<SectionKey, u64> = HashMap::new();

        // Seal a box around camera and build a distant wall at dx = +4 (> 60 blocks).
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                for y in 0..6 {
                    if dx == 1 || dx == 4 || dz == -2 {
                        set_both(&mut o, &mut vis_map, (ox + dx, y, oz + dz), 0);
                    }
                }
            }
        }
        // Add toroidal-colliding columns outside the box.
        for shift in [16, 64, 128] {
            set_both(&mut o, &mut vis_map, (ox + 1 + shift, 2, oz), 0);
            set_both(&mut o, &mut vis_map, (ox + 4 + shift, 2, oz), 0);
        }
        let camera = DVec3::new(
            f64::from(ox * 16) + 6.5,
            f64::from(2 * 16 - 64) + 8.0,
            f64::from(oz * 16) + 9.2,
        );
        o.settle(camera, radius);
        o.settle(camera, radius);

        // Snapshot initial reached state into reference map.
        let (mut ref_nodes, mut ref_sky, graph_box) = {
            let g = o.graph.as_ref().expect("searched");
            let mut m = HashMap::new();
            for x in g.from.0 - g.radius..=g.from.0 + g.radius {
                for z in g.from.2 - g.radius..=g.from.2 + g.radius {
                    for y in g.y0..g.y0 + g.height {
                        let i = g.index((x, y, z)).unwrap();
                        if g.directions[i] & REACHED != 0 {
                            m.insert(
                                (x, y, z),
                                RefNode {
                                    directions: g.directions[i] & !REACHED,
                                    entered: g.entered[i],
                                },
                            );
                        }
                    }
                }
            }
            (
                m,
                g.sky_reached,
                RefBox {
                    from: g.from,
                    radius: g.radius,
                    y0: g.y0,
                    height: g.height,
                    min_y: g.min_y,
                    sections: g.sections,
                },
            )
        };

        // Phase 1: punch a doorway in the near wall (dx = 1) -> propagates up to distant wall.
        // Phase 2: punch a doorway in the distant wall (dx = 4) -> exercises distant ray walk.
        // Phase 3: remove a whole column on dz = -2 -> exercises remove_column + partial update.
        // Phase 4: unreachable edit behind walls -> generation must NOT move.
        let phases: [&[(SectionKey, Option<u64>)]; 4] = [
            &[
                ((ox + 1, 2, oz), Some(connects(4, 5))),
                ((ox + 1, 2, oz + 1), Some(ALL_VISIBLE)),
            ],
            &[((ox + 4, 2, oz), Some(ALL_VISIBLE))],
            &[((ox, 0, oz - 2), None)], // None => remove_column(ox, oz - 2)
            &[((ox + 7, 3, oz - 7), Some(0))],
        ];
        let mut grew_count = 0;
        for edits in phases {
            let mut seeds = Vec::new();
            for &(k, op) in edits {
                match op {
                    Some(v) => {
                        let before = vis_map.get(&k).copied().unwrap_or(ALL_VISIBLE);
                        set_both(&mut o, &mut vis_map, k, v);
                        if before != v {
                            seeds.push(k);
                        }
                    }
                    None => {
                        o.remove_column(k.0, k.2);
                        for y in 0..6 {
                            if vis_map.remove(&(k.0, y, k.2)).is_some() {
                                seeds.push((k.0, y, k.2));
                            }
                        }
                    }
                }
            }
            let gen_before = o.generation();
            o.propagate_changes(camera);
            let expected_grew = ref_partial(
                &mut ref_nodes,
                &mut ref_sky,
                &vis_map,
                graph_box,
                camera,
                &seeds,
            );
            assert_eq!(o.generation() != gen_before, expected_grew);
            grew_count += usize::from(expected_grew);
            assert!(o.reach_is_exact());
            o.columns.assert_invariants();

            let g = o.graph.as_ref().expect("graph present");
            assert_eq!(g.sky_reached, ref_sky);
            for x in g.from.0 - g.radius..=g.from.0 + g.radius {
                for z in g.from.2 - g.radius..=g.from.2 + g.radius {
                    for y in g.y0..g.y0 + g.height {
                        let i = g.index((x, y, z)).unwrap();
                        match ref_nodes.get(&(x, y, z)) {
                            Some(rn) => {
                                assert_eq!(g.directions[i], rn.directions | REACHED, "{x},{y},{z}");
                                assert_eq!(g.entered[i], rn.entered, "{x},{y},{z}");
                                assert!(g.is_reached(i));
                            }
                            None => {
                                assert_eq!(g.directions[i], 0, "{x},{y},{z}");
                                assert!(!g.is_reached(i));
                            }
                        }
                    }
                }
            }
        }
        assert!(grew_count >= 2, "expected multiple growing partial updates");

        // Follow-up full search must match vanilla reference bit-for-bit.
        o.settle(camera, radius);
        o.settle(camera, radius);
        assert_eq!(reached(&o), reference(&o.columns, camera, radius, true));
    }

    /// Exercises full RD 32 column count (4,761 columns -> 128x128 toroidal grid
    /// with 0 collisions), +140-chunk east & south flight wraparound across
    /// toroidal boundaries, differential search at the wrapped coordinates, and
    /// zero-allocation steady-state `SearchScratch` / `Graph` buffer reuse for
    /// both inside-world and above-world (`seed_outside`) RD 32 searches.
    #[test]
    fn rd32_toroidal_wraparound_and_zero_alloc_scratch_reuse() {
        let mut o = Occlusion::new(-64, 24);
        // Populate full 69x69 RD 32 column window (4,761 columns) with solid
        // top layer y=23 and varied cave sections so height=26 and BFS is fast.
        for x in -34..=34 {
            for z in -34..=34 {
                o.set((x, 23, z), 0);
                if (x + z).rem_euclid(3) == 0 {
                    o.set((x, 2, z), 0);
                }
            }
        }
        o.columns.assert_invariants();
        assert_eq!(o.columns.map.len(), 69 * 69);
        assert_eq!(o.columns.grid_shift, 7, "RD 32 promotes to 128x128 grid");
        assert!(
            o.columns.grid.iter().all(|c| c.overflow == 0),
            "RD 32 window must have zero toroidal collisions"
        );

        // Seal a small room at y=2 around (-3..=3, -3..=3) so inside-world RD 32
        // search runs full 4,225-column setup while visiting a small pocket.
        for x in -4..=4 {
            for z in -4..=4 {
                for y in 1..=3 {
                    o.set((x, y, z), 0);
                }
            }
        }
        let mut graph = Graph::default();
        let mut scratch = SearchScratch::default();
        let inside_cam = |cx: i32, cz: i32| {
            DVec3::new(
                f64::from(cx * 16) + 8.0,
                f64::from(2 * 16 - 64) + 8.0,
                f64::from(cz * 16) + 8.0,
            )
        };
        let above_cam = |cx: i32, cz: i32| {
            DVec3::new(
                f64::from(cx * 16) + 5.3,
                f64::from(26 * 16 - 64) + 4.0,
                f64::from(cz * 16) + 11.7,
            )
        };

        // Warm up buffers at RD 32 (radius = 32, side = 65, 4,225 columns) for
        // both inside-world and above-world (`seed_outside`) searches.
        graph.search(&o.columns, inside_cam(0, 0), 32, &mut scratch);
        graph.search(&o.columns, above_cam(0, 0), 32, &mut scratch);
        assert_eq!(scratch.col_slots.len(), 65 * 65);
        assert_eq!(scratch.seeds.len(), 65 * 65);

        let col_slots_buf = (scratch.col_slots.as_ptr(), scratch.col_slots.capacity());
        let seeds_buf = (scratch.seeds.as_ptr(), scratch.seeds.capacity());
        let queue_cap = scratch.queue.capacity();
        let dir_buf = (graph.directions.as_ptr(), graph.directions.capacity());
        let bits_buf = (graph.bits.as_ptr(), graph.bits.capacity());
        let entered_buf = (graph.entered.as_ptr(), graph.entered.capacity());
        let rays_buf = (graph.rays.as_ptr(), graph.rays.capacity());

        for &(cx, cz) in &[(1, -1), (-2, 2), (2, 1), (-1, -2)] {
            graph.search(&o.columns, inside_cam(cx, cz), 32, &mut scratch);
            assert_eq!(
                (scratch.col_slots.as_ptr(), scratch.col_slots.capacity()),
                col_slots_buf
            );
            assert_eq!(
                (scratch.seeds.as_ptr(), scratch.seeds.capacity()),
                seeds_buf
            );
            assert_eq!(scratch.queue.capacity(), queue_cap);
            assert_eq!(
                (graph.directions.as_ptr(), graph.directions.capacity()),
                dir_buf
            );
            assert_eq!((graph.bits.as_ptr(), graph.bits.capacity()), bits_buf);
            assert_eq!(
                (graph.entered.as_ptr(), graph.entered.capacity()),
                entered_buf
            );
            assert_eq!((graph.rays.as_ptr(), graph.rays.capacity()), rays_buf);

            graph.search(&o.columns, above_cam(cx, cz), 32, &mut scratch);
            assert_eq!(
                (scratch.col_slots.as_ptr(), scratch.col_slots.capacity()),
                col_slots_buf
            );
            assert_eq!(
                (scratch.seeds.as_ptr(), scratch.seeds.capacity()),
                seeds_buf
            );
            assert_eq!(scratch.queue.capacity(), queue_cap);
            assert_eq!(
                (graph.directions.as_ptr(), graph.directions.capacity()),
                dir_buf
            );
            assert_eq!((graph.bits.as_ptr(), graph.bits.capacity()), bits_buf);
            assert_eq!(
                (graph.entered.as_ptr(), graph.entered.capacity()),
                entered_buf
            );
            assert_eq!((graph.rays.as_ptr(), graph.rays.capacity()), rays_buf);
        }

        // Simulate flying +140 chunks east and +140 chunks south across the
        // 128-chunk toroidal wraparound boundary while unloading behind.
        let mut min_x = -34;
        let mut min_z = -34;
        for _ in 0..140 {
            let new_x = min_x + 69;
            for z in min_z..min_z + 69 {
                o.remove_column(min_x, z);
                let vis = if (new_x ^ z) & 3 == 0 {
                    0
                } else {
                    connects(new_x.rem_euclid(6) as usize, z.rem_euclid(6) as usize)
                };
                o.set((new_x, (new_x + z).rem_euclid(6), z), vis);
            }
            min_x += 1;
        }
        for _ in 0..140 {
            let new_z = min_z + 69;
            for x in min_x..min_x + 69 {
                o.remove_column(x, min_z);
                let vis = if (x ^ new_z) & 3 == 0 {
                    0
                } else {
                    connects(x.rem_euclid(6) as usize, new_z.rem_euclid(6) as usize)
                };
                o.set((x, (x + new_z).rem_euclid(6), new_z), vis);
            }
            min_z += 1;
        }
        o.columns.assert_invariants();
        assert_eq!(o.columns.map.len(), 69 * 69);
        assert_eq!(o.columns.grid_shift, 7);
        assert!(
            o.columns.grid.iter().all(|c| c.overflow == 0),
            "wrapped RD 32 window must retain zero toroidal collisions"
        );

        // Differential check centered directly on (128, 2, 128) inside the
        // wrapped window (106..175), so the radius-6 box straddles both x=128
        // and z=128 toroidal wraparound seams.
        let wrapped_cam = DVec3::new(
            f64::from(128 * 16) + 3.7,
            f64::from(2 * 16 - 64) + 9.1,
            f64::from(128 * 16) + 12.4,
        );
        o.settle(wrapped_cam, 6);
        o.settle(wrapped_cam, 6);
        assert!(o.reach_is_exact());
        let g = o.graph.as_ref().expect("searched");
        let full = reference(&o.columns, wrapped_cam, 6, true);
        let from = o.columns.section_of(wrapped_cam);
        let side = 13;
        let full_h = o.columns.sections + 2;
        for x in from.0 - 6..=from.0 + 6 {
            for z in from.2 - 6..=from.2 + 6 {
                for y in -1..=o.columns.sections {
                    let expected = full
                        [(((z - from.2 + 6) * side + x - from.0 + 6) * full_h + y + 1) as usize];
                    let actual = o.contains((x, y, z));
                    assert!(!expected || actual, "wrapped search hid ({x},{y},{z})");
                    if actual && !expected {
                        assert!(y >= g.y0 + g.height);
                    }
                }
            }
        }
    }

    /// Verifies that the render-thread fallback (`worker = None`) reuses
    /// `spare` and `scratch` buffers and produces bit-exact identical graphs
    /// to the background worker path.
    #[test]
    fn render_thread_fallback_reuses_spare_and_matches_worker() {
        let mut with_worker = Occlusion::new(0, 6);
        let mut fallback = Occlusion::new(0, 6);
        fallback.worker = None;
        random_world(&mut with_worker, 0x1234_5678, 700, 6);
        random_world(&mut fallback, 0x1234_5678, 700, 6);

        // Warm up both `graph` and `spare` at full height (above-world searches
        // always allocate `sections + 2` and populate `scratch.seeds`).
        let warmup = [
            at((0, 9, 0)) + DVec3::new(1.2, 0.0, 3.4),
            at((2, 9, -2)) + DVec3::new(1.5, 0.0, -2.3),
        ];
        for &cam in &warmup {
            with_worker.settle(cam, 6);
            with_worker.settle(cam, 6);
            fallback.update(cam, 6);
            assert!(fallback.reach_is_exact());
            let wg = with_worker.graph.as_ref().expect("worker searched");
            let fg = fallback.graph.as_ref().expect("fallback searched");
            assert_eq!(fg.directions, wg.directions);
            assert_eq!(fg.entered, wg.entered);
            assert_eq!(fg.bits, wg.bits);
            assert_eq!(fg.sky_reached, wg.sky_reached);
        }
        let buf_a = fallback.graph.as_ref().unwrap().directions.as_ptr();
        let buf_b = fallback.spare.directions.as_ptr();
        let col_slots_ptr = fallback.scratch.col_slots.as_ptr();
        let seeds_ptr = fallback.scratch.seeds.as_ptr();

        let cameras = [
            at((0, 2, 0)) + DVec3::new(2.1, -3.4, 4.7),
            at((1, 10, -1)) + DVec3::new(-1.3, 0.0, 2.7),
            at((-1, -2, 3)) + DVec3::new(-4.2, 0.0, 1.1),
            at((-2, 4, 1)) + DVec3::new(3.0, 2.5, -1.8),
        ];
        for &cam in &cameras {
            with_worker.settle(cam, 6);
            with_worker.settle(cam, 6);
            fallback.update(cam, 6);
            assert!(fallback.reach_is_exact());
            let wg = with_worker.graph.as_ref().expect("worker searched");
            let fg = fallback.graph.as_ref().expect("fallback searched");
            assert_eq!(fg.directions, wg.directions);
            assert_eq!(fg.entered, wg.entered);
            assert_eq!(fg.bits, wg.bits);
            assert_eq!(fg.sky_reached, wg.sky_reached);
            let cur_a = fg.directions.as_ptr();
            let cur_b = fallback.spare.directions.as_ptr();
            assert!(
                (cur_a == buf_a && cur_b == buf_b) || (cur_a == buf_b && cur_b == buf_a),
                "fallback must ping-pong warm graph/spare buffers without reallocating"
            );
            assert_eq!(fallback.scratch.col_slots.as_ptr(), col_slots_ptr);
            assert_eq!(fallback.scratch.seeds.as_ptr(), seeds_ptr);
        }
    }

    /// Verifies `walk()` using L1-resident `contains_bit()` matches `reference`
    /// bit-for-bit on distant (> 60 block) rays across walls, pinholes, and
    /// overhangs at negative coordinates.
    #[test]
    fn walk_contains_bit_matches_directions_in_distant_rays() {
        let mut o = Occlusion::new(-64, 6);
        let (ox, oz) = (-300, -150);
        // Wall at dx = +2 with a pinhole at (y=2, dz=0) and overhang at y=4.
        for dz in -4..=4 {
            for y in 0..6 {
                if !(dz == 0 && y == 2) {
                    o.set((ox + 2, y, oz + dz), 0);
                }
            }
        }
        for dx in 3..=7 {
            for dz in -3..=3 {
                o.set((ox + dx, 4, oz + dz), 0);
            }
        }
        let camera = DVec3::new(
            f64::from(ox * 16) + 11.3,
            f64::from(2 * 16 - 64) + 5.7,
            f64::from(oz * 16) + 3.9,
        );
        o.settle(camera, 12);
        o.settle(camera, 12);
        assert!(o.reach_is_exact());
        let actual = reached(&o);
        let with_rays = reference(&o.columns, camera, 12, true);
        let without_rays = reference(&o.columns, camera, 12, false);
        assert_eq!(actual, with_rays);
        let culled_by_rays = without_rays
            .iter()
            .zip(&actual)
            .filter(|(a, b)| **a && !**b)
            .count();
        assert!(
            culled_by_rays >= 10,
            "distant rays must actively cull behind wall ({culled_by_rays})"
        );
    }
}
