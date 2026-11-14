//! The terrain as acceleration structures, with ray tracing on (#13780 step 2, after the #1180
//! spike; `ray_query`): one BLAS per section's solid quads within `reach` chunks of the camera,
//! compacted once built, and one TLAS of them, built again only when a BLAS changes or the camera
//! enters another chunk column (its instances sit relative to that column). Nothing traces them
//! yet. Without ray queries on the device there is no scene and nothing here runs.
//!
//! The scene keeps every such section's solid quads as vertices (`TerrainVertex`) in one store
//! that lasts from frame to frame (spec #15624): the BLAS are built from it, and a ray that hits
//! a triangle reads that triangle's vertices there. A TLAS instance's custom data is its
//! section's first quad in the store. The unpack pass (`Unpacker`, `rt_unpack.wgsl`) writes a
//! section into its slot (`Slots`) as the solid draws read it, from a record or a vertex, so the
//! section arena keeps its compact layout. A full store grows: the larger one gets the old
//! contents copied over, so every slot keeps its place and no BLAS is built again.

use glam::DVec3;

use crate::fast_hash::FxHashMap;
use crate::gfx::{Renderer, SectionGpu, SectionKey, arena::Arena, resident::Resident};
use crate::mesher::TerrainVertex;

/// BLAS builds per frame at most: a world that just loaded builds over a few frames.
const BUILDS_PER_FRAME: usize = 512;
/// Bytes per vertex in the store.
const STRIDE: u64 = std::mem::size_of::<TerrainVertex>() as u64;
/// Vertices the store starts with.
const FIRST_STORE: u64 = 1 << 20;
/// Vertices the store holds at most: an instance's custom data holds its first quad in 24 bits.
const MOST_STORE: u64 = 1 << 26;
/// Chunks around the camera the TLAS spans, unless `HORIZON_RT_CHUNKS` says otherwise. The
/// #1180 spike's TLAS over every section cost 2-7 ms of CPU a frame at render distance 32.
const REACH: i32 = 8;

/// The renderer's side: the calls gfx.rs makes, each one line there.
impl Renderer {
    /// Section `key` got a new mesh, or lost it.
    pub(in crate::gfx) fn ray_mark(&mut self, key: SectionKey) {
        if let Some(rt) = &mut self.ray_scene {
            rt.mark(key);
        }
    }

    /// Every section is gone (a new world, a new dimension).
    pub(in crate::gfx) fn ray_clear(&mut self) {
        if let Some(rt) = &mut self.ray_scene {
            rt.clear();
        }
    }

    /// The frame's BLAS builds and TLAS, recorded in `encoder` before the world pass.
    pub(in crate::gfx) fn ray_prepare(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        camera: DVec3,
        title: bool,
    ) {
        if let Some(rt) = &mut self.ray_scene
            && !title
        {
            crate::spans::time("ray scene", || {
                rt.prepare(
                    &self.device,
                    &self.queue,
                    encoder,
                    &self.sections,
                    &self.arena,
                    &self.indices,
                    camera,
                )
            });
        }
    }
}

/// The terrain's acceleration structures.
pub(in crate::gfx) struct RayScene {
    /// Each section's BLAS, while it has solid quads within reach.
    blas: FxHashMap<SectionKey, wgpu::Blas>,
    /// Sections meshed again or emptied since their BLAS was built.
    dirty: Vec<SectionKey>,
    tlas: wgpu::Tlas,
    /// The camera's chunk column when the TLAS was built: its instances are placed relative to
    /// that column's corner, so moving inside it needs no new TLAS.
    anchor: Option<(i32, i32)>,
    /// Chunks around the anchor the TLAS spans.
    reach: i32,
    /// BLAS built this frame, to measure for compaction once submitted.
    to_compact: Vec<(SectionKey, wgpu::Blas)>,
    /// BLAS whose compacted size the GPU is measuring.
    compacting: Vec<(SectionKey, wgpu::Blas)>,
    /// The store, and the pass that writes sections into it.
    unpacker: Unpacker,
    /// Each section's place in the store: exactly the sections with a BLAS.
    slots: Slots,
    /// Whether the log said the store is at its largest.
    full_logged: bool,
}

