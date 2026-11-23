use super::{
    DensityError, DensityVolume, GradientTable, PERLIN_GRADIENTS, dot, lerp2, lerp3, smoothstep,
    smoothstep_derivative, wrap,
};
use crate::{geometry::floor_i32, random::RandomSource};

#[derive(Clone, Debug)]
pub struct PerlinNoise {
    table: GradientTable,
    /// `table.permutations[i & 255]` with index 256 mirroring index 0, so
    /// adjacent `(i & 255, (i + 1) & 255)` lookups share one mask.
    perm: [u8; 257],
    /// `perm[i] & 15`, avoiding the final gradient mask on every corner lookup.
    perm_grad: [u8; 257],
    /// Precomputed X-to-Y permutation tables for `shifted_y0 = 0.0 + offsets[1]`
    /// (valid for `y == +0.0`, and for `y == -0.0` unless `offsets[1]` is also `-0.0`).
    perm_y0_lo: [u8; 257],
    perm_y0_hi: [u8; 257],
    /// Precomputed `PERLIN_GRADIENTS[g][1] * dy0` and `* (dy0 - 1.0)` at `y == 0.0`.
    y0_gy_dy: [f32; 16],
    y0_gy_dy1: [f32; 16],
    y0_fade_y: f32,
}

impl PerlinNoise {
    pub fn new(random: &mut impl RandomSource) -> Self {
        let table = GradientTable::new(random, 256.0);
        let raw = &table.permutations;
        let perm: [u8; 257] = std::array::from_fn(|i| raw[i & 255]);
        let perm_grad: [u8; 257] = std::array::from_fn(|i| perm[i] & 15);
        let shifted_y0 = 0.0 + table.offsets[1];
        let cell_y0 = floor_i32(shifted_y0);
        let dy0 = (shifted_y0 - f64::from(cell_y0)) as f32;
        let dy1 = dy0 - 1.0;
        let y0_fade_y = smoothstep(dy0);
        let perm_y0_lo: [u8; 257] = std::array::from_fn(|i| {
            let p = i32::from(perm[i]);
            perm[(p.wrapping_add(cell_y0) & 255) as usize]
        });
        let perm_y0_hi: [u8; 257] = std::array::from_fn(|i| {
            let p = i32::from(perm[i]);
            perm[(p.wrapping_add(cell_y0).wrapping_add(1) & 255) as usize]
        });
        let y0_gy_dy: [f32; 16] = std::array::from_fn(|g| PERLIN_GRADIENTS[g][1] * dy0);
        let y0_gy_dy1: [f32; 16] = std::array::from_fn(|g| PERLIN_GRADIENTS[g][1] * dy1);
        Self {
            table,
            perm,
            perm_grad,
            perm_y0_lo,
            perm_y0_hi,
            y0_gy_dy,
            y0_gy_dy1,
            y0_fade_y,
        }
    }
    pub fn table(&self) -> &GradientTable {
        &self.table
    }
    pub fn range(&self) -> [f32; 2] {
        [-2.0, 2.0]
    }
    #[inline]
    pub fn get2(&self, x: f64, z: f64) -> f32 {
        self.get3_y0(wrap(x), wrap(z))
    }
    #[inline]
    pub fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        if y == 0.0
            && !(y.to_bits() == (-0.0_f64).to_bits()
                && self.table.offsets[1].to_bits() == (-0.0_f64).to_bits())
        {
            return self.get3_y0(x, z);
        }
        let shifted_x = wrap(x) + self.table.offsets[0];
        let shifted_y = wrap(y) + self.table.offsets[1];
        let shifted_z = wrap(z) + self.table.offsets[2];
        let cell_x = floor_i32(shifted_x);
        let cell_y = floor_i32(shifted_y);
        let cell_z = floor_i32(shifted_z);
        let dx = (shifted_x - f64::from(cell_x)) as f32;
        let dy = (shifted_y - f64::from(cell_y)) as f32;
        let dz = (shifted_z - f64::from(cell_z)) as f32;
        self.sample_fast(cell_x, cell_y, cell_z, dx, dy, dz, dy)
    }

    /// Scalar `get3(x, +0.0, z)` reusing the immutable Y=0 cell, fade, and
    /// X-to-Y permutation tables. Bit-identical to `get3(x, y, z)` for `y == +0.0`,
    /// and for `y == -0.0` whenever `table.offsets[1]` is not `-0.0`.
    #[inline]
    pub(super) fn get3_y0(&self, x: f64, z: f64) -> f32 {
        let shifted_x = wrap(x) + self.table.offsets[0];
        let shifted_z = wrap(z) + self.table.offsets[2];
        let cell_x = floor_i32(shifted_x);
        let cell_z = floor_i32(shifted_z);
        let dx = (shifted_x - f64::from(cell_x)) as f32;
        let dz = (shifted_z - f64::from(cell_z)) as f32;
        let dx1 = dx - 1.0;
        let dz1 = dz - 1.0;
        let cx = (cell_x & 255) as usize;
        let lower = i32::from(self.perm_y0_lo[cx]);
        let lower_x = i32::from(self.perm_y0_lo[cx + 1]);
        let upper = i32::from(self.perm_y0_hi[cx]);
        let upper_x = i32::from(self.perm_y0_hi[cx + 1]);
        let z0 = (lower.wrapping_add(cell_z) & 255) as usize;
        let z1 = (lower_x.wrapping_add(cell_z) & 255) as usize;
        let z2 = (upper.wrapping_add(cell_z) & 255) as usize;
        let z3 = (upper_x.wrapping_add(cell_z) & 255) as usize;
        let g0 = usize::from(self.perm_grad[z0]);
        let g4 = usize::from(self.perm_grad[z0 + 1]);
        let g1 = usize::from(self.perm_grad[z1]);
        let g5 = usize::from(self.perm_grad[z1 + 1]);
        let g2 = usize::from(self.perm_grad[z2]);
        let g6 = usize::from(self.perm_grad[z2 + 1]);
        let g3 = usize::from(self.perm_grad[z3]);
        let g7 = usize::from(self.perm_grad[z3 + 1]);
        let dot_lo = |g: usize, px: f32, pz: f32| {
            let grad = PERLIN_GRADIENTS[g];
            grad[0] * px + self.y0_gy_dy[g] + grad[2] * pz
        };
        let dot_hi = |g: usize, px: f32, pz: f32| {
            let grad = PERLIN_GRADIENTS[g];
            grad[0] * px + self.y0_gy_dy1[g] + grad[2] * pz
        };
        let values = [
            dot_lo(g0, dx, dz),
            dot_lo(g1, dx1, dz),
            dot_hi(g2, dx, dz),
            dot_hi(g3, dx1, dz),
            dot_lo(g4, dx, dz1),
            dot_lo(g5, dx1, dz1),
            dot_hi(g6, dx, dz1),
            dot_hi(g7, dx1, dz1),
        ];
        lerp3([smoothstep(dx), self.y0_fade_y, smoothstep(dz)], values)
    }

    #[inline(always)]
    fn plane_grads(&self, p: i32, q: i32, y_cell: i32, cell_z: i32) -> [usize; 4] {
        let r0 = i32::from(self.perm[(p.wrapping_add(y_cell) & 255) as usize]);
        let r1 = i32::from(self.perm[(q.wrapping_add(y_cell) & 255) as usize]);
        let z0 = (r0.wrapping_add(cell_z) & 255) as usize;
        let z1 = (r1.wrapping_add(cell_z) & 255) as usize;
        [
            usize::from(self.perm_grad[z0]),
            usize::from(self.perm_grad[z1]),
            usize::from(self.perm_grad[z0 + 1]),
            usize::from(self.perm_grad[z1 + 1]),
        ]
    }

    #[inline(always)]
    fn cell_grads(&self, p: i32, q: i32, y_cell: i32, cell_z: i32) -> ([usize; 4], [usize; 4]) {
        let py = (p.wrapping_add(y_cell) & 255) as usize;
        let qy = (q.wrapping_add(y_cell) & 255) as usize;
        let r00 = i32::from(self.perm[py]);
        let r01 = i32::from(self.perm[py + 1]);
        let r10 = i32::from(self.perm[qy]);
        let r11 = i32::from(self.perm[qy + 1]);
        let z00 = (r00.wrapping_add(cell_z) & 255) as usize;
        let z10 = (r10.wrapping_add(cell_z) & 255) as usize;
        let z01 = (r01.wrapping_add(cell_z) & 255) as usize;
        let z11 = (r11.wrapping_add(cell_z) & 255) as usize;
        (
            [
                usize::from(self.perm_grad[z00]),
                usize::from(self.perm_grad[z10]),
                usize::from(self.perm_grad[z00 + 1]),
                usize::from(self.perm_grad[z10 + 1]),
            ],
            [
                usize::from(self.perm_grad[z01]),
                usize::from(self.perm_grad[z11]),
                usize::from(self.perm_grad[z01 + 1]),
                usize::from(self.perm_grad[z11 + 1]),
            ],
        )
    }

    #[inline]
    fn sample_fast(
        &self,
        cell_x: i32,
        cell_y: i32,
        cell_z: i32,
        dx: f32,
        dy: f32,
        dz: f32,
        interpolation_y: f32,
    ) -> f32 {
        let cx = (cell_x & 255) as usize;
        let p = i32::from(self.perm[cx]);
        let q = i32::from(self.perm[cx + 1]);
        let ([g0, g1, g4, g5], [g2, g3, g6, g7]) = self.cell_grads(p, q, cell_y, cell_z);
        let dx1 = dx - 1.0;
        let dy1 = dy - 1.0;
        let dz1 = dz - 1.0;
        let cdot = |g: usize, px: f32, py: f32, pz: f32| {
            let grad = PERLIN_GRADIENTS[g];
            grad[0] * px + grad[1] * py + grad[2] * pz
        };
        let values = [
            cdot(g0, dx, dy, dz),
            cdot(g1, dx1, dy, dz),
            cdot(g2, dx, dy1, dz),
            cdot(g3, dx1, dy1, dz),
            cdot(g4, dx, dy, dz1),
            cdot(g5, dx1, dy, dz1),
            cdot(g6, dx, dy1, dz1),
            cdot(g7, dx1, dy1, dz1),
        ];
        lerp3(
            [smoothstep(dx), smoothstep(interpolation_y), smoothstep(dz)],
            values,
        )
    }

    fn coordinates(&self, point: [f64; 3]) -> ([i32; 3], [f64; 3]) {
        let shifted: [f64; 3] = std::array::from_fn(|i| wrap(point[i]) + self.table.offsets[i]);
        let cell = shifted.map(floor_i32);
        (
            cell,
            std::array::from_fn(|i| shifted[i] - f64::from(cell[i])),
        )
    }

    fn gradients(&self, cell: [i32; 3]) -> [[f32; 3]; 8] {
        let cx = (cell[0] & 255) as usize;
        self.gradients_at(
            i32::from(self.perm[cx]),
            i32::from(self.perm[cx + 1]),
            cell[1],
            cell[2],
        )
    }

    /// Corner gradients for a Y cell, with the X column's permutation already
    /// resolved.
    #[inline]
    fn gradients_at(&self, p: i32, q: i32, cell_y: i32, cell_z: i32) -> [[f32; 3]; 8] {
        let ([g0, g1, g4, g5], [g2, g3, g6, g7]) = self.cell_grads(p, q, cell_y, cell_z);
        [
            PERLIN_GRADIENTS[g0],
            PERLIN_GRADIENTS[g1],
            PERLIN_GRADIENTS[g2],
            PERLIN_GRADIENTS[g3],
            PERLIN_GRADIENTS[g4],
            PERLIN_GRADIENTS[g5],
            PERLIN_GRADIENTS[g6],
            PERLIN_GRADIENTS[g7],
        ]
    }

    fn corner_values(gradients: [[f32; 3]; 8], point: [f32; 3]) -> [f32; 8] {
        std::array::from_fn(|i| {
            dot(
                gradients[i],
                [
                    point[0] - (i & 1) as f32,
                    point[1] - ((i >> 1) & 1) as f32,
                    point[2] - ((i >> 2) & 1) as f32,
                ],
            )
        })
    }

    fn sample(&self, cell: [i32; 3], point: [f32; 3], interpolation_y: f32) -> f32 {
        self.sample_fast(
            cell[0],
            cell[1],
            cell[2],
            point[0],
            point[1],
            point[2],
            interpolation_y,
        )
    }

    /// Returns the sampled value and adds its gradient into the supplied array.
    pub fn noise_with_derivative(&self, x: f64, y: f64, z: f64, derivative: &mut [f32; 3]) -> f32 {
        let (cell, relative) = self.coordinates([x, y, z]);
        let point = relative.map(|v| v as f32);
        let gradients = self.gradients(cell);
        let values = Self::corner_values(gradients, point);
        let fade = point.map(smoothstep);
        let gradient =
            std::array::from_fn::<_, 3, _>(|axis| lerp3(fade, gradients.map(|g| g[axis])));
        let changes = [
            lerp2(
                fade[1],
                fade[2],
                [
                    values[1] - values[0],
                    values[3] - values[2],
                    values[5] - values[4],
                    values[7] - values[6],
                ],
            ),
            lerp2(
                fade[2],
                fade[0],
                [
                    values[2] - values[0],
                    values[6] - values[4],
                    values[3] - values[1],
                    values[7] - values[5],
                ],
            ),
            lerp2(
                fade[0],
                fade[1],
                [
                    values[4] - values[0],
                    values[5] - values[1],
                    values[6] - values[2],
                    values[7] - values[3],
                ],
            ),
        ];
        for axis in 0..3 {
            derivative[axis] += gradient[axis] + smoothstep_derivative(point[axis]) * changes[axis];
        }
        lerp3(fade, values)
    }

    /// Adds a scaled volume using vanilla's cached X/Z dot products and Y loop.
    /// Extra buffer elements are untouched; short buffers are rejected up front.
    pub fn add_to_volume(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) -> Result<(), DensityError> {
        self.add_volume::<true>(buffer, volume, xz_scale, y_scale, amplitude, None, None)
    }

    /// [`Self::add_to_volume`] for a caller that reads only the Ys set in `needed`: bit `y` of
    /// `needed[column]`, columns in buffer order. The other Ys may be left unchanged. Without
    /// a bit for every Y (more than 64 Ys, or too few columns), every Y is added.
    pub fn add_to_needed(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
        needed: &[u64],
    ) -> Result<(), DensityError> {
        self.add_volume::<true>(
            buffer,
            volume,
            xz_scale,
            y_scale,
            amplitude,
            None,
            Some(needed),
        )
    }

    /// `BATCH` = false evaluates one Y at a time: the reference the batched loop is tested against.
    #[allow(clippy::too_many_arguments)]
    fn add_volume<const BATCH: bool>(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
        fudge_y_scale: Option<f64>,
        needed: Option<&[u64]>,
    ) -> Result<(), DensityError> {
        if buffer.len() < volume.size() {
            return Err(DensityError::BufferTooShort);
        }
        let [size_x, size_y, size_z] = volume.sizes();
        let columns = size_x * size_z;
        let needed = needed.filter(|needed| size_y <= 64 && needed.len() >= columns);
        if needed.is_some_and(|mask| {
            let valid_mask = if size_y == 64 {
                u64::MAX
            } else {
                (1_u64 << size_y) - 1
            };
            mask[..columns].iter().all(|&m| m & valid_mask == 0)
        }) {
            return Ok(());
        }
        // Every X/Z column walks the same Y coordinates, so their cells, offsets
        // and fades are computed once per volume instead of once per column.
        let mut stack = ([0_i32; STACK_Y], [0.0_f32; STACK_Y], [0.0_f32; STACK_Y]);
        let mut heap = (Vec::new(), Vec::new(), Vec::new());
        let (cells, fades, offsets): (&mut [i32], &mut [f32], &mut [f32]) = if size_y <= STACK_Y {
            (
                &mut stack.0[..size_y],
                &mut stack.1[..size_y],
                &mut stack.2[..size_y],
            )
        } else {
            heap.0.resize(size_y, 0);
            heap.1.resize(size_y, 0.0);
            heap.2.resize(size_y, 0.0);
            (&mut heap.0[..], &mut heap.1[..], &mut heap.2[..])
        };
        let mut changes = 0usize;
        let mut all_same_y = true;
        for y in 0..size_y {
            let input_y = f64::from(volume.coordinate(1, y)) * y_scale;
            let shifted_y = wrap(input_y) + self.table.offsets[1];
            let cell = floor_i32(shifted_y);
            let dy = shifted_y - f64::from(cell);
            let fade = smoothstep(dy as f32);
            // The Y offset inside the cell used for the corner values, after any fudge.
            let offset = match fudge_y_scale {
                Some(scale) => (dy - compute_fudge_y(input_y, dy, scale)) as f32,
                None => dy as f32,
            };
            cells[y] = cell;
            fades[y] = fade;
            offsets[y] = offset;
            if y > 0 {
                if cell != cells[y - 1] {
                    changes += 1;
                    all_same_y = false;
                } else if fade.to_bits() != fades[0].to_bits()
                    || offset.to_bits() != offsets[0].to_bits()
                {
                    all_same_y = false;
                }
            }
        }
        let (cells, fades, offsets) = (&*cells, &*fades, &*offsets);
        // Every Z row walks the same X coordinates, so compute their cells,
        // fades and X permutes once per volume instead of once per Z row.
        let mut x_stack = (
            [0.0_f32; STACK_X],
            [0.0_f32; STACK_X],
            [0_u8; STACK_X],
            [0_u8; STACK_X],
        );
        let mut x_heap = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (x_dx, x_fade, x_p, x_q): (&mut [f32], &mut [f32], &mut [u8], &mut [u8]) =
            if size_x <= STACK_X {
                (
                    &mut x_stack.0[..size_x],
                    &mut x_stack.1[..size_x],
                    &mut x_stack.2[..size_x],
                    &mut x_stack.3[..size_x],
                )
            } else {
                x_heap.0.resize(size_x, 0.0);
                x_heap.1.resize(size_x, 0.0);
                x_heap.2.resize(size_x, 0);
                x_heap.3.resize(size_x, 0);
                (
                    &mut x_heap.0[..],
                    &mut x_heap.1[..],
                    &mut x_heap.2[..],
                    &mut x_heap.3[..],
                )
            };
        for x in 0..size_x {
            let shifted_x =
                wrap(f64::from(volume.coordinate(0, x)) * xz_scale) + self.table.offsets[0];
            let cell_x = floor_i32(shifted_x);
            let dx = (shifted_x - f64::from(cell_x)) as f32;
            x_dx[x] = dx;
            x_fade[x] = smoothstep(dx);
            let cx = (cell_x & 255) as usize;
            x_p[x] = self.perm[cx];
            x_q[x] = self.perm[cx + 1];
        }
        let (x_dx, x_fade, x_p, x_q) = (&*x_dx, &*x_fade, &*x_p, &*x_q);
        // When every Y in the volume has bit-identical `(cell, fade, offset)`
        // (all 2D planes `size_y == 1` and all `y_scale == 0.0` volumes),
        // every Y in a column receives the exact same contribution: precompute
        // the X-to-Y permute rows once and evaluate once per (z, x) column.
        if BATCH && all_same_y {
            let cell_y = cells[0];
            let fade_y = fades[0];
            let dy = offsets[0];
            let dy1 = dy - 1.0;
            let mut xy_stack = [[0_u8; 4]; STACK_X];
            let mut xy_heap = Vec::new();
            let xy_rows: &mut [[u8; 4]] = if size_x <= STACK_X {
                &mut xy_stack[..size_x]
            } else {
                xy_heap.resize(size_x, [0; 4]);
                &mut xy_heap[..]
            };
            for x in 0..size_x {
                let py = (i32::from(x_p[x]).wrapping_add(cell_y) & 255) as usize;
                let qy = (i32::from(x_q[x]).wrapping_add(cell_y) & 255) as usize;
                xy_rows[x] = [
                    self.perm[py],
                    self.perm[qy],
                    self.perm[py + 1],
                    self.perm[qy + 1],
                ];
            }
            let mut index = 0;
            for z in 0..size_z {
                let shifted_z =
                    wrap(f64::from(volume.coordinate(2, z)) * xz_scale) + self.table.offsets[2];
                let cell_z = floor_i32(shifted_z);
                let dz = (shifted_z - f64::from(cell_z)) as f32;
                let dz1 = dz - 1.0;
                let fade_z = smoothstep(dz);
                for x in 0..size_x {
                    let wanted = needed.map(|needed| needed[x + z * size_x]);
                    if wanted == Some(0) {
                        index += size_y;
                        continue;
                    }
                    let dx = x_dx[x];
                    let dx1 = dx - 1.0;
                    let fade_x = x_fade[x];
                    let [r00, r10, r01, r11] = xy_rows[x];
                    let z00 = (i32::from(r00).wrapping_add(cell_z) & 255) as usize;
                    let z10 = (i32::from(r10).wrapping_add(cell_z) & 255) as usize;
                    let z01 = (i32::from(r01).wrapping_add(cell_z) & 255) as usize;
                    let z11 = (i32::from(r11).wrapping_add(cell_z) & 255) as usize;
                    let val = |g: usize, px: f32, py: f32, pz: f32| {
                        let grad = PERLIN_GRADIENTS[g];
                        (grad[0] * px + grad[2] * pz) + grad[1] * py
                    };
                    let values = [
                        val(usize::from(self.perm_grad[z00]), dx, dy, dz),
                        val(usize::from(self.perm_grad[z10]), dx1, dy, dz),
                        val(usize::from(self.perm_grad[z01]), dx, dy1, dz),
                        val(usize::from(self.perm_grad[z11]), dx1, dy1, dz),
                        val(usize::from(self.perm_grad[z00 + 1]), dx, dy, dz1),
                        val(usize::from(self.perm_grad[z10 + 1]), dx1, dy, dz1),
                        val(usize::from(self.perm_grad[z01 + 1]), dx, dy1, dz1),
                        val(usize::from(self.perm_grad[z11 + 1]), dx1, dy1, dz1),
                    ];
                    let contrib = amplitude * lerp3([fade_x, fade_y, fade_z], values);
                    if size_y == 1 {
                        buffer[index] += contrib;
                    } else {
                        for slot in &mut buffer[index..index + size_y] {
                            *slot += contrib;
                        }
                    }
                    index += size_y;
                }
            }
            return Ok(());
        }
        // A column's corner terms depend on its Y cell only through the corners'
        // gradients. When the cell changes often, tabulate per column each of the
        // sixteen gradients' terms once instead of computing them per cell.
        let tabulate = BATCH && changes >= TABULATE_CHANGES;
        let mut index = 0;
        for z in 0..size_z {
            let shifted_z =
                wrap(f64::from(volume.coordinate(2, z)) * xz_scale) + self.table.offsets[2];
            let cell_z = floor_i32(shifted_z);
            let dz = (shifted_z - f64::from(cell_z)) as f32;
            let dz1 = dz - 1.0;
            let fade_z = smoothstep(dz);
            for x in 0..size_x {
                // The Ys of this column the caller reads, if not all of them.
                let wanted = needed.map(|needed| needed[x + z * size_x]);
                if wanted == Some(0) {
                    index += size_y;
                    continue;
                }
                let dx = x_dx[x];
                let dx1 = dx - 1.0;
                let fade_x = x_fade[x];
                let p = i32::from(x_p[x]);
                let q = i32::from(x_q[x]);
                let column = &mut buffer[index..index + size_y];
                if !BATCH {
                    let mut has_prev = false;
                    let mut previous_y = 0;
                    let mut xz_dots = [0.0_f32; 8];
                    let mut y_gradients = [0.0_f32; 8];
                    for y in 0..size_y {
                        if wanted.is_some_and(|wanted| (wanted >> y) & 1 == 0) {
                            continue;
                        }
                        let cell = cells[y];
                        if !has_prev || previous_y != cell {
                            let gradients = self.gradients_at(p, q, cell, cell_z);
                            for i in 0..8 {
                                xz_dots[i] = gradients[i][0] * (dx - (i & 1) as f32)
                                    + gradients[i][2] * (dz - ((i >> 2) & 1) as f32);
                                y_gradients[i] = gradients[i][1];
                            }
                            previous_y = cell;
                            has_prev = true;
                        }
                        let dy = offsets[y];
                        let values = std::array::from_fn(|i| {
                            xz_dots[i] + y_gradients[i] * (dy - ((i >> 1) & 1) as f32)
                        });
                        column[y] += amplitude * lerp3([fade_x, fades[y], fade_z], values);
                    }
                    index += size_y;
                    continue;
                }
                let terms = tabulate.then(|| CornerTerms::new(dx, dz));
                let mut has_prev = false;
                let mut prev_y = 0_i32;
                let mut prev_bottom = [0_usize; 4];
                let mut prev_top = [0_usize; 4];
                let mut dots_valid = false;
                let mut xz_dots = [0.0_f32; 8];
                let mut y_gradients = [0.0_f32; 8];
                let advance_cell = |cell: i32,
                                    has_prev: &mut bool,
                                    prev_y: &mut i32,
                                    prev_bottom: &mut [usize; 4],
                                    prev_top: &mut [usize; 4],
                                    dots_valid: &mut bool| {
                    if *has_prev && cell == *prev_y {
                        return;
                    }
                    if *has_prev && cell == prev_y.wrapping_add(1) {
                        *prev_bottom = *prev_top;
                        *prev_top = self.plane_grads(p, q, cell.wrapping_add(1), cell_z);
                    } else {
                        (*prev_bottom, *prev_top) = self.cell_grads(p, q, cell, cell_z);
                        *has_prev = true;
                    }
                    *prev_y = cell;
                    *dots_valid = false;
                };
                let fill_dots = |bottom: [usize; 4],
                                 top: [usize; 4],
                                 terms: Option<&CornerTerms>,
                                 xz_dots: &mut [f32; 8],
                                 y_gradients: &mut [f32; 8]| {
                    if let Some(terms) = terms {
                        [xz_dots[0], y_gradients[0]] = terms.0[0][bottom[0] & 15];
                        [xz_dots[1], y_gradients[1]] = terms.0[1][bottom[1] & 15];
                        [xz_dots[2], y_gradients[2]] = terms.0[0][top[0] & 15];
                        [xz_dots[3], y_gradients[3]] = terms.0[1][top[1] & 15];
                        [xz_dots[4], y_gradients[4]] = terms.0[2][bottom[2] & 15];
                        [xz_dots[5], y_gradients[5]] = terms.0[3][bottom[3] & 15];
                        [xz_dots[6], y_gradients[6]] = terms.0[2][top[2] & 15];
                        [xz_dots[7], y_gradients[7]] = terms.0[3][top[3] & 15];
                    } else {
                        let grads = [
                            bottom[0], bottom[1], top[0], top[1], bottom[2], bottom[3], top[2],
                            top[3],
                        ];
                        let dxs = [dx, dx1, dx, dx1, dx, dx1, dx, dx1];
                        let dzs = [dz, dz, dz, dz, dz1, dz1, dz1, dz1];
                        for i in 0..8 {
                            let [gx, gy, gz] = PERLIN_GRADIENTS[grads[i] & 15];
                            xz_dots[i] = gx * dxs[i] + gz * dzs[i];
                            y_gradients[i] = gy;
                        }
                    }
                };
                if let Some(mask) = wanted {
                    let valid_mask = if size_y == 64 {
                        u64::MAX
                    } else {
                        (1_u64 << size_y) - 1
                    };
                    let mut rem = mask & valid_mask;
                    while rem != 0 {
                        let y = rem.trailing_zeros() as usize;
                        let cell = cells[y];
                        let group_needed = ((rem >> y) & 0xF).count_ones();
                        if y + 4 <= size_y && group_needed >= 2 {
                            if cells[y + 1] == cell && cells[y + 2] == cell && cells[y + 3] == cell
                            {
                                advance_cell(
                                    cell,
                                    &mut has_prev,
                                    &mut prev_y,
                                    &mut prev_bottom,
                                    &mut prev_top,
                                    &mut dots_valid,
                                );
                                if !dots_valid {
                                    fill_dots(
                                        prev_bottom,
                                        prev_top,
                                        terms.as_ref(),
                                        &mut xz_dots,
                                        &mut y_gradients,
                                    );
                                    dots_valid = true;
                                }
                                add_four_same(
                                    &mut column[y..y + 4],
                                    &offsets[y..y + 4],
                                    &fades[y..y + 4],
                                    &xz_dots,
                                    &y_gradients,
                                    [fade_x, fade_z],
                                    amplitude,
                                );
                                rem &= !(0xF_u64 << y);
                                continue;
                            }
                            if let Some(terms) = &terms {
                                let mut bottoms = [[0_usize; 4]; 4];
                                let mut tops = [[0_usize; 4]; 4];
                                for lane in 0..4 {
                                    advance_cell(
                                        cells[y + lane],
                                        &mut has_prev,
                                        &mut prev_y,
                                        &mut prev_bottom,
                                        &mut prev_top,
                                        &mut dots_valid,
                                    );
                                    bottoms[lane] = prev_bottom;
                                    tops[lane] = prev_top;
                                }
                                add_four_mixed(
                                    &mut column[y..y + 4],
                                    &offsets[y..y + 4],
                                    &fades[y..y + 4],
                                    terms,
                                    &bottoms,
                                    &tops,
                                    [fade_x, fade_z],
                                    amplitude,
                                );
                                rem &= !(0xF_u64 << y);
                                continue;
                            }
                        }
                        advance_cell(
                            cell,
                            &mut has_prev,
                            &mut prev_y,
                            &mut prev_bottom,
                            &mut prev_top,
                            &mut dots_valid,
                        );
                        if !dots_valid {
                            fill_dots(
                                prev_bottom,
                                prev_top,
                                terms.as_ref(),
                                &mut xz_dots,
                                &mut y_gradients,
                            );
                            dots_valid = true;
                        }
                        let dy = offsets[y];
                        let dy1 = dy - 1.0;
                        let values = [
                            xz_dots[0] + y_gradients[0] * dy,
                            xz_dots[1] + y_gradients[1] * dy,
                            xz_dots[2] + y_gradients[2] * dy1,
                            xz_dots[3] + y_gradients[3] * dy1,
                            xz_dots[4] + y_gradients[4] * dy,
                            xz_dots[5] + y_gradients[5] * dy,
                            xz_dots[6] + y_gradients[6] * dy1,
                            xz_dots[7] + y_gradients[7] * dy1,
                        ];
                        column[y] += amplitude * lerp3([fade_x, fades[y], fade_z], values);
                        rem &= rem - 1;
                    }
                } else {
                    let mut y = 0;
                    while y < size_y {
                        let cell = cells[y];
                        if y + 4 <= size_y {
                            if cells[y + 1] == cell && cells[y + 2] == cell && cells[y + 3] == cell
                            {
                                advance_cell(
                                    cell,
                                    &mut has_prev,
                                    &mut prev_y,
                                    &mut prev_bottom,
                                    &mut prev_top,
                                    &mut dots_valid,
                                );
                                if !dots_valid {
                                    fill_dots(
                                        prev_bottom,
                                        prev_top,
                                        terms.as_ref(),
                                        &mut xz_dots,
                                        &mut y_gradients,
                                    );
                                    dots_valid = true;
                                }
                                add_four_same(
                                    &mut column[y..y + 4],
                                    &offsets[y..y + 4],
                                    &fades[y..y + 4],
                                    &xz_dots,
                                    &y_gradients,
                                    [fade_x, fade_z],
                                    amplitude,
                                );
                                y += 4;
                                continue;
                            }
                            if let Some(terms) = &terms {
                                let mut bottoms = [[0_usize; 4]; 4];
                                let mut tops = [[0_usize; 4]; 4];
                                for lane in 0..4 {
                                    advance_cell(
                                        cells[y + lane],
                                        &mut has_prev,
                                        &mut prev_y,
                                        &mut prev_bottom,
                                        &mut prev_top,
                                        &mut dots_valid,
                                    );
                                    bottoms[lane] = prev_bottom;
                                    tops[lane] = prev_top;
                                }
                                add_four_mixed(
                                    &mut column[y..y + 4],
                                    &offsets[y..y + 4],
                                    &fades[y..y + 4],
                                    terms,
                                    &bottoms,
                                    &tops,
                                    [fade_x, fade_z],
                                    amplitude,
                                );
                                y += 4;
                                continue;
                            }
                        }
                        advance_cell(
                            cell,
                            &mut has_prev,
                            &mut prev_y,
                            &mut prev_bottom,
                            &mut prev_top,
                            &mut dots_valid,
                        );
                        if !dots_valid {
                            fill_dots(
                                prev_bottom,
                                prev_top,
                                terms.as_ref(),
                                &mut xz_dots,
                                &mut y_gradients,
                            );
                            dots_valid = true;
                        }
                        let dy = offsets[y];
                        let dy1 = dy - 1.0;
                        let values = [
                            xz_dots[0] + y_gradients[0] * dy,
                            xz_dots[1] + y_gradients[1] * dy,
                            xz_dots[2] + y_gradients[2] * dy1,
                            xz_dots[3] + y_gradients[3] * dy1,
                            xz_dots[4] + y_gradients[4] * dy,
                            xz_dots[5] + y_gradients[5] * dy,
                            xz_dots[6] + y_gradients[6] * dy1,
                            xz_dots[7] + y_gradients[7] * dy1,
                        ];
                        column[y] += amplitude * lerp3([fade_x, fades[y], fade_z], values);
                        y += 1;
                    }
                }
                index += size_y;
            }
        }
        Ok(())
    }
}

