//! Resident sections, stored densely and grouped by chunk column for the
//! per-frame culling walk: a column whose box misses the frustum or lies past
//! the render distance rejects all of its sections with one test, and the
//! sections that remain are reached by index rather than by hashing keys.
#[cfg(test)]
use super::Frustum;
use super::{Cull, SectionKey};
use crate::fast_hash::{FxHashMap, FxHashSet};
#[cfg(test)]
use crate::streaming::in_view_distance;
use glam::{DVec3, Vec3};

/// Hot column culling metadata, packed 4 per 64-byte cache line so `walk` rejects
/// out-of-range and out-of-frustum columns without touching per-column section lists.
#[derive(Clone, Copy)]
struct ColumnHeader {
    x: i32,
    z: i32,
    /// The lowest and highest section y of the column's resident sections.
    low: i32,
    high: i32,
}

/// One chunk column's resident section lists and pre-resolved tight bounds.
struct Column {
    /// Section y (from the world's bottom) and slot, in no particular order.
    sections: Vec<(i32, u32)>,
    /// Resolved local bounds parallel to `sections` (`None` stored as `[0, 16]`).
    section_bounds: Vec<[Vec3; 2]>,
    /// Those of `sections` that are marked (`Resident::insert_marked`), in no
    /// particular order.
    marked: Vec<(i32, u32)>,
    /// Resolved local bounds parallel to `marked`.
    marked_bounds: Vec<[Vec3; 2]>,
}

#[inline(always)]
fn resolve_bounds(bounds: Option<[Vec3; 2]>) -> [Vec3; 2] {
    bounds.unwrap_or([Vec3::ZERO, Vec3::splat(16.0)])
}

impl Column {
    fn bounds(&self) -> (i32, i32) {
        let mut iter = self.sections.iter().map(|s| s.0);
        let Some(first) = iter.next() else {
            return (0, 0);
        };
        iter.fold((first, first), |(lo, hi), y| (lo.min(y), hi.max(y)))
    }
}

/// Values of type `T` per section key, in a dense list with stable keys and
/// slots that move only when a section is removed.
pub struct Resident<T> {
    /// The minimum Y of section index 0 (the level's column).
    min_y: i32,
    items: Vec<(SectionKey, T)>,
    bounds: Vec<Option<[Vec3; 2]>>,
    /// Per slot, whether the section is marked: within the render distance,
    /// `walk` lists only those when `marked_only_near`.
    marked: Vec<bool>,
    slots: FxHashMap<SectionKey, u32>,
    column_headers: Vec<ColumnHeader>,
    /// One bit per column: whether `columns[ci].marked` is non-empty, checked
    /// in L1 before touching `columns` or testing the frustum on GPU-culled frames.
    column_marked_bits: Vec<u64>,
    columns: Vec<Column>,
    column_index: FxHashMap<(i32, i32), u32>,
    /// Calls that removed sections so far: a user that keeps its own per-section state
    /// (the ray scene) sees from it whether any went.
    removals: u64,
}

impl<T> Default for Resident<T> {
    fn default() -> Self {
        Self {
            min_y: crate::chunk::MIN_Y,
            items: Vec::new(),
            bounds: Vec::new(),
            marked: Vec::new(),
            slots: FxHashMap::default(),
            column_headers: Vec::new(),
            column_marked_bits: Vec::new(),
            columns: Vec::new(),
            column_index: FxHashMap::default(),
            removals: 0,
        }
    }
}

