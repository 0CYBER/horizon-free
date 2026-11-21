//! Cloud visibility and flat/fancy cell geometry (`CloudRenderer`).
/// `EnvironmentAttributes.CLOUD_HEIGHT`'s default, and the overworld's `visual/cloud_height`.
pub const CLOUD_HEIGHT: f64 = 192.33;

/// Whether the cloud color has any packed alpha (vanilla gates the clouds on it).
pub fn has_color(alpha: f32) -> bool {
    (alpha.clamp(0.0, 1.0) * 255.0) as u8 > 0
}

/// `LevelRenderer`: skip OFF clouds and colors with no packed alpha.
pub fn visible(mode: &str, alpha: f32) -> bool {
    mode != "false" && has_color(alpha)
}

/// `buildFlatCell`: bottom plane, in both windings for the culled cloud pipeline.
pub fn flat_cell(x: f32, z: f32, bottom: f32) -> [[[f32; 3]; 4]; 2] {
    let p = [
        [x + 12.0, bottom, z],
        [x + 12.0, bottom, z + 12.0],
        [x, bottom, z + 12.0],
        [x, bottom, z],
    ];
    [p, [p[3], p[2], p[1], p[0]]]
}

/// Direction order is DOWN, UP, NORTH, SOUTH, WEST, EAST; inward faces use bits 6..12.
pub fn face_mask(x: i32, z: i32, bottom: f32, empty: [bool; 4]) -> u16 {
    let mut mask = 0;
    if bottom <= 0.0 {
        mask |= 1 << 1;
    }
    if bottom + 4.0 >= 0.0 {
        mask |= 1;
    }
    for (i, visible) in [z > 0, z < 0, x > 0, x < 0].into_iter().enumerate() {
        if empty[i] && visible {
            mask |= 1 << (i + 2);
        }
    }
    if x.abs() <= 1 && z.abs() <= 1 {
        mask |= 63 << 6;
    }
    mask
}

/// Exterior faces first, then reversed interior faces, in vanilla emission order.
pub fn faces(mask: u16) -> impl Iterator<Item = (u8, bool)> {
    [1, 0, 2, 3, 4, 5]
        .into_iter()
        .filter(move |d| mask & (1 << d) != 0)
        .map(|d| (d, false))
        .chain(
            (0..6)
                .filter(move |d| mask & (1 << (d + 6)) != 0)
                .map(|d| (d, true)),
        )
}

/// `CloudRenderer.prepare`: the cell radius for the Cloud Distance option
/// (in chunks), independent of the render distance.
pub fn radius_cells(cloud_range_chunks: i32) -> i32 {
    ((cloud_range_chunks * 16) as f32 / 12.0).ceil() as i32
}

/// Cells the mesh's origin snaps to. The mesh is a window on the cloud pattern around the
/// camera's cell, so it used to be rebuilt, and the whole array re-uploaded, on every cell
/// crossed: 6.5 MB, about 11 ms at the default 128-chunk range, the worst frame of a flight.
/// Anchoring the origin to a coarser grid rebuilds it once every `SNAP_CELLS` cells instead,
/// and `Uniform::offset` carries the remaining shift. The ring of cells the shift can leave
/// uncovered at the far side lies past the cloud fog end, where the cloud fade has already
/// taken the alpha to 0 (`entity.wgsl`'s `fs_entity`), so the extra cells cost only the
/// vertices they add.
const SNAP_CELLS: i64 = 4;

#[derive(Clone)]
pub struct CellIter {
    radius: i32,
    ring: i32,
    x: i32,
    pending_z: Option<i32>,
}

impl CellIter {
    pub fn new(radius: i32) -> Self {
        Self {
            radius,
            ring: 0,
            x: 0,
            pending_z: None,
        }
    }
}

impl Iterator for CellIter {
    type Item = (i32, i32);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(z) = self.pending_z.take() {
            let x = self.x;
            self.x += 1;
            return Some((x, z));
        }
        let r2 = self.radius * self.radius;
        while self.ring <= 2 * self.radius {
            let min_abs = self.ring - self.radius;
            while self.x <= self.ring {
                if min_abs > 0 && self.x > -min_abs && self.x < min_abs {
                    self.x = min_abs;
                }
                let x = self.x;
                let z = self.ring - x.abs();
                if z <= self.radius && x * x + z * z <= r2 {
                    if z != 0 {
                        self.pending_z = Some(z);
                        return Some((x, -z));
                    } else {
                        self.x += 1;
                        return Some((x, 0));
                    }
                }
                self.x += 1;
            }
            self.ring += 1;
            self.x = -self.ring;
        }
        None
    }
}

/// CloudRenderer builds cells in expanding Manhattan rings, nearest first.
pub fn cells(radius: i32) -> CellIter {
    CellIter::new(radius)
}

