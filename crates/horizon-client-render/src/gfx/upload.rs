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
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

/// Large enough for a tick of animated sprites at 16x16 with every mip level.
const CHUNK: u64 = 1 << 20;
/// Regular chunks kept warm so steady-state frames never reallocate.
const MAX_IDLE_REGULAR_CHUNKS: usize = 2;
/// Finishes an unused oversized staging chunk waits before it is released.
const IDLE_OVERSIZED_FINISHES: u64 = 120;
/// Finishes an excess regular staging chunk waits before it is released.
const IDLE_REGULAR_FINISHES: u64 = 240;
/// Wall-clock idle time after which stale chunks are also eligible for release
/// (covers `section_uploads`, whose `finish` only runs when terrain changes).
const IDLE_WALL_TIME: Duration = Duration::from_secs(5);
/// Maximum chunks released in one `finish` call so post-burst reclamation stays smooth.
const MAX_TRIM_PER_FINISH: usize = 2;
/// D3D12 needs `D3D12_TEXTURE_DATA_PLACEMENT_ALIGNMENT` (512) for the start of a
/// buffer-to-texture copy when the driver cannot take an unrestricted row pitch.
/// The row pitch stays at `COPY_BYTES_PER_ROW_ALIGNMENT` (256); an offset below
/// 512 makes wgpu-hal stage the copy through an intermediate buffer (#20012).
const TEXTURE_COPY_OFFSET_ALIGNMENT: u64 = 512;

struct Chunk {
    buffer: wgpu::Buffer,
    offset: u64,
    last_used_epoch: u64,
    last_used_at: Instant,
}

impl Chunk {
    fn can_allocate(&self, size: u64, alignment: u64) -> bool {
        let Some(start) = self.offset.checked_next_multiple_of(alignment) else {
            return false;
        };
        start
            .checked_add(size)
            .is_some_and(|end| end <= self.buffer.size())
    }

    fn allocate(&mut self, size: u64, alignment: u64) -> u64 {
        let start = self.offset.next_multiple_of(alignment);
        let end = start + size;
        debug_assert!(end <= self.buffer.size());
        self.offset = end;
        start
    }
}

pub struct Uploads {
    device: wgpu::Device,
    chunk: u64,
    active_chunks: Vec<Chunk>,
    free_chunks: Vec<Chunk>,
    sender: Mutex<mpsc::Sender<Chunk>>,
    receiver: Mutex<mpsc::Receiver<Chunk>>,
    epoch: u64,
    last_finish_at: Instant,
}