impl<T> Resident<T> {
    pub fn len(&self) -> usize {
        self.items.len()
    }
    /// Heap bytes of its own lists and maps, not counting what a `T` points
    /// to (`horizon-ctl memory`). A hash map entry costs its pair and a
    /// control byte.
    pub fn heap_bytes(&self) -> usize {
        use std::mem::size_of;
        self.items.capacity() * size_of::<(SectionKey, T)>()
            + self.bounds.capacity() * size_of::<Option<[Vec3; 2]>>()
            + self.marked.capacity() * size_of::<bool>()
            + self.slots.capacity() * (size_of::<(SectionKey, u32)>() + 1)
            + self.column_headers.capacity() * size_of::<ColumnHeader>()
            + self.column_marked_bits.capacity() * size_of::<u64>()
            + self.columns.capacity() * size_of::<Column>()
            + self
                .columns
                .iter()
                .map(|c| {
                    (c.sections.capacity() + c.marked.capacity()) * size_of::<(i32, u32)>()
                        + (c.section_bounds.capacity() + c.marked_bounds.capacity())
                            * size_of::<[Vec3; 2]>()
                })
                .sum::<usize>()
            + self.column_index.capacity() * (size_of::<((i32, i32), u32)>() + 1)
    }
    pub fn contains(&self, key: SectionKey) -> bool {
        self.slots.contains_key(&key)
    }
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.items.iter().map(|(_, v)| v)
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items.iter_mut().map(|(_, v)| v)
    }
    /// Every section's key, value and bounds.
    pub fn iter(&self) -> impl Iterator<Item = (SectionKey, &T, Option<[Vec3; 2]>)> {
        let items = self.items.iter().zip(&self.bounds);
        items.map(|((key, value), bounds)| (*key, value, *bounds))
    }
    /// Every section's key, value and bounds.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (SectionKey, &mut T, Option<[Vec3; 2]>)> {
        let items = self.items.iter_mut().zip(&self.bounds);
        items.map(|((key, value), bounds)| (*key, value, *bounds))
    }
    /// Whether the section at `key` is marked (`insert_marked`).
    #[cfg(any(test, debug_assertions))]
    pub fn is_marked(&self, key: SectionKey) -> bool {
        self.slots
            .get(&key)
            .is_some_and(|&slot| self.marked[slot as usize])
    }
    /// `key`'s value, when it is resident.
    pub fn find(&self, key: SectionKey) -> Option<&T> {
        self.slots.get(&key).map(|&slot| self.get(slot))
    }
    pub fn find_mut(&mut self, key: SectionKey) -> Option<&mut T> {
        let slot = *self.slots.get(&key)?;
        Some(self.get_mut(slot))
    }
    /// The section in `slot` (`Visible::slot`).
    pub fn get(&self, slot: u32) -> &T {
        &self.items[slot as usize].1
    }
    pub fn get_mut(&mut self, slot: u32) -> &mut T {
        &mut self.items[slot as usize].1
    }
    /// Padded mesh bounds of the section in `slot`, relative to its origin.
    pub fn bounds(&self, slot: u32) -> Option<[Vec3; 2]> {
        self.bounds[slot as usize]
    }
    /// Sections from now on count from `min_y` (a level of another column).
    pub fn set_min_y(&mut self, min_y: i32) {
        self.min_y = min_y;
    }
    /// The minimum Y of section index 0.
    pub fn min_y(&self) -> i32 {
        self.min_y
    }
    /// Changes whenever sections were removed (`remove`, `remove_columns`, `clear`).
    pub fn removals(&self) -> u64 {
        self.removals
    }
    pub fn clear(&mut self) {
        self.removals += 1;
        self.items.clear();
        self.bounds.clear();
        self.marked.clear();
        self.slots.clear();
        self.column_headers.clear();
        self.column_marked_bits.clear();
        self.columns.clear();
        self.column_index.clear();
    }
    #[inline(always)]
    fn column_has_marked(&self, ci: usize) -> bool {
        (self.column_marked_bits[ci >> 6] >> (ci & 63)) & 1 != 0
    }
    #[inline(always)]
    fn set_column_marked_bit(&mut self, ci: usize, has: bool) {
        let word = ci >> 6;
        let bit = 1u64 << (ci & 63);
        if has {
            self.column_marked_bits[word] |= bit;
        } else {
            self.column_marked_bits[word] &= !bit;
        }
    }
    /// Adds or replaces `key`'s value with default 16x16x16 bounds.
    #[cfg(test)]
    pub fn insert(&mut self, key: SectionKey, value: T) {
        self.insert_with_bounds(key, value, None);
    }
    /// Adds or replaces `key`'s value with optional tight geometry bounds, unmarked.
    #[cfg(test)]
    pub fn insert_with_bounds(&mut self, key: SectionKey, value: T, bounds: Option<[Vec3; 2]>) {
        self.insert_marked(key, value, bounds, false);
    }
    /// Adds or replaces `key`'s value with optional tight geometry bounds, and
    /// whether it is marked: `walk` with `marked_only_near` lists the sections
    /// within the render distance that are.
    pub fn insert_marked(
        &mut self,
        key: SectionKey,
        value: T,
        bounds: Option<[Vec3; 2]>,
        marked: bool,
    ) {
        let resolved = resolve_bounds(bounds);
        if let Some(&slot) = self.slots.get(&key) {
            self.items[slot as usize].1 = value;
            self.bounds[slot as usize] = bounds;
            let c = self.column_index[&(key.0, key.2)] as usize;
            let col = &mut self.columns[c];
            let at = col
                .sections
                .iter()
                .position(|s| s.1 == slot)
                .expect("section in its column");
            col.section_bounds[at] = resolved;
            if self.marked[slot as usize] == marked {
                if marked {
                    let mat = col
                        .marked
                        .iter()
                        .position(|s| s.1 == slot)
                        .expect("marked section in its column's list");
                    col.marked_bounds[mat] = resolved;
                }
            } else {
                self.mark_slot(slot, marked);
            }
            return;
        }
        let slot = self.items.len() as u32;
        self.items.push((key, value));
        self.bounds.push(bounds);
        self.marked.push(marked);
        self.slots.insert(key, slot);
        let c = match self.column_index.get(&(key.0, key.2)) {
            Some(&c) => c as usize,
            None => {
                let c = self.columns.len();
                self.column_index.insert((key.0, key.2), c as u32);
                self.column_headers.push(ColumnHeader {
                    x: key.0,
                    z: key.2,
                    low: key.1,
                    high: key.1,
                });
                if c >> 6 >= self.column_marked_bits.len() {
                    self.column_marked_bits.push(0);
                }
                self.set_column_marked_bit(c, false);
                self.columns.push(Column {
                    sections: Vec::new(),
                    section_bounds: Vec::new(),
                    marked: Vec::new(),
                    marked_bounds: Vec::new(),
                });
                c
            }
        };
        let column = &mut self.columns[c];
        column.sections.push((key.1, slot));
        column.section_bounds.push(resolved);
        if marked {
            column.marked.push((key.1, slot));
            column.marked_bounds.push(resolved);
            self.set_column_marked_bit(c, true);
        }
        let header = &mut self.column_headers[c];
        header.low = header.low.min(key.1);
        header.high = header.high.max(key.1);
    }
    /// Marks or unmarks the section at `key`, if it is resident.
    pub fn set_marked(&mut self, key: SectionKey, marked: bool) {
        if let Some(&slot) = self.slots.get(&key) {
            self.mark_slot(slot, marked);
        }
    }
    /// Marks or unmarks the section in `slot`, in its column's list too.
    fn mark_slot(&mut self, slot: u32, marked: bool) {
        let was = std::mem::replace(&mut self.marked[slot as usize], marked);
        if was == marked {
            return;
        }
        let (x, y, z) = self.items[slot as usize].0;
        let c = self.column_index[&(x, z)] as usize;
        let resolved = resolve_bounds(self.bounds[slot as usize]);
        let column = &mut self.columns[c];
        if marked {
            column.marked.push((y, slot));
            column.marked_bounds.push(resolved);
            self.set_column_marked_bit(c, true);
        } else {
            let at = column
                .marked
                .iter()
                .position(|s| s.1 == slot)
                .expect("marked section in its column's list");
            column.marked.swap_remove(at);
            column.marked_bounds.swap_remove(at);
            if column.marked.is_empty() {
                self.set_column_marked_bit(c, false);
            }
        }
    }
    /// Removes every section of the `gone` columns, the others keeping their order and
    /// marks: one pass over the store, where `remove` moves a slot and looks its column
    /// up for each.
    pub fn remove_columns(&mut self, gone: &FxHashSet<(i32, i32)>) -> Vec<T> {
        self.removals += 1;
        // Every column (a far teleport): emptied, without a lookup per section.
        if self
            .column_headers
            .iter()
            .all(|h| gone.contains(&(h.x, h.z)))
        {
            self.bounds.clear();
            self.marked.clear();
            self.slots.clear();
            self.column_headers.clear();
            self.column_marked_bits.clear();
            self.columns.clear();
            self.column_index.clear();
            return std::mem::take(&mut self.items)
                .into_iter()
                .map(|(_, value)| value)
                .collect();
        }
        let items = std::mem::take(&mut self.items);
        let bounds = std::mem::take(&mut self.bounds);
        let marked = std::mem::take(&mut self.marked);
        self.slots.clear();
        self.column_headers.clear();
        self.column_marked_bits.clear();
        self.columns.clear();
        self.column_index.clear();
        let mut removed = Vec::new();
        for (((key, value), bounds), marked) in items.into_iter().zip(bounds).zip(marked) {
            if gone.contains(&(key.0, key.2)) {
                removed.push(value);
            } else {
                self.insert_marked(key, value, bounds, marked);
            }
        }
        removed
    }
    pub fn remove(&mut self, key: SectionKey) -> Option<T> {
        let slot = self.slots.remove(&key)?;
        self.removals += 1;
        self.mark_slot(slot, false);
        let c = self.column_index[&(key.0, key.2)] as usize;
        let column = &mut self.columns[c];
        let at = column
            .sections
            .iter()
            .position(|s| s.1 == slot)
            .expect("section in its column");
        column.sections.swap_remove(at);
        column.section_bounds.swap_remove(at);
        if column.sections.is_empty() {
            self.column_index.remove(&(key.0, key.2));
            let last_c = self.columns.len() - 1;
            self.columns.swap_remove(c);
            self.column_headers.swap_remove(c);
            let last_marked = self.column_has_marked(last_c);
            self.set_column_marked_bit(c, last_marked);
            self.set_column_marked_bit(last_c, false);
            if let Some(moved) = self.column_headers.get(c) {
                self.column_index.insert((moved.x, moved.z), c as u32);
            }
        } else {
            let header = &mut self.column_headers[c];
            if key.1 == header.low || key.1 == header.high {
                let (lo, hi) = column.bounds();
                header.low = lo;
                header.high = hi;
            }
        }
        let _ = self.bounds.swap_remove(slot as usize);
        let _ = self.marked.swap_remove(slot as usize);
        let (_, value) = self.items.swap_remove(slot as usize);
        // The last item now sits in `slot`: point its key and column, and its
        // column's marked list if it is marked, at it.
        if let Some(&(moved, _)) = self.items.get(slot as usize) {
            let last = self.items.len() as u32;
            self.slots.insert(moved, slot);
            let column = &mut self.columns[self.column_index[&(moved.0, moved.2)] as usize];
            let entry = column
                .sections
                .iter_mut()
                .find(|s| s.1 == last)
                .expect("moved section in its column");
            entry.1 = slot;
            if self.marked[slot as usize] {
                let entry = column
                    .marked
                    .iter_mut()
                    .find(|s| s.1 == last)
                    .expect("moved marked section in its column's list");
                entry.1 = slot;
            }
        }
        Some(value)
    }
}

