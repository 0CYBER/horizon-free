//! The terrain as acceleration structures, with ray tracing on (#13780 step 2, after the #1180
//! spike; `query`): within `reach` chunks of the camera one BLAS per section's solid quads
//! and one per its translucent quads (`Layer`), compacted once built, and one TLAS of them,
//! built again only when a BLAS changes or the camera enters another chunk column (its instances
//! sit relative to that column). A shader pack's `trace` reads them (`traced`,
//! gfx/native_trace.rs). Without ray queries on the device there is no scene and nothing here
//! runs.
//!
//! The scene keeps every such section's quads as vertices (`TerrainVertex`) in one store that
//! lasts from frame to frame (spec #15624): the BLAS are built from it, and a ray that hits a
//! triangle reads that triangle's vertices there. A TLAS instance's custom data is its quads'
//! first in the store. The unpack pass (`Unpacker`, `rt_unpack.wgsl`) writes a section's solid
//! quads into their slot (`Slots`) as the solid draws read them, from a record or a vertex, so
//! the section arena keeps its compact layout. A full store grows: the larger one gets the old
//! contents copied over, so every slot keeps its place and no BLAS is built again.
//!
//! The two layers are told apart twice (spec #16011): by their instances' masks, so a ray's
//! cull mask picks the layers it reads, and by their place in the TLAS, the solid instances
//! first, so a hit's instance index says which layer it is in (`Traced::solid_instances`).
//! Translucent quads are vertices in their arena already, in the mesher's order: drawing sorts
//! them back to front through an index list of its own (gfx/translucency.rs), which the scene
//! never reads, so a camera that moves builds nothing here. Their vertices are copied into the
//! store as they are.
//!
//! Video memory running out is not fatal: what grows with the world (the store, each part's
//! BLAS and its compacted copy, the TLAS) is made inside an out-of-memory scope
//! (`out_of_memory_free`). A part whose slot or BLAS can't be made waits as at a full store, a
//! TLAS that can't grow leaves the frame's scene as it was, and without room for the first store
//! or TLAS there is no scene.
//!
//! For a pack whose files name `RAY_ENTITY` the scene has a third layer, the entities within its
//! reach (spec #16011 item 3, `Entities`): one BLAS over the entity stream's quads that the batch
//! marked, read from its vertex buffer as it is, its instance after the translucent ones. It is
//! built again every frame, after the stream's upload, so the TLAS is too. Its third geometry is
//! the entities the camera doesn't draw (step 2): the game moves them out of what the camera
//! draws (`Renderer::take_ray_only`), and the batch puts them where no pass draws from. The
//! block entities the renderer keeps on the GPU have a BLAS a section (step 3, `retained.rs`),
//! their instances after the stream's.

use std::collections::VecDeque;

use glam::DVec3;

use horizon_shader_api::stages::{RAY_ENTITY, RAY_SOLID, RAY_TRANSLUCENT};

use crate::block_entity_render::StaticSectionSnapshot;
use crate::fast_hash::{FxHashMap, FxHashSet};
use crate::gfx::{Renderer, SectionGpu, SectionKey, arena::Arena, resident::Resident};
use crate::mesher::{QuadLook, TerrainVertex, Vertex};
use retained::Retained;

/// BLAS builds per frame at most: a world that just loaded builds over a few frames.
const BUILDS_PER_FRAME: usize = 512;
/// Bytes per vertex in the store.
const STRIDE: u64 = std::mem::size_of::<TerrainVertex>() as u64;
/// Bytes per quad in the look store.
const LOOK: u64 = std::mem::size_of::<QuadLook>() as u64;
/// Vertices the store starts with.
const FIRST_STORE: u64 = 1 << 20;
/// Vertices the store holds at most: an instance's custom data holds its first quad in 24 bits.
const MOST_STORE: u64 = 1 << 26;
/// Chunks around the camera the TLAS spans, unless `HORIZON_RT_CHUNKS` says otherwise. The
/// #1180 spike's TLAS over every section cost 2-7 ms of CPU a frame at render distance 32.
const REACH: i32 = 8;
/// Bytes per vertex of the entity stream.
const ENTITY_STRIDE: u64 = std::mem::size_of::<Vertex>() as u64;
/// What the entity vertex buffer needs on top of its own usages for the entity layer: its BLAS
/// is built from it, and rays read it.
const ENTITY_INPUT: wgpu::BufferUsages =
    wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::BLAS_INPUT);
/// The entity layer's geometries, each a range of the stream: the ordinary quads the camera
/// draws, improved transparency's, and the ray-only ones (`EntityBatch::ray_ranges`).
pub(in crate::gfx) const GEOMETRIES: usize = 3;

/// The renderer's side: the calls gfx.rs makes, each one line there.
impl Renderer {
    /// Section `key` got a new mesh, or lost it.
    pub(in crate::gfx) fn ray_mark(&mut self, key: SectionKey) {
        if let Some(rt) = &mut self.ray_scene {
            rt.mark(key);
        }
    }

    /// Runs `pack` from now on (none: no pack's pipeline), its manifest's metadata gone until
    /// the caller sets the new one. The scene follows it: a pack that declares `[ray_tracing]`
    /// keeps the scene there is, or gets one built from the sections resident now; another pack,
    /// or none, drops it (`query::wanted`: with `HORIZON_RAY_TRACING=1` every pack keeps
    /// one). Called before the pack's first frame, so the scene is there when it gathers what it
    /// traces. A rebuild of the pack that runs keeps what it kept from frame to frame
    /// (`ExternalPipeline::take_over`); else that goes before the new pack's is made.
    /// Why `pack` was refused, if it was (`take_over` found no room for what it keeps): then
    /// nothing changes and the pack that ran keeps running with its metadata, unless it can't,
    /// and then none runs.
    pub(in crate::gfx) fn replace_executor(
        &mut self,
        mut pack: Option<Box<dyn crate::gfx::external::ExternalPipeline>>,
    ) -> Option<String> {
        // A frame still on the submit thread binds the kept resources `take_over` or `retire`
        // destroys: its submission would fail on them.
        self.land_frame();
        let capabilities = pack.as_ref().and_then(|p| p.capabilities());
        // The size the pack's frames draw at (`NativePack::start_frame`), before its first.
        let window = [self.config.width, self.config.height];
        let upscaled = capabilities.is_some_and(|c| c.engine_upscale);
        let size = (self.world_post).pack_render_size(window, upscaled);
        let taken = match (&mut pack, &mut self.external) {
            (Some(new), old) => new.take_over(&self.device, old.as_deref_mut(), size),
            (None, Some(old)) => {
                old.retire(&self.device);
                Ok(())
            }
            (None, None) => Ok(()),
        };
        if let Err(refused) = taken {
            if !refused.old_runs {
                self.replace_executor(None);
            }
            return Some(refused.why);
        }
        let traces = capabilities.is_some_and(|c| c.ray_scene);
        follow(
            &mut self.ray_scene,
            &self.device,
            super::query::wanted(super::query::switch(), traces),
        );
        // TAA's history holds what the pipeline that ran drew (rule 6 of docs/hdr-passes.md).
        if pack.is_some() || self.external.is_some() {
            self.world_post.forget_history();
        }
        // A pack the renderer resolves gets TAA at its world's size from its first frame on,
        // unless TAA may upscale it.
        (self.world_post)
            .hold_upscale(capabilities.is_some_and(|c| c.engine_resolve && !c.engine_upscale));
        // The arenas' looks follow before the next frame (`follow_terrain_looks`).
        self.external = pack;
        self.executor = capabilities;
        self.pack_metadata = None;
        None
    }

    /// Every section is gone (a new world, a new dimension): so are the pack's kept resources.
    pub(in crate::gfx) fn ray_clear(&mut self) {
        if let Some(pipeline) = &mut self.external {
            pipeline.world_cleared();
        }
        if let Some(rt) = &mut self.ray_scene {
            rt.clear();
        }
    }