impl RayScene {
    /// The scene, when the device has ray queries (`ray_query`).
    pub(in crate::gfx) fn new(device: &wgpu::Device) -> Option<Self> {
        if !super::on(device) {
            return None;
        }
        let reach = std::env::var("HORIZON_RT_CHUNKS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(REACH);
        // The unpack pass binds the store whole.
        let limits = device.limits();
        let bytes = limits
            .max_buffer_size
            .min(limits.max_storage_buffer_binding_size);
        let most = MOST_STORE.min(bytes / STRIDE / 4 * 4);
        let slots = Slots::new(FIRST_STORE.min(most), most);
        Some(Self {
            blas: FxHashMap::default(),
            dirty: Vec::new(),
            tlas: create_tlas(device, 1024),
            anchor: None,
            reach,
            to_compact: Vec::new(),
            compacting: Vec::new(),
            unpacker: Unpacker::new(device, slots.capacity),
            slots,
            full_logged: false,
        })
    }

    /// What a trace reads (spec #15624): the TLAS, the store of its sections' vertices (an
    /// instance's custom data is its section's first quad there), and the chunk column its
    /// instances sit relative to (none before the first TLAS) with the chunks around it.
    #[expect(dead_code, reason = "spec #15624: shader packs' `trace` reads it next")]
    pub(in crate::gfx) fn traced(&self) -> (&wgpu::Tlas, &wgpu::Buffer, Option<(i32, i32)>, i32) {
        (&self.tlas, &self.unpacker.store, self.anchor, self.reach)
    }

    fn mark(&mut self, key: SectionKey) {
        self.dirty.push(key);
    }

    fn clear(&mut self) {
        self.slots.clear();
        self.blas.clear();
        self.dirty.clear();
        self.to_compact.clear();
        self.compacting.clear();
        self.anchor = None;
    }

    /// Compacts BLAS built on earlier frames: built with room to spare, a compacted copy (made
    /// once the GPU has measured it) takes about a quarter. Whether a BLAS was replaced.
    fn compact(&mut self, queue: &wgpu::Queue) -> bool {
        let mut changed = false;
        let blas = &mut self.blas;
        self.compacting.retain(|(key, b)| {
            if blas.get(key) != Some(b) {
                return false;
            }
            if !b.ready_for_compaction() {
                return true;
            }
            blas.insert(*key, queue.compact_blas(b));
            changed = true;
            false
        });
        // Built and submitted last frame: the GPU can now measure them.
        for (key, b) in std::mem::take(&mut self.to_compact) {
            if self.blas.get(&key) == Some(&b) {
                b.prepare_compaction_async(|_| {});
                self.compacting.push((key, b));
            }
        }
        changed
    }

    /// Builds the BLAS of sections marked since the last frame (at most `BUILDS_PER_FRAME`, and
    /// as many as the store has room for), and the TLAS again when a BLAS changed or the camera
    /// entered another chunk column.
    #[allow(clippy::too_many_arguments)]
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        sections: &Resident<SectionGpu>,
        arena: &Arena,
        indices: &wgpu::Buffer,
        camera: DVec3,
    ) {
        let column = crate::gfx::resident::camera_column(camera);
        // Only sections within `reach` get a BLAS (at render distance 32 all of them took 1.3 GB
        // of video memory); those past `reach + 2` lose theirs, so walking along a chunk border
        // doesn't build the same ones over and over.
        let keep = self.reach + 2;
        let mut changed = self.compact(queue);
        if self.anchor != Some(column) {
            let before = self.blas.len();
            let slots = &mut self.slots;
            self.blas.retain(|k, _| {
                let kept = within(k, column, keep);
                if !kept {
                    slots.free(*k);
                }
                kept
            });
            changed |= self.blas.len() != before;
            for (key, s, _) in sections.iter() {
                if within(&key, column, self.reach) && s.solid > 0 && !self.blas.contains_key(&key)
                {
                    self.dirty.push(key);
                }
            }
        }
        let take = self.dirty.len().min(BUILDS_PER_FRAME);
        let mut builds: Vec<Build<'_>> = Vec::new();
        let mut deferred = Vec::new();
        for key in self.dirty.drain(..take) {
            if builds.iter().any(|b| b.key == key) || deferred.contains(&key) {
                continue;
            }
            match sections
                .find(key)
                .filter(|s| s.solid > 0 && within(&key, column, self.reach))
            {
                Some(s) => {
                    // A section meshed again leaves its old slot: its old BLAS goes from the
                    // TLAS below, before anything traces the section written over it.
                    let Some(first) = self.slots.alloc(key, u64::from(s.solid) * 4) else {
                        // The store is at its largest: the section keeps its old BLAS and
                        // slot, if it has them, and waits for a later frame's room.
                        if !self.full_logged {
                            self.full_logged = true;
                            log::warn!(
                                "Horizon renderer: the ray tracing scene's store is full ({} \
                                 vertices); some sections wait for room",
                                self.slots.capacity
                            );
                        }
                        deferred.push(key);
                        continue;
                    };
                    let (size, blas) = new_blas(device, s.solid);
                    builds.push(Build {
                        key,
                        s,
                        size,
                        blas,
                        first,
                    });
                }
                None => {
                    self.slots.free(key);
                    changed |= self.blas.remove(&key).is_some();
                }
            }
        }
        self.dirty.extend(deferred);
        // Before the unpack pass writes into the store.
        self.unpacker.fit(device, encoder, self.slots.capacity);
        if builds.is_empty() && !changed && self.anchor == Some(column) {
            return;
        }
        self.unpacker.run(device, encoder, arena, &builds);
        for b in &builds {
            self.blas.insert(b.key, b.blas.clone());
            self.to_compact.push((b.key, b.blas.clone()));
        }
        let near = |k: &SectionKey| within(k, column, self.reach);
        let count = self.blas.keys().filter(|k| near(k)).count();
        if count > self.tlas.get().len() {
            self.tlas = create_tlas(device, (count as u32).next_power_of_two());
        }
        let min_y = sections.min_y();
        let mut n = 0;
        for (key, blas) in self.blas.iter().filter(|(k, _)| near(k)) {
            let o = instance_origin(*key, min_y, column);
            // Below `MOST_STORE / 4`, so it fits the custom data's 24 bits.
            let first_quad = self.slots.first(*key).map_or(0, |at| at / 4) as u32;
            self.tlas[n] = Some(wgpu::TlasInstance::new(
                blas,
                [
                    1.0, 0.0, 0.0, o[0], 0.0, 1.0, 0.0, o[1], 0.0, 0.0, 1.0, o[2],
                ],
                first_quad,
                0xff,
            ));
            n += 1;
        }
        for i in n..self.tlas.get().len() {
            if self.tlas[i].is_none() {
                break;
            }
            self.tlas[i] = None;
        }
        let entries = build_entries(&builds, &self.unpacker.store, indices);
        encoder.build_acceleration_structures(entries.iter(), std::iter::once(&self.tlas));
        self.anchor = Some(column);
    }
}