/// Per-frame cloud inputs. Resources are immutable for the renderer's lifetime:
/// resource-pack reloads replace the renderer and its cache together.
pub struct Input<'a> {
    pub texture: &'a (u32, u32, Vec<bool>),
    pub uv: [f32; 2],
    pub camera: glam::DVec3,
    pub time: i64,
    pub partial_tick: f64,
    pub range: i32,
    pub fast: bool,
    pub color: [f32; 4],
}

#[derive(Debug, PartialEq, Eq)]
struct MeshKey {
    cell_x: i64,
    cell_z: i64,
    radius: i32,
    fast: bool,
    // The two vertical face tests include both surfaces at the boundaries.
    top: bool,
    bottom: bool,
    // The camera's cell within the snapped origin's, for the 3x3 cells whose inward
    // faces are drawn. Only read while the camera is inside the layer.
    sub_x: i32,
    sub_z: i32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Uniform {
    offset: [f32; 4],
    colors: [[f32; 4]; 4],
}

/// How far the clouds have drifted, in ticks: the game time within one trip across the cloud
/// texture (`width` cells of 400 ticks), plus the partial tick (`CloudRenderer.prepare`).
pub fn drift(time: i64, partial_tick: f64, width: u32) -> f64 {
    time.rem_euclid(i64::from(width) * 400) as f64 + partial_tick
}

impl Input<'_> {
    fn state(&self) -> (MeshKey, Uniform) {
        let (w, h, _) = self.texture;
        let offset = drift(self.time, self.partial_tick, *w);
        let mut x = self.camera.x + offset * 0.030_000_001;
        let mut z = self.camera.z + 3.96;
        let (tw, th) = (f64::from(*w) * 12.0, f64::from(*h) * 12.0);
        x -= (x / tw).floor() * tw;
        z -= (z / th).floor() * th;
        let cell_x = (x / 12.0).floor() as i64 / SNAP_CELLS * SNAP_CELLS;
        let cell_z = (z / 12.0).floor() as i64 / SNAP_CELLS * SNAP_CELLS;
        let bottom = (CLOUD_HEIGHT - self.camera.y) as f32;
        // In fast mode every filled cell emits the same flat double-sided quad regardless of
        // camera height, so vertical movement through the cloud layer needs no mesh rebuild.
        let (top, bottom_face, sub_x, sub_z) = if self.fast {
            (false, false, 0, 0)
        } else {
            let top = bottom <= 0.0;
            let bottom_face = bottom + 4.0 >= 0.0;
            // The inward faces of the 3x3 cells around the camera are only seen from inside the
            // layer; outside it they are back-facing, so the mesh need not follow the camera there.
            let (sub_x, sub_z) = if top && bottom_face {
                (
                    (x / 12.0).floor() as i32 - cell_x as i32,
                    (z / 12.0).floor() as i32 - cell_z as i32,
                )
            } else {
                (0, 0)
            };
            (top, bottom_face, sub_x, sub_z)
        };
        let key = MeshKey {
            cell_x,
            cell_z,
            // One more ring than the range, for the cells the snapped origin can uncover.
            radius: radius_cells(self.range) + SNAP_CELLS as i32,
            fast: self.fast,
            top,
            bottom: bottom_face,
            sub_x,
            sub_z,
        };
        let colors = [1.0, 0.7, 0.8, 0.9].map(|shade| {
            let [r, g, b, a] = self.color;
            [r * shade, g * shade, b * shade, a]
                .map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8 as f32 / 255.0)
        });
        (
            key,
            Uniform {
                offset: [
                    -(x - cell_x as f64 * 12.0) as f32,
                    bottom,
                    -(z - cell_z as f64 * 12.0) as f32,
                    0.0,
                ],
                colors,
            },
        )
    }
}

const CELL_FILLED: u8 = 0x80;

/// Precomputed per-texel occupancy and 4-neighbour emptiness bits for the cloud
/// texture. Replaces 5 wrapped lookups (and 10 `i64::rem_euclid` divisions) per
/// filled cell with a single byte load from an L2-resident table (64 KiB for
/// vanilla's 256×256 texture).
struct CloudMask {
    w: u32,
    h: u32,
    source_ptr: usize,
    source_len: usize,
    filled_count: u32,
    /// `0` for empty cells; `CELL_FILLED` plus bits 2..=5 for empty north/south/west/east
    /// neighbours (matching `face_mask` direction bits).
    bits: Box<[u8]>,
}

impl CloudMask {
    fn matches(&self, texture: &(u32, u32, Vec<bool>)) -> bool {
        self.w == texture.0
            && self.h == texture.1
            && self.source_ptr == texture.2.as_ptr() as usize
            && self.source_len == texture.2.len()
    }