    /// The frame's BLAS builds and TLAS, recorded in `encoder` after the entity stream's
    /// upload and before the passes that trace (the native shadow pass, the world pass).
    pub(in crate::gfx) fn ray_prepare(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        camera: DVec3,
        frame: &crate::gfx::Frame<'_>,
    ) {
        if self.ray_scene.is_none() || frame.title {
            return;
        }
        let entities = self.traces_entities();
        if entities && !self.entity_buffer.usage().contains(ENTITY_INPUT) {
            // The layer just came on: a buffer with its usages, given this frame's vertices again,
            // since the old one can't be copied from. They are the batch's alone: nothing is posed
            // on the GPU while the layer is on (`skins_entities`).
            self.entity_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Entity vertices"),
                size: self.entity_buffer.size(),
                usage: self.entity_usage(),
                mapped_at_creation: false,
            });
            let vertices = bytemuck::cast_slice(&self.entity_batch.combined);
            self.uploads
                .buffer(encoder, &self.entity_buffer, 0, vertices);
        }
        // The ray-only geometry and a retained section can hold more quads than anything drawn:
        // the shared indices cover them.
        let retained = frame.static_block_entities;
        let most = || (self.entity_batch.ray_ranges[2].1).max(Retained::most_quads(retained));
        if entities && most() > self.index_quads {
            self.index_quads = most().next_power_of_two();
            self.indices = Self::quad_indices(&self.device, self.index_quads);
        }
        let stream = entities.then_some(Stream {
            buffer: &self.entity_buffer,
            atlas: &self.entity_atlas,
            ranges: self.entity_batch.ray_ranges,
            retained,
        });
        let Some(rt) = &mut self.ray_scene else {
            return;
        };
        rt.nether = frame.nether_lighting;
        rt.set_looks(&self.device, self.ray_looks);
        crate::spans::time("ray scene", || {
            rt.prepare(
                &self.device,
                &self.queue,
                encoder,
                (&self.sections, &self.arena),
                self.translucent_arena.as_ref().unwrap_or(&self.arena),
                &self.indices,
                camera,
                stream,
            )
        });
    }

    /// Whether the scene has the entity layer: the device traces, and the native pack's rays
    /// read the layer (its files name `RAY_ENTITY`). The game then names the stream's entities.
    pub fn traces_entities(&self) -> bool {
        self.ray_scene.is_some() && self.executor.is_some_and(|c| c.ray_entities)
    }

    /// The camera-relative box of the columns the scene spans around `camera`'s, while it has the
    /// entity layer: the game generates the entities the camera doesn't draw within it.
    pub fn ray_entity_reach(&self, camera: DVec3) -> Option<crate::gfx::Frustum> {
        let rt = self.ray_scene.as_ref().filter(|_| self.traces_entities())?;
        let (lo, hi) = reach_box(camera, rt.reach);
        Some(crate::gfx::Frustum::boxed(lo, hi))
    }

    /// Takes the frame's ray-only entities (`EntityBatch::ray_only`): their vertices, which the
    /// game moved out of the entity stream, and their runs. Both are left empty.
    pub fn take_ray_only(
        &mut self,
        vertices: &mut Vec<Vertex>,
        runs: &mut crate::gfx::native_entity::EntityRuns,
    ) {
        std::mem::swap(&mut self.entity_batch.ray_only, vertices);
        std::mem::swap(&mut self.entity_batch.ray_kinds, runs);
        vertices.clear();
        runs.clear();
    }

    /// The entity vertex buffer's usages, `ENTITY_INPUT` among them with the entity layer.
    pub(in crate::gfx) fn entity_usage(&self) -> wgpu::BufferUsages {
        let usage = crate::gfx::entity_skin::EntitySkin::usage(self.entity_skin.is_some());
        match self.traces_entities() {
            true => usage | ENTITY_INPUT,
            false => usage,
        }
    }
}

/// The camera-relative corners of the columns within `reach` chunks of `camera`'s, which the
/// TLAS spans (`within`), over their whole height: no world reaches a million blocks.
fn reach_box(camera: DVec3, reach: i32) -> (glam::Vec3, glam::Vec3) {
    let (x, z) = crate::gfx::resident::camera_column(camera);
    let edge = |c: i32, at: f64| (f64::from(c) * 16.0 - at) as f32;
    let lo = glam::Vec3::new(edge(x - reach, camera.x), -1.0e6, edge(z - reach, camera.z));
    let hi = glam::Vec3::new(
        edge(x + reach + 1, camera.x),
        1.0e6,
        edge(z + reach + 1, camera.z),
    );
    (lo, hi)
}

/// What the entity layer is built from this frame: the entity stream's vertex buffer, its
/// texture atlas, and where the batch's ordinary, improved transparency and ray-only quads are
/// in it, as (first quad, quads) (`EntityBatch::ray_ranges`); and the retained block entities'
/// sections.
pub(in crate::gfx) struct Stream<'a> {
    buffer: &'a wgpu::Buffer,
    atlas: &'a wgpu::TextureView,
    ranges: [(u32, u32); GEOMETRIES],
    retained: &'a [StaticSectionSnapshot],
}

/// The entity layer: one BLAS of three geometries, the stream's ranges, read where they are
/// in its vertex buffer (positions first, camera-relative) with the shared quad indices, and
/// built again every frame. wgpu has no refit, and a compacted BLAS can't be built again.
struct Entities {
    blas: wgpu::Blas,
    /// Quads each geometry holds at most.
    sizes: [u32; GEOMETRIES],
    /// Quads of the stream the layer's binding holds (`largest_entity_binding`).
    most: u32,
    buffer: wgpu::Buffer,
    atlas: wgpu::TextureView,
    /// Nearest, as entity.wgsl samples the atlas.
    sampler: wgpu::Sampler,
    /// This frame's geometries, within `most`.
    ranges: [(u32, u32); GEOMETRIES],
    /// Whether the log said the binding holds fewer quads than the stream.
    capped_logged: bool,
    /// The block entities kept on the GPU: a BLAS a section.
    retained: Retained,
}

impl Entities {
    fn new(device: &wgpu::Device, stream: &Stream<'_>) -> Self {
        let sizes = [64; GEOMETRIES];
        Self {
            blas: entity_blas(device, sizes),
            sizes,
            most: largest_entity_binding(&device.limits()),
            buffer: stream.buffer.clone(),
            atlas: stream.atlas.clone(),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Ray tracing: entity atlas"),
                ..Default::default()
            }),
            ranges: [(0, 0); GEOMETRIES],
            capped_logged: false,
            retained: Retained::new(device),
        }
    }

    /// Takes this frame's `stream`: its ranges, as far as the binding holds them, and a BLAS
    /// large enough for them. Whether its buffer is another one than last frame's.
    fn fit(&mut self, device: &wgpu::Device, stream: &Stream<'_>) -> bool {
        let (ranges, capped) = within_binding(stream.ranges, self.most);
        if capped && !std::mem::replace(&mut self.capped_logged, true) {
            log::warn!(
                "Horizon renderer: the ray tracing scene's entity layer holds {} entity quads; \
                 the ones past them let rays through",
                self.most
            );
        }
        self.ranges = ranges;
        if ranges.iter().zip(self.sizes).any(|(r, size)| r.1 > size) {
            self.sizes = ranges.map(|r| r.1.next_power_of_two()).map(|n| n.max(64));
            self.blas = entity_blas(device, self.sizes);
        }
        let other = self.buffer != *stream.buffer;
        self.buffer = stream.buffer.clone();
        other
    }

    /// The geometries' sizes this frame.
    fn built_sizes(&self) -> [wgpu::BlasTriangleGeometrySizeDescriptor; GEOMETRIES] {
        self.ranges.map(|r| geometry_size(r.1))
    }

    /// The build of the BLAS from the stream, with the geometries' `sizes` (`built_sizes`).
    fn build<'a>(
        &'a self,
        sizes: &'a [wgpu::BlasTriangleGeometrySizeDescriptor; GEOMETRIES],
        indices: &'a wgpu::Buffer,
    ) -> wgpu::BlasBuildEntry<'a> {
        let geometry = |i: usize| wgpu::BlasTriangleGeometry {
            size: &sizes[i],
            vertex_buffer: &self.buffer,
            first_vertex: self.ranges[i].0 * 4,
            vertex_stride: ENTITY_STRIDE,
            index_buffer: Some(indices),
            first_index: Some(0),
            transform_buffer: None,
            transform_buffer_offset: None,
        };
        wgpu::BlasBuildEntry {
            blas: &self.blas,
            geometry: wgpu::BlasGeometries::TriangleGeometries(
                (0..GEOMETRIES).map(geometry).collect(),
            ),
        }
    }
}

/// A triangle geometry of `quads` quads: their vertices' positions are three floats, and the
/// shared quad indices make two triangles of each.
fn geometry_size(quads: u32) -> wgpu::BlasTriangleGeometrySizeDescriptor {
    wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: quads * 4,
        index_format: Some(wgpu::IndexFormat::Uint32),
        index_count: Some(quads * 6),
        // Not opaque: rays test the texels themselves.
        flags: wgpu::AccelerationStructureGeometryFlags::empty(),
    }
}

/// The entity layer's BLAS, of three geometries of `sizes` quads at most.
fn entity_blas(device: &wgpu::Device, sizes: [u32; GEOMETRIES]) -> wgpu::Blas {
    device.create_blas(
        &wgpu::CreateBlasDescriptor {
            label: Some("Ray tracing: entities"),
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        },
        wgpu::BlasGeometrySizeDescriptors::Triangles {
            descriptors: sizes.map(geometry_size).to_vec(),
        },
    )
}

/// Quads of the entity stream a binding holds on a device with `limits`: rays read the layer's
/// vertices through one.
fn largest_entity_binding(limits: &wgpu::Limits) -> u32 {
    (limits.max_storage_buffer_binding_size / (ENTITY_STRIDE * 4)).min(u64::from(u32::MAX)) as u32
}

