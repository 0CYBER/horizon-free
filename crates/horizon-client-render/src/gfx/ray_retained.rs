//! The block entities the renderer keeps on the GPU (a closed chest, a shulker box, a bell, a
//! decorated pot, a banner: `StaticSectionSnapshot`) in the ray scene's entity layer (spec
//! #16011 item 3, step 3), while it has the layer. Their draws read a vertex buffer a section,
//! which rays can't; the layer copies each section's quads into a store of its own, in the
//! stream's layout (`Vertex`), and builds the section a BLAS from them, as terrain has. A
//! section's BLAS is built again only when its quads change (its snapshot's revision: a chest
//! that opens moves to the stream, and back when it closes), and goes when its block entities
//! do or it leaves the scene's reach. Its instance is in the TLAS after the stream's, mask
//! `RAY_ENTITY`, its custom data its first quad in the store: a hit on an instance from
//! `RayScene::retained_instances` on reads the store (`RAY_ENTITY_RETAINED_BINDING`), the stream's reads the
//! stream.

use horizon_shader_api::stages::{RAY_ENTITY, RAY_ENTITY_QUAD};

use super::{BUILDS_PER_FRAME, ENTITY_STRIDE, Slots, geometry_size, largest_entity_binding};
use crate::block_entity_render::StaticSectionSnapshot;
use crate::fast_hash::FxHashMap;
use crate::mesher::Vertex;

/// A section of retained block entities: `StaticSectionSnapshot::section`, in sections along x
/// and z, from the world's bottom along y.
type Key = [i32; 3];

/// Vertices the store starts with: a few hundred chests.
const FIRST_STORE: u64 = 1 << 14;
/// Quads the store holds at most: an instance's custom data holds its first quad in 24 bits.
const MOST_QUADS: u64 = 1 << 24;

/// A section's BLAS, built from its slot of the store.
struct Section {
    /// Its snapshot's revision when built.
    revision: u64,
    /// The section's corner in the world (`StaticSectionSnapshot::origin`).
    origin: [i32; 3],
    /// None when none of its quads is the layer's: it has no slot either.
    blas: Option<wgpu::Blas>,
}

/// The retained block entities of the entity layer: their store and each section's BLAS.
pub(super) struct Retained {
    sections: FxHashMap<Key, Section>,
    slots: Slots<Key>,
    /// The sections' quads of the layer, in the stream's layout, four vertices a quad.
    store: wgpu::Buffer,
    /// The sections built this frame and their geometry's size.
    built: Vec<(Key, wgpu::BlasTriangleGeometrySizeDescriptor)>,
    /// Whether the log said the store is full.
    full_logged: bool,
}

/// What a frame does with the retained sections: the BLAS it drops, and the snapshots (by their
/// index) whose sections it builds.
#[derive(Debug, Default, PartialEq)]
struct Plan {
    drop: Vec<Key>,
    build: Vec<usize>,
}

/// Whether section `key` is within `reach` chunks of column `column`, along x and z.
fn near(key: Key, column: (i32, i32), reach: i32) -> bool {
    (key[0] - column.0).abs() <= reach && (key[2] - column.1).abs() <= reach
}

/// The plan for this frame's `snapshots` (sorted by section, as the cache publishes them), the
/// camera in column `column`, given the sections that have a BLAS and their revisions (`kept`):
/// - a snapshot within `reach` chunks without a BLAS, or with one of another revision, is built
///   (at most `BUILDS_PER_FRAME`, the others on the next frames);
/// - a BLAS goes when its section has no snapshot any more (its block entities went, or all
///   moved to the stream), when it is past `reach + 2` chunks (so walking along a chunk border
///   doesn't build the same ones over and over, as terrain), or when it is out of date past
///   `reach`, where it would not be built again.
fn plan(
    kept: impl Iterator<Item = (Key, u64)>,
    snapshots: &[StaticSectionSnapshot],
    column: (i32, i32),
    reach: i32,
) -> Plan {
    let mut out = Plan::default();
    let mut revisions = FxHashMap::default();
    for (key, revision) in kept {
        let at = snapshots.binary_search_by_key(&key, |s| s.section).ok();
        let stays = at.is_some_and(|i| {
            let fresh = snapshots[i].revision == revision;
            near(key, column, reach + 2) && (fresh || near(key, column, reach))
        });
        if stays {
            revisions.insert(key, revision);
        } else {
            out.drop.push(key);
        }
    }
    out.drop.sort_unstable();
    out.build = (snapshots.iter().enumerate())
        .filter(|(_, s)| near(s.section, column, reach))
        .filter(|(_, s)| revisions.get(&s.section) != Some(&s.revision))
        .map(|(i, _)| i)
        .take(BUILDS_PER_FRAME)
        .collect();
    out
}