/// Y columns up to this height keep their per-Y samples on the stack (covers
/// full 384-block Overworld columns as well as 96-quart climate and ore volumes).
const STACK_Y: usize = 385;
/// X rows up to this width keep their per-X samples on the stack.
const STACK_X: usize = 32;
/// Y cell changes per column from which [`CornerTerms`] are tabulated: building the
/// table costs about as much as computing the terms for a few cells.
const TABULATE_CHANGES: usize = 4;

/// One column's corner terms by XZ corner (bit 0 X, bit 1 Z) and gradient index:
/// the XZ dot product, computed exactly as for a single cell, and the Y gradient.
struct CornerTerms([[[f32; 2]; 16]; 4]);

impl CornerTerms {
    fn new(dx: f32, dz: f32) -> Self {
        let dxs = [dx, dx - 1.0];
        let dzs = [dz, dz - 1.0];
        Self(std::array::from_fn(|corner| {
            let cx = dxs[corner & 1];
            let cz = dzs[corner >> 1];
            std::array::from_fn(|gradient| {
                let [gx, gy, gz] = PERLIN_GRADIENTS[gradient];
                [gx * cx + gz * cz, gy]
            })
        }))
    }
}

#[derive(Clone, Debug)]
pub struct SmearedPerlinNoise {
    base: PerlinNoise,
    fudge_y_scale: f64,
}