/// `ranges` (first quad, quads) cut to the first `most` quads, and whether any lost quads.
fn within_binding(ranges: [(u32, u32); GEOMETRIES], most: u32) -> ([(u32, u32); GEOMETRIES], bool) {
    let cut = ranges.map(|(first, quads)| {
        let first = first.min(most);
        (first, quads.min(most - first))
    });
    let capped = cut.iter().zip(&ranges).any(|(c, r)| c.1 < r.1);
    (cut, capped)
}

/// Which of a section's quads: the scene has a layer of each, and a ray reads the ones it asks
/// for (the shader API's `trace_layers`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Layer {
    Solid,
    /// What the translucent pass draws of the section: water, stained glass, ice.
    Translucent,
}

impl Layer {
    const BOTH: [Self; 2] = [Self::Solid, Self::Translucent];

    /// Its instances' mask in the TLAS: the bit a ray's cull mask names it by.
    fn mask(self) -> u8 {
        match self {
            Self::Solid => RAY_SOLID as u8,
            Self::Translucent => RAY_TRANSLUCENT as u8,
        }
    }

    /// Section `s`'s quads in it.
    fn quads(self, s: &SectionGpu) -> u32 {
        match self {
            Self::Solid => s.solid,
            Self::Translucent => s.translucent,
        }
    }

    /// Section `s`'s looks of its quads in it, one a quad in the store's order, while the scene
    /// keeps them (`SectionGpu::ray_looks`); else none.
    fn looks(self, s: &SectionGpu) -> &[QuadLook] {
        &s.ray_looks[self as usize]
    }
}

/// A section's quads of one layer: what a BLAS and a slot of the store hold.
type Part = (SectionKey, Layer);

/// The terrain's acceleration structures.
pub struct RayScene {
    /// Each part's BLAS, while it has quads within reach.
    blas: FxHashMap<Part, wgpu::Blas>,
    /// Parts meshed again or emptied since their BLAS was built.
    dirty: Vec<Part>,
    /// Cached slots that need looks without rebuilding their geometry.
    look_retries: VecDeque<Part>,
    look_retry_limit: usize,
    tlas: wgpu::Tlas,
    /// The TLAS's instances of the solid layer, which come first in it.
    solid_instances: u32,
    /// The camera's chunk column when the TLAS was built: its instances are placed relative to
    /// that column's corner, so moving inside it needs no new TLAS.
    anchor: Option<(i32, i32)>,
    /// Chunks around the anchor the TLAS spans.
    reach: i32,
    /// BLAS built this frame, to measure for compaction once submitted.
    to_compact: Vec<(Part, wgpu::Blas)>,
    /// BLAS whose compacted size the GPU is measuring.
    compacting: Vec<(Part, wgpu::Blas)>,
    /// The store, and the pass that writes sections into it.
    unpacker: Unpacker,
    /// Each part's place in the store: exactly the parts with a BLAS.
    slots: Slots,
    /// Whether the log said the store is at its largest.
    full_logged: bool,
    /// Whether the log said the TLAS couldn't grow.
    tlas_logged: bool,
    /// The instances wanted, and the camera's column, when the TLAS last couldn't grow: the
    /// same again doesn't ask video memory again.
    refused: Option<(usize, (i32, i32))>,
    /// A frame kept the old TLAS: the next one that can builds it again.
    rebuild: bool,
    /// How many TLAS and stores were made: a bind group made at another count holds an old one.
    generation: u64,
    /// `Resident::removals` when the scene last dropped the sections no longer resident:
    /// gfx.rs releases sections (unloaded columns, far terrain's fade-out) without `ray_mark`.
    removals: u64,
    /// The TLAS's instances of both terrain layers: the entity layer's instance comes after.
    translucent_instances: u32,
    /// The entity layer, while the native pack's rays read it.
    entities: Option<Entities>,
    /// Where the entity layer's instances of retained block entities start, after the stream's.
    retained_instances: u32,
    /// The frame lights entities as in the Nether (`Frame::nether_lighting`, `ray_prepare`).
    nether: bool,
}

/// What a trace reads this frame (spec #15624).
pub struct Traced<'a> {
    pub tlas: &'a wgpu::Tlas,
    /// How many of its instances are solid terrain's, the first ones: a hit on a later one is
    /// on translucent terrain.
    pub solid_instances: u32,
    /// The sections' vertices: an instance's custom data is its quads' first here.
    pub store: &'a wgpu::Buffer,
    /// The look of each quad of `store`, while the native pack's rays read them.
    pub looks: Option<&'a wgpu::Buffer>,
    /// Where translucent terrain's instances end: the next is the entity layer's.
    pub translucent_instances: u32,
    /// The entity layer, while the native pack's rays read it.
    pub entities: Option<TracedEntities<'a>>,
    /// Changes when `tlas`, `store` or the entity layer's buffer is another object than last
    /// frame's.
    pub generation: u64,
    /// The camera in the TLAS's space: from the anchor column's corner along x and z, the
    /// world's y.
    pub camera: [f32; 3],
    /// Entities are lit as in the Nether: the entity layer's `RayHit.shade`.
    pub nether: bool,
    /// Blocks from the camera, along x and along z, the TLAS spans whichever way one goes.
    pub distance: f32,
}

/// What a trace reads of the entity layer.
pub struct TracedEntities<'a> {
    /// The stream's vertices, as far as a binding holds them.
    pub vertices: wgpu::BufferBinding<'a>,
    pub atlas: &'a wgpu::TextureView,
    pub sampler: &'a wgpu::Sampler,
    /// Each of the BLAS's three geometries' first quad in `vertices`.
    pub bases: [u32; GEOMETRIES],
    /// The retained block entities' quads, and their first instance in the TLAS.
    pub retained: &'a wgpu::Buffer,
    pub retained_instances: u32,
}

impl RayScene {
    /// The scene, when the device has ray queries (`query`) and video memory for the first
    /// TLAS and store.
    pub(in crate::gfx) fn new(device: &wgpu::Device) -> Option<Self> {
        if !super::query::on(device) {
            return None;
        }
        let reach = std::env::var("HORIZON_RT_CHUNKS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(REACH);
        let most = largest_store(&device.limits());
        let slots = Slots::new(FIRST_STORE.min(most), most);
        Some(Self {
            blas: FxHashMap::default(),
            dirty: Vec::new(),
            look_retries: VecDeque::new(),
            look_retry_limit: BUILDS_PER_FRAME,
            tlas: out_of_memory_free(device, || create_tlas(device, 1024))?,
            solid_instances: 0,
            anchor: None,
            reach,
            to_compact: Vec::new(),
            compacting: Vec::new(),
            unpacker: Unpacker::new(device, slots.capacity)?,
            slots,
            full_logged: false,
            tlas_logged: false,
            refused: None,
            rebuild: false,
            generation: 0,
            removals: 0,
            translucent_instances: 0,
            entities: None,
            retained_instances: 0,
            nether: false,
        })
    }

    /// The scene a renderer starts with: one only when `HORIZON_RAY_TRACING=1` asks for it with
    /// any pack or none; otherwise the pack that loads brings it (`Renderer::replace_executor`).
    pub(in crate::gfx) fn forced(device: &wgpu::Device) -> Option<Self> {
        super::query::wanted(super::query::switch(), false)
            .then(|| Self::new(device))
            .flatten()
    }

    /// What a trace from `camera` reads, once this world's first TLAS is built: before that
    /// the TLAS is unbuilt, or still holds the world left.
    pub(in crate::gfx) fn traced(&self, camera: DVec3) -> Option<Traced<'_>> {
        let (camera, distance) = scene_space(camera, self.anchor?, self.reach);
        let entities = self.entities.as_ref().map(|e| TracedEntities {
            vertices: wgpu::BufferBinding {
                buffer: &e.buffer,
                offset: 0,
                size: wgpu::BufferSize::new(
                    e.buffer.size().min(u64::from(e.most) * 4 * ENTITY_STRIDE),
                ),
            },
            atlas: &e.atlas,
            sampler: &e.sampler,
            bases: e.ranges.map(|r| r.0),
            retained: e.retained.store(),
            retained_instances: self.retained_instances,
        });
        Some(Traced {
            tlas: &self.tlas,
            solid_instances: self.solid_instances,
            store: &self.unpacker.store,
            looks: self.unpacker.looks.as_ref(),
            translucent_instances: self.translucent_instances,
            entities,
            generation: self.generation,
            camera,
            nether: self.nether,
            distance,
        })
    }

    /// Keeps the quads' looks (`on`: the native pack's rays read them) or none.
    fn set_looks(&mut self, device: &wgpu::Device, on: bool) {
        if self.unpacker.set_looks(device, on) {
            self.generation += 1;
            self.look_retries.clear();
            self.look_retry_limit = BUILDS_PER_FRAME;
            if on {
                self.look_retries.extend(self.blas.keys().copied());
            }
        }
    }

    fn mark(&mut self, key: SectionKey) {
        self.dirty.extend(Layer::BOTH.map(|layer| (key, layer)));
    }