/// Whether section `key` is within `reach` chunks of column `column`, along x and z.
fn within(key: &SectionKey, column: (i32, i32), reach: i32) -> bool {
    (key.0 - column.0).abs() <= reach && (key.2 - column.1).abs() <= reach
}

/// A BLAS to build this frame.
struct Build<'a> {
    key: SectionKey,
    s: &'a SectionGpu,
    size: wgpu::BlasTriangleGeometrySizeDescriptor,
    blas: wgpu::Blas,
    /// Its first vertex in the store.
    first: u64,
}

/// `rt_unpack.wgsl`'s `RtJob`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RtJob {
    base: u32,
    quads: u32,
    out: u32,
    pad: u32,
}

/// The store, and the pass that writes sections' solid quads into it as vertices: BLAS builds
/// and rays read vertices, while the arena's compact layout holds records only the draws
/// expand.
struct Unpacker {
    pipeline: wgpu::ComputePipeline,
    store: wgpu::Buffer,
}

impl Unpacker {
    fn new(device: &wgpu::Device, capacity: u64) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            // No colon: DXC (DirectX 12) takes the label for a file name and fails to read it.
            label: Some("Unpack sections"),
            source: wgpu::ShaderSource::Wgsl(unpack_shader().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Ray tracing: unpack sections"),
            layout: None,
            module: &module,
            entry_point: Some("cs_rt_unpack"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            pipeline,
            store: store_buffer(device, capacity),
        }
    }