    fn build(texture: &(u32, u32, Vec<bool>)) -> Self {
        let (w, h, cells) = texture;
        let (w_usize, h_usize) = (*w as usize, *h as usize);
        let total = w_usize.saturating_mul(h_usize);
        if total == 0 || cells.len() < total {
            return Self {
                w: *w,
                h: *h,
                source_ptr: cells.as_ptr() as usize,
                source_len: cells.len(),
                filled_count: 0,
                bits: Box::new([]),
            };
        }
        let mut bits = vec![0u8; total].into_boxed_slice();
        let mut filled_count = 0u32;
        for tz in 0..h_usize {
            let row = tz * w_usize;
            let row_n = (if tz == 0 { h_usize - 1 } else { tz - 1 }) * w_usize;
            let row_s = (if tz + 1 == h_usize { 0 } else { tz + 1 }) * w_usize;
            for tx in 0..w_usize {
                if !cells[row + tx] {
                    continue;
                }
                let tx_w = if tx == 0 { w_usize - 1 } else { tx - 1 };
                let tx_e = if tx + 1 == w_usize { 0 } else { tx + 1 };
                let mut b = CELL_FILLED;
                if !cells[row_n + tx] {
                    b |= 1 << 2;
                }
                if !cells[row_s + tx] {
                    b |= 1 << 3;
                }
                if !cells[row + tx_w] {
                    b |= 1 << 4;
                }
                if !cells[row + tx_e] {
                    b |= 1 << 5;
                }
                bits[row + tx] = b;
                filled_count += 1;
            }
        }
        Self {
            w: *w,
            h: *h,
            source_ptr: cells.as_ptr() as usize,
            source_len: cells.len(),
            filled_count,
            bits,
        }
    }
}

/// The cloud cells around a snapped origin, rebuilt when the key changes: once every
/// [`SNAP_CELLS`] cells the camera crosses, and every cell while the camera is inside the
/// layer, where the inward faces follow it. The renderer copies the vertices to the GPU on
/// a rebuild, which is why the origin snaps at all.
#[derive(Default)]
pub(crate) struct Mesh {
    key: Option<MeshKey>,
    cells: Vec<(i16, i16)>,
    cached_radius: Option<i32>,
    mask: Option<CloudMask>,
    wrap_x: Vec<usize>,
    wrap_z: Vec<usize>,
    pub vertices: Vec<crate::mesher::Vertex>,
}

#[inline(always)]
fn push_quad(
    vertices: &mut Vec<crate::mesher::Vertex>,
    p: [[f32; 3]; 4],
    shade: u32,
    normal: [i8; 4],
    uv: [f32; 2],
) {
    let make = |pos| crate::mesher::Vertex {
        normal,
        pos,
        uv,
        layer: 4,
        color: u32::MAX,
        light: shade,
        glint_uv: [0.0, 0.0],
    };
    vertices.extend_from_slice(&[make(p[0]), make(p[1]), make(p[2]), make(p[3])]);
}

#[cold]
fn emit_inward_faces(vertices: &mut Vec<crate::mesher::Vertex>, x0: f32, z0: f32, uv: [f32; 2]) {
    let x1 = x0 + 12.0;
    let z1 = z0 + 12.0;
    const Y0: f32 = 0.0;
    const Y1: f32 = 4.0;
    // Vanilla inward emission order is directions 0..6 with reversed winding and inverted normals.
    let faces = [
        (
            [[x0, Y0, z0], [x0, Y0, z1], [x1, Y0, z1], [x1, Y0, z0]],
            1,
            [0, 127, 0, 0],
        ),
        (
            [[x1, Y1, z0], [x1, Y1, z1], [x0, Y1, z1], [x0, Y1, z0]],
            0,
            [0, -127, 0, 0],
        ),
        (
            [[x1, Y0, z0], [x1, Y1, z0], [x0, Y1, z0], [x0, Y0, z0]],
            2,
            [0, 0, 127, 0],
        ),
        (
            [[x0, Y0, z1], [x0, Y1, z1], [x1, Y1, z1], [x1, Y0, z1]],
            2,
            [0, 0, -127, 0],
        ),
        (
            [[x0, Y0, z0], [x0, Y1, z0], [x0, Y1, z1], [x0, Y0, z1]],
            3,
            [127, 0, 0, 0],
        ),
        (
            [[x1, Y0, z1], [x1, Y1, z1], [x1, Y1, z0], [x1, Y0, z0]],
            3,
            [-127, 0, 0, 0],
        ),
    ];
    for (p, shade, normal) in faces {
        push_quad(vertices, p, shade, normal, uv);
    }
}