    fn clear(&mut self) {
        self.slots.clear();
        self.blas.clear();
        self.dirty.clear();
        self.look_retries.clear();
        self.look_retry_limit = BUILDS_PER_FRAME;
        self.to_compact.clear();
        self.compacting.clear();
        self.anchor = None;
    }

    /// Compacts BLAS built on earlier frames: built with room to spare, a compacted copy (made
    /// once the GPU has measured it) takes about a quarter. Whether a BLAS was replaced.
    fn compact(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        let mut changed = false;
        let blas = &mut self.blas;
        self.compacting.retain(|(key, b)| {
            if blas.get(key) != Some(b) {
                return false;
            }
            if !b.ready_for_compaction() {
                return true;
            }
            // Without the memory for the copy the part keeps its uncompacted BLAS.
            if let Some(compact) = out_of_memory_free(device, || queue.compact_blas(b)) {
                blas.insert(*key, compact);
                changed = true;
            }
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

    /// Builds the BLAS of parts marked since the last frame (at most `BUILDS_PER_FRAME`, and as
    /// many as the store has room for), and the TLAS again when a BLAS changed or the camera
    /// entered another chunk column, or every frame with the entity layer (from `stream`).
    /// `translucent` is the arena the sections' translucent quads are in
    /// (`SectionGpu::translucent_at`).
    #[allow(clippy::too_many_arguments)]
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        (sections, arena): (&Resident<SectionGpu>, &Arena),
        translucent: &Arena,
        indices: &wgpu::Buffer,
        camera: DVec3,
        stream: Option<Stream<'_>>,
    ) {
        let retained = stream.as_ref().map_or(&[][..], |s| s.retained);
        match (&mut self.entities, stream) {
            (_, None) => self.entities = None,
            (Some(e), Some(stream)) => {
                if e.fit(device, &stream) {
                    self.generation += 1;
                }
            }
            (None, Some(stream)) => {
                let mut e = Entities::new(device, &stream);
                e.fit(device, &stream);
                self.entities = Some(e);
                self.generation += 1;
            }
        }
        let column = crate::gfx::resident::camera_column(camera);
        if let Some(e) = &mut self.entities
            && e.retained
                .update(device, encoder, retained, column, self.reach)
        {
            self.generation += 1;
        }
        // Only sections within `reach` get a BLAS (at render distance 32 all of them took 1.3 GB
        // of video memory); those past `reach + 2` lose theirs, so walking along a chunk border
        // doesn't build the same ones over and over.
        let keep = self.reach + 2;
        let mut changed = self.compact(device, queue) | std::mem::take(&mut self.rebuild);
        if self.removals != sections.removals() {
            self.removals = sections.removals();
            changed |= drop_released(&mut self.blas, &mut self.slots, |k| sections.contains(k));
        }
        // A frame that keeps the old TLAS takes back what it added here.
        let dirty_before = self.dirty.len();
        if self.anchor != Some(column) {
            changed |= leave(&mut self.blas, &mut self.slots, column, keep);
            for (key, s, _) in sections.iter() {
                if !within(&key, column, self.reach) {
                    continue;
                }
                for layer in Layer::BOTH {
                    if layer.quads(s) > 0 && !self.blas.contains_key(&(key, layer)) {
                        self.dirty.push((key, layer));
                    }
                }
            }
        }
        // Room in the TLAS for every part this frame may add, before any is built: one too small
        // that can't grow keeps the frame's scene as it was (the TLAS with its instances and
        // solid count, the anchor, the store unwritten), and a later frame tries again once the
        // parts wanted or the camera's column change. A cut instance list would put parts on the
        // wrong side of `solid_instances`. The first frame needs no more than the first TLAS
        // holds (`BUILDS_PER_FRAME`), so it never keeps one that was never built.
        let reach = self.reach;
        let near = |part: &Part| within(&part.0, column, reach);
        let entity_layer = self
            .entities
            .as_ref()
            .is_some_and(|e| e.ranges.iter().any(|r| r.1 > 0));
        let retained_wanted = (self.entities.as_ref())
            .map_or(0, |e| e.retained.instances(column, self.reach).count());
        let wanted = instances_wanted(&self.blas, &self.dirty, near)
            + usize::from(entity_layer)
            + retained_wanted;
        let len = self.tlas.get().len();
        let retry = self.refused != Some((wanted, column));
        match tlas_room(&mut self.tlas, len, wanted, |capacity| {
            retry
                .then(|| out_of_memory_free(device, || create_tlas(device, capacity)))
                .flatten()
        }) {
            Some(grown) => {
                self.refused = None;
                changed |= grown;
                self.generation += u64::from(grown);
            }
            None => {
                self.refused = Some((wanted, column));
                if !self.tlas_logged {
                    self.tlas_logged = true;
                    log::warn!(
                        "Horizon renderer: no video memory for a ray tracing TLAS of {wanted} \
                         instances; the scene stays as it is until there is"
                    );
                }
                self.rebuild = true;
                self.dirty.truncate(dirty_before);
                return;
            }
        }
        let mut builds: Vec<Build<'_>> = Vec::new();
        let mut deferred = Vec::new();
        // A part with nothing to build (most sections have no translucent quads) costs a
        // lookup: only the builds count toward the frame's share.
        let mut taken = 0;
        while taken < self.dirty.len() && builds.len() < BUILDS_PER_FRAME {
            let part = self.dirty[taken];
            taken += 1;
            if builds.iter().any(|b| b.part == part) || deferred.contains(&part) {
                continue;
            }
            let (key, layer) = part;
            match sections
                .find(key)
                .filter(|s| layer.quads(s) > 0 && within(&key, column, self.reach))
            {
                Some(s) => {
                    let quads = layer.quads(s);
                    // A part meshed again leaves its old slot: its old BLAS goes from the
                    // TLAS below, before anything traces the part written over it. A store
                    // too small grows here, before this frame's unpack pass writes into it.
                    let old = self.slots.slot(part);
                    let (unpacker, generation) = (&mut self.unpacker, &mut self.generation);
                    let first = self.slots.alloc(part, u64::from(quads) * 4, |vertices| {
                        // Asked only for a larger store: true is a new one.
                        let grown = unpacker.fit(device, encoder, vertices, look_buffer);
                        *generation += u64::from(grown);
                        grown
                    });
                    // Only with a slot: a full store makes no BLAS each frame a part waits.
                    let blas = first.and_then(|_| new_blas(device, quads));
                    if first.is_some() && blas.is_none() {
                        self.slots.restore(part, old);
                    }
                    let (Some((size, blas)), Some(first)) = (blas, first) else {
                        // The store is at its largest, or video memory has no room for the
                        // BLAS: the part keeps its old BLAS and slot, if it has them, and waits
                        // for a later frame.
                        if !self.full_logged {
                            self.full_logged = true;
                            log::warn!(
                                "Horizon renderer: the ray tracing scene is out of room (a store \
                                 of {} vertices); some sections wait for room",
                                self.slots.capacity
                            );
                        }
                        deferred.push(part);
                        continue;
                    };
                    builds.push(Build {
                        part,
                        s,
                        quads,
                        size,
                        blas,
                        first,
                    });
                }
                None => {
                    self.slots.free(part);
                    changed |= self.blas.remove(&part).is_some();
                }
            }
        }
        self.dirty.drain(..taken);
        self.dirty.extend(deferred);
        self.upload_looks(encoder, sections, &builds, |records| {
            stage_looks(device, records)
        });
        if builds.is_empty() && !changed && self.anchor == Some(column) && self.entities.is_none() {
            return;
        }
        self.unpacker
            .run(device, encoder, arena, translucent, &builds);
        for b in &builds {
            self.blas.insert(b.part, b.blas.clone());
            self.to_compact.push((b.part, b.blas.clone()));
        }
        // A layer without quads this frame has no instance: an empty BLAS can't be built.
        let entities = self
            .entities
            .as_ref()
            .filter(|e| e.ranges.iter().any(|r| r.1 > 0));
        let retained = self.entities.as_ref().map(|e| &e.retained);
        let retained_count = retained.map_or(0, |r| r.instances(column, self.reach).count());
        // `instances_wanted` counted these, and more.
        debug_assert!(
            self.blas.keys().filter(|part| near(part)).count()
                + usize::from(entities.is_some())
                + retained_count
                <= self.tlas.get().len()
        );
        let min_y = sections.min_y();
        let mut n = 0;
        // The solid layer's instances first: a hit tells its layer by its instance's index.
        for layer in Layer::BOTH {
            let parts = self.blas.iter();
            for (part, blas) in parts.filter(|(part, _)| part.1 == layer && near(part)) {
                let o = instance_origin(part.0, min_y, column);
                // Below `MOST_STORE / 4`, so it fits the custom data's 24 bits.
                let first_quad = self.slots.first(*part).map_or(0, |at| at / 4) as u32;
                self.tlas[n] = Some(wgpu::TlasInstance::new(
                    blas,
                    [
                        1.0, 0.0, 0.0, o[0], 0.0, 1.0, 0.0, o[1], 0.0, 0.0, 1.0, o[2],
                    ],
                    first_quad,
                    layer.mask(),
                ));
                n += 1;
            }
            if layer == Layer::Solid {
                self.solid_instances = n as u32;
            }
        }
        self.translucent_instances = n as u32;
        if let Some(e) = entities {
            // The stream's vertices are camera-relative: the camera's place in the TLAS's space.
            let ([x, y, z], _) = scene_space(camera, column, self.reach);
            let transform = [1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, y, 0.0, 0.0, 1.0, z];
            let mask = RAY_ENTITY as u8;
            self.tlas[n] = Some(wgpu::TlasInstance::new(&e.blas, transform, 0, mask));
            n += 1;
        }
        self.retained_instances = n as u32;
        let retained = self.entities.as_ref().map(|e| &e.retained);
        for instance in retained
            .into_iter()
            .flat_map(|r| r.instances(column, self.reach))
        {
            self.tlas[n] = Some(instance);
            n += 1;
        }
        for i in n..self.tlas.get().len() {
            if self.tlas[i].is_none() {
                break;
            }
            self.tlas[i] = None;
        }
        let mut entries = build_entries(&builds, &self.unpacker.store, indices);
        let sizes = entities.map(Entities::built_sizes);
        if let (Some(e), Some(sizes)) = (entities, &sizes) {
            entries.push(e.build(sizes, indices));
        }
        entries.extend(retained.into_iter().flat_map(|r| r.builds(indices)));
        encoder.build_acceleration_structures(entries.iter(), std::iter::once(&self.tlas));
        self.anchor = Some(column);
    }

    fn upload_looks(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        sections: &Resident<SectionGpu>,
        builds: &[Build<'_>],
        mut stage: impl FnMut(&[QuadLook]) -> Option<wgpu::Buffer>,
    ) {
        let Some(looks) = self.unpacker.looks.as_ref() else {
            return;
        };
        let mut retries = Vec::new();
        if !self.look_retries.is_empty() {
            // A dirty section's records may belong to a replacement mesh. Its normal build
            // will write the looks once the new geometry can take its slot.
            let pending: FxHashSet<_> = self
                .dirty
                .iter()
                .copied()
                .chain(builds.iter().map(|b| b.part))
                .collect();
            self.look_retries.retain(|part| !pending.contains(part));
            let count = self.look_retries.len().min(self.look_retry_limit);
            retries.extend(
                self.look_retries
                    .drain(..count)
                    .filter_map(|part @ (key, layer)| {
                        let section = sections.find(key)?;
                        let (first, vertices) = self.slots.slot(part)?;
                        let quads = layer.quads(section);
                        (vertices == u64::from(quads) * 4).then_some(LookWrite {
                            part,
                            first,
                            quads,
                            section,
                        })
                    }),
            );
        }
        // Stage retries independently of fresh builds so a steady stream of new sections
        // cannot keep a small retry from fitting.
        let fresh: Vec<_> = builds.iter().map(LookWrite::from).collect();
        for writes in [&retries[..], &fresh[..]] {
            if write_looks(encoder, looks, writes, &mut stage) {
                continue;
            }
            let failed = writes
                .iter()
                .filter(|w| w.records().is_some())
                .map(|w| w.part);
            // Retry smaller batches and put failed parts at the back so a large
            // part cannot prevent smaller ones from getting their looks back.
            let before = self.look_retries.len();
            self.look_retries.extend(failed);
            let count = self.look_retries.len() - before;
            self.look_retry_limit = self.look_retry_limit.min((count / 2).max(1));
        }
        if self.look_retries.is_empty() {
            self.look_retry_limit = BUILDS_PER_FRAME;
        }
    }
}

/// `scene` for a pack that `wants` one: the one there is, or a new one; none otherwise.
fn follow(scene: &mut Option<RayScene>, device: &wgpu::Device, wants: bool) {
    if !wants {
        *scene = None;
    } else if scene.is_none() {
        *scene = RayScene::new(device);
    }
}

/// Drops the BLAS and slot of every part whose section is not `resident`; whether any went.
fn drop_released<B>(
    blas: &mut FxHashMap<Part, B>,
    slots: &mut Slots,
    resident: impl Fn(SectionKey) -> bool,
) -> bool {
    let before = blas.len();
    blas.retain(|&part, _| {
        let kept = resident(part.0);
        if !kept {
            slots.free(part);
        }
        kept
    });
    blas.len() != before
}

/// Drops from `blas` both layers of the sections past `keep` chunks of column `column`, and
/// frees their slots. Whether any went.
fn leave<T>(
    blas: &mut FxHashMap<Part, T>,
    slots: &mut Slots,
    column: (i32, i32),
    keep: i32,
) -> bool {
    let before = blas.len();
    blas.retain(|part, _| {
        let kept = within(&part.0, column, keep);
        if !kept {
            slots.free(*part);
        }
        kept
    });
    blas.len() != before
}

/// Instances the TLAS may need this frame at most: the parts with a BLAS `near` the camera, and
/// as many again as the `dirty` parts without one that the frame may build.
fn instances_wanted<B>(
    blas: &FxHashMap<Part, B>,
    dirty: &[Part],
    near: impl Fn(&Part) -> bool,
) -> usize {
    let built = blas.keys().filter(|part| near(part)).count();
    let new = dirty.iter().filter(|part| !blas.contains_key(part)).count();
    built + new.min(BUILDS_PER_FRAME)
}

/// Room in `tlas` (`len` instances) for `wanted`: it as it is, or a larger one from `make`
/// (whether it was replaced). None, with `tlas` as it was, when `make` has no memory for it.
fn tlas_room<T>(
    tlas: &mut T,
    len: usize,
    wanted: usize,
    make: impl FnOnce(u32) -> Option<T>,
) -> Option<bool> {
    if wanted <= len {
        return Some(false);
    }
    *tlas = make((wanted as u32).next_power_of_two())?;
    Some(true)
}

/// Vertices the store may grow to on a device with `limits`: the unpack pass binds it whole.
fn largest_store(limits: &wgpu::Limits) -> u64 {
    let bytes = limits
        .max_buffer_size
        .min(limits.max_storage_buffer_binding_size);
    MOST_STORE.min(bytes / STRIDE / 4 * 4)
}

/// Whether section `key` is within `reach` chunks of column `column`, along x and z.
fn within(key: &SectionKey, column: (i32, i32), reach: i32) -> bool {
    (key.0 - column.0).abs() <= reach && (key.2 - column.1).abs() <= reach
}

/// A BLAS to build this frame.
struct Build<'a> {
    part: Part,
    s: &'a SectionGpu,
    /// The part's quads.
    quads: u32,
    size: wgpu::BlasTriangleGeometrySizeDescriptor,
    blas: wgpu::Blas,
    /// Its first vertex in the store.
    first: u64,
}

/// `rt_unpack.wgsl`'s `RtJob`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(in crate::gfx) struct RtJob {
    pub base: u32,
    pub quads: u32,
    pub out: u32,
    pub pad: u32,
}

/// The store, and the pass that writes sections' solid quads into it as vertices: BLAS builds
/// and rays read vertices, while the arena's compact layout holds records only the draws
/// expand. Translucent quads are vertices wherever they are kept, and are copied.
struct Unpacker {
    pipeline: wgpu::ComputePipeline,
    store: wgpu::Buffer,
    /// The look of each quad of `store`, while the native pack's rays read them (`set_looks`):
    /// a part's are written with its vertices, from its section's copy. It grows with `store`.
    looks: Option<wgpu::Buffer>,
    /// Whether the log said the look store couldn't be made, since it last was.
    looks_logged: bool,
}

impl Unpacker {
    /// None without the video memory for the first store.
    fn new(device: &wgpu::Device, capacity: u64) -> Option<Self> {
        // The store first: a scene that is asked for again each frame without room for it must
        // not compile the pipeline each time.
        let store = store_buffer(device, capacity)?;
        Some(Self {
            pipeline: unpack_pipeline(device),
            store,
            looks: None,
            looks_logged: false,
        })
    }

