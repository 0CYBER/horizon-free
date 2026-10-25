use super::{
    DensityError, DensityVolume, GradientTable, dot, lerp2, lerp3, smoothstep,
    smoothstep_derivative, wrap,
};
use crate::{geometry::floor_i32, random::RandomSource};

#[derive(Clone, Debug)]
pub struct PerlinNoise {
    table: GradientTable,
}

impl PerlinNoise {
    pub fn new(random: &mut impl RandomSource) -> Self {
        Self {
            table: GradientTable::new(random, 256.0),
        }
    }
    pub fn table(&self) -> &GradientTable {
        &self.table
    }
    pub fn range(&self) -> [f32; 2] {
        [-2.0, 2.0]
    }
    pub fn get2(&self, x: f64, z: f64) -> f32 {
        self.get3(wrap(x), 0.0, wrap(z))
    }
    pub fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        let (cell, relative) = self.coordinates([x, y, z]);
        let point = relative.map(|v| v as f32);
        self.sample(cell, point, point[1])
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
        self.gradients_at(
            self.table.permute(cell[0]),
            self.table.permute(cell[0].wrapping_add(1)),
            cell[1],
            cell[2],
        )
    }

    /// Corner gradients for a Y cell, with the X column's permutation already
    /// resolved. `add_volume` reuses one X column across every Y, so hoisting
    /// the two X permutes out of that loop avoids repeating them per Y.
    #[inline]
    fn gradients_at(&self, p: i32, q: i32, cell_y: i32, cell_z: i32) -> [[f32; 3]; 8] {
        let lower = self.table.permute(p.wrapping_add(cell_y));
        let upper = self.table.permute(p.wrapping_add(cell_y).wrapping_add(1));
        let lower_x = self.table.permute(q.wrapping_add(cell_y));
        let upper_x = self.table.permute(q.wrapping_add(cell_y).wrapping_add(1));
        let rows = [
            lower, lower_x, upper, upper_x, lower, lower_x, upper, upper_x,
        ];
        std::array::from_fn(|i| {
            self.table
                .gradient(rows[i].wrapping_add(cell_z).wrapping_add((i / 4) as i32))
        })
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
        let values = Self::corner_values(self.gradients(cell), point);
        lerp3(
            [
                smoothstep(point[0]),
                smoothstep(interpolation_y),
                smoothstep(point[2]),
            ],
            values,
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
        self.add_volume::<true>(buffer, volume, xz_scale, y_scale, amplitude, None)
    }

    /// `BATCH` = false evaluates one Y at a time: the reference the batched loop is tested against.
    fn add_volume<const BATCH: bool>(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
        fudge_y_scale: Option<f64>,
    ) -> Result<(), DensityError> {
        if buffer.len() < volume.size() {
            return Err(DensityError::BufferTooShort);
        }
        let [size_x, size_y, size_z] = volume.sizes();
        // Every X/Z column walks the same Y coordinates, so their cells, offsets
        // and fades are computed once per volume instead of once per column.
        let mut stack = [YSample::default(); STACK_Y];
        let mut heap = Vec::new();
        let ys = if size_y <= STACK_Y {
            &mut stack[..size_y]
        } else {
            heap.resize(size_y, YSample::default());
            &mut heap[..]
        };
        for (y, sample) in ys.iter_mut().enumerate() {
            let input_y = f64::from(volume.coordinate(1, y)) * y_scale;
            let shifted_y = wrap(input_y) + self.table.offsets[1];
            let cell = floor_i32(shifted_y);
            let dy = shifted_y - f64::from(cell);
            *sample = YSample {
                cell,
                fade: smoothstep(dy as f32),
                offset: match fudge_y_scale {
                    Some(scale) => (dy - compute_fudge_y(input_y, dy, scale)) as f32,
                    None => dy as f32,
                },
            };
        }
        let ys = &*ys;
        let mut index = 0;
        for z in 0..size_z {
            let shifted_z =
                wrap(f64::from(volume.coordinate(2, z)) * xz_scale) + self.table.offsets[2];
            let cell_z = floor_i32(shifted_z);
            let dz = (shifted_z - f64::from(cell_z)) as f32;
            let fade_z = smoothstep(dz);
            for x in 0..size_x {
                let shifted_x =
                    wrap(f64::from(volume.coordinate(0, x)) * xz_scale) + self.table.offsets[0];
                let cell_x = floor_i32(shifted_x);
                let dx = (shifted_x - f64::from(cell_x)) as f32;
                let fade_x = smoothstep(dx);
                let p = self.table.permute(cell_x);
                let q = self.table.permute(cell_x.wrapping_add(1));
                let mut previous_y = i32::MIN;
                let mut xz_dots = [0.0_f32; 8];
                let mut y_gradients = [0.0_f32; 8];
                let column = &mut buffer[index..index + size_y];
                let mut y = 0;
                while y < size_y {
                    let sample = &ys[y];
                    if previous_y != sample.cell {
                        let gradients = self.gradients_at(p, q, sample.cell, cell_z);
                        for i in 0..8 {
                            xz_dots[i] = gradients[i][0] * (dx - (i & 1) as f32)
                                + gradients[i][2] * (dz - ((i >> 2) & 1) as f32);
                            y_gradients[i] = gradients[i][1];
                        }
                        previous_y = sample.cell;
                    }
                    // Four Ys in one cell share the corners: side by side, LLVM computes them
                    // as vectors. Cell boundaries and the column's tail go one at a time.
                    if BATCH
                        && y + 4 <= size_y
                        && ys[y + 1..y + 4].iter().all(|s| s.cell == sample.cell)
                    {
                        add_four(
                            &mut column[y..y + 4],
                            &ys[y..y + 4],
                            xz_dots,
                            y_gradients,
                            fade_x,
                            fade_z,
                            amplitude,
                        );
                        y += 4;
                        continue;
                    }
                    let out = &mut column[y];
                    let dy = sample.offset;
                    let values = std::array::from_fn(|i| {
                        xz_dots[i] + y_gradients[i] * (dy - ((i >> 1) & 1) as f32)
                    });
                    *out += amplitude * lerp3([fade_x, sample.fade, fade_z], values);
                    y += 1;
                }
                index += size_y;
            }
        }
        Ok(())
    }
}