#[inline(always)]
fn emit_fancy_cell(
    vertices: &mut Vec<crate::mesher::Vertex>,
    vert_mask: u8,
    side_bits: u8,
    rel: [i32; 2],
    pos: [f32; 2],
    uv: [f32; 2],
) {
    let [x0, z0] = pos;
    let x1 = x0 + 12.0;
    let z1 = z0 + 12.0;
    const Y0: f32 = 0.0;
    const Y1: f32 = 4.0;
    if vert_mask & (1 << 1) != 0 {
        push_quad(
            vertices,
            [[x0, Y1, z0], [x0, Y1, z1], [x1, Y1, z1], [x1, Y1, z0]],
            0,
            [0, 127, 0, 0],
            uv,
        );
    }
    if vert_mask & 1 != 0 {
        push_quad(
            vertices,
            [[x1, Y0, z0], [x1, Y0, z1], [x0, Y0, z1], [x0, Y0, z0]],
            1,
            [0, -127, 0, 0],
            uv,
        );
    }
    let [rel_x, rel_z] = rel;
    let vis = (u8::from(rel_z > 0) << 2)
        | (u8::from(rel_z < 0) << 3)
        | (u8::from(rel_x > 0) << 4)
        | (u8::from(rel_x < 0) << 5);
    let sides = side_bits & vis;
    if sides != 0 {
        if sides & (1 << 2) != 0 {
            push_quad(
                vertices,
                [[x0, Y0, z0], [x0, Y1, z0], [x1, Y1, z0], [x1, Y0, z0]],
                2,
                [0, 0, -127, 0],
                uv,
            );
        }
        if sides & (1 << 3) != 0 {
            push_quad(
                vertices,
                [[x1, Y0, z1], [x1, Y1, z1], [x0, Y1, z1], [x0, Y0, z1]],
                2,
                [0, 0, 127, 0],
                uv,
            );
        }
        if sides & (1 << 4) != 0 {
            push_quad(
                vertices,
                [[x0, Y0, z1], [x0, Y1, z1], [x0, Y1, z0], [x0, Y0, z0]],
                3,
                [-127, 0, 0, 0],
                uv,
            );
        }
        if sides & (1 << 5) != 0 {
            push_quad(
                vertices,
                [[x1, Y0, z0], [x1, Y1, z0], [x1, Y1, z1], [x1, Y0, z1]],
                3,
                [127, 0, 0, 0],
                uv,
            );
        }
    }
    if rel_x.abs() <= 1 && rel_z.abs() <= 1 {
        emit_inward_faces(vertices, x0, z0, uv);
    }
}

#[inline(always)]
fn emit_flat_cell(vertices: &mut Vec<crate::mesher::Vertex>, x0: f32, z0: f32, uv: [f32; 2]) {
    let x1 = x0 + 12.0;
    let z1 = z0 + 12.0;
    let make = |pos| crate::mesher::Vertex {
        normal: [0; 4],
        pos,
        uv,
        layer: 4,
        color: u32::MAX,
        light: 0,
        glint_uv: [0.0, 0.0],
    };
    let v0 = make([x1, 0.0, z0]);
    let v1 = make([x1, 0.0, z1]);
    let v2 = make([x0, 0.0, z1]);
    let v3 = make([x0, 0.0, z0]);
    vertices.extend_from_slice(&[v0, v1, v2, v3, v3, v2, v1, v0]);
}

impl Mesh {
    /// Returns true only when vertices must be uploaded. Sub-cell motion and
    /// color changes update only the uniform, as in CloudRenderer.prepare.
    /// Builds the mesh again on the next `prepare`, as for new clouds: its vertices did not
    /// reach the GPU.
    pub fn forget(&mut self) {
        self.key = None;
    }