/// A section in view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visible {
    /// Its centre's squared distance from the camera, as bits: non-negative
    /// floats order like their bits, so sorting compares integers.
    pub distance: u64,
    pub key: SectionKey,
    pub slot: u32,
    /// Its camera-relative origin.
    pub origin: [f32; 3],
}

/// The camera-relative origin of the section at `key`.
pub(super) fn origin(key: SectionKey, min_y: i32, camera: DVec3) -> DVec3 {
    DVec3::new(
        f64::from(key.0 * 16),
        f64::from(min_y + key.1 * 16),
        f64::from(key.2 * 16),
    ) - camera
}

/// The camera's chunk column.
pub(super) fn camera_column(camera: DVec3) -> (i32, i32) {
    (
        (camera.x.floor() as i32) >> 4,
        (camera.z.floor() as i32) >> 4,
    )
}

/// The world AABB (min, max) of a section from its origin and optional tight bounds.
#[cfg(test)]
#[inline]
fn section_box(min: Vec3, bounds: Option<[Vec3; 2]>) -> (Vec3, Vec3) {
    match bounds {
        Some([bmin, bmax]) => (min + bmin, min + bmax),
        None => (min, min + Vec3::splat(16.0)),
    }
}

/// Number of precomputed section-Y offsets kept on the stack per `walk`.
/// Covers all standard Overworld/Nether/End section indices (`0..24`).
const Y_TABLE_LEN: usize = 32;

#[inline(always)]
fn column_oy_f32(y: i32, min_y: i32, cam_y: f64, table: &[f32; Y_TABLE_LEN]) -> f32 {
    if (0..Y_TABLE_LEN as i32).contains(&y) {
        table[y as usize]
    } else {
        (f64::from(min_y + y * 16) - cam_y) as f32
    }
}

#[inline(always)]
fn section_oy(
    y: i32,
    min_y: i32,
    cam_y: f64,
    y_oy_f32: &[f32; Y_TABLE_LEN],
    y_dy8_sq: &[f64; Y_TABLE_LEN],
) -> (f32, f64) {
    if (0..Y_TABLE_LEN as i32).contains(&y) {
        let i = y as usize;
        (y_oy_f32[i], y_dy8_sq[i])
    } else {
        let oy = f64::from(min_y + y * 16) - cam_y;
        let dy8 = oy + 8.0;
        (oy as f32, dy8 * dy8)
    }
}

/// In-place 512-bucket American Flag Sort on `Visible::distance` followed by
/// L1-resident comparison sorts per bucket, preserving exact `(distance, key)` order.
///
/// `256.0_f64.to_bits() >> 47 == 32_992` (16 blocks = 1 chunk); 512 bins span
/// 16 powers of two in squared distance (`[16, 4096]` blocks = RD 1..256), with
/// 64 bins across the outer half of any render distance.
const SORT_BUCKETS: usize = 512;
const BUCKET_BASE: u64 = 32_992;

#[inline(always)]
fn distance_bucket(bits: u64) -> usize {
    (bits >> 47)
        .saturating_sub(BUCKET_BASE)
        .min((SORT_BUCKETS - 1) as u64) as usize
}

