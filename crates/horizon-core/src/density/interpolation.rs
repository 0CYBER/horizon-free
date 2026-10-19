use super::*;

/// Interpolation over a coarse density grid. Scalar queries and volume queries
/// retain vanilla's distinct floating-point paths and child bulk calls.
#[derive(Clone, Debug)]
pub struct InterpolatedSampler {
    input: Arc<dyn DensitySampler>,
    cell_size: [i32; 3],
    inverse: [f32; 3],
}
impl InterpolatedSampler {
    pub fn new(
        input: Arc<dyn DensitySampler>,
        cell_size_xz: i32,
        cell_size_y: i32,
    ) -> Result<Self, SampleError> {
        if cell_size_xz <= 0 || cell_size_y <= 0 {
            return Err(DensityError::NonpositiveSteps.into());
        }
        let cell_size = [cell_size_xz, cell_size_y, cell_size_xz];
        Ok(Self {
            input,
            cell_size,
            inverse: cell_size.map(|size| 1.0 / size as f32),
        })
    }

    fn sample_block_volume(
        &self,
        context: &mut SamplerContext,
        output: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        let minimum = volume.minimum();
        let maximum = volume.maximum();
        let first: [i32; 3] = std::array::from_fn(|i| minimum[i].div_euclid(self.cell_size[i]));
        let counts: [i32; 3] = std::array::from_fn(|i| {
            maximum[i]
                .div_euclid(self.cell_size[i])
                .wrapping_sub(first[i])
                .wrapping_add(1)
        });
        let sizes = std::array::from_fn(|i| {
            counts[i].wrapping_add(i32::from(maximum[i].rem_euclid(self.cell_size[i]) != 0))
        });
        let coarse = DensityVolume::new(
            sizes,
            std::array::from_fn(|i| first[i].wrapping_mul(self.cell_size[i])),
            self.cell_size,
        )?;
        context.with_buffer(coarse.size(), |context, samples| {
            self.input.sample_volume(context, samples, coarse)?;
            for z in 0..counts[2] {
                for x in 0..counts[0] {
                    for y in 0..counts[1] {
                        let cell = [x as usize, y as usize, z as usize];
                        let corners = std::array::from_fn(|corner| {
                            let position = std::array::from_fn(|axis| {
                                (cell[axis] + ((corner >> axis) & 1)).min(coarse.sizes()[axis] - 1)
                            });
                            samples[coarse.index(position).unwrap()]
                        });
                        self.fill_cell(output, volume, coarse, cell, corners)?;
                    }
                }
            }
            Ok(())
        })
    }

    fn fill_cell(
        &self,
        output: &mut [f32],
        volume: DensityVolume,
        coarse: DensityVolume,
        cell: [usize; 3],
        corners: [f32; 8],
    ) -> Result<(), SampleError> {
        let offset: [i32; 3] = std::array::from_fn(|i| {
            coarse.minimum()[i]
                .wrapping_add((cell[i] as i32).wrapping_mul(self.cell_size[i]))
                .wrapping_sub(volume.minimum()[i])
        });
        let start = offset.map(|offset| offset.wrapping_neg().max(0));
        let end: [i32; 3] = std::array::from_fn(|i| {
            self.cell_size[i]
                .min((volume.sizes()[i] as i32).wrapping_sub(offset[i]))
                .wrapping_sub(1)
        });
        for z in start[2]..=end[2] {
            let alpha_z = z as f32 * self.inverse[2];
            let z00 = lerp(alpha_z, corners[0], corners[4]);
            let z01 = lerp(alpha_z, corners[2], corners[6]);
            let z10 = lerp(alpha_z, corners[1], corners[5]);
            let z11 = lerp(alpha_z, corners[3], corners[7]);
            for x in start[0]..=end[0] {
                let alpha_x = x as f32 * self.inverse[0];
                let low = lerp(alpha_x, z00, z10);
                let high = lerp(alpha_x, z01, z11);
                let increment = (high - low) * self.inverse[1];
                let mut value = low + increment * start[1] as f32;
                if end[1] < start[1] {
                    continue;
                }
                let index = volume
                    .index([
                        offset[0].wrapping_add(x) as usize,
                        offset[1].wrapping_add(start[1]) as usize,
                        offset[2].wrapping_add(z) as usize,
                    ])
                    .ok_or(DensityError::VolumeTooLarge)?;
                let length = (end[1] - start[1] + 1) as usize;
                let end_index = index
                    .checked_add(length)
                    .ok_or(DensityError::VolumeTooLarge)?;
                let column = output
                    .get_mut(index..end_index)
                    .ok_or(DensityError::VolumeTooLarge)?;
                for output in column {
                    *output = value;
                    // Repeated addition is observable: multiplication by the
                    // current Y offset does not have the same rounding.
                    value += increment;
                }
            }
        }
        Ok(())
    }
}

