//! Per-frame uploads through one reused staging belt. `Queue::write_buffer`
//! and `Queue::write_texture` create a fresh staging buffer for every call (on
//! Metal a kernel call each), which made the animated sprites' few hundred
//! texture writes a spike on every tick that changes a frame. The belt keeps a
//! few mapped buffers and records the copies into the frame's encoder instead.
//!
//! Large textures made with their texels ([`create_texture_with_data`]) are
//! staged a band at a time: wgpu's `create_texture_with_data` stages every
//! byte at once and keeps the copy until a submission carrying it finishes, so
//! the shared atlas (112 MiB at 4096x7168) stood twice in memory while the
//! title artwork loaded, and the world renderer staged it again beside the
//! block sprites (#15608). Phones share that memory with the game.
use std::num::NonZeroU64;

/// Large enough for a tick of animated sprites at 16x16 with every mip level.
const CHUNK: u64 = 1 << 20;

pub struct Uploads {
    belt: wgpu::util::StagingBelt,
    chunk: u64,
}

impl Uploads {
    pub fn new(device: &wgpu::Device) -> Self {
        Self::with_chunk(device, CHUNK)
    }
    /// A belt whose staging buffers hold `chunk` bytes (a larger write gets
    /// a buffer of its own, rounded up to a power of two).
    pub fn with_chunk(device: &wgpu::Device, chunk: u64) -> Self {
        Self {
            belt: wgpu::util::StagingBelt::new(device.clone(), chunk),
            chunk,
        }
    }
    /// A mapped staging slice of at least `size` bytes. The belt gives a
    /// write larger than its chunk a buffer of exactly that size, keeps it for
    /// good and only reuses it for writes that fit: an upload that grows a
    /// little every frame (the entity batch while mobs appear) took a new
    /// buffer each frame until the game ran out of memory (#2826). Rounded up
    /// to a power of two, the large buffers it keeps are reused, one per
    /// doubling.
    fn allocate(&mut self, size: NonZeroU64, alignment: NonZeroU64) -> wgpu::BufferSlice<'_> {
        let size = if size.get() > self.chunk {
            size.checked_next_power_of_two().unwrap_or(size)
        } else {
            size
        };
        self.belt.allocate(size, alignment)
    }
    /// Copy tightly packed RGBA8 `pixels` (`width` by `height`) into one mip
    /// level of one array layer, padding rows to the copy alignment.
    pub fn texture(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        destination: wgpu::TexelCopyTextureInfo<'_>,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) {
        let row = width as usize * 4;
        debug_assert_eq!(pixels.len(), row * height as usize);
        let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
        let Some(size) = NonZeroU64::new((padded * height as usize) as u64) else {
            return;
        };
        let alignment = NonZeroU64::new(u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT))
            .expect("non-zero alignment");
        let slice = self.allocate(size, alignment);
        {
            let mut view = slice
                .get_mapped_range_mut()
                .expect("staging belt slices are mapped");
            for (i, src) in pixels.chunks(row).enumerate() {
                view.slice(i * padded..i * padded + row)
                    .copy_from_slice(src);
            }
        }
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: slice.buffer(),
                layout: wgpu::TexelCopyBufferLayout {
                    offset: slice.offset(),
                    bytes_per_row: Some(padded as u32),
                    rows_per_image: Some(height),
                },
            },
            destination,
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
    /// Copy `bytes` into `target` at `offset`, as `Queue::write_buffer`
    /// would, in the encoder's order.
    pub fn buffer(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::Buffer,
        offset: u64,
        bytes: &[u8],
    ) {
        let Some(size) = NonZeroU64::new(bytes.len() as u64) else {
            return;
        };
        // The belt's own `write_buffer` checks, kept now that a large write's
        // slice is rounded up past them.
        assert!(
            offset.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)
                && size.get().is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT),
            "buffer upload at {offset} of {size} bytes must be 4-byte aligned"
        );
        let alignment = NonZeroU64::new(wgpu::COPY_BUFFER_ALIGNMENT).expect("non-zero alignment");
        let slice = self.allocate(size, alignment).slice(..size.get());
        encoder.copy_buffer_to_buffer(slice.buffer(), slice.offset(), target, offset, size.get());
        slice
            .get_mapped_range_mut()
            .expect("staging belt slices are mapped")
            .slice(..)
            .copy_from_slice(bytes);
    }
    /// Close this frame's staging buffers: they come back for reuse once the
    /// encoder's submission finishes on the GPU.
    pub fn finish(&mut self, encoder: &wgpu::CommandEncoder) {
        self.belt.finish_and_recall_on_submit(encoder);
    }
}