impl SmearedPerlinNoise {
    pub fn new(random: &mut impl RandomSource, fudge_y_scale: f64) -> Self {
        Self {
            base: PerlinNoise::new(random),
            fudge_y_scale,
        }
    }
    pub fn table(&self) -> &GradientTable {
        self.base.table()
    }
    pub fn range(&self) -> Result<[f32; 2], super::NanRange> {
        let bound = (self.fudge_y_scale.abs() + 2.0) as f32;
        if bound.is_nan() {
            Err(super::NanRange)
        } else {
            Ok([-bound, bound])
        }
    }
    pub fn get2(&self, x: f64, z: f64) -> f32 {
        self.get3(wrap(x), 0.0, wrap(z))
    }
    pub fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        let (cell, relative) = self.base.coordinates([x, y, z]);
        let fudge = compute_fudge_y(y, relative[1], self.fudge_y_scale);
        self.base.sample(
            cell,
            [
                relative[0] as f32,
                (relative[1] - fudge) as f32,
                relative[2] as f32,
            ],
            relative[1] as f32,
        )
    }
    // This inherited method samples the unsmeared field, just as vanilla does.
    pub fn noise_with_derivative(&self, x: f64, y: f64, z: f64, derivative: &mut [f32; 3]) -> f32 {
        self.base.noise_with_derivative(x, y, z, derivative)
    }
    pub fn add_to_volume(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) -> Result<(), DensityError> {
        self.base.add_volume::<true>(
            buffer,
            volume,
            xz_scale,
            y_scale,
            amplitude,
            Some(self.fudge_y_scale),
            None,
        )
    }
    pub fn add_to_needed(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
        needed: &[u64],
    ) -> Result<(), DensityError> {
        self.base.add_volume::<true>(
            buffer,
            volume,
            xz_scale,
            y_scale,
            amplitude,
            Some(self.fudge_y_scale),
            Some(needed),
        )
    }
}