    pub fn prepare(&mut self, input: &Input<'_>) -> (bool, Uniform) {
        let (key, uniform) = input.state();
        if self.key.as_ref() == Some(&key) {
            return (false, uniform);
        }
        if self.mask.as_ref().is_none_or(|m| !m.matches(input.texture)) {
            self.mask = Some(CloudMask::build(input.texture));
        }
        let mask = self.mask.as_ref().expect("cloud mask built");
        self.vertices.clear();
        if mask.filled_count == 0 {
            self.vertices.shrink_to_fit();
            self.key = Some(key);
            return (true, uniform);
        }

        if self.cached_radius != Some(key.radius) {
            let r = key.radius.max(0);
            let approx =
                (std::f64::consts::PI * f64::from(r) * f64::from(r)) as usize + 4 * r as usize + 16;
            self.cells.clear();
            self.cells.reserve(approx);
            self.cells
                .extend(self::cells(key.radius).map(|(x, z)| (x as i16, z as i16)));
            if self.cells.capacity() > (self.cells.len() + self.cells.len() / 2).max(1024) {
                self.cells.shrink_to_fit();
            }
            self.cached_radius = Some(key.radius);
        }

        // Pre-size on cold builds using the texture's actual cloud fill ratio so
        // `vertices` avoids repeated doubling reallocations and 2x capacity overshoot.
        if self.vertices.capacity() == 0 {
            let total = (mask.w as usize).saturating_mul(mask.h as usize).max(1);
            let est_filled =
                (self.cells.len() as u64 * u64::from(mask.filled_count) / total as u64) as usize;
            let verts_per_cell = if !input.fast && key.top && key.bottom {
                12
            } else {
                8
            };
            self.vertices
                .reserve((est_filled * verts_per_cell + est_filled * 2).max(256));
        }

        let (w, h) = (mask.w, mask.h);
        let mask_bits = &mask.bits;
        let (cell_x, cell_z) = (key.cell_x, key.cell_z);
        let uv = input.uv;
        let vert_mask = u8::from(key.bottom) | (u8::from(key.top) << 1);
        let (sub_x, sub_z) = (key.sub_x, key.sub_z);

        if w.is_power_of_two() && h.is_power_of_two() {
            let w_mask = (w - 1) as usize;
            let h_mask = (h - 1) as usize;
            let w_shift = w.trailing_zeros();
            let base_x = cell_x.rem_euclid(i64::from(w)) as usize;
            let base_z = cell_z.rem_euclid(i64::from(h)) as usize;
            if input.fast {
                for &(dx, dz) in &self.cells {
                    let (dx, dz) = (i32::from(dx), i32::from(dz));
                    let tx = base_x.wrapping_add(dx as isize as usize) & w_mask;
                    let tz = base_z.wrapping_add(dz as isize as usize) & h_mask;
                    if mask_bits[(tz << w_shift) | tx] == 0 {
                        continue;
                    }
                    emit_flat_cell(&mut self.vertices, dx as f32 * 12.0, dz as f32 * 12.0, uv);
                }
            } else {
                for &(dx, dz) in &self.cells {
                    let (dx, dz) = (i32::from(dx), i32::from(dz));
                    let tx = base_x.wrapping_add(dx as isize as usize) & w_mask;
                    let tz = base_z.wrapping_add(dz as isize as usize) & h_mask;
                    let bits = mask_bits[(tz << w_shift) | tx];
                    if bits == 0 {
                        continue;
                    }
                    emit_fancy_cell(
                        &mut self.vertices,
                        vert_mask,
                        bits,
                        [dx - sub_x, dz - sub_z],
                        [dx as f32 * 12.0, dz as f32 * 12.0],
                        uv,
                    );
                }
            }
        } else {
            let r = key.radius.max(0);
            let span = (2 * r + 1) as usize;
            let (w_usize, h_usize) = (w as usize, h as usize);
            let mut tx = (cell_x - i64::from(r)).rem_euclid(i64::from(w)) as usize;
            self.wrap_x.clear();
            self.wrap_x.reserve(span);
            for _ in 0..span {
                self.wrap_x.push(tx);
                tx += 1;
                if tx == w_usize {
                    tx = 0;
                }
            }
            let mut tz = (cell_z - i64::from(r)).rem_euclid(i64::from(h)) as usize;
            self.wrap_z.clear();
            self.wrap_z.reserve(span);
            for _ in 0..span {
                self.wrap_z.push(tz * w_usize);
                tz += 1;
                if tz == h_usize {
                    tz = 0;
                }
            }
            let (wrap_x, wrap_z) = (&self.wrap_x, &self.wrap_z);
            if input.fast {
                for &(dx, dz) in &self.cells {
                    let (dx, dz) = (i32::from(dx), i32::from(dz));
                    let idx = wrap_z[(dz + r) as usize] + wrap_x[(dx + r) as usize];
                    if mask_bits[idx] == 0 {
                        continue;
                    }
                    emit_flat_cell(&mut self.vertices, dx as f32 * 12.0, dz as f32 * 12.0, uv);
                }
            } else {
                for &(dx, dz) in &self.cells {
                    let (dx, dz) = (i32::from(dx), i32::from(dz));
                    let idx = wrap_z[(dz + r) as usize] + wrap_x[(dx + r) as usize];
                    let bits = mask_bits[idx];
                    if bits == 0 {
                        continue;
                    }
                    emit_fancy_cell(
                        &mut self.vertices,
                        vert_mask,
                        bits,
                        [dx - sub_x, dz - sub_z],
                        [dx as f32 * 12.0, dz as f32 * 12.0],
                        uv,
                    );
                }
            }
        }

        if self.vertices.is_empty() {
            self.vertices.shrink_to_fit();
        } else {
            let len = self.vertices.len();
            // Release excess capacity if cloud distance shrank or growth overshot,
            // leaving 25% headroom so normal snap-to-snap density shifts don't reallocate.
            if self.vertices.capacity() > (len + len / 2).max(4096) {
                self.vertices.shrink_to(len + len / 4);
            }
        }

        self.key = Some(key);
        (true, uniform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_mesh_tracks_geometry_but_not_subcell_motion_or_color() {
        let texture = (8, 8, (0..64).map(|i| i % 3 != 0).collect());
        let mut input = Input {
            texture: &texture,
            uv: [0.5; 2],
            camera: glam::DVec3::new(1.0, 100.0, 1.0),
            time: 0,
            partial_tick: 0.0,
            range: 2,
            fast: false,
            color: [1.0; 4],
        };
        let mut mesh = Mesh::default();
        assert!(mesh.prepare(&input).0);
        let vertices = bytemuck::cast_slice::<_, u8>(&mesh.vertices).to_vec();
        let pointer = mesh.vertices.as_ptr();
        for tick in 0..100 {
            input.time = tick;
            input.partial_tick = 0.5;
            input.camera.x = 2.0;
            input.camera.y = 101.0;
            input.color = [0.8, 0.7, 0.6, 0.5];
            let (rebuilt, uniform) = mesh.prepare(&input);
            assert!(!rebuilt);
            assert_eq!(mesh.vertices.as_ptr(), pointer);
            assert_eq!(bytemuck::cast_slice::<_, u8>(&mesh.vertices), vertices);
            assert_eq!(uniform.offset[1], (192.33 - input.camera.y) as f32);
            assert_eq!(
                uniform.colors[1],
                [0.8 * 0.7, 0.7 * 0.7, 0.6 * 0.7, 0.5].map(|v| (v * 255.0) as u8 as f32 / 255.0)
            );
            for color in uniform.colors {
                assert_eq!(color[3], 127.0 / 255.0);
            }
        }
        // One block into the second snap cell: every case below moves the snapped origin, or
        // another part of the key, so each one rebuilds.
        let far_cell = SNAP_CELLS as f64 * 12.0 + 1.0;
        for (x, y, z, range, fast) in [
            (far_cell, 101.0, 1.0, 2, false),
            (far_cell, 101.0, -5.0, 2, false),
            (far_cell, 192.33, -5.0, 2, false),
            (far_cell, 196.34, -5.0, 2, false),
            (far_cell, 196.34, -5.0, 3, false),
            (far_cell, 196.34, -5.0, 3, true),
        ] {
            input.camera = glam::DVec3::new(x, y, z);
            input.range = range;
            input.fast = fast;
            assert!(mesh.prepare(&input).0);
            let mut fresh = Mesh::default();
            fresh.prepare(&input);
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&mesh.vertices),
                bytemuck::cast_slice::<_, u8>(&fresh.vertices)
            );
            assert!(!mesh.prepare(&input).0);
        }
        // Drift alone crosses the snapped origin, even with a stationary camera.
        input.camera = glam::DVec3::new(1.0, 101.0, 1.0);
        input.time = 0;
        input.partial_tick = 0.0;
        assert!(mesh.prepare(&input).0);
        assert!(!mesh.prepare(&input).0);
        // 400 ticks of drift move the clouds 12 blocks: one whole snap cell.
        input.time = SNAP_CELLS * 400;
        assert!(mesh.prepare(&input).0);
        // Resource reload constructs a new renderer and cache, including empty packs.
        let mut reloaded = Mesh::default();
        let empty = (1, 1, vec![false]);
        input.texture = &empty;
        assert!(reloaded.prepare(&input).0);
        assert!(reloaded.vertices.is_empty());
        assert!(!reloaded.prepare(&input).0);
    }