/// The quads of `vertices` the layer takes, marked as the batch marks the stream's
/// (`RAY_ENTITY_QUAD`): those of the ordinary material, which the retained draw lights and
/// alpha tests. The shader passes their unlit overlays, as the stream's.
fn layer_quads(vertices: &[Vertex]) -> Vec<Vertex> {
    let quads = vertices.as_chunks::<4>().0.iter();
    quads
        .filter(|quad| crate::gfx::entity_batch::material(quad[0].layer, false) == 0)
        .flat_map(|quad| {
            quad.map(|v| Vertex {
                layer: v.layer | RAY_ENTITY_QUAD,
                ..v
            })
        })
        .collect()
}

/// Vertices the store may hold on a device with `limits`: rays read it through one binding.
fn largest_store(limits: &wgpu::Limits) -> u64 {
    let bytes = limits.max_buffer_size / (ENTITY_STRIDE * 4);
    u64::from(largest_entity_binding(limits))
        .min(bytes)
        .min(MOST_QUADS)
        * 4
}

fn store_buffer(device: &wgpu::Device, vertices: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Ray tracing: retained block entities"),
        size: vertices * ENTITY_STRIDE,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::BLAS_INPUT
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl Retained {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let most = largest_store(&device.limits());
        let slots = Slots::new(FIRST_STORE.min(most), most);
        Self {
            sections: FxHashMap::default(),
            store: store_buffer(device, slots.capacity),
            slots,
            built: Vec::new(),
            full_logged: false,
        }
    }

    /// The store, as rays read it.
    pub(super) fn store(&self) -> &wgpu::Buffer {
        &self.store
    }

    /// Takes this frame's `snapshots`, the camera in column `column`: drops the BLAS the plan
    /// drops, and writes the quads of the ones it builds into their slots, recorded in `encoder`
    /// (`builds` then builds them). Whether the store is a new one.
    pub(super) fn update(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        snapshots: &[StaticSectionSnapshot],
        column: (i32, i32),
        reach: i32,
    ) -> bool {
        self.built.clear();
        let kept = self.sections.iter().map(|(k, s)| (*k, s.revision));
        let plan = plan(kept, snapshots, column, reach);
        for key in plan.drop {
            self.sections.remove(&key);
            self.slots.free(key);
        }
        let mut writes: Vec<(u64, Vec<Vertex>)> = Vec::new();
        for i in plan.build {
            let snapshot = &snapshots[i];
            let key = snapshot.section;
            let quads = crate::spans::time("ray retained BLAS", || layer_quads(&snapshot.vertices));
            if quads.is_empty() {
                // Kept without a BLAS, so the same snapshot isn't looked through again.
                let section = Section {
                    revision: snapshot.revision,
                    origin: snapshot.origin,
                    blas: None,
                };
                self.sections.insert(key, section);
                self.slots.free(key);
                continue;
            }
            // The store grows below, after the loop, with every slot's contents copied over.
            let Some(first) = self.slots.alloc(key, quads.len() as u64, |_| true) else {
                // The store is at its largest: the section keeps its old BLAS, if it has one,
                // and waits for room.
                if !std::mem::replace(&mut self.full_logged, true) {
                    log::warn!(
                        "Horizon renderer: the ray tracing scene's retained block entities fill \
                         their store ({} vertices); some wait for room",
                        self.slots.capacity
                    );
                }
                continue;
            };
            let size = geometry_size(quads.len() as u32 / 4);
            let blas = device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some("Ray tracing: retained block entities"),
                    flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                },
                wgpu::BlasGeometrySizeDescriptors::Triangles {
                    descriptors: vec![size.clone()],
                },
            );
            let section = Section {
                revision: snapshot.revision,
                origin: snapshot.origin,
                blas: Some(blas),
            };
            self.sections.insert(key, section);
            self.built.push((key, size));
            writes.push((first, quads));
        }
        // A larger store gets the old one's contents first, so every slot keeps its place.
        let grown = self.slots.capacity * ENTITY_STRIDE > self.store.size();
        if grown {
            let store = store_buffer(device, self.slots.capacity);
            encoder.copy_buffer_to_buffer(&self.store, 0, &store, 0, self.store.size());
            self.store = store;
        }
        if !writes.is_empty() {
            let all: Vec<Vertex> = writes.iter().flat_map(|(_, q)| q.iter().copied()).collect();
            let staging = wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Ray tracing: retained block entity quads"),
                    contents: bytemuck::cast_slice(&all),
                    usage: wgpu::BufferUsages::COPY_SRC,
                },
            );
            let mut from = 0;
            for (first, quads) in &writes {
                let bytes = quads.len() as u64 * ENTITY_STRIDE;
                encoder.copy_buffer_to_buffer(
                    &staging,
                    from,
                    &self.store,
                    first * ENTITY_STRIDE,
                    bytes,
                );
                from += bytes;
            }
        }
        grown
    }

    /// The most quads a section built this frame has: the shared quad indices cover them.
    pub(super) fn most_quads(snapshots: &[StaticSectionSnapshot]) -> u32 {
        let quads = snapshots.iter().map(|s| s.vertices.len() / 4);
        quads.max().unwrap_or(0) as u32
    }

    /// The builds of this frame's sections, from their slots with the shared quad `indices`.
    pub(super) fn builds<'a>(
        &'a self,
        indices: &'a wgpu::Buffer,
    ) -> impl Iterator<Item = wgpu::BlasBuildEntry<'a>> {
        let built = self.built.iter();
        let blas =
            built.filter_map(|(key, size)| Some((key, size, self.sections[key].blas.as_ref()?)));
        blas.map(move |(key, size, blas)| wgpu::BlasBuildEntry {
            blas,
            geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                size,
                vertex_buffer: &self.store,
                first_vertex: self.slots.first(*key).unwrap_or(0) as u32,
                vertex_stride: ENTITY_STRIDE,
                index_buffer: Some(indices),
                first_index: Some(0),
                transform_buffer: None,
                transform_buffer_offset: None,
            }]),
        })
    }

    /// The instances of the sections within `reach` chunks of column `column`, in the TLAS's
    /// space (relative to that column's corner), mask `RAY_ENTITY`, custom data their first quad.
    pub(super) fn instances(
        &self,
        column: (i32, i32),
        reach: i32,
    ) -> impl Iterator<Item = wgpu::TlasInstance> + '_ {
        let sections = self.sections.iter();
        let near = sections.filter(move |(key, _)| near(**key, column, reach));
        let built = near.filter_map(|(key, s)| Some((key, s, s.blas.as_ref()?)));
        built.map(move |(key, s, blas)| {
            let x = (s.origin[0] - column.0 * 16) as f32;
            let z = (s.origin[2] - column.1 * 16) as f32;
            let y = s.origin[1] as f32;
            let transform = [1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, y, 0.0, 0.0, 1.0, z];
            // Below `MOST_QUADS`, so it fits the custom data's 24 bits.
            let first_quad = self.slots.first(*key).map_or(0, |at| at / 4) as u32;
            wgpu::TlasInstance::new(blas, transform, first_quad, RAY_ENTITY as u8)
        })
    }
}

#[cfg(test)]
#[path = "ray_retained_tests.rs"]
mod tests;