fn compute_fudge_y(input_y: f64, relative_y: f64, scale: f64) -> f64 {
    let limited_y = if input_y >= 0.0 && input_y < relative_y {
        input_y
    } else {
        relative_y
    };
    f64::from(floor_i32(limited_y / scale + f64::from(1e-7_f32))) * scale
}

#[inline(always)]
fn lerp_s4(d: f32, a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0] + d * (b[0] - a[0]),
        a[1] + d * (b[1] - a[1]),
        a[2] + d * (b[2] - a[2]),
        a[3] + d * (b[3] - a[3]),
    ]
}

#[inline(always)]
fn lerp_v4(d: [f32; 4], a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0] + d[0] * (b[0] - a[0]),
        a[1] + d[1] * (b[1] - a[1]),
        a[2] + d[2] * (b[2] - a[2]),
        a[3] + d[3] * (b[3] - a[3]),
    ]
}

#[inline(always)]
fn lerp3_4(fade_x: f32, fy: [f32; 4], fade_z: f32, v: [[f32; 4]; 8]) -> [f32; 4] {
    let x00 = lerp_s4(fade_x, v[0], v[1]);
    let x01 = lerp_s4(fade_x, v[2], v[3]);
    let y0 = lerp_v4(fy, x00, x01);
    let x10 = lerp_s4(fade_x, v[4], v[5]);
    let x11 = lerp_s4(fade_x, v[6], v[7]);
    let y1 = lerp_v4(fy, x10, x11);
    lerp_s4(fade_z, y0, y1)
}