    #[test]
    fn a_snap_cell_of_motion_keeps_the_mesh_at_the_same_world_cells() {
        let texture = (8, 8, (0..64).map(|i| i % 3 != 0).collect());
        let mut input = Input {
            texture: &texture,
            uv: [0.5; 2],
            camera: glam::DVec3::new(1.0, 100.0, 1.0),
            time: 0,
            partial_tick: 0.0,
            range: 2,
            fast: false,
            color: [1.0; 4],
        };
        // One snap cell, in blocks.
        let cell = SNAP_CELLS as f64 * 12.0;
        let mut mesh = Mesh::default();
        let (rebuilt, uniform) = mesh.prepare(&input);
        assert!(rebuilt);
        // The shader draws a vertex at `camera + position + offset`, so the world cells the
        // cloud covers are `camera` plus this: the same ones anywhere inside a snap cell.
        assert_eq!(input.camera.x as f32 + uniform.offset[0], 0.0);
        for x in [1.5, cell * 0.15, cell * 0.5, cell * 0.75, cell - 1.5] {
            input.camera.x = x;
            let (rebuilt, uniform) = mesh.prepare(&input);
            assert!(!rebuilt, "rebuilt at {x}");
            assert_eq!(input.camera.x as f32 + uniform.offset[0], 0.0);
            // The trailing origin still covers the whole range past the camera.
            let far = mesh
                .vertices
                .iter()
                .map(|v| v.pos[0] + uniform.offset[0])
                .fold(f32::MIN, f32::max);
            assert!(far >= 32.0, "clouds end {far} blocks ahead at {x}");
        }
        // The next snap cell takes the origin with it.
        input.camera.x = cell;
        let (rebuilt, uniform) = mesh.prepare(&input);
        assert!(rebuilt);
        assert_eq!(input.camera.x as f32 + uniform.offset[0], cell as f32);
    }