impl Uploads {
    pub fn new(device: &wgpu::Device) -> Self {
        Self::with_chunk(device, CHUNK)
    }
    /// A staging pool whose regular buffers hold `chunk` bytes (a larger write
    /// gets a buffer rounded up to a power of two, and idle oversized/burst
    /// buffers are trimmed after they go unused).
    pub fn with_chunk(device: &wgpu::Device, chunk: u64) -> Self {
        let chunk = chunk
            .max(wgpu::MAP_ALIGNMENT)
            .next_multiple_of(wgpu::MAP_ALIGNMENT);
        let (sender, receiver) = mpsc::channel();
        Self {
            device: device.clone(),
            chunk,
            active_chunks: Vec::new(),
            free_chunks: Vec::new(),
            sender: Mutex::new(sender),
            receiver: Mutex::new(receiver),
            epoch: 0,
            last_finish_at: Instant::now(),
        }
    }
    /// Power-of-two capacity for an oversized write, or `self.chunk` for regular writes.
    /// Rounding oversized buffers prevents #2826 (one permanent buffer per frame while a
    /// large upload grows slightly each frame) while still advancing `offset` only by the
    /// actual write size so the tail stays usable for later writes in the same frame.
    fn bucket_size(&self, size: u64) -> u64 {
        if size > self.chunk {
            size.checked_next_power_of_two().unwrap_or(size)
        } else {
            self.chunk
        }
    }
    /// Smallest active chunk that can hold `size` at `alignment`.
    fn pick_active(&self, size: u64, alignment: u64) -> Option<usize> {
        self.active_chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| c.can_allocate(size, alignment))
            .min_by_key(|(_, c)| c.buffer.size())
            .map(|(i, _)| i)
    }
    /// Smallest free chunk matching `size`'s tier: regular writes never pull an
    /// oversized burst chunk (keeping it free for large writes and letting it go
    /// idle for trimming), and oversized writes never keep a much larger burst
    /// chunk alive indefinitely.
    fn pick_free(&self, size: u64, alignment: u64) -> Option<usize> {
        let max_size = if size <= self.chunk {
            self.chunk
        } else {
            self.bucket_size(size).saturating_mul(2)
        };
        self.free_chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| c.buffer.size() <= max_size && c.can_allocate(size, alignment))
            .min_by_key(|(_, c)| c.buffer.size())
            .map(|(i, _)| i)
    }
    /// A mapped staging slice of exactly `size` bytes, aligned to at least
    /// `alignment` and `MAP_ALIGNMENT`.
    fn allocate(&mut self, size: NonZeroU64, alignment: NonZeroU64) -> wgpu::BufferSlice<'_> {
        let size = size.get();
        assert!(
            size.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT),
            "staging allocation size {size} must be a multiple of COPY_BUFFER_ALIGNMENT"
        );
        let alignment = alignment.get().max(wgpu::MAP_ALIGNMENT);
        debug_assert!(alignment.is_power_of_two());

        let index = if let Some(i) = self.pick_active(size, alignment) {
            i
        } else {
            self.receive_chunks();
            let chunk = if let Some(i) = self.pick_free(size, alignment) {
                self.free_chunks.swap_remove(i)
            } else {
                let capacity = self.bucket_size(size);
                Chunk {
                    buffer: self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Horizon staging upload buffer"),
                        size: capacity,
                        usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::MAP_WRITE,
                        mapped_at_creation: true,
                    }),
                    offset: 0,
                    last_used_epoch: self.epoch,
                    last_used_at: self.last_finish_at,
                }
            };
            self.active_chunks.push(chunk);
            self.active_chunks.len() - 1
        };

        let chunk = &mut self.active_chunks[index];
        let start = chunk.allocate(size, alignment);
        chunk.buffer.slice(start..start + size)
    }
    fn receive_chunks(&mut self) {
        let receiver = self
            .receiver
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while let Ok(mut chunk) = receiver.try_recv() {
            chunk.offset = 0;
            self.free_chunks.push(chunk);
        }
    }
    /// Releases idle oversized chunks and excess regular burst chunks a couple at
    /// a time so memory returns after world load, cloud rebuilds, or mob spikes
    /// without causing frame-time hitches or reallocating in steady state.
    fn trim_idle(&mut self, now: Instant) {
        if self.free_chunks.is_empty() {
            return;
        }
        let mut regular_count = self
            .free_chunks
            .iter()
            .filter(|c| c.buffer.size() <= self.chunk)
            .count();
        let mut trimmed = Vec::new();
        let mut i = 0;
        while i < self.free_chunks.len() && trimmed.len() < MAX_TRIM_PER_FINISH {
            let chunk = &self.free_chunks[i];
            let oversized = chunk.buffer.size() > self.chunk;
            if !oversized && regular_count <= MAX_IDLE_REGULAR_CHUNKS {
                i += 1;
                continue;
            }
            let idle_finishes = self.epoch.saturating_sub(chunk.last_used_epoch);
            let finish_limit = if oversized {
                IDLE_OVERSIZED_FINISHES
            } else {
                IDLE_REGULAR_FINISHES
            };
            let stale = idle_finishes >= finish_limit
                || (idle_finishes >= 2
                    && now.saturating_duration_since(chunk.last_used_at) >= IDLE_WALL_TIME);
            if stale {
                if !oversized {
                    regular_count -= 1;
                }
                trimmed.push(self.free_chunks.swap_remove(i).buffer);
            } else {
                i += 1;
            }
        }
        if !trimmed.is_empty() && trimmed.iter().any(|b| b.size() >= CHUNK) {
            super::arena::free_later(trimmed);
        }
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
        let alignment = NonZeroU64::new(TEXTURE_COPY_OFFSET_ALIGNMENT).expect("non-zero alignment");
        let slice = self.allocate(size, alignment);
        {
            let mut view = slice
                .get_mapped_range_mut()
                .expect("staging belt slices are mapped");
            if row == padded {
                view.slice(..).copy_from_slice(pixels);
            } else {
                for (i, src) in pixels.chunks(row).enumerate() {
                    view.slice(i * padded..i * padded + src.len())
                        .copy_from_slice(src);
                }
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
        assert!(
            offset.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)
                && size.get().is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT),
            "buffer upload at {offset} of {size} bytes must be 4-byte aligned"
        );
        let alignment = NonZeroU64::new(wgpu::COPY_BUFFER_ALIGNMENT).expect("non-zero alignment");
        let slice = self.allocate(size, alignment);
        encoder.copy_buffer_to_buffer(slice.buffer(), slice.offset(), target, offset, size.get());
        slice
            .get_mapped_range_mut()
            .expect("staging belt slices are mapped")
            .slice(..)
            .copy_from_slice(bytes);
    }
    /// Close this frame's staging buffers: they come back for reuse once the
    /// encoder's submission finishes on the GPU, and stale burst chunks are
    /// gradually released.
    pub fn finish(&mut self, encoder: &wgpu::CommandEncoder) {
        let now = Instant::now();
        self.last_finish_at = now;
        let sender = self
            .sender
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for mut chunk in self.active_chunks.drain(..) {
            chunk.last_used_epoch = self.epoch;
            chunk.last_used_at = now;
            chunk.buffer.unmap();
            let buffer = chunk.buffer.clone();
            let sender = sender.clone();
            encoder.map_buffer_on_submit(&buffer, wgpu::MapMode::Write, .., move |result| {
                if result.is_ok() {
                    let _ = sender.send(chunk);
                }
            });
        }
        self.epoch = self.epoch.saturating_add(1);
        self.receive_chunks();
        self.trim_idle(now);
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

    /// Staging buffers the pool holds, once every submitted frame is done with them.
    fn kept_buffers(device: &wgpu::Device, uploads: &mut Uploads) -> usize {
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        // Finishing an empty frame takes back every recalled buffer.
        uploads.finish(&device.create_command_encoder(&Default::default()));
        uploads.active_chunks.len() + uploads.free_chunks.len()
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

    #[test]
    fn idle_oversized_and_burst_chunks_are_trimmed() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        const TEST_CHUNK: u64 = 1 << 16;
        let target = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 4 * TEST_CHUNK,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut uploads = Uploads::with_chunk(&device, TEST_CHUNK);
        // Burst frame: one oversized write (3.5 chunks -> 4-chunk buffer with 0.5 chunk
        // tail, too small for a full regular write) plus four full regular chunks.
        let mut encoder = device.create_command_encoder(&Default::default());
        let big = vec![0xABu8; 3 * TEST_CHUNK as usize + TEST_CHUNK as usize / 2];
        let regular = vec![0xCDu8; TEST_CHUNK as usize];
        uploads.buffer(&mut encoder, &target, 0, &big);
        for _ in 0..4 {
            uploads.buffer(&mut encoder, &target, 0, &regular);
        }
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
        assert_eq!(kept_buffers(&device, &mut uploads), 5);
        // After `IDLE_OVERSIZED_FINISHES` idle finishes, the oversized chunk is released.
        for _ in 0..IDLE_OVERSIZED_FINISHES {
            uploads.finish(&device.create_command_encoder(&Default::default()));
        }
        assert!(
            uploads
                .free_chunks
                .iter()
                .all(|c| c.buffer.size() <= TEST_CHUNK),
            "oversized chunk should be trimmed after idle window"
        );
        // After `IDLE_REGULAR_FINISHES` total idle finishes, excess regular chunks trim
        // back down to the warm reserve (`MAX_IDLE_REGULAR_CHUNKS`).
        for _ in 0..(IDLE_REGULAR_FINISHES - IDLE_OVERSIZED_FINISHES + 4) {
            uploads.finish(&device.create_command_encoder(&Default::default()));
        }
        assert_eq!(uploads.free_chunks.len(), MAX_IDLE_REGULAR_CHUNKS);
        // Subsequent uploads still reuse the warm reserve and land accurately.
        let payload = [0x5Au8; 64];
        let mut encoder = device.create_command_encoder(&Default::default());
        uploads.buffer(&mut encoder, &target, 0, &payload);
        encoder.copy_buffer_to_buffer(&target, 0, &readback, 0, 64);
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(*readback.slice(..).get_mapped_range().unwrap(), payload[..]);
        readback.unmap();
    }

    #[test]
    fn aligned_row_texture_upload_matches_queue_write() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let make = || {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 2,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let (expected, actual) = (make(), make());
        let pixels: Vec<u8> = (0..64 * 64 * 4).map(|i| (i * 13 + 7) as u8).collect();
        let at = |texture| wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: 0, y: 0, z: 1 },
            aspect: wgpu::TextureAspect::All,
        };
        queue.write_texture(
            at(&expected),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(64 * 4),
                rows_per_image: Some(64),
            },
            wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
        );
        let mut uploads = Uploads::new(&device);
        let mut encoder = device.create_command_encoder(&Default::default());
        uploads.texture(&mut encoder, at(&actual), 64, 64, &pixels);
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(
            read_back(&device, &queue, &actual),
            read_back(&device, &queue, &expected)
        );
    }
}