#[inline]
fn sort_visible(out: &mut [Visible]) {
    if out.len() <= 1 {
        return;
    }
    if out.len() < 256 {
        out.sort_unstable_by(|a, b| a.distance.cmp(&b.distance).then_with(|| a.key.cmp(&b.key)));
        return;
    }
    let mut ends = [0usize; SORT_BUCKETS];
    for v in &*out {
        ends[distance_bucket(v.distance)] += 1;
    }
    let mut offsets = [0usize; SORT_BUCKETS];
    let mut sum = 0usize;
    for (offset, end) in offsets.iter_mut().zip(&mut ends) {
        let c = *end;
        *offset = sum;
        sum += c;
        *end = sum;
    }
    for b in 0..SORT_BUCKETS {
        while offsets[b] < ends[b] {
            let cur = offsets[b];
            let target = distance_bucket(out[cur].distance);
            if target == b {
                offsets[b] = cur + 1;
            } else {
                let dst = offsets[target];
                offsets[target] = dst + 1;
                out.swap(cur, dst);
            }
        }
    }
    let mut start = 0usize;
    for &end in &ends {
        if end - start > 1 {
            out[start..end].sort_unstable_by(|a, b| {
                a.distance.cmp(&b.distance).then_with(|| a.key.cmp(&b.key))
            });
        }
        start = end;
    }
}

/// Whether the section at `key` is drawn: its column within the render
/// distance of the camera's (`SectionOcclusionGraph.isInViewDistance`), and
/// partly inside the frustum. `None` when it is not, else its squared centre
/// distance and origin. `Resident::visible` walks columns instead.
#[cfg(test)]
fn in_view(key: SectionKey, camera: DVec3, radius: i32, frustum: &Frustum) -> Option<(f64, Vec3)> {
    in_view_with_bounds(key, camera, radius, frustum, None)
}

/// `in_view` with optional tight bounds.
#[cfg(test)]
fn in_view_with_bounds(
    key: SectionKey,
    camera: DVec3,
    radius: i32,
    frustum: &Frustum,
    bounds: Option<[Vec3; 2]>,
) -> Option<(f64, Vec3)> {
    if !in_view_distance(camera_column(camera), radius, key.0, key.2) {
        return None;
    }
    in_frustum(key, crate::chunk::MIN_Y, camera, frustum, false, bounds)
}

/// `in_view` without the distance test; `inside`: the section's column is
/// entirely inside the frustum, so the section is too.
#[cfg(test)]
fn in_frustum(
    key: SectionKey,
    min_y: i32,
    camera: DVec3,
    frustum: &impl Cull,
    inside: bool,
    bounds: Option<[Vec3; 2]>,
) -> Option<(f64, Vec3)> {
    let origin = origin(key, min_y, camera);
    let min = origin.as_vec3();
    if !inside {
        let (bmin, bmax) = section_box(min, bounds);
        if !frustum.intersects(bmin, bmax) {
            return None;
        }
    }
    Some(((origin + DVec3::splat(8.0)).length_squared(), min))
}