    #[test]
    fn only_inside_the_layer_does_the_mesh_follow_the_camera_between_snap_cells() {
        let texture = (8, 8, (0..64).map(|i| i % 3 != 0).collect());
        let mut input = Input {
            texture: &texture,
            uv: [0.5; 2],
            camera: glam::DVec3::new(1.0, 101.0, 1.0),
            time: 0,
            partial_tick: 0.0,
            range: 2,
            fast: false,
            color: [1.0; 4],
        };
        let mut mesh = Mesh::default();
        assert!(mesh.prepare(&input).0);
        // Below the layer, cells crossed inside one snap cell change nothing visible.
        for (x, z) in [(11.9, 11.9), (24.0, 12.0), (35.5, 23.9)] {
            input.camera.x = x;
            input.camera.z = z;
            assert!(
                !mesh.prepare(&input).0,
                "rebuilt at {x},{z} below the layer"
            );
        }
        // Inside it, the inward faces around the camera do move with its own cell.
        input.camera.y = 194.0;
        assert!(mesh.prepare(&input).0);
        input.camera.x = 12.0;
        assert!(mesh.prepare(&input).0);
        input.camera.y = 101.0;
        assert!(mesh.prepare(&input).0);
    }

    #[test]
    fn settings_and_packed_alpha_gate_clouds() {
        for mode in ["false", "fast", "true"] {
            assert_eq!(visible(mode, 1.0), mode != "false");
            assert!(!visible(mode, 0.0));
            assert!(!visible(mode, 0.5 / 255.0));
        }
        assert!(visible("fast", 1.0 / 255.0));
    }

    #[test]
    fn fast_cells_are_flat_at_the_bottom_and_visible_from_both_sides() {
        for bottom in [-10.0, 0.0, 10.0] {
            let [down, up] = flat_cell(-3.0, 7.0, bottom);
            assert_eq!(
                down,
                [
                    [9.0, bottom, 7.0],
                    [9.0, bottom, 19.0],
                    [-3.0, bottom, 19.0],
                    [-3.0, bottom, 7.0],
                ]
            );
            assert_eq!(up, [down[3], down[2], down[1], down[0]]);
            for (quad, sign) in [(down, -1.0), (up, 1.0)] {
                let [a, b, c] = [quad[0], quad[1], quad[2]].map(glam::Vec3::from_array);
                assert!((b - a).cross(c - a).y * sign > 0.0);
            }
        }
    }

    #[test]
    fn cells_are_nearest_first_and_unique() {
        let cells: Vec<_> = cells(2).collect();
        assert_eq!(cells[0], (0, 0));
        assert_eq!(cells.len(), 13);
        assert!(
            cells
                .windows(2)
                .all(|w| w[0].0.abs() + w[0].1.abs() <= w[1].0.abs() + w[1].1.abs())
        );
        assert_eq!(
            cells.iter().collect::<std::collections::HashSet<_>>().len(),
            cells.len()
        );
    }
    #[test]
    fn only_camera_facing_exterior_faces() {
        assert_eq!(face_mask(2, 2, 10.0, [true; 4]), 1 | 4 | 16);
        assert_eq!(face_mask(-2, -2, -5.0, [true; 4]), 2 | 8 | 32);
        assert_eq!(face_mask(2, 2, 0.0, [false; 4]), 3);
    }
    #[test]
    fn cloud_distance_sets_the_radius() {
        assert_eq!(radius_cells(2), 3);
        assert_eq!(radius_cells(12), 16);
        assert_eq!(radius_cells(128), 171);
    }
    #[test]
    fn connected_cells_have_no_exterior_walls() {
        assert_eq!(face_mask(2, 2, 10.0, [false; 4]), 1);
    }
    #[test]
    fn nearby_cells_keep_inward_faces_for_camera_inside_clouds() {
        assert_eq!(face_mask(0, 0, 0.0, [false; 4]), 3 | (63 << 6));
    }