/// About how much one band stages; a large upload holds at most about two.
const BAND: usize = 8 << 20;

/// `device.create_texture_with_data(queue, desc, LayerMajor, data)`, staged a
/// band at a time: each band is submitted, and the one before waited for, so
/// its staging buffer is freed before the next is written.
pub(crate) fn create_texture_with_data(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    desc: &wgpu::TextureDescriptor<'_>,
    data: &[u8],
) -> wgpu::Texture {
    create_banded(device, queue, desc, data, BAND)
}

fn create_banded(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    desc: &wgpu::TextureDescriptor<'_>,
    data: &[u8],
    band: usize,
) -> wgpu::Texture {
    use wgpu::util::DeviceExt as _;
    let block = desc.format.block_copy_size(None).unwrap_or(0) as usize;
    if data.len() <= band
        || block == 0
        || desc.format.block_dimensions() != (1, 1)
        || desc.dimension != wgpu::TextureDimension::D2
    {
        return device.create_texture_with_data(
            queue,
            desc,
            wgpu::util::TextureDataOrder::LayerMajor,
            data,
        );
    }
    let mut desc = desc.clone();
    desc.usage |= wgpu::TextureUsages::COPY_DST;
    let texture = device.create_texture(&desc);
    let (mut offset, mut staged, mut previous) = (0, 0, None);
    // The order `create_texture_with_data` reads `LayerMajor` data in.
    for layer in 0..desc.array_layer_count() {
        for mip in 0..desc.mip_level_count {
            let Some(size) = desc.mip_level_size(mip) else {
                continue;
            };
            let row = size.width as usize * block;
            let rows_per_band = (band / row).clamp(1, size.height as usize) as u32;
            let mut y = 0;
            while y < size.height {
                let rows = rows_per_band.min(size.height - y);
                let bytes = row * rows as usize;
                let Some(texels) = data.get(offset..offset + bytes) else {
                    return texture;
                };
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: mip,
                        origin: wgpu::Origin3d { x: 0, y, z: layer },
                        aspect: wgpu::TextureAspect::All,
                    },
                    texels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row as u32),
                        rows_per_image: Some(rows),
                    },
                    wgpu::Extent3d {
                        width: size.width,
                        height: rows,
                        depth_or_array_layers: 1,
                    },
                );
                (offset, staged, y) = (offset + bytes, staged + bytes, y + rows);
                if staged >= band {
                    let index = queue.submit([]);
                    if let Some(earlier) = previous.replace(index) {
                        let _ = device.poll(wgpu::PollType::Wait {
                            submission_index: Some(earlier),
                            timeout: None,
                        });
                    }
                    staged = 0;
                }
            }
        }
    }
    texture
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    /// Every mip level of one layer, read back as tightly packed RGBA8.
    fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
        let mut out = Vec::new();
        for level in 0..texture.mip_level_count() {
            let edge = texture.width() >> level;
            let padded = (edge * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(padded * edge),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: level,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: 1 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded),
                        rows_per_image: Some(edge),
                    },
                },
                wgpu::Extent3d {
                    width: edge,
                    height: edge,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let data = buffer.slice(..).get_mapped_range().unwrap();
            for row in data.chunks(padded as usize) {
                out.extend_from_slice(&row[..edge as usize * 4]);
            }
        }
        out
    }

    #[test]
    fn belt_uploads_match_queue_writes_on_every_mip_level() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let texture = || {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 3,
                },
                mip_level_count: 5,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let (expected, actual) = (texture(), texture());
        let mut uploads = Uploads::new(&device);
        // Two frames, so the second reuses the belt's recalled buffers.
        for frame in 0..2u8 {
            let mut encoder = device.create_command_encoder(&Default::default());
            for level in 0..5 {
                let edge = 16 >> level;
                let pixels: Vec<u8> = (0..edge * edge * 4)
                    .map(|i| {
                        (i as u8)
                            .wrapping_mul(31)
                            .wrapping_add(level as u8 * 7 + frame)
                    })
                    .collect();
                let at = |texture| wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: level,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: 1 },
                    aspect: wgpu::TextureAspect::All,
                };
                queue.write_texture(
                    at(&expected),
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
                uploads.texture(&mut encoder, at(&actual), edge, edge, &pixels);
            }
            uploads.finish(&encoder);
            queue.submit([encoder.finish()]);
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            assert_eq!(
                read_back(&device, &queue, &actual),
                read_back(&device, &queue, &expected),
                "frame {frame}"
            );
        }
    }

    /// Staging buffers the belt holds, once every submitted frame is done
    /// with them. The belt only reports its buffers through `Debug`.
    fn kept_buffers(device: &wgpu::Device, uploads: &mut Uploads) -> usize {
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        // Finishing an empty frame takes back every recalled buffer.
        uploads.finish(&device.create_command_encoder(&Default::default()));
        let belt = format!("{:?}", uploads.belt);
        let count = |field: &str| -> usize {
            let at = belt.find(&format!("{field}: ")).expect(&belt) + field.len() + 2;
            let digits = belt[at..].split(|c: char| !c.is_ascii_digit()).next();
            digits.and_then(|d| d.parse().ok()).expect(&belt)
        };
        count("active_chunks") + count("closed_chunks") + count("free_chunks")
    }

    /// #2826: a write larger than the chunk that grows a little every frame
    /// (the entity batch while mobs appear) got a new exact-size buffer each
    /// frame, all of them kept, until the game ran out of memory.
    #[test]
    fn growing_large_writes_reuse_the_belt_buffers() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        const CHUNK: u64 = 1 << 16;
        const TARGET: usize = 4 * CHUNK as usize;
        let target = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: TARGET as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: TARGET as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut uploads = Uploads::with_chunk(&device, CHUNK);
        let mut expected = vec![0u8; TARGET];
        let frames = 64;
        for frame in 0..frames {
            let mut encoder = device.create_command_encoder(&Default::default());
            // One small write, then the large one, 1.5 to 3.5 chunks long.
            uploads.buffer(&mut encoder, &target, TARGET as u64 - 4, &[9; 4]);
            expected[TARGET - 4..].fill(9);
            let len = (3 * CHUNK as usize / 2 + frame * 2048).next_multiple_of(4);
            let bytes: Vec<u8> = (0..len).map(|i| (i + frame) as u8).collect();
            uploads.buffer(&mut encoder, &target, 0, &bytes);
            expected[..len].copy_from_slice(&bytes);
            encoder.copy_buffer_to_buffer(&target, 0, &readback, 0, TARGET as u64);
            uploads.finish(&encoder);
            queue.submit([encoder.finish()]);
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            // Only the written bytes land: the rest of the target is untouched.
            assert!(
                *readback.slice(..).get_mapped_range().unwrap() == expected[..],
                "frame {frame}"
            );
            readback.unmap();
        }
        // A chunk for the small writes and a buffer per doubling of the large
        // one (2 and 4 chunks), maybe one more where a small write took the
        // start of a buffer the large one needed whole: not one per frame.
        let kept = kept_buffers(&device, &mut uploads);
        assert!(
            kept <= 4,
            "the belt kept {kept} buffers over {frames} frames"
        );
    }

    #[test]
    fn belt_buffer_writes_land_in_order_at_their_offsets() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let target = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut uploads = Uploads::new(&device);
        let mut expected = [0u8; 64];
        for frame in 0..3u8 {
            let mut encoder = device.create_command_encoder(&Default::default());
            // Overlapping writes: the later one wins, as with the queue.
            for (offset, len, value) in [(0, 32, frame + 1), (16, 32, frame + 10), (60, 4, 7)] {
                let bytes = vec![value; len];
                uploads.buffer(&mut encoder, &target, offset as u64, &bytes);
                expected[offset..offset + len].copy_from_slice(&bytes);
            }
            uploads.buffer(&mut encoder, &target, 0, &[]);
            encoder.copy_buffer_to_buffer(&target, 0, &readback, 0, 64);
            uploads.finish(&encoder);
            queue.submit([encoder.finish()]);
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            assert_eq!(
                *readback.slice(..).get_mapped_range().unwrap(),
                expected[..]
            );
            readback.unmap();
        }
    }

    /// Banded uploads, by rows of a layer and by layers of mips, put every
    /// texel where wgpu's one-shot upload does.
    #[test]
    fn banded_uploads_match_one_upload() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        for (edge, mips) in [(64, 1), (16, 5)] {
            let desc = wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: edge,
                    height: edge,
                    depth_or_array_layers: 3,
                },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            };
            let len = (0..mips)
                .map(|l| (edge >> l) * (edge >> l) * 4)
                .sum::<u32>()
                * 3;
            let data: Vec<u8> = (0..len).map(|i| (i * 7 + i / 251) as u8).collect();
            // Three rows of the largest level a band: many submissions, each waited for.
            let banded = create_banded(&device, &queue, &desc, &data, 3 * edge as usize * 4);
            let whole = wgpu::util::DeviceExt::create_texture_with_data(
                &device,
                &queue,
                &desc,
                wgpu::util::TextureDataOrder::LayerMajor,
                &data,
            );
            assert_eq!(
                read_back(&device, &queue, &banded),
                read_back(&device, &queue, &whole),
                "{edge} px, {mips} mips"
            );
        }
    }
}