impl<T> Resident<T> {
    /// Appends the sections `in_view` and `reachable` to `out`, nearest
    /// first (ties by key, so the order never depends on storage), and the
    /// sections in the frustum of columns past the render distance that
    /// `beyond` keeps (distant terrain still leaves them out; the occlusion
    /// graph never reaches them).
    ///
    /// Columns are tested first, with the same distance test (it depends
    /// only on x and z) and a box spanning their sections: every section box
    /// lies inside it and the frustum test rounds monotonically, so a column
    /// the frustum rejects holds no section it would accept, and one
    /// entirely inside holds none it would reject (its sections skip the
    /// test). `radius` is the render distance in chunks.
    pub fn visible(
        &self,
        camera: DVec3,
        radius: i32,
        frustum: &impl Cull,
        reachable: impl Fn(SectionKey) -> bool,
        beyond: impl Fn(i32, i32) -> bool,
        out: &mut Vec<Visible>,
    ) {
        self.walk(camera, radius, frustum, reachable, beyond, false, out);
    }
    /// `visible`, less the sections within the render distance that are not
    /// marked (`insert_marked`) when `marked_only_near`, in the same order: GPU
    /// culling's frames, where the GPU draws those. A column's box spans all of
    /// its sections either way, so both decide alike on those they share.
    pub fn walk(
        &self,
        camera: DVec3,
        radius: i32,
        frustum: &impl Cull,
        reachable: impl Fn(SectionKey) -> bool,
        beyond: impl Fn(i32, i32) -> bool,
        marked_only_near: bool,
        out: &mut Vec<Visible>,
    ) {
        let from = camera_column(camera);
        let radius_sq = i64::from(radius) * i64::from(radius);
        let tight = frustum.tight();
        let min_y = self.min_y;
        let cam_x = camera.x;
        let cam_y = camera.y;
        let cam_z = camera.z;

        // Precompute camera-relative Y offsets and (oy + 8)^2 once for y in 0..32.
        let mut y_oy_f32 = [0.0f32; Y_TABLE_LEN];
        let mut y_dy8_sq = [0.0f64; Y_TABLE_LEN];
        let min_y_i64 = i64::from(min_y);
        for (i, (oy_slot, dy8_slot)) in y_oy_f32.iter_mut().zip(&mut y_dy8_sq).enumerate() {
            let oy = (min_y_i64 + i as i64 * 16) as f64 - cam_y;
            *oy_slot = oy as f32;
            let dy8 = oy + 8.0;
            *dy8_slot = dy8 * dy8;
        }

        for (ci, h) in self.column_headers.iter().enumerate() {
            let dx = i64::from(((h.x - from.0).abs() - 1).max(0));
            let dz = i64::from(((h.z - from.1).abs() - 1).max(0));
            let near = dx * dx + dz * dz < radius_sq;
            if near {
                if marked_only_near && !self.column_has_marked(ci) {
                    continue;
                }
            } else if !beyond(h.x, h.z) {
                continue;
            }
            let ox = f64::from(h.x * 16) - cam_x;
            let oz = f64::from(h.z * 16) - cam_z;
            let min_x = ox as f32;
            let min_z = oz as f32;
            let low_oy = column_oy_f32(h.low, min_y, cam_y, &y_oy_f32);
            let high_oy = if h.high == h.low {
                low_oy
            } else {
                column_oy_f32(h.high, min_y, cam_y, &y_oy_f32)
            };
            let col_min = Vec3::new(min_x, low_oy, min_z);
            let col_max = Vec3::new(min_x + 16.0, high_oy + 16.0, min_z + 16.0);
            // Reject out-of-frustum columns (~75-80%) before testing `contains`
            // or touching per-column section lists.
            if !frustum.intersects(col_min, col_max) {
                continue;
            }
            let inside = frustum.contains(col_min, col_max);
            let dx8 = ox + 8.0;
            let dz8 = oz + 8.0;
            let xz_sq = dx8 * dx8 + dz8 * dz8;

            let column = &self.columns[ci];
            let (list, bounds_list) = if marked_only_near && near {
                (&column.marked, &column.marked_bounds)
            } else {
                (&column.sections, &column.section_bounds)
            };

            if inside {
                if near {
                    for &(y, slot) in list {
                        let key = (h.x, y, h.z);
                        if !reachable(key) {
                            continue;
                        }
                        let (oy_f32, dy8_sq) = section_oy(y, min_y, cam_y, &y_oy_f32, &y_dy8_sq);
                        let distance = xz_sq + dy8_sq;
                        out.push(Visible {
                            distance: distance.to_bits(),
                            key,
                            slot,
                            origin: [min_x, oy_f32, min_z],
                        });
                    }
                } else {
                    for &(y, slot) in list {
                        let (oy_f32, dy8_sq) = section_oy(y, min_y, cam_y, &y_oy_f32, &y_dy8_sq);
                        let distance = xz_sq + dy8_sq;
                        out.push(Visible {
                            distance: distance.to_bits(),
                            key: (h.x, y, h.z),
                            slot,
                            origin: [min_x, oy_f32, min_z],
                        });
                    }
                }
            } else if tight {
                if near {
                    for (&(y, slot), &[bmin_off, bmax_off]) in list.iter().zip(bounds_list) {
                        let key = (h.x, y, h.z);
                        if !reachable(key) {
                            continue;
                        }
                        let (oy_f32, dy8_sq) = section_oy(y, min_y, cam_y, &y_oy_f32, &y_dy8_sq);
                        let min = Vec3::new(min_x, oy_f32, min_z);
                        if !frustum.intersects(min + bmin_off, min + bmax_off) {
                            continue;
                        }
                        let distance = xz_sq + dy8_sq;
                        out.push(Visible {
                            distance: distance.to_bits(),
                            key,
                            slot,
                            origin: [min_x, oy_f32, min_z],
                        });
                    }
                } else {
                    for (&(y, slot), &[bmin_off, bmax_off]) in list.iter().zip(bounds_list) {
                        let (oy_f32, dy8_sq) = section_oy(y, min_y, cam_y, &y_oy_f32, &y_dy8_sq);
                        let min = Vec3::new(min_x, oy_f32, min_z);
                        if !frustum.intersects(min + bmin_off, min + bmax_off) {
                            continue;
                        }
                        let distance = xz_sq + dy8_sq;
                        out.push(Visible {
                            distance: distance.to_bits(),
                            key: (h.x, y, h.z),
                            slot,
                            origin: [min_x, oy_f32, min_z],
                        });
                    }
                }
            } else {
                let sixteen = Vec3::splat(16.0);
                if near {
                    for &(y, slot) in list {
                        let key = (h.x, y, h.z);
                        if !reachable(key) {
                            continue;
                        }
                        let (oy_f32, dy8_sq) = section_oy(y, min_y, cam_y, &y_oy_f32, &y_dy8_sq);
                        let min = Vec3::new(min_x, oy_f32, min_z);
                        if !frustum.intersects(min, min + sixteen) {
                            continue;
                        }
                        let distance = xz_sq + dy8_sq;
                        out.push(Visible {
                            distance: distance.to_bits(),
                            key,
                            slot,
                            origin: [min_x, oy_f32, min_z],
                        });
                    }
                } else {
                    for &(y, slot) in list {
                        let (oy_f32, dy8_sq) = section_oy(y, min_y, cam_y, &y_oy_f32, &y_dy8_sq);
                        let min = Vec3::new(min_x, oy_f32, min_z);
                        if !frustum.intersects(min, min + sixteen) {
                            continue;
                        }
                        let distance = xz_sq + dy8_sq;
                        out.push(Visible {
                            distance: distance.to_bits(),
                            key: (h.x, y, h.z),
                            slot,
                            origin: [min_x, oy_f32, min_z],
                        });
                    }
                }
            }
        }
        sort_visible(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn column_walk_finds_exactly_the_sections_in_view() {
        let mut r = Resident::default();
        for x in -12i32..=12 {
            for z in -12..=12 {
                for y in 0..24 {
                    // Leave holes so columns have uneven bounds.
                    if (x * 7 + y * 3 + z * 5).rem_euclid(4) != 0 {
                        // Mix of sections with fallback None and randomized tight bounds.
                        let bounds = if (x + y + z).rem_euclid(2) == 0 {
                            let y_min = ((x + z).rem_euclid(4)) as f32;
                            let y_max = y_min + 2.0 + ((y.rem_euclid(5)) as f32);
                            Some([Vec3::new(0.0, y_min, 0.0), Vec3::new(16.0, y_max, 16.0)])
                        } else {
                            None
                        };
                        r.insert_with_bounds((x, y, z), (), bounds);
                    }
                }
            }
        }
        let reachable = |k: SectionKey| (k.0 + k.1 + k.2).rem_euclid(9) != 0;
        let cameras = [
            (DVec3::new(0.5, 70.0, 0.5), 0.0, 0.0),
            (DVec3::new(-37.25, 130.5, 21.75), 135.0, 35.0),
            (DVec3::new(100.0, -60.0, -3.0), 270.0, -80.0),
            (DVec3::new(8.0, 64.0, 8.0), 45.0, 89.0),
        ];
        for (camera, yaw, pitch) in cameras {
            for radius in [2, 6, 10] {
                let projection = super::super::world_projection(70.0, 16.0 / 9.0, 512.0);
                let view = crate::camera::view_matrix(yaw, pitch);
                let frustum = Frustum {
                    planes: super::super::frustum(projection * view),
                };
                let mut fast = Vec::new();
                // Columns past the distance kept for distant terrain.
                let beyond = |x: i32, z: i32| (x + z).rem_euclid(3) == 0;
                r.visible(camera, radius, &frustum, reachable, beyond, &mut fast);
                let mut slow: Vec<(SectionKey, f64, [f32; 3])> = r
                    .items
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, &(k, _))| {
                        let near = in_view_distance(camera_column(camera), radius, k.0, k.2);
                        if near {
                            in_view_with_bounds(k, camera, radius, &frustum, r.bounds[slot])
                                .filter(|_| reachable(k))
                        } else if beyond(k.0, k.2) {
                            in_frustum(
                                k,
                                crate::chunk::MIN_Y,
                                camera,
                                &frustum,
                                false,
                                r.bounds[slot],
                            )
                        } else {
                            None
                        }
                        .map(|(d, o)| (k, d, o.to_array()))
                    })
                    .collect();
                slow.sort_unstable_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
                let fast: Vec<_> = fast
                    .iter()
                    .map(|v| {
                        assert_eq!(r.items[v.slot as usize].0, v.key);
                        (v.key, f64::from_bits(v.distance), v.origin)
                    })
                    .collect();
                assert!(!slow.is_empty());
                assert_eq!(fast, slow, "{camera} {yaw} {pitch} {radius}");
            }
        }
    }