    /// A look store beside the store (`on`) or none; whether that changed. Without the video
    /// memory for one the rays read every quad as without a look, and a later frame asks again.
    /// The scene fills slots already resident once the store is available.
    fn set_looks(&mut self, device: &wgpu::Device, on: bool) -> bool {
        let before = self.looks.is_some();
        if on != before {
            self.looks = on
                .then(|| look_buffer(device, self.store.size() / STRIDE))
                .flatten();
            if on && self.looks.is_none() {
                self.log_look_refusal();
            } else {
                self.looks_logged = false;
            }
        }
        self.looks.is_some() != before
    }

    fn log_look_refusal(&mut self) {
        if !std::mem::replace(&mut self.looks_logged, true) {
            log::warn!(
                "Horizon renderer: no video memory for the ray tracing look store; \
                 hits use vertex colours until it can be allocated"
            );
        }
    }

    /// A store of `vertices` when it holds fewer: the larger one gets the old contents first
    /// (recorded before this frame's unpack pass), so every slot keeps its place. The old one
    /// lives on in `encoder` until it is submitted. False, with the store as it was, when video
    /// memory has no room for the larger one.
    fn fit(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        vertices: u64,
        grow_looks: impl FnOnce(&wgpu::Device, u64) -> Option<wgpu::Buffer>,
    ) -> bool {
        if vertices * STRIDE <= self.store.size() {
            return true;
        }
        let Some(store) = store_buffer(device, vertices) else {
            log::warn!(
                "Horizon renderer: no video memory for a ray tracing store of {vertices} \
                 vertices; it stays at {}",
                self.store.size() / STRIDE
            );
            return false;
        };
        encoder.copy_buffer_to_buffer(&self.store, 0, &store, 0, self.store.size());
        self.store = store;
        if let Some(old) = self.looks.take() {
            self.looks = grow_looks(device, vertices);
            if let Some(new) = &self.looks {
                encoder.copy_buffer_to_buffer(&old, 0, new, 0, old.size());
            } else {
                // Optional looks cannot set the geometry store's permanent capacity limit.
                self.log_look_refusal();
            }
        }
        true
    }