    /// A store of `vertices` when it holds fewer: the larger one gets the old contents first
    /// (recorded before this frame's unpack pass), so every slot keeps its place. The old one
    /// lives on in `encoder` until it is submitted.
    fn fit(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, vertices: u64) {
        if vertices * STRIDE <= self.store.size() {
            return;
        }
        let store = store_buffer(device, vertices);
        encoder.copy_buffer_to_buffer(&self.store, 0, &store, 0, self.store.size());
        self.store = store;
    }

    /// Writes the builds' sections into their slots, one dispatch per arena page.
    fn run(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        arena: &Arena,
        builds: &[Build<'_>],
    ) {
        let mut pages: Vec<u32> = builds.iter().map(|b| b.s.page).collect();
        pages.sort_unstable();
        pages.dedup();
        if pages.is_empty() {
            return;
        }
        let empty = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Ray tracing: unpack sections"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &empty, &[]);
        for page in pages {
            let jobs: Vec<RtJob> = builds
                .iter()
                .filter(|b| b.s.page == page)
                .map(|b| RtJob {
                    base: b.s.solid_base(),
                    quads: b.s.solid,
                    out: b.first as u32,
                    pad: 0,
                })
                .collect();
            let jobs_buffer = wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Ray tracing: unpack jobs"),
                    contents: bytemuck::cast_slice(&jobs),
                    usage: wgpu::BufferUsages::STORAGE,
                },
            );
            let arena_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.pipeline.get_bind_group_layout(1),
                entries: &[wgpu::BindGroupEntry {
                    binding: 19,
                    resource: arena.buffer(page).as_entire_binding(),
                }],
            });
            let out_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.pipeline.get_bind_group_layout(2),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: jobs_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.store.as_entire_binding(),
                    },
                ],
            });
            let most = jobs.iter().map(|j| j.quads * 4).max().unwrap_or(0);
            pass.set_bind_group(1, &arena_group, &[]);
            pass.set_bind_group(2, &out_group, &[]);
            pass.dispatch_workgroups(most.div_ceil(64), jobs.len() as u32, 1);
        }
    }
}

/// terrain.wgsl with the unpacking entry point after it, which reads sections as its solid
/// draws do (`section_corner`).
pub(in crate::gfx) fn unpack_shader() -> String {
    crate::gfx::terrain_shader() + include_str!("../rt_unpack.wgsl")
}

fn store_buffer(device: &wgpu::Device, vertices: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Ray tracing: section vertices"),
        size: vertices * STRIDE,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::BLAS_INPUT
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_tlas(device: &wgpu::Device, capacity: u32) -> wgpu::Tlas {
    device.create_tlas(&wgpu::CreateTlasDescriptor {
        label: Some("Terrain sections"),
        max_instances: capacity,
        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
    })
}

/// A new BLAS for `solid` quads, and its size.
fn new_blas(
    device: &wgpu::Device,
    solid: u32,
) -> (wgpu::BlasTriangleGeometrySizeDescriptor, wgpu::Blas) {
    let size = wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: solid * 4,
        index_format: Some(wgpu::IndexFormat::Uint32),
        index_count: Some(solid * 6),
        // Not opaque: rays test cut-out texels (leaves, plants) themselves.
        flags: wgpu::AccelerationStructureGeometryFlags::empty(),
    };
    let blas = device.create_blas(
        &wgpu::CreateBlasDescriptor {
            label: None,
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE
                | wgpu::AccelerationStructureFlags::ALLOW_COMPACTION,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        },
        wgpu::BlasGeometrySizeDescriptors::Triangles {
            descriptors: vec![size.clone()],
        },
    );
    (size, blas)
}