    #[test]
    fn columns_entirely_inside_skip_no_section() {
        // Short columns ahead of the camera: most lie entirely inside.
        let mut r = Resident::default();
        for x in -12i32..=12 {
            for z in 1..=12 {
                for y in 3..6 {
                    if (x + y + z).rem_euclid(3) != 0 {
                        r.insert((x, y, z), ());
                    }
                }
            }
        }
        let camera = DVec3::new(0.5, -8.0, 0.5);
        let projection = super::super::world_projection(70.0, 16.0 / 9.0, 512.0);
        let frustum = Frustum {
            planes: super::super::frustum(projection * crate::camera::view_matrix(0.0, 5.0)),
        };
        let inside = r
            .column_headers
            .iter()
            .filter(|c| {
                let min = origin((c.x, c.low, c.z), crate::chunk::MIN_Y, camera).as_vec3();
                let top = origin((c.x, c.high, c.z), crate::chunk::MIN_Y, camera).as_vec3();
                frustum.contains(min, top + Vec3::splat(16.0))
            })
            .count();
        assert!(inside > 50 && inside < r.columns.len(), "{inside}");
        let mut fast = Vec::new();
        r.visible(camera, 12, &frustum, |_| true, |_, _| false, &mut fast);
        let mut slow: Vec<_> = r
            .items
            .iter()
            .filter_map(|&(k, _)| in_view(k, camera, 12, &frustum).map(|(d, _)| (d, k)))
            .collect();
        slow.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let fast: Vec<_> = fast
            .iter()
            .map(|v| (f64::from_bits(v.distance), v.key))
            .collect();
        assert_eq!(fast, slow);
    }

    /// Every key reaches its value and mark through its slot and column, every
    /// column's bounds and sections match the keys, and its marked list holds
    /// its marked sections.
    fn check(r: &Resident<u32>, model: &HashMap<SectionKey, (u32, bool)>) {
        assert_eq!(r.len(), model.len());
        assert_eq!(r.columns.len(), r.column_headers.len());
        let mut seen = 0;
        for (ci, (header, column)) in r.column_headers.iter().zip(&r.columns).enumerate() {
            assert!(!column.sections.is_empty());
            assert_eq!(column.sections.len(), column.section_bounds.len());
            assert_eq!(column.marked.len(), column.marked_bounds.len());
            assert_eq!(r.column_has_marked(ci), !column.marked.is_empty());
            let ys: Vec<i32> = column.sections.iter().map(|s| s.0).collect();
            assert_eq!(header.low, *ys.iter().min().unwrap());
            assert_eq!(header.high, *ys.iter().max().unwrap());
            for (&(y, slot), &resolved) in column.sections.iter().zip(&column.section_bounds) {
                let (key, value) = &r.items[slot as usize];
                assert_eq!(*key, (header.x, y, header.z));
                assert_eq!(model[key], (*value, r.marked[slot as usize]));
                assert_eq!(resolved, resolve_bounds(r.bounds[slot as usize]));
                seen += 1;
            }
            for (&(_, slot), &resolved) in column.marked.iter().zip(&column.marked_bounds) {
                assert_eq!(resolved, resolve_bounds(r.bounds[slot as usize]));
            }
            let mut marked: Vec<(i32, u32)> = column
                .sections
                .iter()
                .copied()
                .filter(|&(_, slot)| r.marked[slot as usize])
                .collect();
            let mut listed = column.marked.clone();
            marked.sort_unstable();
            listed.sort_unstable();
            assert_eq!(listed, marked, "column ({}, {})", header.x, header.z);
        }
        assert_eq!(seen, model.len());
        for (key, &(_, marked)) in model {
            assert!(r.contains(*key));
            assert_eq!(r.is_marked(*key), marked, "{key:?}");
        }
    }

    #[test]
    fn every_removal_changes_the_count_and_nothing_else_does() {
        let mut r = Resident::default();
        let start = r.removals();
        r.insert((0, 0, 0), 1u32);
        r.insert((0, 1, 0), 2);
        r.insert((1, 0, 0), 3);
        r.insert((0, 0, 0), 4);
        r.set_marked((0, 0, 0), true);
        assert_eq!(r.remove((5, 0, 5)), None);
        assert_eq!(
            r.removals(),
            start,
            "inserts, replacements, marks and misses"
        );
        r.remove((0, 1, 0));
        let one = r.removals();
        assert_ne!(one, start);
        r.remove_columns(&[(1, 0)].into_iter().collect());
        let two = r.removals();
        assert_ne!(two, one);
        r.clear();
        assert_ne!(r.removals(), two);
    }