    /// Writes the builds' quads into their slots: the translucent ones a copy each out of
    /// `translucent`, the solid ones one dispatch per page of `arena`.
    fn run(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        arena: &Arena,
        translucent: &Arena,
        builds: &[Build<'_>],
    ) {
        let (builds, copied): (Vec<_>, Vec<_>) =
            builds.iter().partition(|b| b.part.1 == Layer::Solid);
        for b in copied {
            let (page, at) = b.s.translucent_at;
            encoder.copy_buffer_to_buffer(
                translucent.buffer(page),
                at * STRIDE,
                &self.store,
                b.first * STRIDE,
                u64::from(b.quads) * 4 * STRIDE,
            );
        }
        let mut pages: Vec<u32> = builds.iter().map(|b| b.s.page).collect();
        pages.sort_unstable();
        pages.dedup();
        if pages.is_empty() {
            return;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Ray tracing: unpack sections"),
            timestamp_writes: None,
        });
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
            unpack_page(
                device,
                &mut pass,
                &self.pipeline,
                arena.buffer(page),
                &jobs,
                &self.store,
            );
        }
    }
}

/// The unpacking pipeline (`rt_unpack.wgsl`); `[displacement]` writes its quads with it too
/// (gfx/displace.rs).
pub(in crate::gfx) fn unpack_pipeline(device: &wgpu::Device) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        // No colon: DXC (DirectX 12) takes the label for a file name and fails to read it.
        label: Some("Unpack sections"),
        source: wgpu::ShaderSource::Wgsl(unpack_shader().into()),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Ray tracing: unpack sections"),
        layout: None,
        module: &module,
        entry_point: Some("cs_rt_unpack"),
        compilation_options: Default::default(),
        cache: None,
    })
}

/// Writes `jobs`' solid quads, in the arena page `page`, out as vertices into `out`, in `pass`.
pub(in crate::gfx) fn unpack_page(
    device: &wgpu::Device,
    pass: &mut wgpu::ComputePass<'_>,
    pipeline: &wgpu::ComputePipeline,
    page: &wgpu::Buffer,
    jobs: &[RtJob],
    out: &wgpu::Buffer,
) {
    let empty = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[],
    });
    let jobs_buffer = wgpu::util::DeviceExt::create_buffer_init(
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("Ray tracing: unpack jobs"),
            contents: bytemuck::cast_slice(jobs),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let arena_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(1),
        entries: &[wgpu::BindGroupEntry {
            binding: 19,
            resource: page.as_entire_binding(),
        }],
    });
    let out_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(2),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: jobs_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: out.as_entire_binding(),
            },
        ],
    });
    let most = jobs.iter().map(|j| j.quads * 4).max().unwrap_or(0);
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &empty, &[]);
    pass.set_bind_group(1, &arena_group, &[]);
    pass.set_bind_group(2, &out_group, &[]);
    pass.dispatch_workgroups(most.div_ceil(64), jobs.len() as u32, 1);
}

/// terrain.wgsl with the unpacking entry point after it, which reads sections as its solid
/// draws do (`section_corner`).
pub(in crate::gfx) fn unpack_shader() -> String {
    crate::gfx::terrain_shader() + include_str!("../../rt_unpack.wgsl")
}

struct LookWrite<'a> {
    part: Part,
    first: u64,
    quads: u32,
    section: &'a SectionGpu,
}

impl<'a> From<&Build<'a>> for LookWrite<'a> {
    fn from(build: &Build<'a>) -> Self {
        Self {
            part: build.part,
            first: build.first,
            quads: build.quads,
            section: build.s,
        }
    }
}

impl LookWrite<'_> {
    fn records(&self) -> Option<&[QuadLook]> {
        let looks = self.part.1.looks(self.section);
        (looks.len() == self.quads as usize).then_some(looks)
    }
}

/// Writes the looks of new builds and cached slots. A refused staging allocation clears the
/// affected slots before they can be traced and leaves their records eligible for retry.
fn write_looks(
    encoder: &mut wgpu::CommandEncoder,
    looks: &wgpu::Buffer,
    writes: &[LookWrite<'_>],
    stage: impl FnOnce(&[QuadLook]) -> Option<wgpu::Buffer>,
) -> bool {
    let all: Vec<QuadLook> = writes
        .iter()
        .filter_map(LookWrite::records)
        .flatten()
        .copied()
        .collect();
    let staged = (!all.is_empty()).then(|| stage(&all)).flatten();
    let mut at = 0;
    for write in writes {
        let (offset, size) = (write.first / 4 * LOOK, u64::from(write.quads) * LOOK);
        match staged.as_ref().filter(|_| write.records().is_some()) {
            Some(staged) => {
                encoder.copy_buffer_to_buffer(staged, at, looks, offset, size);
                at += size;
            }
            None => encoder.clear_buffer(looks, offset, Some(size)),
        }
    }
    all.is_empty() || staged.is_some()
}

fn stage_looks(device: &wgpu::Device, records: &[QuadLook]) -> Option<wgpu::Buffer> {
    let bytes = bytemuck::cast_slice(records);
    let buffer = out_of_memory_free(device, || {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Ray tracing: looks to write"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        })
    })?;
    buffer.get_mapped_range_mut(..).ok()?.copy_from_slice(bytes);
    buffer.unmap();
    Some(buffer)
}

/// The look store of a store of `vertices`: a look a quad.
fn look_buffer(device: &wgpu::Device, vertices: u64) -> Option<wgpu::Buffer> {
    crate::gfx::arena::try_create(
        device,
        "Ray tracing: section looks",
        vertices / 4 * LOOK,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
    )
}

fn store_buffer(device: &wgpu::Device, vertices: u64) -> Option<wgpu::Buffer> {
    crate::gfx::arena::try_create(
        device,
        "Ray tracing: section vertices",
        vertices * STRIDE,
        wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::BLAS_INPUT
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
    )
}

/// What `make` makes, or None when video memory had no room for it: an out-of-memory error
/// raised outside a scope is fatal (`arena::try_create`).
fn out_of_memory_free<T>(device: &wgpu::Device, make: impl FnOnce() -> T) -> Option<T> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let made = make();
    // Native error scopes resolve at once.
    pollster::block_on(scope.pop()).is_none().then_some(made)
}

fn create_tlas(device: &wgpu::Device, capacity: u32) -> wgpu::Tlas {
    device.create_tlas(&wgpu::CreateTlasDescriptor {
        label: Some("Terrain sections"),
        max_instances: capacity,
        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
    })
}

/// A new BLAS for `quads` quads, and its size; None without the video memory for it.
fn new_blas(
    device: &wgpu::Device,
    quads: u32,
) -> Option<(wgpu::BlasTriangleGeometrySizeDescriptor, wgpu::Blas)> {
    let size = wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: quads * 4,
        index_format: Some(wgpu::IndexFormat::Uint32),
        index_count: Some(quads * 6),
        // Not opaque: rays test cut-out texels (leaves, plants) and clear ones themselves.
        flags: wgpu::AccelerationStructureGeometryFlags::empty(),
    };
    let blas = out_of_memory_free(device, || {
        device.create_blas(
            &wgpu::CreateBlasDescriptor {
                label: None,
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE
                    | wgpu::AccelerationStructureFlags::ALLOW_COMPACTION,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            },
            wgpu::BlasGeometrySizeDescriptors::Triangles {
                descriptors: vec![size.clone()],
            },
        )
    })?;
    Some((size, blas))
}

/// Build entries reading each part's vertices in the store (`pos` is `TerrainVertex`'s first
/// field) with the shared quad indices.
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