/// Build entries reading each section's solid vertices in the store (`pos` is
/// `TerrainVertex`'s first field) with the shared quad indices.
fn build_entries<'a>(
    builds: &'a [Build<'a>],
    store: &'a wgpu::Buffer,
    indices: &'a wgpu::Buffer,
) -> Vec<wgpu::BlasBuildEntry<'a>> {
    builds
        .iter()
        .map(|b| wgpu::BlasBuildEntry {
            blas: &b.blas,
            geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                size: &b.size,
                vertex_buffer: store,
                first_vertex: b.first as u32,
                vertex_stride: STRIDE,
                index_buffer: Some(indices),
                first_index: Some(0),
                transform_buffer: None,
                transform_buffer_offset: None,
            }]),
        })
        .collect()
}

/// Where each section's vertices sit in the store: first fit over the free ranges, which merge
/// with their neighbours when freed. A store with no range to fit grows, up to `most`.
struct Slots {
    taken: FxHashMap<SectionKey, (u64, u64)>,
    /// Free (first vertex, vertices), in order, none touching another.
    free: Vec<(u64, u64)>,
    /// Vertices the store holds.
    capacity: u64,
    most: u64,
}

impl Slots {
    fn new(capacity: u64, most: u64) -> Self {
        Self {
            taken: FxHashMap::default(),
            free: vec![(0, capacity)],
            capacity,
            most,
        }
    }

    /// Section `key`'s first vertex.
    fn first(&self, key: SectionKey) -> Option<u64> {
        self.taken.get(&key).map(|s| s.0)
    }

    /// A slot of `vertices` for section `key` in place of its old one, the first that fits, in
    /// a store twice as large (at least large enough) when none does. None at `most`: `key`
    /// then keeps its old slot.
    fn alloc(&mut self, key: SectionKey, vertices: u64) -> Option<u64> {
        let old = self.taken.remove(&key);
        if let Some(old) = old {
            self.release(old);
        }
        let fits = |free: &[(u64, u64)]| free.iter().find(|r| r.1 >= vertices).map(|r| r.0);
        let mut at = fits(&self.free);
        if at.is_none() {
            let capacity = (self.capacity * 2)
                .max(self.capacity + vertices)
                .min(self.most);
            if capacity > self.capacity {
                self.release((self.capacity, capacity - self.capacity));
                self.capacity = capacity;
                at = fits(&self.free);
            }
        }
        let slot = match at {
            Some(at) => (at, vertices),
            None => old?,
        };
        self.take(slot);
        self.taken.insert(key, slot);
        at
    }

    fn free(&mut self, key: SectionKey) {
        if let Some(slot) = self.taken.remove(&key) {
            self.release(slot);
        }
    }

    /// Every slot free; the store keeps its size.
    fn clear(&mut self) {
        self.taken.clear();
        self.free = vec![(0, self.capacity)];
    }

    /// Takes `slot` out of the free range holding it.
    fn take(&mut self, (at, n): (u64, u64)) {
        let i = self.free.partition_point(|r| r.0 + r.1 <= at);
        let (start, len) = self.free[i];
        let parts = [(start, at - start), (at + n, start + len - at - n)];
        self.free
            .splice(i..=i, parts.into_iter().filter(|p| p.1 > 0));
    }

    /// Gives `slot` back, merged with the free ranges it touches.
    fn release(&mut self, (mut at, mut n): (u64, u64)) {
        let mut i = self.free.partition_point(|r| r.0 < at);
        if let Some(next) = self.free.get(i).filter(|r| at + n == r.0) {
            n += next.1;
            self.free.remove(i);
        }
        if let Some(before) = i.checked_sub(1).map(|j| self.free[j])
            && before.0 + before.1 == at
        {
            (at, n) = (before.0, before.1 + n);
            i -= 1;
            self.free.remove(i);
        }
        self.free.insert(i, (at, n));
    }
}