    #[test]
    fn slots_and_columns_follow_inserts_and_removals() {
        let mut r = Resident::default();
        let mut model = HashMap::new();
        // Bounds that tell values apart.
        let bounds = |value: u32| {
            (!value.is_multiple_of(3)).then(|| [Vec3::splat((value % 5) as f32), Vec3::splat(16.0)])
        };
        // A deterministic mix of inserts, replacements and removals, marked at
        // random: a replacement can mark or unmark its section, and so can a
        // mark set alone (`set_marked`), which leaves a key not resident alone.
        let mut seed = 0x2545_f491_u32;
        for step in 0..4000u32 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let key = (
                (seed % 5) as i32 - 2,
                ((seed >> 4) % 6) as i32 - 1,
                ((seed >> 8) % 5) as i32 - 2,
            );
            if (seed >> 12).is_multiple_of(389) {
                // Every column at once (a far teleport), and one that holds nothing.
                let gone: FxHashSet<(i32, i32)> = (-2..=3)
                    .flat_map(|x| (-2..=2).map(move |z| (x, z)))
                    .collect();
                let mut removed = r.remove_columns(&gone);
                let mut expected: Vec<u32> = model.drain().map(|(_, (value, _))| value).collect();
                removed.sort_unstable();
                expected.sort_unstable();
                assert_eq!(removed, expected);
                assert!(r.columns.is_empty() && r.column_index.is_empty());
            } else if (seed >> 12).is_multiple_of(97) {
                // Whole columns at once: the column of `key` and the next one along x.
                // The others keep their values, bounds and marks (`check`).
                let gone: FxHashSet<(i32, i32)> =
                    [(key.0, key.2), (key.0 + 1, key.2)].into_iter().collect();
                let mut removed = r.remove_columns(&gone);
                let mut expected: Vec<u32> = model
                    .extract_if(|k: &SectionKey, _| gone.contains(&(k.0, k.2)))
                    .map(|(_, (value, _))| value)
                    .collect();
                removed.sort_unstable();
                expected.sort_unstable();
                assert_eq!(removed, expected);
            } else if (seed >> 12).is_multiple_of(3) {
                assert_eq!(r.remove(key), model.remove(&key).map(|(value, _)| value));
            } else if (seed >> 12).is_multiple_of(5) {
                let marked = (seed >> 20).is_multiple_of(2);
                r.set_marked(key, marked);
                if let Some(held) = model.get_mut(&key) {
                    held.1 = marked;
                }
            } else {
                let marked = (seed >> 20).is_multiple_of(2);
                r.insert_marked(key, step, bounds(step), marked);
                model.insert(key, (step, marked));
            }
            check(&r, &model);
            // `iter_mut` yields every key once, with its value and bounds.
            let mut seen = HashSet::new();
            for (key, value, b) in r.iter_mut() {
                assert_eq!(*value, model[&key].0, "{key:?}");
                assert_eq!(b, bounds(*value), "{key:?}");
                assert!(seen.insert(key), "{key:?} twice");
            }
            assert_eq!(seen.len(), model.len());
        }
        for (_, value, _) in r.iter_mut() {
            *value += 1;
        }
        for (key, (value, _)) in &model {
            assert_eq!(r.find(*key), Some(&(value + 1)));
        }
        r.clear();
        assert_eq!(r.len(), 0);
        assert!(r.columns.is_empty() && r.marked.is_empty());
    }

    /// GPU frames' walk (`walk` with `marked_only_near`) is `visible` less the
    /// sections within the render distance that are not marked, in the same
    /// order: with marks at random, bounds reaching out of their cells, short
    /// columns entirely inside the frustum, cave reach, and columns past the
    /// distance kept for far terrain (GPU frames have no water mirror,
    /// `gpu_frame`).
    #[test]
    fn the_marked_walk_is_the_walk_less_unmarked_sections_within_the_distance() {
        let mut r = Resident::default();
        let mut seed = 0x9e37_79b9_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for x in -14i32..=14 {
            for z in -14i32..=14 {
                // Short columns ahead of the first camera: most lie entirely inside.
                let rows = if z > 0 && x.abs() <= 12 { 3..6 } else { 0..24 };
                for y in rows {
                    let bits = next();
                    if bits.is_multiple_of(5) {
                        continue;
                    }
                    let bounds = match (bits >> 4) % 4 {
                        0 => None,
                        1 => Some([Vec3::splat(-1.5), Vec3::splat(17.5)]),
                        2 => Some([Vec3::new(0.0, 18.0, 0.0), Vec3::new(16.0, 20.0, 16.0)]),
                        _ => Some([Vec3::new(2.0, 0.0, 3.0), Vec3::new(14.0, 5.5, 16.0)]),
                    };
                    r.insert_marked((x, y, z), (), bounds, (bits >> 8).is_multiple_of(3));
                }
            }
        }
        let reachable = |k: SectionKey| (3 * k.0 + k.1 + 5 * k.2).rem_euclid(7) != 0;
        let beyond = |x: i32, z: i32| (x - z).rem_euclid(3) == 0;
        let projection = super::super::world_projection(70.0, 16.0 / 9.0, 512.0);
        let (mut kept, mut dropped, mut inside) = (0, 0, 0);
        for (camera, yaw, pitch) in [
            (DVec3::new(0.5, -8.0, 0.5), 0.0, 5.0),
            (DVec3::new(-37.25, 130.5, 21.75), 135.0, 35.0),
            (DVec3::new(100.0, -60.0, -3.0), 270.0, -80.0),
            (DVec3::new(8.0, 64.0, 8.0), 45.0, 89.0),
        ] {
            let view = Frustum {
                planes: super::super::frustum(projection * crate::camera::view_matrix(yaw, pitch)),
            };
            inside += r
                .column_headers
                .iter()
                .filter(|c| {
                    let min = origin((c.x, c.low, c.z), r.min_y, camera).as_vec3();
                    let top = origin((c.x, c.high, c.z), r.min_y, camera).as_vec3();
                    view.contains(min, top + Vec3::splat(16.0))
                })
                .count();
            for radius in [3, 7, 12] {
                let (mut all, mut marked) = (Vec::new(), Vec::new());
                r.visible(camera, radius, &view, reachable, beyond, &mut all);
                r.walk(camera, radius, &view, reachable, beyond, true, &mut marked);
                let from = camera_column(camera);
                let near = |v: &Visible| in_view_distance(from, radius, v.key.0, v.key.2);
                let expected: Vec<Visible> = all
                    .iter()
                    .copied()
                    .filter(|v| r.is_marked(v.key) || !near(v))
                    .collect();
                assert_eq!(marked, expected, "{camera} {yaw} {pitch} {radius}");
                kept += marked.len();
                dropped += all.len() - marked.len();
            }
        }
        assert!(
            kept > 100 && dropped > 100,
            "{kept} kept, {dropped} dropped"
        );
        assert!(inside > 50, "{inside} columns entirely inside");
    }

    /// The old walk (every key of a hash map, sorted by distance, then two
    /// lookups per visible section) against `visible` on a render distance
    /// 32 world. Run with `cargo test --profile fast -p horizon-client --lib
    /// -- --ignored column_walk_speed --nocapture`.
    #[test]
    #[ignore = "timing, not a check"]
    fn column_walk_speed() {
        use std::time::{Duration, Instant};
        let mut r = Resident::default();
        let mut map = HashMap::new();
        for x in -33i32..=33 {
            for z in -33i32..=33 {
                for y in 0..24 {
                    if (x * 7 + y * 3 + z * 5).rem_euclid(4) != 0 {
                        r.insert((x, y, z), [x, y, z]);
                        map.insert((x, y, z), [x, y, z]);
                    }
                }
            }
        }
        let camera = DVec3::new(8.5, 100.0, 8.5);
        let projection = super::super::world_projection(70.0, 16.0 / 9.0, 1024.0);
        let frustum = Frustum {
            planes: super::super::frustum(projection * crate::camera::view_matrix(30.0, 35.0)),
        };
        let (mut old, mut new) = (Duration::MAX, Duration::MAX);
        let (mut list, mut keys) = (Vec::new(), Vec::new());
        let mut sum = 0i64;
        for _ in 0..200 {
            let t = Instant::now();
            keys.clear();
            keys.extend(map.keys().filter_map(|&k| {
                in_view(k, camera, 32, &frustum).map(|(d, o)| (k, d, o.to_array()))
            }));
            keys.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));
            for (k, _, _) in &keys {
                sum += i64::from(map[k][1] + map[k][0]);
            }
            old = old.min(t.elapsed());
            let t = Instant::now();
            list.clear();
            r.visible(camera, 32, &frustum, |_| true, |_, _| false, &mut list);
            for v in &list {
                sum += i64::from(r.get(v.slot)[1] + r.get(v.slot)[0]);
            }
            new = new.min(t.elapsed());
        }
        assert_eq!(keys.len(), list.len());
        println!(
            "{} resident, {} visible: old walk {old:?}, column walk {new:?} (checksum {sum})",
            r.len(),
            list.len()
        );
    }

    #[test]
    fn tight_mesh_bounds_culls_section_outside_frustum() {
        let mut r = Resident::default();
        let key = (0, 0, 0);
        // Section geometry only occupies local Y in [0.0, 2.0]
        let tight = Some([Vec3::new(0.0, 0.0, 0.0), Vec3::new(16.0, 2.0, 16.0)]);
        r.insert_with_bounds(key, (), tight);

        let camera = DVec3::new(8.0, f64::from(crate::chunk::MIN_Y + 15), -2.0);
        let projection = super::super::world_projection(70.0, 16.0 / 9.0, 512.0);
        let view = crate::camera::view_matrix(0.0, -60.0);
        let frustum = Frustum {
            planes: super::super::frustum(projection * view),
        };

        // Tight bounds culls the section whose geometry is entirely outside the frustum
        let mut out = Vec::new();
        r.visible(camera, 4, &frustum, |_| true, |_, _| false, &mut out);
        assert!(
            out.is_empty(),
            "tight bounds should cull section whose geometry is below frustum"
        );

        // With fallback 16x16x16 bounds, the section intersects the frustum and is visible
        r.insert_with_bounds(key, (), None);
        out.clear();
        r.visible(camera, 4, &frustum, |_| true, |_, _| false, &mut out);
        assert_eq!(
            out.len(),
            1,
            "full 16x16x16 bounds intersect frustum and are accepted"
        );
    }

    #[test]
    #[ignore = "benchmark"]
    fn tight_bounds_benchmark() {
        use std::time::Instant;

        let mut r_fallback = Resident::default();
        let mut r_tight = Resident::default();

        let radius = 16;
        for x in -radius..=radius {
            for z in -radius..=radius {
                // Section 0..4: underground rock
                for y in 0..4 {
                    r_fallback.insert((x, y, z), ());
                    r_tight.insert_with_bounds((x, y, z), (), None);
                }
                // Section 4: surface terrain with grass / hills (e.g. Y in [0, 4] or [0, 6])
                let height = 2.0 + (((x * 3 + z * 5).rem_euclid(5)) as f32) * 1.5;
                let surface_bounds =
                    Some([Vec3::new(0.0, 0.0, 0.0), Vec3::new(16.0, height, 16.0)]);
                r_fallback.insert((x, 4, z), ());
                r_tight.insert_with_bounds((x, 4, z), (), surface_bounds);
            }
        }

        println!("\n=== Tight Bounds Culling Benchmark ===");
        println!("Total Resident Sections: {}", r_tight.len());

        let test_views = [
            ("Looking level across terrain (pitch 0°)", 0.0, 0.0),
            ("Looking slightly up (pitch -15°)", 45.0, -15.0),
            ("Looking up towards sky (pitch -35°)", 90.0, -35.0),
            ("Looking down at ground (pitch 25°)", 180.0, 25.0),
            ("Diagonal view (yaw 135°, pitch -10°)", 135.0, -10.0),
        ];

        let camera = DVec3::new(8.0, f64::from(crate::chunk::MIN_Y + 4 * 16 + 10), 8.0);
        let projection = super::super::world_projection(70.0, 16.0 / 9.0, 512.0);

        for (desc, yaw, pitch) in test_views {
            let view = crate::camera::view_matrix(yaw, pitch);
            let frustum = Frustum {
                planes: super::super::frustum(projection * view),
            };

            let mut out_fallback = Vec::new();
            let mut out_tight = Vec::new();

            r_fallback.visible(
                camera,
                radius,
                &frustum,
                |_| true,
                |_, _| false,
                &mut out_fallback,
            );
            r_tight.visible(
                camera,
                radius,
                &frustum,
                |_| true,
                |_, _| false,
                &mut out_tight,
            );

            let culled_extra = out_fallback.len().saturating_sub(out_tight.len());
            let pct_saved = if !out_fallback.is_empty() {
                (culled_extra as f64 / out_fallback.len() as f64) * 100.0
            } else {
                0.0
            };

            // Timing over 1,000 passes
            let iters = 1_000;
            let t0 = Instant::now();
            for _ in 0..iters {
                out_fallback.clear();
                r_fallback.visible(
                    camera,
                    radius,
                    &frustum,
                    |_| true,
                    |_, _| false,
                    &mut out_fallback,
                );
            }
            let dur_fallback = t0.elapsed();

            let t1 = Instant::now();
            for _ in 0..iters {
                out_tight.clear();
                r_tight.visible(
                    camera,
                    radius,
                    &frustum,
                    |_| true,
                    |_, _| false,
                    &mut out_tight,
                );
            }
            let dur_tight = t1.elapsed();

            println!(
                "{desc}\n  Fallback 16x16: {} sections | {:?}\n  Tight Bounds:   {} sections | {:?}\n  Reduction:      -{} sections ({:.1}% culled)\n",
                out_fallback.len(),
                dur_fallback / iters,
                out_tight.len(),
                dur_tight / iters,
                culled_extra,
                pct_saved
            );
        }
    }
}