/// Adds four Y samples in the same cell, laid out lane by lane without
/// transposition so LLVM vectorizes them into 4-wide SIMD operations.
/// Each lane executes the one-at-a-time loop's FP operations in the exact same order.
#[inline(always)]
fn add_four_same(
    out: &mut [f32],
    offsets: &[f32],
    fades: &[f32],
    xz_dots: &[f32; 8],
    y_gradients: &[f32; 8],
    [fade_x, fade_z]: [f32; 2],
    amplitude: f32,
) {
    let dy = [offsets[0], offsets[1], offsets[2], offsets[3]];
    let dy1 = [dy[0] - 1.0, dy[1] - 1.0, dy[2] - 1.0, dy[3] - 1.0];
    let fy = [fades[0], fades[1], fades[2], fades[3]];
    let corner_lo = |i: usize| {
        let dot = xz_dots[i];
        let grad = y_gradients[i];
        [
            dot + grad * dy[0],
            dot + grad * dy[1],
            dot + grad * dy[2],
            dot + grad * dy[3],
        ]
    };
    let corner_hi = |i: usize| {
        let dot = xz_dots[i];
        let grad = y_gradients[i];
        [
            dot + grad * dy1[0],
            dot + grad * dy1[1],
            dot + grad * dy1[2],
            dot + grad * dy1[3],
        ]
    };
    let res = lerp3_4(
        fade_x,
        fy,
        fade_z,
        [
            corner_lo(0),
            corner_lo(1),
            corner_hi(2),
            corner_hi(3),
            corner_lo(4),
            corner_lo(5),
            corner_hi(6),
            corner_hi(7),
        ],
    );
    for lane in 0..4 {
        out[lane] += amplitude * res[lane];
    }
}