fn lerp(alpha: f32, first: f32, second: f32) -> f32 {
    first + alpha * (second - first)
}
impl DensitySampler for InterpolatedSampler {
    fn sample_value(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<f32, SampleError> {
        let remainder: [i32; 3] =
            std::array::from_fn(|i| position[i].rem_euclid(self.cell_size[i]));
        if remainder == [0; 3] {
            return self.input.sample_value(context, position);
        }
        let volume = DensityVolume::new(
            [2; 3],
            std::array::from_fn(|i| position[i].wrapping_sub(remainder[i])),
            self.cell_size,
        )?;
        context.with_buffer(8, |context, corners| {
            self.input.sample_volume(context, corners, volume)?;
            let alpha: [f32; 3] =
                std::array::from_fn(|i| remainder[i] as f32 / self.cell_size[i] as f32);
            // Samples use Y-innermost storage; interpolation blends X, Y, Z.
            let xy0 = lerp(
                alpha[1],
                lerp(alpha[0], corners[0], corners[2]),
                lerp(alpha[0], corners[1], corners[3]),
            );
            let xy1 = lerp(
                alpha[1],
                lerp(alpha[0], corners[4], corners[6]),
                lerp(alpha[0], corners[5], corners[7]),
            );
            Ok(lerp(alpha[2], xy0, xy1))
        })
    }
    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        output: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        check_buffer(output, volume)?;
        if context.sparse {
            if (0..3).all(|i| {
                volume.steps()[i] % self.cell_size[i] == 0
                    && volume.minimum()[i].rem_euclid(self.cell_size[i]) == 0
            }) {
                return self.input.sample_volume(context, output, volume);
            }
            return visit(volume, |index, position| {
                output[index] = self.sample_value(context, position)?;
                Ok(())
            });
        }
        if (0..3).all(|i| {
            (volume.steps()[i] == self.cell_size[i] || volume.sizes()[i] == 1)
                && volume.minimum()[i].rem_euclid(self.cell_size[i]) == 0
        }) {
            return self.input.sample_volume(context, output, volume);
        }
        if volume.steps() == [1; 3] {
            return self.sample_block_volume(context, output, volume);
        }
        // The reference evaluates the complete block-resolution bounding volume
        // before selecting strided samples, including trailing unsampled blocks.
        let dense = DensityVolume::new(
            std::array::from_fn(|i| (volume.sizes()[i] as i32).wrapping_mul(volume.steps()[i])),
            volume.minimum(),
            [1; 3],
        )?;
        context.with_buffer(dense.size(), |context, samples| {
            self.sample_block_volume(context, samples, dense)?;
            let [sx, sy, sz] = volume.sizes();
            for z in 0..sz {
                for x in 0..sx {
                    for y in 0..sy {
                        let source = dense.index(std::array::from_fn(|i| {
                            [x, y, z][i] * volume.steps()[i] as usize
                        }));
                        output[(x + z * sx) * sy + y] = samples[source.unwrap()];
                    }
                }
            }
            Ok(())
        })
    }
}