/// Y columns up to this height keep their per-Y samples on the stack.
const STACK_Y: usize = 64;

#[derive(Clone, Copy, Default)]
struct YSample {
    cell: i32,
    fade: f32,
    /// The Y offset inside the cell used for the corner values, after any fudge.
    offset: f32,
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

/// Adds four Y samples of one cell, laid out lane by lane so LLVM vectorizes them (no intrinsics,
/// no target feature). Each lane does the one-at-a-time loop's operations in the same order.
#[inline]
fn add_four(
    out: &mut [f32],
    ys: &[YSample],
    dots: [f32; 8],
    gradients: [f32; 8],
    fade_x: f32,
    fade_z: f32,
    amplitude: f32,
) {
    let dy: [f32; 4] = std::array::from_fn(|lane| ys[lane].offset);
    let fy: [f32; 4] = std::array::from_fn(|lane| ys[lane].fade);
    let values: [[f32; 4]; 8] = std::array::from_fn(|i| {
        std::array::from_fn(|lane| dots[i] + gradients[i] * (dy[lane] - ((i >> 1) & 1) as f32))
    });
    for lane in 0..4 {
        out[lane] += amplitude * lerp3([fade_x, fy[lane], fade_z], values.map(|v| v[lane]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::LegacyRandom;

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
                        [4, if case % 3 == 0 { i32::MAX } else { 1 }, 16],
                    )
                    .unwrap();
                    for fudge in [None, Some(0.125), Some(-0.25), Some(0.0), Some(f64::NAN)] {
                        let initial = [0.0, -0.0, 1.5, f32::from_bits(1)];
                        let mut a: Vec<_> =
                            (0..volume.size() + 3).map(|i| initial[i % 4]).collect();
                        let mut b = a.clone();
                        let amplitude = amplitudes[case % amplitudes.len()];
                        noise
                            .add_volume::<false>(&mut a, volume, scale, scale, amplitude, fudge)
                            .unwrap();
                        noise
                            .add_volume::<true>(&mut b, volume, scale, scale, amplitude, fudge)
                            .unwrap();
                        for (i, (a, b)) in a.iter().zip(&b).enumerate() {
                            assert_eq!(
                                a.to_bits(),
                                b.to_bits(),
                                "seed={seed} height={height} scale={scale} fudge={fudge:?} index={i}"
                            );
                        }
                    }
                }
            }
        }
    }
}