/// Where each part's vertices sit in the store: first fit over the free ranges, which merge
/// with their neighbours when freed. A store with no range to fit grows, up to `most` and as far
/// as video memory lets it. The retained block entities' store keeps its sections by their key
/// (`retained.rs`).
struct Slots<K = Part> {
    taken: FxHashMap<K, (u64, u64)>,
    /// Free (first vertex, vertices), in order, none touching another.
    free: Vec<(u64, u64)>,
    /// Vertices the store holds.
    capacity: u64,
    most: u64,
}

impl<K: Copy + Eq + std::hash::Hash> Slots<K> {
    fn new(capacity: u64, most: u64) -> Self {
        Self {
            taken: FxHashMap::default(),
            free: vec![(0, capacity)],
            capacity,
            most,
        }
    }

    /// Part `key`'s first vertex.
    fn first(&self, key: K) -> Option<u64> {
        self.taken.get(&key).map(|s| s.0)
    }

    /// Part `key`'s slot: first vertex and vertices.
    fn slot(&self, key: K) -> Option<(u64, u64)> {
        self.taken.get(&key).copied()
    }

    /// Puts part `key` back in `old`, its slot (if any) before `alloc` moved it.
    fn restore(&mut self, key: K, old: Option<(u64, u64)>) {
        self.free(key);
        if let Some(old) = old {
            self.take(old);
            self.taken.insert(key, old);
        }
    }