/// Where section `key` sits relative to the corner of chunk column `anchor`.
fn instance_origin(key: SectionKey, min_y: i32, anchor: (i32, i32)) -> [f32; 3] {
    [
        ((key.0 - anchor.0) * 16) as f32,
        (min_y + key.1 * 16) as f32,
        ((key.2 - anchor.1) * 16) as f32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instances_sit_relative_to_the_anchor_column() {
        // Section 5 of column (3, -2), the world starting at y = -64, seen from column (2, -2).
        assert_eq!(instance_origin((3, 5, -2), -64, (2, -2)), [16.0, 16.0, 0.0]);
        assert_eq!(instance_origin((2, 0, -2), -64, (2, -2)), [0.0, -64.0, 0.0]);
    }

    const A: SectionKey = (0, 0, 0);
    const B: SectionKey = (1, 0, 0);
    const C: SectionKey = (2, 0, 0);

    #[test]
    fn freed_slots_merge_and_are_taken_first_fit() {
        let mut slots = Slots::new(64, 64);
        assert_eq!(slots.alloc(A, 16), Some(0));
        assert_eq!(slots.alloc(B, 16), Some(16));
        assert_eq!(slots.alloc(C, 16), Some(32));
        slots.free(B);
        slots.free(A);
        // 32 fit only where A's and B's slots merged; the 16 left at the end come next.
        assert_eq!(slots.alloc(A, 32), Some(0));
        assert_eq!(slots.alloc(B, 8), Some(48));
        assert_eq!(slots.capacity, 64);
    }

    #[test]
    fn a_section_meshed_again_leaves_its_old_slot() {
        let mut slots = Slots::new(64, 64);
        slots.alloc(A, 8);
        slots.alloc(B, 8);
        assert_eq!(slots.alloc(A, 4), Some(0));
        // Its old 8 are free again, but 16 don't fit before B.
        assert_eq!(slots.alloc(A, 16), Some(16));
        assert_eq!(slots.alloc(C, 8), Some(0));
        assert_eq!(slots.first(B), Some(8));
    }

    #[test]
    fn a_full_store_grows_and_every_slot_stays() {
        let mut slots = Slots::new(16, 1 << 26);
        assert_eq!(slots.alloc(A, 16), Some(0));
        assert_eq!(slots.alloc(B, 8), Some(16));
        assert_eq!(slots.capacity, 32);
        // Larger than twice the store: the store takes it whole after its free 8.
        assert_eq!(slots.alloc(C, 100), Some(24));
        assert_eq!(slots.capacity, 132);
        assert_eq!((slots.first(A), slots.first(B)), (Some(0), Some(16)));
    }

    #[test]
    fn at_the_largest_store_a_section_waits_in_its_old_slot() {
        let mut slots = Slots::new(16, 32);
        slots.alloc(A, 16);
        slots.alloc(B, 16);
        assert_eq!(slots.alloc(C, 4), None);
        assert_eq!(slots.alloc(A, 20), None);
        assert_eq!(slots.first(A), Some(0));
        // Nothing was lost on the way: A's slot was taken back.
        assert_eq!(slots.alloc(C, 4), None);
        slots.free(B);
        assert_eq!(slots.alloc(A, 20), Some(0));
        slots.clear();
        assert_eq!(slots.alloc(C, 32), Some(0));
    }

    #[test]
    fn the_unpack_shader_validates_with_terrain() {
        let source = unpack_shader();
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    /// The off path is main's: a device opened without ray queries (every device without
    /// `HORIZON_RAY_TRACING`, and every plain build) gets no scene, and its buffers no usage.
    #[test]
    fn without_ray_queries_there_is_no_scene() {
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let Ok((device, _)) = pollster::block_on(adapter.request_device(&Default::default()))
        else {
            return;
        };
        assert!(!super::super::on(&device));
        assert!(RayScene::new(&device).is_none());
        assert!(super::super::blas_input(&device).is_empty());
        assert!(super::super::arena_input(&device).is_empty());
    }
}