/// Adds four Y samples from arbitrary tabulated cells, lane by lane.
/// Each lane executes the one-at-a-time loop's FP operations in the exact same order.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn add_four_mixed(
    out: &mut [f32],
    offsets: &[f32],
    fades: &[f32],
    terms: &CornerTerms,
    bottom: &[[usize; 4]; 4],
    top: &[[usize; 4]; 4],
    [fade_x, fade_z]: [f32; 2],
    amplitude: f32,
) {
    let dy = [offsets[0], offsets[1], offsets[2], offsets[3]];
    let dy1 = [dy[0] - 1.0, dy[1] - 1.0, dy[2] - 1.0, dy[3] - 1.0];
    let fy = [fades[0], fades[1], fades[2], fades[3]];
    let lane_lo = |xz: usize| {
        let table = &terms.0[xz];
        let t0 = table[bottom[0][xz] & 15];
        let t1 = table[bottom[1][xz] & 15];
        let t2 = table[bottom[2][xz] & 15];
        let t3 = table[bottom[3][xz] & 15];
        [
            t0[0] + t0[1] * dy[0],
            t1[0] + t1[1] * dy[1],
            t2[0] + t2[1] * dy[2],
            t3[0] + t3[1] * dy[3],
        ]
    };
    let lane_hi = |xz: usize| {
        let table = &terms.0[xz];
        let t0 = table[top[0][xz] & 15];
        let t1 = table[top[1][xz] & 15];
        let t2 = table[top[2][xz] & 15];
        let t3 = table[top[3][xz] & 15];
        [
            t0[0] + t0[1] * dy1[0],
            t1[0] + t1[1] * dy1[1],
            t2[0] + t2[1] * dy1[2],
            t3[0] + t3[1] * dy1[3],
        ]
    };
    let res = lerp3_4(
        fade_x,
        fy,
        fade_z,
        [
            lane_lo(0),
            lane_lo(1),
            lane_hi(0),
            lane_hi(1),
            lane_lo(2),
            lane_lo(3),
            lane_hi(2),
            lane_hi(3),
        ],
    );
    for lane in 0..4 {
        out[lane] += amplitude * res[lane];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::LegacyRandom;

    /// The batched and unbatched volume loops, with and without a mask of needed Ys,
    /// compared bit-for-bit against `main`'s verbatim reference implementation.
    #[test]
    fn batched_volume_is_bit_identical() {
        let scales = [
            0.0,
            -0.0,
            0.001,
            -0.001,
            0.125,
            -0.5,
            1.0,
            4.0,
            1e100,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ];
        let amplitudes = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            f32::from_bits(1),
        ];
        for seed in [0, -1, 1234, i64::MIN] {
            let noise = PerlinNoise::new(&mut LegacyRandom::new(seed));
            let mut random = LegacyRandom::new(seed);
            for height in [1, 2, 3, 4, 5, 7, 8, 9, 31, 64, 65, 97] {
                for (case, &scale) in scales.iter().enumerate() {
                    let minimum = [
                        [-17, -64, 49],
                        [i32::MAX, i32::MIN, -16777216],
                        [16777216, 33554431, -33554432],
                    ][case % 3];
                    let volume = DensityVolume::new(
                        [2, height, 2],
                        minimum,
                        [4, if case.is_multiple_of(3) { i32::MAX } else { 1 }, 16],
                    )
                    .unwrap();
                    for fudge in [None, Some(0.125), Some(-0.25), Some(0.0), Some(f64::NAN)] {
                        let initial = [0.0, -0.0, 1.5, f32::from_bits(1)];
                        let mut expected: Vec<_> =
                            (0..volume.size() + 3).map(|i| initial[i % 4]).collect();
                        let (mut a, mut b, mut c) =
                            (expected.clone(), expected.clone(), expected.clone());
                        let amplitude = amplitudes[case % amplitudes.len()];
                        // No Y, every Y, and two random sets.
                        let needed = [
                            0,
                            u64::MAX,
                            random.next_long() as u64,
                            random.next_long() as u64,
                        ];
                        let (s, f) = (scale, fudge);
                        crate::noise::reference::ref_add_to_volume(
                            noise.table(),
                            &mut expected,
                            volume,
                            s,
                            s,
                            amplitude,
                            f,
                        )
                        .unwrap();
                        noise
                            .add_volume::<false>(&mut a, volume, s, s, amplitude, f, None)
                            .unwrap();
                        noise
                            .add_volume::<true>(&mut b, volume, s, s, amplitude, f, None)
                            .unwrap();
                        noise
                            .add_volume::<true>(&mut c, volume, s, s, amplitude, f, Some(&needed))
                            .unwrap();
                        for (i, (((exp, a), b), c)) in
                            expected.iter().zip(&a).zip(&b).zip(&c).enumerate()
                        {
                            let case = (seed, height, scale, fudge, i);
                            assert_eq!(exp.to_bits(), a.to_bits(), "unbatched {case:?}");
                            assert_eq!(exp.to_bits(), b.to_bits(), "batched {case:?}");
                            // Unneeded Ys (and the tail) hold either the sample or the initial value.
                            let (column, y) = (i / height as usize, i % height as usize);
                            let read = i < volume.size()
                                && (height > 64 || (needed[column] >> y) & 1 == 1);
                            if read || c.to_bits() != initial[i % 4].to_bits() {
                                assert_eq!(exp.to_bits(), c.to_bits(), "needed {case:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    /// Volumes wider than `STACK_X` exercise both heap paths (`x_heap` and
    /// `all_same_y`'s `xy_heap`) with non-zero `needed` masks, compared bit-for-bit
    /// against `main`'s verbatim reference implementation.
    #[test]
    fn wide_volume_heap_path_with_nonzero_mask_matches_reference() {
        for seed in [0_i64, -1, 1234, i64::MIN] {
            let perlin = PerlinNoise::new(&mut LegacyRandom::new(seed));
            let smeared_scale = 0.125_f64;
            let smeared = SmearedPerlinNoise::new(&mut LegacyRandom::new(seed), smeared_scale);
            let mut rng = LegacyRandom::new(seed ^ 0x243f_6a88_85a3_08d3_i64);

            for size_x in [STACK_X as i32 + 1, 40, 64] {
                for (height, xz_scale, y_scale) in [
                    (1_i32, 0.25_f64, 0.25_f64),
                    (5, 0.25, 0.0),
                    (17, 0.125, 0.5),
                    (64, 0.25, 0.125),
                    (65, 0.25, 0.25),
                ] {
                    let volume =
                        DensityVolume::new([size_x, height, 2], [-19, -32, 41], [3, 1, 5]).unwrap();
                    let [sx, sy, sz] = volume.sizes();
                    let columns = sx * sz;
                    let mut needed = vec![0_u64; columns];
                    for col in 0..columns {
                        needed[col] = match col % 5 {
                            0 => 0,
                            1 => u64::MAX,
                            2 => 1_u64 << (col % sy.min(64)),
                            3 if sy < 64 => 1_u64 << sy,
                            _ => (rng.next_long() as u64) | 1,
                        };
                    }
                    let initial: Vec<f32> = (0..volume.size() + 5)
                        .map(|i| [0.0, -0.0, 1.25, f32::from_bits(1)][i % 4])
                        .collect();
                    let amp = -0.75_f32;

                    // PerlinNoise
                    let mut exp_p = initial.clone();
                    crate::noise::reference::ref_add_to_volume(
                        perlin.table(),
                        &mut exp_p,
                        volume,
                        xz_scale,
                        y_scale,
                        amp,
                        None,
                    )
                    .unwrap();
                    let mut got_p_full = initial.clone();
                    let mut got_p_masked = initial.clone();
                    perlin
                        .add_to_volume(&mut got_p_full, volume, xz_scale, y_scale, amp)
                        .unwrap();
                    perlin
                        .add_to_needed(&mut got_p_masked, volume, xz_scale, y_scale, amp, &needed)
                        .unwrap();
                    for (i, ((&exp, &full), &masked)) in
                        exp_p.iter().zip(&got_p_full).zip(&got_p_masked).enumerate()
                    {
                        assert_eq!(
                            exp.to_bits(),
                            full.to_bits(),
                            "wide perlin full seed={seed} sx={size_x} sy={height} i={i}"
                        );
                        let read =
                            i < volume.size() && (sy > 64 || (needed[i / sy] >> (i % sy)) & 1 == 1);
                        if read || masked.to_bits() != initial[i].to_bits() {
                            assert_eq!(
                                exp.to_bits(),
                                masked.to_bits(),
                                "wide perlin masked seed={seed} sx={size_x} sy={height} i={i}"
                            );
                        }
                    }

                    // SmearedPerlinNoise
                    let mut exp_s = initial.clone();
                    crate::noise::reference::ref_add_to_volume(
                        smeared.table(),
                        &mut exp_s,
                        volume,
                        xz_scale,
                        y_scale,
                        amp,
                        Some(smeared_scale),
                    )
                    .unwrap();
                    let mut got_s_full = initial.clone();
                    let mut got_s_masked = initial.clone();
                    smeared
                        .add_to_volume(&mut got_s_full, volume, xz_scale, y_scale, amp)
                        .unwrap();
                    smeared
                        .add_to_needed(&mut got_s_masked, volume, xz_scale, y_scale, amp, &needed)
                        .unwrap();
                    for (i, ((&exp, &full), &masked)) in
                        exp_s.iter().zip(&got_s_full).zip(&got_s_masked).enumerate()
                    {
                        assert_eq!(
                            exp.to_bits(),
                            full.to_bits(),
                            "wide smeared full seed={seed} sx={size_x} sy={height} i={i}"
                        );
                        let read =
                            i < volume.size() && (sy > 64 || (needed[i / sy] >> (i % sy)) & 1 == 1);
                        if read || masked.to_bits() != initial[i].to_bits() {
                            assert_eq!(
                                exp.to_bits(),
                                masked.to_bits(),
                                "wide smeared masked seed={seed} sx={size_x} sy={height} i={i}"
                            );
                        }
                    }
                }
            }
        }
    }

    struct NegZeroOffsetRandom {
        step: usize,
        swaps: [i32; 256],
    }

    impl NegZeroOffsetRandom {
        fn new(swaps: [i32; 256]) -> Self {
            Self { step: 0, swaps }
        }
    }

    struct NegZeroPositional;

    impl crate::random::PositionalRandom for NegZeroPositional {
        type Source = NegZeroOffsetRandom;
        fn at(&self, _position: crate::position::BlockPos) -> Self::Source {
            unreachable!()
        }
        fn from_seed(&self, _seed: i64) -> Self::Source {
            unreachable!()
        }
        fn from_hash_of(&self, _name: &str) -> Self::Source {
            unreachable!()
        }
        fn from_hash_of_utf16(&self, _name: &[u16]) -> Self::Source {
            unreachable!()
        }
    }

    impl RandomSource for NegZeroOffsetRandom {
        type Positional = NegZeroPositional;
        fn set_seed(&mut self, _seed: i64) {}
        fn next_int(&mut self) -> i32 {
            0
        }
        fn next_int_bound(&mut self, bound: i32) -> Result<i32, crate::random::InvalidBound> {
            if bound <= 0 {
                return Err(crate::random::InvalidBound);
            }
            let draw = self.swaps[self.step];
            self.step += 1;
            Ok(draw)
        }
        fn next_long(&mut self) -> i64 {
            0
        }
        fn next_boolean(&mut self) -> bool {
            false
        }
        fn next_float(&mut self) -> f32 {
            0.0
        }
        fn next_double(&mut self) -> f64 {
            -0.0
        }
        fn next_gaussian(&mut self) -> f64 {
            0.0
        }
        fn fork_positional(&mut self) -> Self::Positional {
            NegZeroPositional
        }
        fn fork(&mut self) -> Self {
            Self::new(self.swaps)
        }
    }

    /// A custom `RandomSource` returning `-0.0` offsets must preserve the sign
    /// of zero across `get3`, `get2`, and `NoiseStack::get3`.
    #[test]
    fn neg_zero_offset_preserves_zero_sign() {
        // Produces P(1)=6, P(6)=2, P(2)=5, P(5)=1 with all other entries fixed.
        let mut swaps = [0_i32; 256];
        swaps[1] = 5;
        swaps[2] = 3;
        swaps[5] = 1;
        let noise = PerlinNoise::new(&mut NegZeroOffsetRandom::new(swaps));
        assert_eq!(
            noise.table().offsets().map(f64::to_bits),
            [(-0.0_f64).to_bits(); 3]
        );
        assert_eq!(noise.table().permutations()[1], 6);
        assert_eq!(noise.table().permutations()[6], 2);
        assert_eq!(noise.table().permutations()[2], 5);
        assert_eq!(noise.table().permutations()[5], 1);

        // Uses `main`'s verbatim pre-PR `coordinates` + `gradients` + `corner_values` + `lerp3`
        // scalar reference in `noise::reference`, independent of `sample` / `sample_fast`.
        let reference_get3 = |x: f64, y: f64, z: f64| {
            crate::noise::reference::ref_perlin_get3(noise.table(), x, y, z)
        };

        assert_eq!(noise.get3(-0.0, 0.0, -0.0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(noise.get2(-0.0, -0.0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(noise.get3(-0.0, -0.0, -0.0).to_bits(), (-0.0_f32).to_bits());

        let zeros = [0.0_f64, -0.0_f64];
        for x in zeros {
            for z in zeros {
                assert_eq!(
                    noise.get2(x, z).to_bits(),
                    reference_get3(wrap(x), 0.0, wrap(z)).to_bits(),
                    "get2({x:?}, {z:?})"
                );
                assert_eq!(
                    noise.get3_y0(x, z).to_bits(),
                    reference_get3(x, 0.0, z).to_bits(),
                    "get3_y0({x:?}, {z:?})"
                );
                for y in zeros {
                    assert_eq!(
                        noise.get3(x, y, z).to_bits(),
                        reference_get3(x, y, z).to_bits(),
                        "get3({x:?}, {y:?}, {z:?})"
                    );
                }
            }
        }

        let sampler: std::sync::Arc<dyn crate::noise::Noise> = std::sync::Arc::new(noise.clone());
        for freq in [1.0_f64, -1.0_f64] {
            let mut builder = crate::noise::NoiseStack::builder();
            builder.add(std::sync::Arc::clone(&sampler), freq, 1.0);
            let stack = builder.build().unwrap();
            for x in zeros {
                for z in zeros {
                    assert_eq!(
                        crate::noise::Noise::get2(&stack, x, z).to_bits(),
                        (0.0_f32 + 1.0_f32 * reference_get3(wrap(x * freq), 0.0, wrap(z * freq)))
                            .to_bits(),
                        "stack get2({x:?}, {z:?}) freq={freq}"
                    );
                    for y in zeros {
                        assert_eq!(
                            crate::noise::Noise::get3(&stack, x, y, z).to_bits(),
                            (0.0_f32 + 1.0_f32 * reference_get3(x * freq, y * freq, z * freq))
                                .to_bits(),
                            "stack get3({x:?}, {y:?}, {z:?}) freq={freq}"
                        );
                    }
                }
            }
        }
    }

    /// An accepted all-zero needed mask returns immediately without touching the
    /// buffer, while still validating buffer length and preserving fallbacks for
    /// short masks or tall volumes (> 64 Ys).
    #[test]
    fn all_zero_needed_mask_skips_wide_volume_and_preserves_fallbacks() {
        let noise = PerlinNoise::new(&mut LegacyRandom::new(0));
        let smeared = SmearedPerlinNoise::new(&mut LegacyRandom::new(0), 0.125);

        let wide_x = 1_048_576_i32;
        let wide = DensityVolume::new([wide_x, 1, 1], [0; 3], [1; 3]).unwrap();
        let mut needed = vec![0_u64; wide_x as usize + 1];
        // Trailing entry outside the used column range must be ignored.
        needed[wide_x as usize] = u64::MAX;

        // Buffer validation must still reject short buffers before early return.
        let mut short_buf = vec![1.25_f32; 4];
        assert_eq!(
            noise.add_to_needed(&mut short_buf, wide, 0.25, 0.25, 1.0, &needed),
            Err(DensityError::BufferTooShort)
        );
        assert_eq!(
            smeared.add_to_needed(&mut short_buf, wide, 0.25, 0.25, 1.0, &needed),
            Err(DensityError::BufferTooShort)
        );

        let mut wide_buf = vec![1.25_f32; wide.size()];
        noise
            .add_to_needed(&mut wide_buf, wide, 0.25, 0.25, 1.0, &needed)
            .unwrap();
        smeared
            .add_to_needed(&mut wide_buf, wide, 0.25, 0.25, 1.0, &needed)
            .unwrap();
        assert!(wide_buf.iter().all(|v| v.to_bits() == 1.25_f32.to_bits()));

        // High bits above `size_y` select no in-range Y samples.
        let small = DensityVolume::new([2, 3, 2], [-3, 5, 7], [2, 1, 3]).unwrap();
        let high_only = [1_u64 << 3; 4];
        let mut high_buf = vec![2.5_f32; small.size()];
        noise
            .add_to_needed(&mut high_buf, small, 0.25, 0.25, 1.0, &high_only)
            .unwrap();
        assert!(high_buf.iter().all(|v| v.to_bits() == 2.5_f32.to_bits()));

        // A mask shorter than `size_x * size_z` falls back to adding every sample.
        let short_mask = [0_u64; 3];
        let mut expected = vec![0.5_f32; small.size()];
        let mut actual = expected.clone();
        noise
            .add_to_volume(&mut expected, small, 0.25, 0.25, 1.0)
            .unwrap();
        noise
            .add_to_needed(&mut actual, small, 0.25, 0.25, 1.0, &short_mask)
            .unwrap();
        assert_eq!(
            actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );

        // Volumes taller than 64 Ys ignore the mask and add every sample.
        let tall = DensityVolume::new([2, 65, 2], [0; 3], [1; 3]).unwrap();
        let zero_mask = [0_u64; 4];
        let mut tall_expected = vec![0.5_f32; tall.size()];
        let mut tall_actual = tall_expected.clone();
        noise
            .add_to_volume(&mut tall_expected, tall, 0.25, 0.25, 1.0)
            .unwrap();
        noise
            .add_to_needed(&mut tall_actual, tall, 0.25, 0.25, 1.0, &zero_mask)
            .unwrap();
        assert_eq!(
            tall_actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            tall_expected
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        );
    }

    /// Directly compares `get2`, `get3`, `get3_y0`, and `NoiseStack` against
    /// `main`'s verbatim scalar reference across seeds, explicitly hitting
    /// `cell_x = 255` (the `perm[256]` mirror), negative cells, and `|x|` near `2^24`.
    #[test]
    fn scalar_fast_paths_match_reference_at_wrap_and_negative_cells() {
        use crate::noise::{Noise, NoiseStack};
        use std::sync::Arc;

        let half = f64::from_bits(16_777_216.0_f64.to_bits() - 1);
        let target_cells = [-257_i32, -256, -255, -2, -1, 0, 254, 255, 256];

        for seed in [0_i64, -1, 1234, i64::MIN] {
            let noise = PerlinNoise::new(&mut LegacyRandom::new(seed));
            let second = PerlinNoise::new(&mut LegacyRandom::new(seed.wrapping_add(17)));
            let [ox, oy, oz] = *noise.table().offsets();

            let mut builder = NoiseStack::builder();
            builder.add(Arc::new(noise.clone()), 1.0, 0.5).add(
                Arc::new(second.clone()),
                -1.0,
                -0.25,
            );
            let stack = builder.build().unwrap();

            let coord = |target: i32, offset: f64| f64::from(target) + 0.375 - offset;

            let mut xs = Vec::with_capacity(target_cells.len() + 10);
            for cx in target_cells {
                let x = coord(cx, ox);
                assert_eq!(floor_i32(wrap(x) + ox), cx);
                xs.push(x);
            }
            let x_near_pos_255 = 16_776_960.0 + (255.375 - ox);
            let x_near_neg_255 = -16_777_216.0 + (255.375 - ox);
            assert_eq!(floor_i32(wrap(x_near_pos_255) + ox) & 255, 255);
            assert_eq!(floor_i32(wrap(x_near_neg_255) + ox) & 255, 255);
            xs.extend_from_slice(&[
                -16_777_217.25,
                -16_777_216.0,
                -half,
                -(16_777_216.0 - 0.375),
                x_near_neg_255,
                x_near_pos_255,
                16_777_216.0 - 0.375,
                half,
                16_777_216.0,
                16_777_217.25,
            ]);

            let mut zs = Vec::with_capacity(7);
            for cz in [-256_i32, -1, 0, 255, 256] {
                let z = coord(cz, oz);
                assert_eq!(floor_i32(wrap(z) + oz), cz);
                zs.push(z);
            }
            zs.extend_from_slice(&[-half, 16_777_216.0]);

            let ys = [
                0.0_f64,
                -0.0,
                coord(-1, oy),
                coord(0, oy),
                coord(255, oy),
                16_777_216.0,
            ];
            assert_eq!(floor_i32(wrap(ys[2]) + oy), -1);
            assert_eq!(floor_i32(wrap(ys[4]) + oy), 255);

            for &x in &xs {
                for &z in &zs {
                    assert_eq!(
                        noise.get2(x, z).to_bits(),
                        crate::noise::reference::ref_perlin_get2(noise.table(), x, z).to_bits(),
                        "get2 seed={seed} ({x:?}, {z:?})"
                    );
                    assert_eq!(
                        noise.get3_y0(x, z).to_bits(),
                        crate::noise::reference::ref_perlin_get3(noise.table(), x, 0.0, z)
                            .to_bits(),
                        "get3_y0 seed={seed} ({x:?}, {z:?})"
                    );
                    let expected_stack_get2 = (0.0_f32
                        + 0.5_f32
                            * crate::noise::reference::ref_perlin_get2(
                                noise.table(),
                                x * 1.0,
                                z * 1.0,
                            ))
                        + (-0.25_f32)
                            * crate::noise::reference::ref_perlin_get2(second.table(), -x, -z);
                    assert_eq!(
                        stack.get2(x, z).to_bits(),
                        expected_stack_get2.to_bits(),
                        "stack.get2 seed={seed} ({x:?}, {z:?})"
                    );
                    for &y in &ys {
                        assert_eq!(
                            noise.get3(x, y, z).to_bits(),
                            crate::noise::reference::ref_perlin_get3(noise.table(), x, y, z)
                                .to_bits(),
                            "get3 seed={seed} ({x:?}, {y:?}, {z:?})"
                        );
                        let expected_stack_get3 = (0.0_f32
                            + 0.5_f32
                                * crate::noise::reference::ref_perlin_get3(
                                    noise.table(),
                                    x * 1.0,
                                    y * 1.0,
                                    z * 1.0,
                                ))
                            + (-0.25_f32)
                                * crate::noise::reference::ref_perlin_get3(
                                    second.table(),
                                    -x,
                                    -y,
                                    -z,
                                );
                        assert_eq!(
                            stack.get3(x, y, z).to_bits(),
                            expected_stack_get3.to_bits(),
                            "stack.get3 seed={seed} ({x:?}, {y:?}, {z:?})"
                        );
                    }
                }
            }
        }
    }
}