    /// A slot of `vertices` for part `key` in place of its old one, the first that fits, in
    /// a store twice as large (at least large enough) when none does: `grow` makes that store,
    /// and false from it (no video memory) makes the current size the largest. None at the
    /// largest: `key` then keeps its old slot.
    fn alloc(&mut self, key: K, vertices: u64, grow: impl FnOnce(u64) -> bool) -> Option<u64> {
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
            if capacity > self.capacity && !grow(capacity) {
                self.most = self.capacity;
            } else if capacity > self.capacity {
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

    fn free(&mut self, key: K) {
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

/// `camera` in the space of a TLAS anchored at chunk column `anchor` (worked out in doubles:
/// far from the world's origin a float holds a position to centimetres), and how far from it
/// along x and along z a TLAS of `reach` chunks around that column spans at least.
fn scene_space(camera: DVec3, anchor: (i32, i32), reach: i32) -> ([f32; 3], f32) {
    let x = camera.x - f64::from(anchor.0) * 16.0;
    let z = camera.z - f64::from(anchor.1) * 16.0;
    // The columns span from `reach` chunks before the anchor's corner to `reach + 1` after it.
    let (low, high) = (f64::from(reach) * 16.0, f64::from(reach + 1) * 16.0);
    let distance = (x + low).min(high - x).min(z + low).min(high - z);
    (
        [x as f32, camera.y as f32, z as f32],
        distance.max(0.0) as f32,
    )
}

/// Where section `key` sits relative to the corner of chunk column `anchor`.
fn instance_origin(key: SectionKey, min_y: i32, anchor: (i32, i32)) -> [f32; 3] {
    [
        ((key.0 - anchor.0) * 16) as f32,
        (min_y + key.1 * 16) as f32,
        ((key.2 - anchor.1) * 16) as f32,
    ]
}

#[path = "retained.rs"]
mod retained;

#[cfg(test)]
#[path = "entity_tests.rs"]
pub(in crate::gfx) mod entity_tests;

#[cfg(test)]
#[path = "terrain_tests.rs"]
mod terrain_tests;

#[cfg(test)]
#[path = "looks_tests.rs"]
pub(in crate::gfx) mod looks_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instances_sit_relative_to_the_anchor_column() {
        // Section 5 of column (3, -2), the world starting at y = -64, seen from column (2, -2).
        assert_eq!(instance_origin((3, 5, -2), -64, (2, -2)), [16.0, 16.0, 0.0]);
        assert_eq!(instance_origin((2, 0, -2), -64, (2, -2)), [0.0, -64.0, 0.0]);
    }

    #[test]
    fn the_camera_sits_in_the_anchor_column_s_space_far_from_the_origin() {
        // 30 million blocks out, 3.25 and 12.5 blocks into column (1_875_000, -1_875_000).
        let camera = DVec3::new(30_000_003.25, 70.5, -29_999_987.5);
        let (at, distance) = scene_space(camera, (1_875_000, -1_875_000), 8);
        assert_eq!(at, [3.25, 70.5, 12.5]);
        // Nearest edge: 8 chunks and 3.25 blocks west; east it is 9 chunks less 3.25.
        assert_eq!(distance, 131.25);
        // A reach of no chunks is the anchor column alone.
        assert_eq!(scene_space(camera, (1_875_000, -1_875_000), 0).1, 3.25);
    }

    const A: Part = ((0, 0, 0), Layer::Solid);
    const B: Part = ((1, 0, 0), Layer::Solid);
    const C: Part = ((2, 0, 0), Layer::Solid);
    /// Section A's translucent quads.
    const A_CLEAR: Part = ((0, 0, 0), Layer::Translucent);

    /// A store that video memory lets grow.
    fn grows(_: u64) -> bool {
        true
    }

    #[test]
    fn freed_slots_merge_and_are_taken_first_fit() {
        let mut slots = Slots::new(64, 64);
        assert_eq!(slots.alloc(A, 16, grows), Some(0));
        assert_eq!(slots.alloc(B, 16, grows), Some(16));
        assert_eq!(slots.alloc(C, 16, grows), Some(32));
        slots.free(B);
        slots.free(A);
        // 32 fit only where A's and B's slots merged; the 16 left at the end come next.
        assert_eq!(slots.alloc(A, 32, grows), Some(0));
        assert_eq!(slots.alloc(B, 8, grows), Some(48));
        assert_eq!(slots.capacity, 64);
    }

    #[test]
    fn sections_no_longer_resident_leave_the_scene_and_the_store() {
        let mut slots = Slots::new(64, 64);
        let mut blas = FxHashMap::default();
        for key in [A, B, C] {
            slots.alloc(key, 8, grows);
            blas.insert(key, ());
        }
        // B's column was unloaded without a new mesh.
        assert!(drop_released(&mut blas, &mut slots, |k| k != B.0));
        assert_eq!(blas.len(), 2);
        assert!(!blas.contains_key(&B));
        assert_eq!(slots.first(B), None);
        assert_eq!(slots.alloc(B, 8, grows), Some(8), "its slot is free again");
        assert!(!drop_released(&mut blas, &mut slots, |_| true));
    }

    #[test]
    fn a_section_meshed_again_leaves_its_old_slot() {
        let mut slots = Slots::new(64, 64);
        slots.alloc(A, 8, grows);
        slots.alloc(B, 8, grows);
        assert_eq!(slots.alloc(A, 4, grows), Some(0));
        // Its old 8 are free again, but 16 don't fit before B.
        assert_eq!(slots.alloc(A, 16, grows), Some(16));
        assert_eq!(slots.alloc(C, 8, grows), Some(0));
        assert_eq!(slots.first(B), Some(8));
    }

    #[test]
    fn a_section_s_translucent_quads_come_and_go_beside_its_solid_ones() {
        let mut slots = Slots::new(64, 64);
        assert_eq!(slots.alloc(A, 16, grows), Some(0));
        assert_eq!(slots.alloc(A_CLEAR, 8, grows), Some(16));
        assert_eq!(slots.alloc(B, 8, grows), Some(24));
        // Water placed in the section: both parts are meshed again, each into a slot of its own.
        assert_eq!(slots.alloc(A, 16, grows), Some(0));
        assert_eq!(slots.alloc(A_CLEAR, 12, grows), Some(32));
        // The water is gone: its slot goes, the solid quads stay where they are.
        slots.free(A_CLEAR);
        assert_eq!((slots.first(A), slots.first(A_CLEAR)), (Some(0), None));
        assert_eq!(slots.alloc(C, 32, grows), Some(32));
        // Only translucent quads left (a section of water alone): they take the solid ones' room.
        slots.free(A);
        assert_eq!(slots.alloc(A_CLEAR, 16, grows), Some(0));
        assert_eq!((slots.first(A), slots.first(B)), (None, Some(24)));
    }

    #[test]
    fn the_translucent_layer_grows_the_store_and_waits_at_its_cap() {
        const B_CLEAR: Part = ((1, 0, 0), Layer::Translucent);
        let mut slots = Slots::new(16, 48);
        assert_eq!(slots.alloc(A, 16, grows), Some(0));
        assert_eq!(slots.alloc(A_CLEAR, 8, grows), Some(16));
        assert_eq!(slots.capacity, 32);
        assert_eq!(slots.alloc(B, 8, grows), Some(24));
        // Grown to its largest, the store has 16 free: B's water waits, and nothing moved.
        assert_eq!(slots.alloc(B_CLEAR, 32, grows), None);
        assert_eq!(slots.capacity, 48);
        let firsts = [A, A_CLEAR, B, B_CLEAR].map(|p| slots.first(p));
        assert_eq!(firsts, [Some(0), Some(16), Some(24), None]);
        // A's water spreads: no room for it either, so it keeps its old slot.
        assert_eq!(slots.alloc(A_CLEAR, 24, grows), None);
        assert_eq!(slots.first(A_CLEAR), Some(16));
        // Another world: every slot of both layers is free.
        slots.clear();
        assert_eq!(slots.alloc(B_CLEAR, 48, grows), Some(0));
    }

    #[test]
    fn a_section_leaving_reach_takes_both_its_layers() {
        const FAR: Part = ((20, 0, 0), Layer::Solid);
        const FAR_CLEAR: Part = ((20, 0, 0), Layer::Translucent);
        let mut slots = Slots::new(64, 64);
        let mut blas = FxHashMap::default();
        for (part, vertices) in [(A, 8), (A_CLEAR, 8), (FAR, 16), (FAR_CLEAR, 8)] {
            slots.alloc(part, vertices, grows);
            blas.insert(part, ());
        }
        assert!(leave(&mut blas, &mut slots, (1, 0), 10));
        let mut kept: Vec<Part> = blas.keys().copied().collect();
        kept.sort_by_key(|p| p.1 == Layer::Translucent);
        assert_eq!(kept, [A, A_CLEAR]);
        assert_eq!((slots.first(FAR), slots.first(FAR_CLEAR)), (None, None));
        // Their 24 vertices are free again, after A's two slots.
        assert_eq!(slots.alloc(C, 48, grows), Some(16));
        assert!(!leave(&mut blas, &mut slots, (1, 0), 10));
    }

    #[test]
    fn the_store_s_cap_comes_from_the_device_s_limits() {
        let mut limits = wgpu::Limits {
            max_buffer_size: 1 << 40,
            max_storage_buffer_binding_size: 1 << 30,
            ..Default::default()
        };
        // A gibibyte of 24-byte vertices, whole quads of them.
        assert_eq!(STRIDE, 24);
        assert_eq!(largest_store(&limits), 44_739_240);
        limits.max_storage_buffer_binding_size = 1 << 40;
        assert_eq!(largest_store(&limits), MOST_STORE);
    }

    #[test]
    fn a_full_store_grows_and_every_slot_stays() {
        let mut slots = Slots::new(16, 1 << 26);
        assert_eq!(slots.alloc(A, 16, grows), Some(0));
        assert_eq!(slots.alloc(B, 8, grows), Some(16));
        assert_eq!(slots.capacity, 32);
        // Larger than twice the store: the store takes it whole after its free 8.
        assert_eq!(slots.alloc(C, 100, grows), Some(24));
        assert_eq!(slots.capacity, 132);
        assert_eq!((slots.first(A), slots.first(B)), (Some(0), Some(16)));
    }

    #[test]
    fn at_the_largest_store_a_section_waits_in_its_old_slot() {
        let mut slots = Slots::new(16, 32);
        slots.alloc(A, 16, grows);
        slots.alloc(B, 16, grows);
        assert_eq!(slots.alloc(C, 4, grows), None);
        assert_eq!(slots.alloc(A, 20, grows), None);
        assert_eq!(slots.first(A), Some(0));
        // Nothing was lost on the way: A's slot was taken back.
        assert_eq!(slots.alloc(C, 4, grows), None);
        slots.free(B);
        assert_eq!(slots.alloc(A, 20, grows), Some(0));
        slots.clear();
        assert_eq!(slots.alloc(C, 32, grows), Some(0));
    }

    #[test]
    fn a_store_video_memory_cant_grow_keeps_every_slot() {
        let mut slots = Slots::new(16, 1 << 26);
        slots.alloc(A, 8, grows);
        slots.alloc(A_CLEAR, 8, grows);
        let mut asked = Vec::new();
        assert_eq!(
            slots.alloc(B, 12, |n| {
                asked.push(n);
                false
            }),
            None
        );
        assert_eq!(asked, [32]);
        assert_eq!((slots.first(A), slots.first(A_CLEAR)), (Some(0), Some(8)));
        assert_eq!(slots.capacity, 16);
        // That size is now the largest: no part asks again, and freed room is used.
        assert_eq!(slots.alloc(C, 4, |_| unreachable!()), None);
        slots.free(A_CLEAR);
        assert_eq!(slots.alloc(B, 8, |_| unreachable!()), Some(8));
    }

    #[test]
    fn a_part_whose_blas_failed_goes_back_to_its_old_slot() {
        let mut slots = Slots::new(64, 64);
        slots.alloc(A, 8, grows);
        slots.alloc(A_CLEAR, 8, grows);
        slots.alloc(B, 8, grows);
        // A's water spreads; its new slot is given, then its BLAS fails.
        let old = slots.slot(A_CLEAR);
        assert_eq!(slots.alloc(A_CLEAR, 16, grows), Some(24));
        slots.restore(A_CLEAR, old);
        assert_eq!(slots.slot(A_CLEAR), Some((8, 8)));
        // The 16 it was given are free again, merged with the rest.
        assert_eq!(slots.alloc(C, 40, grows), Some(24));
        // A part new to the scene goes back to none.
        let new = ((3, 0, 0), Layer::Translucent);
        slots.free(C);
        assert_eq!(slots.alloc(new, 8, grows), Some(24));
        slots.restore(new, None);
        assert_eq!(slots.first(new), None);
        assert_eq!(slots.alloc(C, 40, grows), Some(24));
    }

    #[test]
    fn a_tlas_that_cant_grow_stays_as_it_was() {
        // Instances in TLAS order: the solid ones first.
        let mut tlas = vec![A, B, A_CLEAR];
        let mut asked = Vec::new();
        let mut no_memory = |n| {
            asked.push(n);
            None
        };
        assert_eq!(tlas_room(&mut tlas, 4, 5, &mut no_memory), None);
        assert_eq!(
            (tlas.as_slice(), asked.as_slice()),
            ([A, B, A_CLEAR].as_slice(), [8].as_slice())
        );
        // Room enough: nothing is made.
        assert_eq!(tlas_room(&mut tlas, 4, 4, |_| unreachable!()), Some(false));
        assert_eq!(
            tlas_room(&mut tlas, 4, 5, |n| Some(vec![C; n as usize])),
            Some(true)
        );
        assert_eq!(tlas.len(), 8);
    }

    #[test]
    fn the_tlas_makes_room_for_both_layers_before_building() {
        let mut blas = FxHashMap::default();
        for part in [A, A_CLEAR, ((20, 0, 0), Layer::Solid)] {
            blas.insert(part, ());
        }
        let near = |part: &Part| within(&part.0, (0, 0), 8);
        // A and its water are built and near; B's two layers and A's again may come.
        let dirty = [B, ((1, 0, 0), Layer::Translucent), A_CLEAR];
        assert_eq!(instances_wanted(&blas, &dirty, near), 4);
        let many = vec![C; BUILDS_PER_FRAME + 10];
        assert_eq!(instances_wanted(&blas, &many, near), 2 + BUILDS_PER_FRAME);
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
        let _gpu = crate::gfx::test_gpu::lock();
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let Ok((device, _)) = pollster::block_on(adapter.request_device(&Default::default()))
        else {
            return;
        };
        assert!(!super::super::query::on(&device));
        assert!(RayScene::new(&device).is_none());
        assert!(super::super::query::blas_input(&device).is_empty());
        assert!(super::super::query::arena_input(&device).is_empty());
    }

    /// Spec #16011 item 5: the scene follows the pack that loads. A pack with `[ray_tracing]`
    /// gets one and keeps it across its reloads; another pack drops it, and the next tracing one
    /// starts afresh from the sections resident then. `HORIZON_RAY_TRACING=1` keeps one without.
    #[test]
    fn the_scene_follows_a_pack_that_traces() {
        use super::super::query::{Switch, experimental, pick, wanted};
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let features = pick(true, adapter.features());
        let desc = wgpu::DeviceDescriptor {
            required_features: features,
            required_limits: adapter.limits(),
            experimental_features: experimental(features),
            ..Default::default()
        };
        let Ok((device, _)) = pollster::block_on(adapter.request_device(&desc)) else {
            return;
        };
        if features.is_empty() {
            return;
        }
        let mut scene = None;
        follow(&mut scene, &device, wanted(Switch::Pack, false));
        assert!(scene.is_none());
        follow(&mut scene, &device, wanted(Switch::Pack, true));
        scene.as_mut().unwrap().mark(A.0);
        follow(&mut scene, &device, wanted(Switch::Pack, true));
        // A marked section waits in both layers, the solid one and the translucent one.
        assert_eq!(scene.as_ref().map(|s| s.dirty.len()), Some(2));
        follow(&mut scene, &device, wanted(Switch::Pack, false));
        assert!(scene.is_none());
        follow(&mut scene, &device, wanted(Switch::Pack, true));
        let fresh = scene.as_ref().unwrap();
        assert!(fresh.dirty.is_empty() && fresh.anchor.is_none());
        follow(&mut scene, &device, wanted(Switch::Off, true));
        assert!(scene.is_none());
        follow(&mut scene, &device, wanted(Switch::On, false));
        assert!(scene.is_some());
    }
}