    #[test]
    fn optimized_prepare_matches_reference_for_pow2_and_non_pow2_textures() {
        fn reference_vertices(input: &Input<'_>) -> Vec<crate::mesher::Vertex> {
            let (key, uniform) = input.state();
            let (w, h, tex_cells) = input.texture;
            let (w_i64, h_i64) = (i64::from(*w), i64::from(*h));
            let filled = |cx: i64, cz: i64| {
                tex_cells[(cz.rem_euclid(h_i64) * w_i64 + cx.rem_euclid(w_i64)) as usize]
            };
            let bottom = uniform.offset[1];
            let mut out = Vec::new();
            for (dx, dz) in cells(key.radius) {
                let (cx, cz) = (key.cell_x + i64::from(dx), key.cell_z + i64::from(dz));
                if !filled(cx, cz) {
                    continue;
                }
                let (x0, z0) = (dx as f32 * 12.0, dz as f32 * 12.0);
                if input.fast {
                    emit_flat_cell(&mut out, x0, z0, input.uv);
                    continue;
                }
                let mask = face_mask(
                    dx - key.sub_x,
                    dz - key.sub_z,
                    bottom,
                    [
                        !filled(cx, cz - 1),
                        !filled(cx, cz + 1),
                        !filled(cx - 1, cz),
                        !filled(cx + 1, cz),
                    ],
                );
                for (d, inside) in faces(mask) {
                    let x1 = x0 + 12.0;
                    let z1 = z0 + 12.0;
                    let (p, shade, normal) = match d {
                        0 => (
                            [[x1, 0.0, z0], [x1, 0.0, z1], [x0, 0.0, z1], [x0, 0.0, z0]],
                            1,
                            [0, -127, 0, 0],
                        ),
                        1 => (
                            [[x0, 4.0, z0], [x0, 4.0, z1], [x1, 4.0, z1], [x1, 4.0, z0]],
                            0,
                            [0, 127, 0, 0],
                        ),
                        2 => (
                            [[x0, 0.0, z0], [x0, 4.0, z0], [x1, 4.0, z0], [x1, 0.0, z0]],
                            2,
                            [0, 0, -127, 0],
                        ),
                        3 => (
                            [[x1, 0.0, z1], [x1, 4.0, z1], [x0, 4.0, z1], [x0, 0.0, z1]],
                            2,
                            [0, 0, 127, 0],
                        ),
                        4 => (
                            [[x0, 0.0, z1], [x0, 4.0, z1], [x0, 4.0, z0], [x0, 0.0, z0]],
                            3,
                            [-127, 0, 0, 0],
                        ),
                        _ => (
                            [[x1, 0.0, z0], [x1, 4.0, z0], [x1, 4.0, z1], [x1, 0.0, z1]],
                            3,
                            [127, 0, 0, 0],
                        ),
                    };
                    if inside {
                        push_quad(
                            &mut out,
                            [p[3], p[2], p[1], p[0]],
                            shade,
                            normal.map(|c| -c),
                            input.uv,
                        );
                    } else {
                        push_quad(&mut out, p, shade, normal, input.uv);
                    }
                }
            }
            out
        }

        let pow2 = (
            16u32,
            16u32,
            (0..256).map(|i| (i * 13 + i / 16 * 7) % 5 < 3).collect(),
        );
        let non_pow2 = (
            7u32,
            11u32,
            (0..77).map(|i| (i * 5 + i / 7 * 3) % 4 != 0).collect(),
        );
        for texture in [&pow2, &non_pow2] {
            let mut mesh = Mesh::default();
            for fast in [false, true] {
                for y in [100.0, 194.0, 250.0] {
                    for (x, z) in [(-123.4, 57.8), (245.1, -310.7)] {
                        let input = Input {
                            texture,
                            uv: [0.25, 0.75],
                            camera: glam::DVec3::new(x, y, z),
                            time: 1234,
                            partial_tick: 0.4,
                            range: 4,
                            fast,
                            color: [1.0; 4],
                        };
                        mesh.prepare(&input);
                        let expected = reference_vertices(&input);
                        assert_eq!(
                            bytemuck::cast_slice::<_, u8>(&mesh.vertices),
                            bytemuck::cast_slice::<_, u8>(&expected),
                            "mismatch for {}x{} fast={fast} pos=({x},{y},{z})",
                            texture.0,
                            texture.1
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn fast_clouds_skip_vertical_rebuilds_and_shrink_excess_capacity() {
        let texture = (16, 16, (0..256).map(|i| i % 2 == 0).collect());
        let mut input = Input {
            texture: &texture,
            uv: [0.5; 2],
            camera: glam::DVec3::new(1.0, 100.0, 1.0),
            time: 0,
            partial_tick: 0.0,
            range: 32,
            fast: true,
            color: [1.0; 4],
        };
        let mut mesh = Mesh::default();
        assert!(mesh.prepare(&input).0);
        let big_cap = mesh.vertices.capacity();
        assert!(big_cap > 4096);
        // Crossing into and above the cloud layer in fast mode emits identical quads.
        for y in [192.33, 194.0, 196.33, 250.0] {
            input.camera.y = y;
            assert!(!mesh.prepare(&input).0, "fast clouds rebuilt at y={y}");
        }
        // Shrinking cloud range releases excess vertex buffer capacity.
        input.range = 2;
        assert!(mesh.prepare(&input).0);
        assert!(mesh.vertices.capacity() < big_cap / 2);
    }
}
