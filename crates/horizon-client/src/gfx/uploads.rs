//! Square RGBA8 texture regions packed into one upload buffer. A tick's
//! animated sprites then cost one staging allocation and one submission
//! instead of a `Queue::write_texture` (and its own staging buffer) per sprite
//! and mip level, which made every tick's frame slower than the frames between
//! ticks.

/// One queued region: `edge` x `edge` texels of array `layer`, mip `level`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Region {
    offset: u64,
    bytes_per_row: u32,
    layer: u32,
    level: u32,
    edge: u32,
}

#[derive(Default)]
pub(super) struct TextureUploads {
    /// The queued texels, each row padded to `COPY_BYTES_PER_ROW_ALIGNMENT`.
    data: Vec<u8>,
    regions: Vec<Region>,
    /// Kept between flushes and grown when a flush needs more room.
    buffer: Option<wgpu::Buffer>,
}

impl TextureUploads {
    /// Queue `pixels` (tightly packed RGBA8 rows) for `layer` at mip `level`.
    pub(super) fn push(&mut self, layer: u32, level: u32, edge: u32, pixels: &[u8]) {
        let row = edge as usize * 4;
        debug_assert_eq!(pixels.len(), row * edge as usize);
        let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
        let offset = self.data.len();
        self.data.reserve(padded * edge as usize);
        for source in pixels.chunks_exact(row) {
            self.data.extend_from_slice(source);
            self.data.resize(self.data.len() + padded - row, 0);
        }
        self.regions.push(Region {
            offset: offset as u64,
            bytes_per_row: padded as u32,
            layer,
            level,
            edge,
        });
    }

    /// Copy every queued region into `texture`, in the order they were queued,
    /// and empty the queue.
    pub(super) fn flush(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
    ) {
        if self.regions.is_empty() {
            return;
        }
        let size = self.data.len() as u64;
        if self.buffer.as_ref().is_none_or(|b| b.size() < size) {
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Texture uploads"),
                size: size.next_power_of_two(),
                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let buffer = self.buffer.as_ref().expect("created above");
        // Queue writes land before the next submission's commands, and
        // submissions run in order, so the previous flush's copies have read
        // the buffer before this write replaces it.
        queue.write_buffer(buffer, 0, &self.data);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Texture uploads"),
        });
        for r in self.regions.drain(..) {
            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: r.offset,
                        bytes_per_row: Some(r.bytes_per_row),
                        rows_per_image: Some(r.edge),
                    },
                },
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: r.level,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: r.layer,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: r.edge,
                    height: r.edge,
                    depth_or_array_layers: 1,
                },
            );
        }
        queue.submit([encoder.finish()]);
        self.data.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_padded_and_regions_follow_each_other() {
        let mut uploads = TextureUploads::default();
        let big: Vec<u8> = (0..16 * 16 * 4).map(|i| i as u8).collect();
        let small: Vec<u8> = (0..2 * 2 * 4).map(|i| 200 + i as u8).collect();
        uploads.push(3, 0, 16, &big);
        uploads.push(3, 3, 2, &small);
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        assert_eq!(
            uploads.regions,
            [
                Region {
                    offset: 0,
                    bytes_per_row: align as u32,
                    layer: 3,
                    level: 0,
                    edge: 16,
                },
                Region {
                    offset: (16 * align) as u64,
                    bytes_per_row: align as u32,
                    layer: 3,
                    level: 3,
                    edge: 2,
                },
            ]
        );
        assert_eq!(uploads.data.len(), 18 * align);
        // Every row starts on the alignment and holds the source row.
        for y in 0..16 {
            assert_eq!(
                &uploads.data[y * align..y * align + 64],
                &big[y * 64..(y + 1) * 64]
            );
        }
        for y in 0..2 {
            let start = (16 + y) * align;
            assert_eq!(&uploads.data[start..start + 8], &small[y * 8..(y + 1) * 8]);
        }
    }

    /// Every texel of a 64-pixel array texture with a full mip chain.
    fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut buffers = Vec::new();
        for level in 0..texture.mip_level_count() {
            let edge = texture.width() >> level;
            let row = (edge * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
            let layers = texture.depth_or_array_layers();
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(row * edge * layers),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(edge),
                    },
                },
                wgpu::Extent3d {
                    width: edge,
                    height: edge,
                    depth_or_array_layers: layers,
                },
            );
            buffers.push(buffer);
        }
        queue.submit([encoder.finish()]);
        let mut out = Vec::new();
        for buffer in &buffers {
            buffer.map_async(wgpu::MapMode::Read, .., |r| r.expect("map"));
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll");
            out.extend_from_slice(&buffer.get_mapped_range(..).expect("mapped"));
        }
        out
    }

    /// The batched copies leave a texture exactly as one `write_texture` per
    /// region does, over several flushes reusing (and growing) the buffer.
    #[test]
    fn batched_uploads_match_write_texture_on_a_gpu() {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("device");
        let texture = || {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 4,
                },
                mip_level_count: 7,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let (expected, batched) = (texture(), texture());
        let mut uploads = TextureUploads::default();
        let mut seed = 1u32;
        // Small flushes first, then whole chains: the buffer has to grow.
        for (round, layers) in [[1u32, 2].as_slice(), &[0, 1, 2, 3], &[3]]
            .into_iter()
            .enumerate()
        {
            for &layer in layers {
                let levels = if round == 0 { 0..3 } else { 0..7 };
                for level in levels {
                    let edge = 64 >> level;
                    let pixels: Vec<u8> = (0..edge * edge * 4)
                        .map(|_| {
                            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                            (seed >> 24) as u8
                        })
                        .collect();
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &expected,
                            mip_level: level,
                            origin: wgpu::Origin3d {
                                x: 0,
                                y: 0,
                                z: layer,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        &pixels,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(edge * 4),
                            rows_per_image: Some(edge),
                        },
                        wgpu::Extent3d {
                            width: edge,
                            height: edge,
                            depth_or_array_layers: 1,
                        },
                    );
                    uploads.push(layer, level, edge, &pixels);
                }
            }
            uploads.flush(&device, &queue, &batched);
            assert!(uploads.regions.is_empty() && uploads.data.is_empty());
            assert_eq!(
                read_back(&device, &queue, &expected),
                read_back(&device, &queue, &batched),
                "round {round}"
            );
        }
    }
}
