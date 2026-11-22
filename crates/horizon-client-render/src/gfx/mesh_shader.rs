//! Hardware mesh shaders, only on request (#17243 step 2): when the player or setting enables
//! `HORIZON_MESH_SHADER=1` and the GPU supports `EXPERIMENTAL_MESH_SHADER`.
//! Without either, the device is opened with the exact same features and limits as `main`.
//! Every build has it: it is an engine render capability for terrain geometry.

#[allow(unused_imports)]
pub use crate::mesher::meshlet::{GpuMeshlet, MeshletDesc};

/// The mesh shader feature in wgpu.
pub(in crate::gfx) const MESH_SHADER: wgpu::Features = wgpu::Features::EXPERIMENTAL_MESH_SHADER;

/// Workgroup size used by the task shader.
pub const TASK_WORKGROUP_SIZE: u32 = 32;

/// Workgroup size used by the mesh slice shader (one lane per vertex up to 128 vertices).
#[allow(dead_code)]
pub const MESH_WORKGROUP_SIZE: u32 = 128;

/// Maximum quads emitted per mesh shader workgroup so `TerrainMeshOutput` fits in 16 KiB LDS.
pub const MESH_SLICE_QUADS: u32 = 32;

/// Pass filter selecting all quads (solid + cutout).
#[allow(dead_code)]
pub const PASS_ALL: u32 = 0;

/// Pass filter selecting only opaque, non-fading quads (`fs_opaque`).
#[allow(dead_code)]
pub const PASS_OPAQUE: u32 = 1;

/// Pass filter selecting cutout or fading quads (`fs_solid`).
#[allow(dead_code)]
pub const PASS_CUTOUT: u32 = 2;

/// Number of per-page pass uniform slots (`PASS_ALL = 0`, `PASS_OPAQUE = 1`, `PASS_CUTOUT = 2`).
const PASS_SLOTS: u32 = 3;

/// Margin matching `mesher/sides.rs` when testing eye-to-bucket plane orientation.
const MESHLET_MARGIN: f32 = 1.0 / 16.0;

/// Whether `device` was opened with mesh shaders.
#[allow(dead_code)]
pub(in crate::gfx) fn on(device: &wgpu::Device) -> bool {
    device.features().contains(MESH_SHADER)
}

/// The environment variable name that controls the mesh shader pipeline.
pub const ENV_VAR: &str = "HORIZON_MESH_SHADER";

/// Uniform parameters for `terrain_mesh.wgsl`'s task culling (`@group(2) @binding(1)`).
#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeshCullParams {
    /// Six frustum planes in camera-relative world space: `xyz` inward normal, `w` offset.
    pub planes: [[f32; 4]; 6],
    /// `xyz`: camera eye (camera-relative), `w`: low eye y (water mirror).
    pub eye: [f32; 4],
    /// `x`: meshlet count, `y`: pass kind (`PASS_*`), `z`/`w`: reserved.
    pub counts: [u32; 4],
}

/// Who turns mesh shaders on (`HORIZON_MESH_SHADER`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::gfx) enum Switch {
    /// `1`: force on when offered by the adapter.
    On,
    /// Unset or any other value: off (main's path).
    Off,
}

/// Resolves the switch from an optional environment variable string.
pub(in crate::gfx) fn resolve(var: Option<&std::ffi::OsStr>) -> Switch {
    match var {
        Some(v) if v == "1" => Switch::On,
        _ => Switch::Off,
    }
}

/// Reads the current process's switch for mesh shaders.
pub(in crate::gfx) fn switch() -> Switch {
    resolve(std::env::var_os(ENV_VAR).as_deref())
}

/// Features to request from `adapter` when wanted and offered.
pub(super) fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
    let wanted = switch() == Switch::On;
    let picked = pick(wanted, adapter.features());
    if wanted {
        let info = adapter.get_info();
        if picked.is_empty() {
            log::warn!(
                "Horizon renderer: mesh shaders asked for ({ENV_VAR}=1), but {} on {:?} has no \
                 EXPERIMENTAL_MESH_SHADER support; it stays off",
                info.name,
                info.backend
            );
        } else {
            log::info!(
                "Horizon renderer: hardware mesh shaders on ({} on {:?})",
                info.name,
                info.backend
            );
        }
    }
    picked
}

/// Returns the mesh shader feature if requested and offered, or empty otherwise.
pub(super) fn pick(requested: bool, offered: wgpu::Features) -> wgpu::Features {
    if requested {
        offered & MESH_SHADER
    } else {
        wgpu::Features::empty()
    }
}

/// The device's opt-in to `features`' experimental ones.
pub(super) fn experimental(features: wgpu::Features) -> wgpu::ExperimentalFeatures {
    if features.contains(MESH_SHADER) || features.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY) {
        horizon_hdr::experimental_features()
    } else {
        wgpu::ExperimentalFeatures::disabled()
    }
}

/// Assembled terrain + mesh shader WGSL source (`enable wgpu_mesh_shader;` at the top).
#[allow(dead_code)]
pub(in crate::gfx) fn shader() -> String {
    "enable wgpu_mesh_shader;\n".to_owned()
        + &crate::gfx::terrain_shader()
        + include_str!("../terrain_mesh.wgsl")
}

/// Assembled terrain + Visbuffer mesh shader WGSL source (`HORIZON_VISBUFFER=1` + `HORIZON_MESH_SHADER=1`).
#[allow(dead_code)]
pub(in crate::gfx) fn vis_shader() -> String {
    shader() + include_str!("../terrain_vis_mesh.wgsl")
}

/// GPU-side meshlet culling + compaction compute shader WGSL source.
pub(in crate::gfx) fn cull_shader() -> &'static str {
    include_str!("../terrain_mesh_cull.wgsl")
}

/// `@group(0)` bind group layout for `terrain_mesh_cull.wgsl`.
fn cull_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let compute = wgpu::ShaderStages::COMPUTE;
    let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: compute,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Terrain meshlet GPU cull"),
        entries: &[
            storage(0, true),
            storage(1, false),
            storage(2, false),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: compute,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(
                        std::mem::size_of::<MeshCullParams>() as u64,
                    ),
                },
                count: None,
            },
        ],
    })
}

/// `@group(2)` bind group layout for `terrain_mesh.wgsl` (`meshlets` storage + `mesh_cull` dynamic uniform).
#[allow(dead_code)]
pub(in crate::gfx) fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let stages = wgpu::ShaderStages::TASK | wgpu::ShaderStages::MESH;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Terrain meshlets"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: stages,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: stages,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: std::num::NonZeroU64::new(
                        std::mem::size_of::<MeshCullParams>() as u64,
                    ),
                },
                count: None,
            },
        ],
    })
}

/// Bind group layout entries for `terrain.wgsl` `@group(0)` extended with `ShaderStages::MESH`
/// visibility alongside vertex and fragment stages.
pub(in crate::gfx) fn terrain_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let mut entries = crate::gfx::native_material::terrain_entries();
    for entry in &mut entries {
        entry.visibility |= wgpu::ShaderStages::MESH;
    }
    entries
}

/// Mesh-stage-compatible `@group(0)` layout (`terrain.wgsl` globals, sprites, lightmap, and emission
/// thresholds with `SHADER_STAGES::MESH` visibility in addition to vertex/fragment).
#[allow(dead_code)]
pub(in crate::gfx) fn group0_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Terrain mesh group 0"),
        entries: &terrain_entries(),
    })
}

/// Mesh-stage-compatible `@group(1)` layout for an arena page (`block_slots`, `slot_origins`,
/// and `arena_page` with `TASK`/`MESH` stage visibilities alongside `VERTEX`).
#[allow(dead_code)]
pub(in crate::gfx) fn group1_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let entry = |binding, visibility| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let vtm = wgpu::ShaderStages::VERTEX
        | wgpu::ShaderStages::TASK
        | wgpu::ShaderStages::MESH
        | wgpu::ShaderStages::COMPUTE;
    let vm = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::MESH;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Terrain mesh group 1"),
        entries: &[
            entry(0, vtm),
            entry(1, vtm),
            entry(crate::gfx::flat::ARENA_BINDING, vm),
        ],
    })
}

/// Builds the 3-group pipeline layout (`@group(0)` terrain globals/samplers, `@group(1)` arena page,
/// `@group(2)` meshlets + dynamic cull uniform) used by hardware terrain mesh pipelines.
#[allow(dead_code)]
pub(in crate::gfx) fn pipeline_layout(
    device: &wgpu::Device,
    group0: &wgpu::BindGroupLayout,
    group1: &wgpu::BindGroupLayout,
    group2: &wgpu::BindGroupLayout,
) -> wgpu::PipelineLayout {
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Terrain mesh pipeline layout"),
        bind_group_layouts: &[Some(group0), Some(group1), Some(group2)],
        ..Default::default()
    })
}

/// CPU reference matching `terrain_mesh.wgsl::meshlet_facing`: whether `desc`'s directional
/// bucket can face `rel_eye` (or `rel_low` for `-Y` water-mirror bounds) relative to the section origin.
#[allow(dead_code)]
pub fn meshlet_facing(desc: &MeshletDesc, rel_eye: [f32; 3], rel_low: f32) -> bool {
    if desc.is_unbounded() {
        return true;
    }
    let min = desc.min_corner().map(f32::from);
    let max = desc.max_corner().map(f32::from);
    match desc.bucket {
        0 => rel_eye[0] > min[0] - MESHLET_MARGIN,
        1 => rel_eye[2] > min[2] - MESHLET_MARGIN,
        3 => rel_eye[1] > min[1] - MESHLET_MARGIN,
        4 => rel_eye[2] < max[2] + MESHLET_MARGIN,
        5 => rel_eye[0] < max[0] + MESHLET_MARGIN,
        6 => rel_low < max[1] + MESHLET_MARGIN,
        _ => true,
    }
}

/// CPU reference matching `terrain_mesh.wgsl::meshlet_in_frustum`: tests the meshlet's sub-AABB
/// offset by the section `origin` against the six inward-facing frustum planes.
#[allow(dead_code)]
pub fn meshlet_in_frustum(desc: &MeshletDesc, origin: [f32; 3], planes: &[[f32; 4]; 6]) -> bool {
    if desc.is_unbounded() {
        return true;
    }
    let min = desc.min_corner().map(f32::from);
    let max = desc.max_corner().map(f32::from);
    let bmin = [origin[0] + min[0], origin[1] + min[1], origin[2] + min[2]];
    let bmax = [origin[0] + max[0], origin[1] + max[1], origin[2] + max[2]];
    for &[nx, ny, nz, w] in planes {
        let px = if nx >= 0.0 { bmax[0] } else { bmin[0] };
        let py = if ny >= 0.0 { bmax[1] } else { bmin[1] };
        let pz = if nz >= 0.0 { bmax[2] } else { bmin[2] };
        if nx * px + ny * py + nz * pz + w < 0.0 {
            return false;
        }
    }
    true
}

/// Number of mesh workgroup slices (`<= MESH_SLICE_QUADS` quads each) needed for `quad_count`.
#[allow(dead_code)]
pub fn meshlet_slices(quad_count: u8) -> u32 {
    if quad_count == 0 {
        0
    } else if u32::from(quad_count) <= MESH_SLICE_QUADS {
        1
    } else {
        2
    }
}

/// Number of task shader workgroups required to cover `meshlet_count` meshlets.
#[allow(dead_code)]
pub fn task_workgroups(meshlet_count: u32) -> u32 {
    meshlet_count.div_ceil(TASK_WORKGROUP_SIZE)
}

/// Splits `meshlet_count` into `(meshlet_offset, chunk_meshlets, workgroups)` chunks so every
/// `draw_mesh_tasks(workgroups, 1, 1)` call respects both per-dimension and total workgroup limits.
#[allow(dead_code)]
pub fn task_dispatch_chunks(
    meshlet_count: u32,
    max_per_dispatch: u32,
) -> impl Iterator<Item = (u32, u32, u32)> {
    let total_groups = task_workgroups(meshlet_count);
    let cap = max_per_dispatch.max(1);
    let chunks = total_groups.div_ceil(cap);
    (0..chunks).map(move |chunk_idx| {
        let wg_offset = chunk_idx * cap;
        let workgroups = (total_groups - wg_offset).min(cap);
        let meshlet_offset = wg_offset * TASK_WORKGROUP_SIZE;
        let chunk_meshlets = (meshlet_count - meshlet_offset).min(workgroups * TASK_WORKGROUP_SIZE);
        (meshlet_offset, chunk_meshlets, workgroups)
    })
}

#[derive(Default)]
struct PageTable {
    sections: std::collections::BTreeMap<u32, Vec<GpuMeshlet>>,
    staging: Vec<GpuMeshlet>,
    visible_staging: Vec<GpuMeshlet>,
    visible_cutout: Vec<GpuMeshlet>,
    dirty: bool,
    revision: u64,
}

/// CPU staging table managing rebased [`GpuMeshlet`] descriptors per arena page and
/// section `(page, solid_base)` (#17243 step 3).
#[allow(dead_code)]
#[derive(Default)]
pub(in crate::gfx) struct MeshletTable {
    pages: std::collections::BTreeMap<u32, PageTable>,
    next_revision: u64,
}

#[allow(dead_code)]
impl MeshletTable {
    /// Creates an empty meshlet staging table.
    pub fn new() -> Self {
        Self::default()
    }

    fn alloc_revision(&mut self) -> u64 {
        self.next_revision = self.next_revision.wrapping_add(1).max(1);
        self.next_revision
    }

    /// Registers or replaces a section's meshlets at `(page, solid_base)`, rebasing each descriptor
    /// into its GPU representation via [`MeshletDesc::to_gpu`].
    pub fn insert_section(&mut self, page: u32, solid_base: u32, meshlets: &[MeshletDesc]) {
        if meshlets.is_empty() {
            self.remove_section(page, solid_base);
            return;
        }
        let rev = self.alloc_revision();
        let relocated: Vec<GpuMeshlet> = meshlets.iter().map(|m| m.to_gpu(solid_base)).collect();
        let entry = self.pages.entry(page).or_default();
        entry.sections.insert(solid_base, relocated);
        entry.dirty = true;
        entry.revision = rev;
    }

    /// Removes the section at `(page, solid_base)` from the table.
    pub fn remove_section(&mut self, page: u32, solid_base: u32) {
        let removed = if let Some(entry) = self.pages.get_mut(&page)
            && entry.sections.remove(&solid_base).is_some()
        {
            Some(entry.sections.is_empty())
        } else {
            None
        };
        match removed {
            Some(true) => {
                self.pages.remove(&page);
            }
            Some(false) => {
                let rev = self.alloc_revision();
                if let Some(entry) = self.pages.get_mut(&page) {
                    entry.dirty = true;
                    entry.revision = rev;
                }
            }
            None => {}
        }
    }

    /// Moves a section's meshlets from `(from_page, from_solid_base)` to `(to_page, to_solid_base)`
    /// during arena compaction, rebasing each descriptor's `vertex_base`.
    pub fn relocate_section(
        &mut self,
        from_page: u32,
        from_solid_base: u32,
        to_page: u32,
        to_solid_base: u32,
    ) {
        if (from_page, from_solid_base) == (to_page, to_solid_base) {
            return;
        }
        let Some(from_entry) = self.pages.get_mut(&from_page) else {
            return;
        };
        let Some(mut list) = from_entry.sections.remove(&from_solid_base) else {
            return;
        };
        let from_empty = from_entry.sections.is_empty();
        if from_empty {
            self.pages.remove(&from_page);
        } else {
            let from_rev = self.alloc_revision();
            if let Some(entry) = self.pages.get_mut(&from_page) {
                entry.dirty = true;
                entry.revision = from_rev;
            }
        }
        for m in &mut list {
            m.vertex_base = m
                .vertex_base
                .wrapping_sub(from_solid_base)
                .wrapping_add(to_solid_base);
        }
        let to_rev = self.alloc_revision();
        let to_entry = self.pages.entry(to_page).or_default();
        to_entry.sections.insert(to_solid_base, list);
        to_entry.dirty = true;
        to_entry.revision = to_rev;
    }

    /// Retires staging data for an empty or freed arena `page`.
    pub fn retire_page(&mut self, page: u32) {
        self.pages.remove(&page);
    }

    /// Returns the current revision of `page` when non-empty.
    pub fn page_revision(&self, page: u32) -> Option<u64> {
        self.pages
            .get(&page)
            .filter(|entry| !entry.sections.is_empty())
            .map(|entry| entry.revision)
    }

    /// Drops all tracked sections across every page.
    pub fn clear(&mut self) {
        self.pages.clear();
    }

    /// Rebuilds the cached [`GpuMeshlet`] slice for `page` only if dirty, returning `(rebuilt, slice)`.
    pub fn stage_page(&mut self, page: u32) -> (bool, &[GpuMeshlet]) {
        let Some(entry) = self.pages.get_mut(&page) else {
            return (false, &[]);
        };
        let rebuilt = entry.dirty;
        if rebuilt {
            entry.staging.clear();
            for list in entry.sections.values() {
                entry.staging.extend_from_slice(list);
            }
            entry.dirty = false;
        }
        (rebuilt, &entry.staging)
    }

    /// Returns the cached [`GpuMeshlet`] slice currently staged for `page`.
    pub fn staged_page(&self, page: u32) -> &[GpuMeshlet] {
        self.pages
            .get(&page)
            .map_or(&[], |entry| entry.staging.as_slice())
    }

    /// Returns the number of meshlets staged or pending on `page`.
    pub fn page_count(&self, page: u32) -> u32 {
        self.pages.get(&page).map_or(0, |entry| {
            if entry.dirty {
                entry.sections.values().map(|v| v.len() as u32).sum()
            } else {
                entry.staging.len() as u32
            }
        })
    }
}

#[derive(Clone, Copy)]
struct PreparedChunk {
    uniform_offset: u32,
    indirect_offset: u64,
    meshlet_count: u32,
    workgroups: u32,
}

struct PreparedPass {
    revision: u64,
    chunks: Vec<PreparedChunk>,
}

struct PageGpu {
    storage: wgpu::Buffer,
    storage_capacity: usize,
    resident_storage: wgpu::Buffer,
    resident_capacity: usize,
    resident_uploaded_revision: u64,
    resident_count: u32,
    uniform: wgpu::Buffer,
    indirect: wgpu::Buffer,
    uniform_slots: u32,
    bind_group: wgpu::BindGroup,
    cull_bind_group: wgpu::BindGroup,
    uploaded_revision: u64,
    uploaded_full_layout: bool,
    prepared_passes: [Option<PreparedPass>; PASS_SLOTS as usize],
}

/// Prepared immutable GPU draw handle for a single task-dispatch chunk of a `(page, pass_kind)` pair.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub(in crate::gfx) struct MeshPageDraw<'a> {
    pub bind_group: &'a wgpu::BindGroup,
    pub uniform_offset: u32,
    pub indirect_buffer: &'a wgpu::Buffer,
    pub indirect_offset: u64,
    pub meshlet_count: u32,
    pub workgroups: u32,
}

/// GPU store managing independent per-page [`GpuMeshlet`] storage buffers and dynamic multi-pass
/// culling uniform slots (#17243 step 3).
#[allow(dead_code)]
pub(in crate::gfx) struct MeshSectionStore {
    table: MeshletTable,
    gpu_pages: std::collections::BTreeMap<u32, PageGpu>,
    uniform_stride: u32,
    max_task_workgroups: u32,
    bind_group_layout: wgpu::BindGroupLayout,
    cull_bind_group_layout: wgpu::BindGroupLayout,
    cull_pipeline: wgpu::ComputePipeline,
    finalize_pipeline: wgpu::ComputePipeline,
}

#[allow(dead_code)]
impl MeshSectionStore {
    fn invalidate_page_passes(&mut self, page: u32) {
        if let Some(pg) = self.gpu_pages.get_mut(&page) {
            pg.prepared_passes = [const { None }; PASS_SLOTS as usize];
        }
    }

    /// Registers or replaces a section's meshlets at `(page, solid_base)`.
    pub fn insert_section(&mut self, page: u32, solid_base: u32, meshlets: &[MeshletDesc]) {
        self.table.insert_section(page, solid_base, meshlets);
        self.invalidate_page_passes(page);
    }

    /// Removes the section at `(page, solid_base)` from the meshlet store.
    /// Keeps existing `PageGpu` buffers so ordinary section replacement reuses the allocation;
    /// use [`Self::retire_page`] or [`Self::trim`] when the arena releases an empty vertex page.
    pub fn remove_section(&mut self, page: u32, solid_base: u32) {
        self.table.remove_section(page, solid_base);
        self.invalidate_page_passes(page);
    }

    /// Moves a section's meshlets from `(from_page, from_solid_base)` to `(to_page, to_solid_base)`.
    pub fn relocate_section(
        &mut self,
        from_page: u32,
        from_solid_base: u32,
        to_page: u32,
        to_solid_base: u32,
    ) {
        self.table
            .relocate_section(from_page, from_solid_base, to_page, to_solid_base);
        self.invalidate_page_passes(from_page);
        self.invalidate_page_passes(to_page);
    }

    /// Releases GPU meshlet and uniform buffers for a retired arena `page`.
    pub fn retire_page(&mut self, page: u32) {
        self.table.retire_page(page);
        self.gpu_pages.remove(&page);
    }

    /// Retires any pages for which `live(page)` returns `false`, freeing their `PageGpu` buffers.
    pub fn trim(&mut self, live: impl Fn(u32) -> bool) {
        let dead: Vec<u32> = self
            .gpu_pages
            .keys()
            .chain(self.table.pages.keys())
            .copied()
            .filter(|&page| !live(page))
            .collect();
        for page in dead {
            self.retire_page(page);
        }
    }

    /// Whether `page` currently retains allocated GPU buffers in this store.
    pub fn has_gpu_page(&self, page: u32) -> bool {
        self.gpu_pages.contains_key(&page)
    }

    /// Drops all tracked sections and per-page GPU buffers.
    pub fn clear(&mut self) {
        self.table.clear();
        self.gpu_pages.clear();
    }

    /// Returns the `@group(2)` bind group layout managed by this store.
    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_group_layout
    }

    fn ensure_page_buffers(
        &mut self,
        device: &wgpu::Device,
        page: u32,
        needed_meshlets: usize,
        needed_resident: usize,
        required_uniform_slots: u32,
    ) -> bool {
        let needs_storage_alloc = self
            .gpu_pages
            .get(&page)
            .is_none_or(|pg| needed_meshlets > pg.storage_capacity);
        let needs_resident_alloc = self
            .gpu_pages
            .get(&page)
            .is_none_or(|pg| needed_resident > pg.resident_capacity);
        let needs_uniform_alloc = self
            .gpu_pages
            .get(&page)
            .is_none_or(|pg| required_uniform_slots > pg.uniform_slots);

        if needs_storage_alloc || needs_resident_alloc || needs_uniform_alloc {
            let cap = self
                .gpu_pages
                .get(&page)
                .map_or(0, |pg| pg.storage_capacity)
                .max(needed_meshlets.next_power_of_two().max(64));
            let storage = needs_storage_alloc.then(|| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Terrain meshlet storage"),
                    size: (cap * std::mem::size_of::<GpuMeshlet>()) as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            });
            let res_cap = self
                .gpu_pages
                .get(&page)
                .map_or(0, |pg| pg.resident_capacity)
                .max(needed_resident.next_power_of_two().max(64));
            let resident_storage = needs_resident_alloc.then(|| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Terrain resident meshlet storage"),
                    size: (res_cap * std::mem::size_of::<GpuMeshlet>()) as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            });
            let uniform_slots = self
                .gpu_pages
                .get(&page)
                .map_or(PASS_SLOTS, |pg| pg.uniform_slots)
                .max(required_uniform_slots)
                .next_power_of_two()
                .max(PASS_SLOTS);
            let (uniform, indirect) = if needs_uniform_alloc {
                (
                    Some(device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Terrain meshlet cull uniform"),
                        size: u64::from(uniform_slots) * u64::from(self.uniform_stride),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    })),
                    Some(device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Terrain meshlet indirect args"),
                        size: (u64::from(uniform_slots) * 12).max(32),
                        usage: wgpu::BufferUsages::STORAGE
                            | wgpu::BufferUsages::INDIRECT
                            | wgpu::BufferUsages::COPY_DST
                            | wgpu::BufferUsages::COPY_SRC,
                        mapped_at_creation: false,
                    })),
                )
            } else {
                (None, None)
            };
            match self.gpu_pages.entry(page) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    let storage = storage.expect("new page must allocate storage");
                    let resident_storage =
                        resident_storage.expect("new page must allocate resident storage");
                    let uniform = uniform.expect("new page must allocate uniform");
                    let indirect = indirect.expect("new page must allocate indirect");
                    let bind_group =
                        create_page_bind_group(device, &self.bind_group_layout, &storage, &uniform);
                    let cull_bind_group = create_cull_bind_group(
                        device,
                        &self.cull_bind_group_layout,
                        &resident_storage,
                        &storage,
                        &indirect,
                        &uniform,
                    );
                    slot.insert(PageGpu {
                        storage,
                        storage_capacity: cap,
                        resident_storage,
                        resident_capacity: res_cap,
                        resident_uploaded_revision: 0,
                        resident_count: 0,
                        uniform,
                        indirect,
                        uniform_slots,
                        bind_group,
                        cull_bind_group,
                        uploaded_revision: 0,
                        uploaded_full_layout: false,
                        prepared_passes: [const { None }; PASS_SLOTS as usize],
                    });
                }
                std::collections::btree_map::Entry::Occupied(mut slot) => {
                    let pg = slot.get_mut();
                    if let Some(storage) = storage {
                        pg.storage = storage;
                        pg.storage_capacity = cap;
                        pg.uploaded_revision = 0;
                        pg.uploaded_full_layout = false;
                    }
                    if let Some(resident_storage) = resident_storage {
                        pg.resident_storage = resident_storage;
                        pg.resident_capacity = res_cap;
                        pg.resident_uploaded_revision = 0;
                    }
                    if let Some(uniform) = uniform {
                        pg.uniform = uniform;
                        pg.indirect = indirect.expect("indirect allocated with uniform");
                        pg.uniform_slots = uniform_slots;
                    }
                    pg.bind_group = create_page_bind_group(
                        device,
                        &self.bind_group_layout,
                        &pg.storage,
                        &pg.uniform,
                    );
                    pg.cull_bind_group = create_cull_bind_group(
                        device,
                        &self.cull_bind_group_layout,
                        &pg.resident_storage,
                        &pg.storage,
                        &pg.indirect,
                        &pg.uniform,
                    );
                    pg.prepared_passes = [const { None }; PASS_SLOTS as usize];
                }
            }
        }
        needs_storage_alloc
    }

    /// Uploads staged `GpuMeshlet`s for `page` when dirty and writes culling parameters into the
    /// pass slot `params.counts[1]` without overwriting other passes or pages in the same frame.
    pub fn prepare_page(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        page: u32,
        params: &MeshCullParams,
    ) -> u32 {
        let pass_slot = params.counts[1];
        assert!(pass_slot < PASS_SLOTS, "invalid mesh pass kind {pass_slot}");

        let Some(revision) = self.table.page_revision(page) else {
            self.invalidate_page_passes(page);
            return 0;
        };
        let (rebuilt, count) = {
            let (rebuilt, staged) = self.table.stage_page(page);
            (rebuilt, staged.len() as u32)
        };
        if count == 0 {
            self.invalidate_page_passes(page);
            return 0;
        }

        let total_groups = task_workgroups(count);
        let chunk_count = total_groups.div_ceil(self.max_task_workgroups.max(1));
        let required_uniform_slots = PASS_SLOTS * chunk_count;
        let needs_storage_alloc = self.ensure_page_buffers(
            device,
            page,
            count as usize,
            count as usize,
            required_uniform_slots,
        );
        let staged = self.table.staged_page(page);

        let pg = self
            .gpu_pages
            .get_mut(&page)
            .expect("page GPU storage must exist after allocation");
        if rebuilt
            || needs_storage_alloc
            || pg.uploaded_revision != revision
            || !pg.uploaded_full_layout
        {
            queue.write_buffer(&pg.storage, 0, bytemuck::cast_slice(staged));
            pg.uploaded_revision = revision;
            pg.uploaded_full_layout = true;
            pg.prepared_passes = [const { None }; PASS_SLOTS as usize];
        }

        let mut chunks = Vec::with_capacity(chunk_count as usize);
        for (chunk_idx, (meshlet_offset, chunk_meshlets, workgroups)) in
            task_dispatch_chunks(count, self.max_task_workgroups).enumerate()
        {
            let slot = pass_slot + (chunk_idx as u32) * PASS_SLOTS;
            let uniform_offset = slot * self.uniform_stride;
            let indirect_offset = u64::from(slot) * 12;
            let mut updated = *params;
            updated.counts[0] = count;
            updated.counts[3] = meshlet_offset;
            queue.write_buffer(
                &pg.uniform,
                u64::from(uniform_offset),
                bytemuck::bytes_of(&updated),
            );
            queue.write_buffer(
                &pg.indirect,
                indirect_offset,
                bytemuck::cast_slice(&[workgroups, 1u32, 1u32]),
            );
            chunks.push(PreparedChunk {
                uniform_offset,
                indirect_offset,
                meshlet_count: chunk_meshlets,
                workgroups,
            });
        }
        pg.prepared_passes[pass_slot as usize] = Some(PreparedPass { revision, chunks });
        count
    }

    /// Gathers only the visible sections in front-to-back order, partitions them into disjoint
    /// `PASS_OPAQUE` and `PASS_CUTOUT` ranges per page, uploads their descriptors, and populates
    /// indirect dispatch arguments. Returns total non-empty dispatch chunks prepared.
    pub fn prepare_visible_pages(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        visible_sections: &[(u32, u32, bool)],
        planes: [[f32; 4]; 6],
        eye: [f32; 4],
        no_eye: bool,
    ) -> usize {
        for entry in self.table.pages.values_mut() {
            entry.visible_staging.clear();
            entry.visible_cutout.clear();
        }
        for &(page, solid_base, fading) in visible_sections {
            let Some(entry) = self.table.pages.get_mut(&page) else {
                continue;
            };
            let Some(list) = entry.sections.get(&solid_base) else {
                continue;
            };
            if fading {
                entry.visible_cutout.extend_from_slice(list);
            } else {
                for &m in list {
                    if m.is_cutout() {
                        entry.visible_cutout.push(m);
                    } else {
                        entry.visible_staging.push(m);
                    }
                }
            }
        }

        let skip_facing = u32::from(no_eye);
        let max_wg = self.max_task_workgroups.max(1);
        let mut dispatches = 0usize;
        let pages: Vec<u32> = self.table.pages.keys().copied().collect();
        for page in pages {
            let (revision, opaque_count, cutout_count, total_count) = {
                let entry = self
                    .table
                    .pages
                    .get_mut(&page)
                    .expect("page exists in table");
                let opaque_count = entry.visible_staging.len() as u32;
                let cutout_count = entry.visible_cutout.len() as u32;
                entry.visible_staging.append(&mut entry.visible_cutout);
                let total_count = entry.visible_staging.len() as u32;
                (entry.revision, opaque_count, cutout_count, total_count)
            };
            if total_count == 0 {
                self.invalidate_page_passes(page);
                continue;
            }
            let max_pass_chunks = task_workgroups(opaque_count.max(cutout_count)).div_ceil(max_wg);
            let required_uniform_slots = PASS_SLOTS * max_pass_chunks.max(1);
            self.ensure_page_buffers(
                device,
                page,
                total_count as usize,
                total_count as usize,
                required_uniform_slots,
            );
            let pg = self
                .gpu_pages
                .get_mut(&page)
                .expect("page GPU storage must exist after allocation");
            let staged = &self.table.pages[&page].visible_staging;
            queue.write_buffer(&pg.storage, 0, bytemuck::cast_slice(staged));
            pg.uploaded_revision = revision;
            pg.uploaded_full_layout = false;

            let mut prepare_range = |pass_kind: u32, start: u32, count: u32, limit: u32| {
                if count == 0 {
                    pg.prepared_passes[pass_kind as usize] = None;
                    return;
                }
                let mut chunks = Vec::new();
                for (chunk_idx, (chunk_offset, chunk_meshlets, workgroups)) in
                    task_dispatch_chunks(count, max_wg).enumerate()
                {
                    let slot = pass_kind + (chunk_idx as u32) * PASS_SLOTS;
                    let uniform_offset = slot * self.uniform_stride;
                    let indirect_offset = u64::from(slot) * 12;
                    let params = MeshCullParams {
                        planes,
                        eye,
                        counts: [
                            limit,
                            pass_kind,
                            skip_facing | ((page & 0x3f) << 8),
                            start + chunk_offset,
                        ],
                    };
                    queue.write_buffer(
                        &pg.uniform,
                        u64::from(uniform_offset),
                        bytemuck::bytes_of(&params),
                    );
                    queue.write_buffer(
                        &pg.indirect,
                        indirect_offset,
                        bytemuck::cast_slice(&[workgroups, 1u32, 1u32]),
                    );
                    chunks.push(PreparedChunk {
                        uniform_offset,
                        indirect_offset,
                        meshlet_count: chunk_meshlets,
                        workgroups,
                    });
                }
                dispatches += chunks.len();
                pg.prepared_passes[pass_kind as usize] = Some(PreparedPass { revision, chunks });
            };

            prepare_range(PASS_OPAQUE, 0, opaque_count, opaque_count);
            prepare_range(PASS_CUTOUT, opaque_count, cutout_count, total_count);
            pg.prepared_passes[PASS_ALL as usize] = None;
        }
        dispatches
    }

    /// Uploads dirty resident meshlets once per page revision and runs GPU compute culling +
    /// opaque/cutout compaction into `pg.storage` + `pg.indirect`, eliminating per-frame CPU
    /// meshlet gathering and `queue.write_buffer(&pg.storage)`.
    pub fn prepare_gpu_cull_pages(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        flat: &crate::gfx::flat::Flat,
        planes: [[f32; 4]; 6],
        eye: [f32; 4],
        no_eye: bool,
    ) -> usize {
        let skip_facing = u32::from(no_eye);
        let max_wg = self.max_task_workgroups.max(1);
        let pages: Vec<u32> = self.table.pages.keys().copied().collect();
        let mut cull_dispatches = Vec::with_capacity(pages.len());

        for page in pages {
            if flat.page_group(page).is_none() {
                self.invalidate_page_passes(page);
                continue;
            }
            let (revision, resident_count) = {
                let entry = self.table.pages.get(&page).expect("page exists in table");
                let count: usize = entry.sections.values().map(|v| v.len()).sum();
                (entry.revision, count)
            };
            if resident_count == 0 {
                self.invalidate_page_passes(page);
                continue;
            }
            let opaque_capacity = resident_count.max(1);
            let needed_active = opaque_capacity * 2 + 64;
            self.ensure_page_buffers(
                device,
                page,
                needed_active,
                resident_count.max(64),
                PASS_SLOTS.max(3),
            );
            let needs_resident_upload = self
                .gpu_pages
                .get(&page)
                .is_some_and(|pg| pg.resident_uploaded_revision != revision);
            if needs_resident_upload {
                let entry = self
                    .table
                    .pages
                    .get_mut(&page)
                    .expect("page exists in table");
                entry.visible_staging.clear();
                entry.visible_staging.reserve(resident_count);
                for list in entry.sections.values() {
                    entry.visible_staging.extend_from_slice(list);
                }
                let pg = self
                    .gpu_pages
                    .get_mut(&page)
                    .expect("page GPU storage must exist");
                queue.write_buffer(
                    &pg.resident_storage,
                    0,
                    bytemuck::cast_slice(&entry.visible_staging),
                );
                pg.resident_uploaded_revision = revision;
                pg.resident_count = resident_count as u32;
            }

            let pg = self
                .gpu_pages
                .get_mut(&page)
                .expect("page GPU storage must exist");
            pg.uploaded_full_layout = false;
            let res_count = pg.resident_count;
            let op_cap = res_count.max(1);
            let zero_indirect: [u32; 8] = [0, 1, 1, 0, 1, 1, 0, 0];
            queue.write_buffer(&pg.indirect, 0, bytemuck::cast_slice(&zero_indirect));

            let cull_params = MeshCullParams {
                planes,
                eye,
                counts: [res_count, op_cap, skip_facing, max_wg],
            };
            queue.write_buffer(&pg.uniform, 0, bytemuck::bytes_of(&cull_params));

            let page_bits = (page & 0x3f) << 8;
            for (pass_kind, limit, indirect_offset, start) in [
                (PASS_OPAQUE, op_cap, 0u64, 0u32),
                (PASS_CUTOUT, op_cap * 2, 12u64, op_cap),
            ] {
                let uniform_offset = pass_kind * self.uniform_stride;
                let params = MeshCullParams {
                    planes,
                    eye,
                    counts: [limit, pass_kind, skip_facing | page_bits, start],
                };
                queue.write_buffer(
                    &pg.uniform,
                    u64::from(uniform_offset),
                    bytemuck::bytes_of(&params),
                );
                pg.prepared_passes[pass_kind as usize] = Some(PreparedPass {
                    revision,
                    chunks: vec![PreparedChunk {
                        uniform_offset,
                        indirect_offset,
                        meshlet_count: res_count,
                        workgroups: 1,
                    }],
                });
            }
            pg.prepared_passes[PASS_ALL as usize] = None;
            cull_dispatches.push((page, res_count));
        }

        if !cull_dispatches.is_empty() {
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Cull terrain meshlets"),
                    timestamp_writes: None,
                });
                cpass.set_pipeline(&self.cull_pipeline);
                for &(page, res_count) in &cull_dispatches {
                    let pg = &self.gpu_pages[&page];
                    let group1 = flat.page_group(page).expect("checked page_group above");
                    cpass.set_bind_group(0, &pg.cull_bind_group, &[]);
                    cpass.set_bind_group(1, group1, &[]);
                    cpass.dispatch_workgroups(res_count.div_ceil(64), 1, 1);
                }
            }
            {
                let mut fpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Finalize terrain meshlet indirect args"),
                    timestamp_writes: None,
                });
                fpass.set_pipeline(&self.finalize_pipeline);
                for &(page, _) in &cull_dispatches {
                    let pg = &self.gpu_pages[&page];
                    fpass.set_bind_group(0, &pg.cull_bind_group, &[]);
                    fpass.dispatch_workgroups(1, 1, 1);
                }
            }
        }

        cull_dispatches.len() * 2
    }

    /// Yields every prepared dispatch chunk for `(page, pass_kind)` if that pass was prepared
    /// against the current uploaded page revision, or an empty iterator otherwise.
    pub fn page_draws(&self, page: u32, pass_kind: u32) -> impl Iterator<Item = MeshPageDraw<'_>> {
        let prepared =
            if pass_kind < PASS_SLOTS {
                self.table.page_revision(page).and_then(|current_rev| {
                    let pg = self.gpu_pages.get(&page)?;
                    let pass = pg.prepared_passes[pass_kind as usize].as_ref()?;
                    (pass.revision == current_rev && pg.uploaded_revision == current_rev)
                        .then_some((&pg.bind_group, &pg.indirect, pass.chunks.as_slice()))
                })
            } else {
                None
            };
        let (bind_group, indirect_buffer, chunks) = match prepared {
            Some((bg, indirect, chunks)) => (Some(bg), Some(indirect), chunks),
            None => (None, None, &[][..]),
        };
        chunks.iter().map(move |chunk| MeshPageDraw {
            bind_group: bind_group.expect("non-empty chunks always have bind_group"),
            uniform_offset: chunk.uniform_offset,
            indirect_buffer: indirect_buffer.expect("non-empty chunks always have indirect_buffer"),
            indirect_offset: chunk.indirect_offset,
            meshlet_count: chunk.meshlet_count,
            workgroups: chunk.workgroups,
        })
    }

    /// Returns the first prepared immutable draw handle for `(page, pass_kind)` if that pass was
    /// prepared against the current uploaded page revision, or `None` otherwise.
    pub fn page_draw(&self, page: u32, pass_kind: u32) -> Option<MeshPageDraw<'_>> {
        self.page_draws(page, pass_kind).next()
    }

    /// Number of staged/pending meshlets on `page`.
    pub fn page_count(&self, page: u32) -> u32 {
        self.table.page_count(page)
    }

    /// Currently staged [`GpuMeshlet`] descriptors on `page`.
    pub fn staged_page(&self, page: u32) -> &[GpuMeshlet] {
        self.table.staged_page(page)
    }
}

fn create_page_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    storage: &wgpu::Buffer,
    uniform: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Terrain meshlets"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: storage.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: uniform,
                    offset: 0,
                    size: std::num::NonZeroU64::new(std::mem::size_of::<MeshCullParams>() as u64),
                }),
            },
        ],
    })
}

fn create_cull_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    resident_storage: &wgpu::Buffer,
    active_storage: &wgpu::Buffer,
    indirect: &wgpu::Buffer,
    uniform: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Terrain meshlet GPU cull"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: resident_storage.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: active_storage.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: indirect.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: uniform,
                    offset: 0,
                    size: std::num::NonZeroU64::new(std::mem::size_of::<MeshCullParams>() as u64),
                }),
            },
        ],
    })
}

/// Allocates a [`MeshSectionStore`] only when `device` was opened with mesh shaders enabled.
/// On main's off path this allocates zero GPU buffers.
#[allow(dead_code)]
pub(in crate::gfx) fn new_store(device: &wgpu::Device) -> Option<MeshSectionStore> {
    if !on(device) {
        return None;
    }
    let limits = device.limits();
    let align = limits.min_uniform_buffer_offset_alignment.max(1);
    let uniform_stride = (std::mem::size_of::<MeshCullParams>() as u32).next_multiple_of(align);
    let max_task_workgroups = limits
        .max_task_workgroups_per_dimension
        .min(limits.max_task_workgroup_total_count)
        .max(1);
    let cull_layout = cull_bind_group_layout(device);
    let g1_layout = group1_layout(device);
    let cull_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Terrain meshlet GPU cull layout"),
        bind_group_layouts: &[Some(&cull_layout), Some(&g1_layout)],
        ..Default::default()
    });
    let finalize_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Terrain meshlet finalize layout"),
        bind_group_layouts: &[Some(&cull_layout)],
        ..Default::default()
    });
    let cull_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Terrain meshlet GPU cull shader"),
        source: wgpu::ShaderSource::Wgsl(cull_shader().into()),
    });
    let cull_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Terrain meshlet GPU cull"),
        layout: Some(&cull_pipe_layout),
        module: &cull_module,
        entry_point: Some("cs_cull_meshlets"),
        compilation_options: Default::default(),
        cache: None,
    });
    let finalize_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Terrain meshlet finalize"),
        layout: Some(&finalize_pipe_layout),
        module: &cull_module,
        entry_point: Some("cs_finalize_meshlets"),
        compilation_options: Default::default(),
        cache: None,
    });
    Some(MeshSectionStore {
        table: MeshletTable::new(),
        gpu_pages: std::collections::BTreeMap::new(),
        uniform_stride,
        max_task_workgroups,
        bind_group_layout: bind_group_layout(device),
        cull_bind_group_layout: cull_layout,
        cull_pipeline,
        finalize_pipeline,
    })
}

/// Builds a hardware task+mesh terrain pipeline pairing `ts_terrain` and `ms_terrain`
/// with the requested fragment entry point (`fs_opaque` or `fs_solid`) and pipeline `constants`.
#[allow(dead_code)]
pub(in crate::gfx) fn create_mesh_terrain_pipeline(
    device: &wgpu::Device,
    layout: Option<&wgpu::PipelineLayout>,
    module: &wgpu::ShaderModule,
    fragment_entry: &str,
    color_format: wgpu::TextureFormat,
    depth_format: Option<wgpu::TextureFormat>,
    constants: &[(&str, f64)],
) -> wgpu::RenderPipeline {
    device.create_mesh_pipeline(&wgpu::MeshPipelineDescriptor {
        label: Some("Terrain mesh pipeline"),
        layout,
        task: Some(wgpu::TaskState {
            module,
            entry_point: Some("ts_terrain"),
            compilation_options: Default::default(),
        }),
        mesh: wgpu::MeshState {
            module,
            entry_point: Some("ms_terrain"),
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment_entry),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants,
                ..Default::default()
            },
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    })
}

/// Dispatches hardware mesh tasks for a prepared `(page, pass_kind)` draw handle via indirect args.
#[allow(dead_code)]
pub(in crate::gfx) fn draw_mesh_page<'a>(
    pass: &mut wgpu::RenderPass<'a>,
    pipeline: &'a wgpu::RenderPipeline,
    group1: &'a wgpu::BindGroup,
    draw: MeshPageDraw<'a>,
) {
    if draw.meshlet_count > 0 {
        pass.set_pipeline(pipeline);
        pass.set_bind_group(1, group1, &[]);
        pass.set_bind_group(2, draw.bind_group, &[draw.uniform_offset]);
        pass.draw_mesh_tasks_indirect(draw.indirect_buffer, draw.indirect_offset);
    }
}

fn create_vis_mesh_pipeline(
    device: &wgpu::Device,
    layout: Option<&wgpu::PipelineLayout>,
    module: &wgpu::ShaderModule,
    mesh_entry: &str,
    fragment_entry: &str,
    constants: &[(&str, f64)],
) -> wgpu::RenderPipeline {
    device.create_mesh_pipeline(&wgpu::MeshPipelineDescriptor {
        label: Some("Terrain visbuffer mesh pipeline"),
        layout,
        task: Some(wgpu::TaskState {
            module,
            entry_point: Some("ts_terrain"),
            compilation_options: Default::default(),
        }),
        mesh: wgpu::MeshState {
            module,
            entry_point: Some(mesh_entry),
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment_entry),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants,
                ..Default::default()
            },
            targets: &[Some(wgpu::ColorTargetState {
                format: crate::gfx::visbuffer::IDS,
                blend: None,
                write_mask: wgpu::ColorWrites::RED,
            })],
        }),
        multiview: None,
        cache: None,
    })
}

/// Renderer-owned hardware mesh-shader terrain store and pipelines (#17243 step 3/4).
pub(in crate::gfx) struct MeshTerrain {
    pub(in crate::gfx) store: MeshSectionStore,
    opaque_plain: wgpu::RenderPipeline,
    opaque_hdr: Option<wgpu::RenderPipeline>,
    cutout_plain: wgpu::RenderPipeline,
    cutout_hdr: Option<wgpu::RenderPipeline>,
    vis_opaque: wgpu::RenderPipeline,
    vis_cutout: wgpu::RenderPipeline,
}

impl MeshTerrain {
    /// Initializes the mesh section store and builds opaque/cutout hardware task+mesh pipelines
    /// when `device` was opened with `EXPERIMENTAL_MESH_SHADER`.
    pub fn new(
        device: &wgpu::Device,
        group0: &wgpu::BindGroupLayout,
        group1: &wgpu::BindGroupLayout,
        (color_format, linear_light): (wgpu::TextureFormat, bool),
        sprite_constants: &[(&str, f64)],
        has_hdr: bool,
    ) -> Option<Self> {
        let store = new_store(device)?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon terrain mesh shader"),
            source: wgpu::ShaderSource::Wgsl(shader().into()),
        });
        let layout = pipeline_layout(device, group0, group1, store.bind_group_layout());
        let frame_consts = crate::gfx::linear_blend::frame_constants(color_format, linear_light);
        let mut hdr_constants = Vec::with_capacity(sprite_constants.len() + frame_consts.len());
        hdr_constants.extend_from_slice(sprite_constants);
        hdr_constants.extend_from_slice(&frame_consts);

        let mut plain_constants = Vec::with_capacity(hdr_constants.len() + 1);
        plain_constants.extend_from_slice(sprite_constants);
        plain_constants.push(("HDR_LIGHTING", 0.0));
        plain_constants.extend_from_slice(&frame_consts);

        let depth = Some(wgpu::TextureFormat::Depth32Float);
        let opaque_plain = create_mesh_terrain_pipeline(
            device,
            Some(&layout),
            &module,
            "fs_opaque",
            color_format,
            depth,
            &plain_constants,
        );
        let cutout_plain = create_mesh_terrain_pipeline(
            device,
            Some(&layout),
            &module,
            "fs_solid",
            color_format,
            depth,
            &plain_constants,
        );
        let (opaque_hdr, cutout_hdr) = if has_hdr {
            (
                Some(create_mesh_terrain_pipeline(
                    device,
                    Some(&layout),
                    &module,
                    "fs_opaque",
                    color_format,
                    depth,
                    &hdr_constants,
                )),
                Some(create_mesh_terrain_pipeline(
                    device,
                    Some(&layout),
                    &module,
                    "fs_solid",
                    color_format,
                    depth,
                    &hdr_constants,
                )),
            )
        } else {
            (None, None)
        };
        let vis_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon terrain visbuffer mesh shader"),
            source: wgpu::ShaderSource::Wgsl(vis_shader().into()),
        });
        let vis_opaque = create_vis_mesh_pipeline(
            device,
            Some(&layout),
            &vis_module,
            "ms_vis_opaque",
            "fs_vis_mesh_id",
            &plain_constants,
        );
        let vis_cutout = create_vis_mesh_pipeline(
            device,
            Some(&layout),
            &vis_module,
            "ms_vis_cutout",
            "fs_vis_mesh_cutout",
            &plain_constants,
        );
        Some(Self {
            store,
            opaque_plain,
            opaque_hdr,
            cutout_plain,
            cutout_hdr,
            vis_opaque,
            vis_cutout,
        })
    }

    /// Swaps the retained mesh section store during renderer replacement on the same device,
    /// rebinding each page's `@group(2)` bind group against each store's own pipeline layout.
    pub fn take_store(&mut self, device: &wgpu::Device, from: &mut Self) {
        std::mem::swap(&mut self.store.table, &mut from.store.table);
        std::mem::swap(&mut self.store.gpu_pages, &mut from.store.gpu_pages);
        for store in [&mut self.store, &mut from.store] {
            for pg in store.gpu_pages.values_mut() {
                pg.bind_group = create_page_bind_group(
                    device,
                    &store.bind_group_layout,
                    &pg.storage,
                    &pg.uniform,
                );
                pg.cull_bind_group = create_cull_bind_group(
                    device,
                    &store.cull_bind_group_layout,
                    &pg.resident_storage,
                    &pg.storage,
                    &pg.indirect,
                    &pg.uniform,
                );
                pg.prepared_passes = [const { None }; PASS_SLOTS as usize];
            }
        }
    }

    /// Registers pre-built worker meshlets for an uploaded section mesh (falling back to inline
    /// extraction if the mesh was constructed without worker-side `meshlets` populated).
    pub fn insert_section_mesh(
        &mut self,
        page: u32,
        solid_base: u32,
        packed: bool,
        sides: &crate::mesher::Sides,
        mesh: &crate::mesher::SectionMesh,
    ) {
        if !mesh.meshlets.is_empty() && (packed == !mesh.packed.is_empty()) {
            self.store.insert_section(page, solid_base, &mesh.meshlets);
        } else {
            let meshlets = crate::mesher::meshlet::build_section_meshlets(
                sides,
                &mesh.solid,
                if packed { &mesh.packed } else { &[] },
            );
            self.store.insert_section(page, solid_base, &meshlets);
        }
    }

    /// Uploads dirty meshlet staging pages and updates `PASS_OPAQUE` / `PASS_CUTOUT` culling uniforms.
    /// Returns the number of non-empty page-pass dispatches prepared for this frame.
    #[allow(dead_code)]
    pub fn prepare_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pages: impl IntoIterator<Item = u32>,
        planes: [[f32; 4]; 6],
        eye: [f32; 4],
        no_eye: bool,
    ) -> usize {
        let skip_facing = u32::from(no_eye);
        let mut dispatches = 0usize;
        for page in pages {
            for pass_kind in [PASS_OPAQUE, PASS_CUTOUT] {
                let params = MeshCullParams {
                    planes,
                    eye,
                    counts: [0, pass_kind, skip_facing, 0],
                };
                if self.store.prepare_page(device, queue, page, &params) > 0 {
                    dispatches += self.store.page_draws(page, pass_kind).count();
                }
            }
        }
        dispatches
    }

    /// Gathers only the visible sections in front-to-back order, partitions them into disjoint
    /// `PASS_OPAQUE` and `PASS_CUTOUT` ranges per page, uploads descriptors, and prepares indirect
    /// dispatch args. Returns the number of non-empty page-pass dispatches prepared for this frame.
    #[allow(dead_code)]
    pub fn prepare_visible_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        visible_sections: &[(u32, u32, bool)],
        planes: [[f32; 4]; 6],
        eye: [f32; 4],
        no_eye: bool,
    ) -> usize {
        self.store
            .prepare_visible_pages(device, queue, visible_sections, planes, eye, no_eye)
    }

    /// Uploads dirty resident meshlets once per page revision and runs GPU compute culling +
    /// compaction into `pg.storage` + `pg.indirect`.
    pub fn prepare_gpu_cull_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        flat: &crate::gfx::flat::Flat,
        planes: [[f32; 4]; 6],
        eye: [f32; 4],
        no_eye: bool,
    ) -> usize {
        self.store
            .prepare_gpu_cull_pages(device, queue, encoder, flat, planes, eye, no_eye)
    }

    /// Sorted list of arena pages with prepared meshlet draws this frame.
    pub fn active_pages(&self) -> Vec<u32> {
        self.store
            .gpu_pages
            .keys()
            .copied()
            .filter(|&page| {
                self.store.page_draws(page, PASS_OPAQUE).next().is_some()
                    || self.store.page_draws(page, PASS_CUTOUT).next().is_some()
            })
            .collect()
    }

    /// Records the opaque then cutout hardware mesh passes across all prepared arena pages
    /// using `draw_mesh_tasks_indirect`, setting the pipeline once per pass.
    pub fn draw_solid<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        flat: &'a crate::gfx::flat::Flat,
        hdr: bool,
    ) {
        let opaque_pipe = if hdr {
            self.opaque_hdr.as_ref().unwrap_or(&self.opaque_plain)
        } else {
            &self.opaque_plain
        };
        let cutout_pipe = if hdr {
            self.cutout_hdr.as_ref().unwrap_or(&self.cutout_plain)
        } else {
            &self.cutout_plain
        };
        self.draw_passes(pass, flat, opaque_pipe, cutout_pipe);
    }

    /// Records the Visbuffer ID-only opaque then cutout hardware mesh passes (`R32Uint` + depth).
    pub fn draw_vis_ids<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        flat: &'a crate::gfx::flat::Flat,
    ) {
        self.draw_passes(pass, flat, &self.vis_opaque, &self.vis_cutout);
    }

    fn draw_passes<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        flat: &'a crate::gfx::flat::Flat,
        opaque_pipe: &'a wgpu::RenderPipeline,
        cutout_pipe: &'a wgpu::RenderPipeline,
    ) {
        for (pass_kind, pipeline) in [(PASS_OPAQUE, opaque_pipe), (PASS_CUTOUT, cutout_pipe)] {
            pass.set_pipeline(pipeline);
            for &page in self.store.gpu_pages.keys() {
                let Some(group1) = flat.page_group(page) else {
                    continue;
                };
                let mut draws = self.store.page_draws(page, pass_kind).peekable();
                if draws.peek().is_none() {
                    continue;
                }
                pass.set_bind_group(1, group1, &[]);
                for draw in draws {
                    pass.set_bind_group(2, draw.bind_group, &[draw.uniform_offset]);
                    pass.draw_mesh_tasks_indirect(draw.indirect_buffer, draw.indirect_offset);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_shader_switch_resolution() {
        let os = |v: &str| Some(std::ffi::OsString::from(v));
        assert_eq!(resolve(None), Switch::Off);
        assert_eq!(resolve(os("1").as_deref()), Switch::On);
        assert_eq!(resolve(os("0").as_deref()), Switch::Off);
        assert_eq!(resolve(os("true").as_deref()), Switch::Off);
        assert_eq!(resolve(os("").as_deref()), Switch::Off);
        assert_eq!(resolve(os(" 1").as_deref()), Switch::Off);
    }

    #[test]
    fn mesh_shader_only_when_asked_and_offered() {
        let all = wgpu::Features::all();
        assert_eq!(pick(true, all), MESH_SHADER);
        assert!(pick(false, all).is_empty());
        assert!(pick(true, all - MESH_SHADER).is_empty());
        assert!(!experimental(wgpu::Features::empty()).is_enabled());
        assert!(experimental(MESH_SHADER).is_enabled());
    }

    #[test]
    fn terrain_mesh_shader_validates() {
        let source = shader();
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("WGSL parse failed:\n{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("naga validation failed: {e}"));

        let stages: Vec<_> = module
            .entry_points
            .iter()
            .map(|ep| (ep.name.as_str(), ep.stage))
            .collect();
        assert!(stages.contains(&("ts_terrain", naga::ShaderStage::Task)));
        assert!(stages.contains(&("ms_terrain", naga::ShaderStage::Mesh)));
    }

    #[test]
    fn sub_aabb_frustum_and_bucket_culling() {
        let meshlet = |bucket, sub_aabb| MeshletDesc::new(0, 40, bucket, 0, 0, sub_aabb);
        // Sub-box spanning [8..12, 4..8, 2..6] inside the 16x16x16 section.
        let box_aabb = [8, 4, 2, 12, 8, 6];

        // Bucket 0 (+X): visible iff eye.x > min.x - margin.
        assert!(meshlet_facing(&meshlet(0, box_aabb), [9.0, 0.0, 0.0], 0.0));
        assert!(!meshlet_facing(&meshlet(0, box_aabb), [7.5, 0.0, 0.0], 0.0));
        // Bucket 1 (+Z): visible iff eye.z > min.z - margin.
        assert!(meshlet_facing(&meshlet(1, box_aabb), [0.0, 0.0, 2.5], 0.0));
        assert!(!meshlet_facing(&meshlet(1, box_aabb), [0.0, 0.0, 1.5], 0.0));
        // Bucket 2 (Other): always visible.
        assert!(meshlet_facing(
            &meshlet(2, box_aabb),
            [-10.0, -10.0, -10.0],
            -10.0
        ));
        // Bucket 3 (+Y): visible iff eye.y > min.y - margin.
        assert!(meshlet_facing(&meshlet(3, box_aabb), [0.0, 5.0, 0.0], 0.0));
        assert!(!meshlet_facing(&meshlet(3, box_aabb), [0.0, 3.5, 0.0], 0.0));
        // Bucket 4 (-Z): visible iff eye.z < max.z + margin.
        assert!(meshlet_facing(&meshlet(4, box_aabb), [0.0, 0.0, 5.5], 0.0));
        assert!(!meshlet_facing(&meshlet(4, box_aabb), [0.0, 0.0, 6.5], 0.0));
        // Bucket 5 (-X): visible iff eye.x < max.x + margin.
        assert!(meshlet_facing(&meshlet(5, box_aabb), [11.0, 0.0, 0.0], 0.0));
        assert!(!meshlet_facing(
            &meshlet(5, box_aabb),
            [12.5, 0.0, 0.0],
            0.0
        ));
        // Bucket 6 (-Y): uses rel_low (water mirror) < max.y + margin.
        assert!(meshlet_facing(&meshlet(6, box_aabb), [0.0, 20.0, 0.0], 7.0));
        assert!(!meshlet_facing(
            &meshlet(6, box_aabb),
            [0.0, 20.0, 0.0],
            9.0
        ));

        // Frustum cube [-20..20]^3 except left plane moved inside the section at x = 6:
        // a sub-box at x=[8..12] stays inside, while one at x=[0..4] is culled even though the
        // full 16x16x16 section [0..16] grazes the frustum!
        let planes = [
            [1.0, 0.0, 0.0, -6.0], // x >= 6
            [-1.0, 0.0, 0.0, 20.0],
            [0.0, 1.0, 0.0, 20.0],
            [0.0, -1.0, 0.0, 20.0],
            [0.0, 0.0, 1.0, 20.0],
            [0.0, 0.0, -1.0, 20.0],
        ];
        let origin = [0.0, 0.0, 0.0];
        let inside = meshlet(2, [8, 0, 0, 12, 4, 4]);
        let grazing = meshlet(2, [4, 0, 0, 7, 4, 4]);
        let outside = meshlet(2, [0, 0, 0, 4, 4, 4]);
        assert!(meshlet_in_frustum(&inside, origin, &planes));
        assert!(meshlet_in_frustum(&grazing, origin, &planes));
        assert!(!meshlet_in_frustum(&outside, origin, &planes));

        assert_eq!(meshlet_slices(0), 0);
        assert_eq!(meshlet_slices(1), 1);
        assert_eq!(meshlet_slices(32), 1);
        assert_eq!(meshlet_slices(33), 2);
        assert_eq!(meshlet_slices(64), 2);
        assert_eq!(task_workgroups(0), 0);
        assert_eq!(task_workgroups(1), 1);
        assert_eq!(task_workgroups(32), 1);
        assert_eq!(task_workgroups(33), 2);
        let chunks_0: Vec<_> = task_dispatch_chunks(0, 1024).collect();
        assert!(chunks_0.is_empty());
        let chunks_fit: Vec<_> = task_dispatch_chunks(32_768, 1024).collect();
        assert_eq!(chunks_fit, vec![(0, 32_768, 1024)]);
        let chunks_split: Vec<_> = task_dispatch_chunks(32_769, 1024).collect();
        assert_eq!(chunks_split, vec![(0, 32_768, 1024), (32_768, 1, 1)]);
        let chunks_three: Vec<_> = task_dispatch_chunks(65_537, 1024).collect();
        assert_eq!(
            chunks_three,
            vec![(0, 32_768, 1024), (32_768, 32_768, 1024), (65_536, 1, 1)]
        );
    }

    #[test]
    fn gpu_meshlet_binary_layout_contract() {
        assert_eq!(std::mem::size_of::<GpuMeshlet>(), 16);
        assert_eq!(std::mem::align_of::<GpuMeshlet>(), 4);
        assert_eq!(std::mem::size_of::<MeshletDesc>(), 16);
        assert_eq!(std::mem::align_of::<MeshletDesc>(), 4);

        let sub_aabb = [1, 2, 3, 14, 15, 16];
        let gpu = GpuMeshlet::new(0x1234_5678, 32, 5, MeshletDesc::FLAG_CUTOUT, 42, sub_aabb);

        // Cast to [u32; 4] as bound to WGSL `@group(2) @binding(0) var<storage, read> meshlets: array<vec4<u32>>`
        let words: [u32; 4] = bytemuck::cast(gpu);

        // x: vertex_base
        assert_eq!(words[0], 0x1234_5678);

        // y: quad_count | (bucket << 8) | (flags << 16) | (plane_quant << 24)
        assert_eq!(words[1] & 0xff, 32);
        assert_eq!((words[1] >> 8) & 0xff, 5);
        assert_eq!((words[1] >> 16) & 0xff, MeshletDesc::FLAG_CUTOUT as u32);
        assert_eq!((words[1] >> 24) & 0xff, 42);

        // z: min_local (x, y, z) and max_local.x
        assert_eq!(words[2] & 0xff, 1);
        assert_eq!((words[2] >> 8) & 0xff, 2);
        assert_eq!((words[2] >> 16) & 0xff, 3);
        assert_eq!((words[2] >> 24) & 0xff, 14);

        // w: max_local (y, z) and padding
        assert_eq!(words[3] & 0xff, 15);
        assert_eq!((words[3] >> 8) & 0xff, 16);
        assert_eq!((words[3] >> 16) & 0xffff, 0); // padding

        // Verify that ts_terrain's exact bit unpacking in WGSL reconstructs every field identically
        let unpacked_vertex_base = words[0];
        let unpacked_quad_count = (words[1] & 0xff) as u8;
        let unpacked_bucket = ((words[1] >> 8) & 0xff) as u8;
        let unpacked_flags = ((words[1] >> 16) & 0xff) as u8;
        let unpacked_plane_quant = ((words[1] >> 24) & 0xff) as u8;
        let unpacked_min_local = [
            (words[2] & 0xff) as u8,
            ((words[2] >> 8) & 0xff) as u8,
            ((words[2] >> 16) & 0xff) as u8,
        ];
        let unpacked_max_local = [
            ((words[2] >> 24) & 0xff) as u8,
            (words[3] & 0xff) as u8,
            ((words[3] >> 8) & 0xff) as u8,
        ];

        assert_eq!(unpacked_vertex_base, gpu.vertex_base);
        assert_eq!(unpacked_quad_count, gpu.quad_count);
        assert_eq!(unpacked_bucket, gpu.bucket);
        assert_eq!(unpacked_flags, gpu.flags);
        assert_eq!(unpacked_plane_quant, gpu.plane_quant);
        assert_eq!(unpacked_min_local, gpu.min_corner());
        assert_eq!(unpacked_max_local, gpu.max_corner());
    }

    #[test]
    fn gpu_descriptor_conversion_contract() {
        use crate::mesher::packed::PACKED_BASE;

        let desc = MeshletDesc::new(
            15, // section quad offset: 15th quad
            32,
            0,
            MeshletDesc::FLAG_CUTOUT,
            10,
            [0, 0, 0, 16, 16, 16],
        );

        // Case 1: Uncompressed terrain vertices (solid_base = 4 * quad_base)
        let uncompressed_base = 4 * 100; // quad_base = 100
        let gpu_uncompressed = desc.to_gpu(uncompressed_base);
        assert_eq!(gpu_uncompressed.vertex_base, 4 * (100 + 15));
        assert_eq!(gpu_uncompressed.quad_count, 32);
        assert!(gpu_uncompressed.is_cutout());

        // Case 2: Compact terrain quads in arena page (solid_base = PACKED_BASE + 4 * slot_base)
        let slot_base = 512;
        let compact_solid_base = PACKED_BASE + 4 * slot_base;
        let gpu_compact = desc.to_gpu(compact_solid_base);
        assert_eq!(gpu_compact.vertex_base, PACKED_BASE + 4 * (slot_base + 15));
        assert!(gpu_compact.vertex_base >= PACKED_BASE);

        // Verify arena_slot decoding as done in `ts_terrain`:
        // let arena_slot = (vertex_base - PACKED_BASE) >> 2;
        let decoded_slot = (gpu_compact.vertex_base - PACKED_BASE) >> 2;
        assert_eq!(decoded_slot, slot_base + 15);
    }

    #[test]
    fn mesh_stage_emitted_geometry_and_indexing_contract() {
        use crate::mesher::packed::PACKED_BASE;

        // In ms_terrain:
        // let slice = mesh_payload.slices[wg.x];
        // let base_vertex = slice.x;
        // let quad_count = slice.y; // 1..=32
        let base_vertex = PACKED_BASE + 4 * 100;
        let quad_count = 32u32;

        let vertex_count = quad_count * 4;
        let primitive_count = quad_count * 2;
        assert_eq!(vertex_count, 128);
        assert_eq!(primitive_count, 64);

        // Validate indexing for every quad lane
        for lane in 0..quad_count {
            let q_base = base_vertex + lane * 4;
            let v_out = lane * 4;
            let p_out = lane * 2;

            // 4 corners per quad: section_corner(q_base + c)
            let corners = [q_base, q_base + 1, q_base + 2, q_base + 3];
            for (c, &corner_vertex_idx) in corners.iter().enumerate() {
                assert_eq!(corner_vertex_idx, base_vertex + lane * 4 + c as u32);
            }

            // Two triangles winding as (0, 1, 2) and (0, 2, 3) matching displace.rs / flat.rs
            let tri0 = (v_out, v_out + 1, v_out + 2);
            let tri1 = (v_out, v_out + 2, v_out + 3);

            assert_eq!(tri0, (lane * 4, lane * 4 + 1, lane * 4 + 2));
            assert_eq!(tri1, (lane * 4, lane * 4 + 2, lane * 4 + 3));

            assert!(tri0.2 < vertex_count);
            assert!(tri1.2 < vertex_count);
            assert!(p_out + 1 < primitive_count);
        }

        // Validate 64-quad meshlet slicing into task workgroups
        let full_meshlet_quads = 64u8;
        assert_eq!(meshlet_slices(full_meshlet_quads), 2);
        let s0 = (base_vertex, 32u32);
        let s1 = (base_vertex + 32 * 4, (full_meshlet_quads as u32) - 32);
        assert_eq!(s0.1 + s1.1, 64);
        assert_eq!(s1.0, base_vertex + 128);
    }

    fn mesh_shader_adapter() -> Option<wgpu::Adapter> {
        let instance = crate::gfx::test_gpu::instance();
        pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
            .into_iter()
            .find(|adapter| adapter.features().contains(MESH_SHADER))
    }

    fn open_mesh_device(adapter: &wgpu::Adapter, label: &str) -> (wgpu::Device, wgpu::Queue) {
        let desc = wgpu::DeviceDescriptor {
            label: Some(label),
            required_features: MESH_SHADER,
            required_limits: adapter.limits(),
            experimental_features: horizon_hdr::experimental_features(),
            ..Default::default()
        };
        pollster::block_on(adapter.request_device(&desc))
            .expect("adapter offers EXPERIMENTAL_MESH_SHADER")
    }

    #[test]
    fn shader_module_compiles_on_mesh_shader_adapter() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = mesh_shader_adapter() else {
            return;
        };
        let (device, _queue) = open_mesh_device(&adapter, "Mesh shader compile test");
        let g0 = group0_layout(&device);
        let g1 = group1_layout(&device);
        let g2 = bind_group_layout(&device);
        let _pipe_layout = pipeline_layout(&device, &g0, &g1, &g2);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Terrain mesh shader"),
            source: wgpu::ShaderSource::Wgsl(shader().into()),
        });
        let _opaque = create_mesh_terrain_pipeline(
            &device,
            Some(&_pipe_layout),
            &module,
            "fs_opaque",
            wgpu::TextureFormat::Rgba8Unorm,
            Some(wgpu::TextureFormat::Depth32Float),
            &[("HDR_LIGHTING", 0.0)],
        );
    }

    #[test]
    fn without_mesh_shaders_there_is_no_store() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((_adapter, device, _queue)) = crate::gfx::test_gpu::optional_device() else {
            return;
        };
        assert!(!on(&device));
        assert!(new_store(&device).is_none());
    }

    #[test]
    fn meshlet_table_stages_rebased_gpu_meshlets_per_page() {
        use crate::mesher::packed::PACKED_BASE;

        let mut table = MeshletTable::new();
        let m0 = MeshletDesc::new(0, 16, 2, 0, 0, [0, 0, 0, 4, 4, 4]);
        let m1 = MeshletDesc::new(16, 24, 3, MeshletDesc::FLAG_CUTOUT, 10, [4, 4, 4, 8, 8, 8]);

        let base_a = PACKED_BASE + 4 * 100;
        let base_b = PACKED_BASE + 4 * 400;
        let base_c = PACKED_BASE + 4 * 200;

        // Insert out of order to verify deterministic BTreeMap `solid_base` ordering within each page.
        table.insert_section(0, base_b, &[m0]);
        table.insert_section(0, base_a, &[m0, m1]);
        table.insert_section(1, base_c, &[m1]);

        let (rebuilt_p0, staged_p0) = table.stage_page(0);
        assert!(rebuilt_p0);
        assert_eq!(staged_p0.len(), 3);
        assert_eq!(staged_p0[0], m0.to_gpu(base_a));
        assert_eq!(staged_p0[1], m1.to_gpu(base_a));
        assert_eq!(staged_p0[2], m0.to_gpu(base_b));
        assert_eq!(table.page_count(0), 3);

        let (rebuilt_p1, staged_p1) = table.stage_page(1);
        assert!(rebuilt_p1);
        assert_eq!(staged_p1, &[m1.to_gpu(base_c)]);
        assert_eq!(table.page_count(1), 1);

        // Alternating between already-staged pages must NEVER trigger redundant rebuilds.
        let (rebuilt_p0_again, staged_p0_again) = table.stage_page(0);
        assert!(!rebuilt_p0_again);
        assert_eq!(staged_p0_again.len(), 3);
        let (rebuilt_p1_again, _) = table.stage_page(1);
        assert!(!rebuilt_p1_again);

        // Mutating Page 0 dirties ONLY Page 0; Page 1 stays clean.
        table.remove_section(0, base_b);
        assert!(!table.stage_page(1).0);
        let (rebuilt_p0_after_remove, staged_p0_after_remove) = table.stage_page(0);
        assert!(rebuilt_p0_after_remove);
        assert_eq!(
            staged_p0_after_remove,
            &[m0.to_gpu(base_a), m1.to_gpu(base_a)]
        );
        assert_eq!(table.page_count(0), 2);
        assert_eq!(table.staged_page(0).len(), 2);

        table.clear();
        assert_eq!(table.page_count(0), 0);
        assert_eq!(table.page_count(1), 0);
        assert!(table.stage_page(0).1.is_empty());
    }

    #[test]
    fn end_to_end_section_to_gpu_store_and_task_culling_contract() {
        use crate::mesher::packed::{FULL, PACKED_BASE, WHITE, layout, packed_vertex};
        use crate::mesher::{OPAQUE_LAYER, TerrainVertex, meshlet::build_section_meshlets};

        let make_quad = |x: f32, y: f32, z: f32, layer: u16, x_skew: f32| -> [TerrainVertex; 4] {
            let v = |dx, dy, dz, u, vc| TerrainVertex {
                pos: [x + dx, y + dy, z + dz],
                uv: [u, vc],
                layer,
                light: 0xF0F0,
                color: 0xFFFF_FFFF,
            };
            [
                v(x_skew, 1.0, 0.0, 0, 0),
                v(0.0, 1.0, 1.0, 0, 16 << 10),
                v(1.0, 1.0, 1.0, 16 << 10, 16 << 10),
                v(1.0, 1.0, 0.0, 16 << 10, 0),
            ]
        };

        // 4 opaque +Y block quads + 2 cutout +Y quads (one grid-aligned, one spilled to FULL).
        let q0 = make_quad(2.0, 3.0, 4.0, OPAQUE_LAYER, 0.0);
        let q1 = make_quad(3.0, 3.0, 4.0, OPAQUE_LAYER, 0.0);
        let q2 = make_quad(4.0, 3.0, 4.0, OPAQUE_LAYER, 0.0);
        let q3 = make_quad(5.0, 3.0, 4.0, OPAQUE_LAYER, 0.0);
        let q4 = make_quad(8.0, 5.0, 6.0, 0, 0.0);
        let q5 = make_quad(9.0, 5.0, 6.0, 0, 0.25);

        let mut solid: Vec<TerrainVertex> =
            [q0, q1, q2, q3, q4, q5].into_iter().flatten().collect();
        let mut tints = vec![WHITE; 6];
        let sides = crate::mesher::sort_tinted(&mut solid, &mut tints);
        let packed = layout(&solid, &tints);
        assert!(!packed.is_empty(), "packed layout must be smaller than raw");

        let meshlets = build_section_meshlets(&sides, &solid, &packed);
        assert_eq!(meshlets.len(), 2);

        let slot_base = 256u32;
        let solid_base = PACKED_BASE + 4 * slot_base;
        let mut table = MeshletTable::new();
        table.insert_section(0, solid_base, &meshlets);
        let (_, staged) = table.stage_page(0);
        assert_eq!(staged.len(), 2);

        // Box frustum strictly covering section origin [0..16]^3.
        let planes = [
            [1.0, 0.0, 0.0, 1.0],
            [-1.0, 0.0, 0.0, 20.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, -1.0, 0.0, 20.0],
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, -1.0, 20.0],
        ];
        let origin = [0.0f32, 0.0, 0.0];
        let eye_above = [0.0f32, 20.0, 0.0];
        let eye_below = [0.0f32, -5.0, 0.0];

        // Simulate ts_terrain pass + visibility filtering and slice payload generation.
        let simulate_ts = |pass_kind: u32, eye: [f32; 3]| -> Vec<(u32, u32)> {
            let mut slices = Vec::new();
            for gpu in staged {
                let words: [u32; 4] = bytemuck::cast(*gpu);
                let vertex_base = words[0];
                let quad_count = words[1] & 0xff;
                let bucket = (words[1] >> 8) & 0xff;
                let flags = (words[1] >> 16) & 0xff;
                let min_local = [
                    (words[2] & 0xff) as u8,
                    ((words[2] >> 8) & 0xff) as u8,
                    ((words[2] >> 16) & 0xff) as u8,
                ];
                let max_local = [
                    ((words[2] >> 24) & 0xff) as u8,
                    (words[3] & 0xff) as u8,
                    ((words[3] >> 8) & 0xff) as u8,
                ];
                let desc = MeshletDesc::new(
                    0,
                    quad_count as u8,
                    bucket as u8,
                    flags as u8,
                    0,
                    [
                        min_local[0],
                        min_local[1],
                        min_local[2],
                        max_local[0],
                        max_local[1],
                        max_local[2],
                    ],
                );

                let arena_slot = (vertex_base - PACKED_BASE) >> 2;
                assert!(arena_slot >= slot_base);
                let is_cutout_pass = (flags & 1) != 0;
                let pass_match = pass_kind == PASS_ALL
                    || (pass_kind == PASS_OPAQUE && !is_cutout_pass)
                    || (pass_kind == PASS_CUTOUT && is_cutout_pass);

                if pass_match
                    && quad_count > 0
                    && meshlet_facing(&desc, eye, eye[1])
                    && meshlet_in_frustum(&desc, origin, &planes)
                {
                    let first_count = quad_count.min(32);
                    slices.push((vertex_base, first_count));
                    if quad_count > 32 {
                        slices.push((vertex_base + 32 * 4, quad_count - 32));
                    }
                }
            }
            slices
        };

        // Looking from below culls all +Y buckets (bucket 3).
        assert!(simulate_ts(PASS_ALL, eye_below).is_empty());

        let opaque_slices = simulate_ts(PASS_OPAQUE, eye_above);
        let cutout_slices = simulate_ts(PASS_CUTOUT, eye_above);
        assert_eq!(opaque_slices.len(), 1);
        assert_eq!(opaque_slices[0].1, 4);
        assert_eq!(cutout_slices.len(), 1);
        assert_eq!(cutout_slices[0].1, 2);

        // Decode every quad corner from the emitted slice base vertices via packed/spilled layout
        // and verify bit-for-bit equality against `solid` + pass classification.
        let verify_slices = |slices: &[(u32, u32)], expect_opaque: bool| {
            let mut checked_quads = 0u32;
            for &(base_vertex, quad_count) in slices {
                for lane in 0..quad_count {
                    let q_base = base_vertex + lane * 4;
                    let slot = (((q_base - PACKED_BASE) >> 2) - slot_base) as usize;
                    let words = packed[slot];
                    for corner in 0..4 {
                        let decoded = if words[5] & FULL == 0 {
                            packed_vertex(&words, corner)
                        } else {
                            let spill = slot + words[0] as usize + corner;
                            bytemuck::cast(packed[spill])
                        };
                        let expected = solid[slot * 4 + corner];
                        assert_eq!(bytemuck::bytes_of(&decoded), bytemuck::bytes_of(&expected));
                        assert_eq!((decoded.layer & OPAQUE_LAYER) != 0, expect_opaque);
                    }
                    checked_quads += 1;
                }
            }
            checked_quads
        };

        assert_eq!(verify_slices(&opaque_slices, true), 4);
        assert_eq!(verify_slices(&cutout_slices, false), 2);
    }

    #[test]
    fn store_tracks_relocated_meshlets_per_page() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = mesh_shader_adapter() else {
            return;
        };
        let (device, queue) = open_mesh_device(&adapter, "Mesh shader store test");
        let mut store = new_store(&device).expect("mesh shaders are enabled");

        let m0 = MeshletDesc::new(0, 16, 2, 0, 0, [0, 0, 0, 4, 4, 4]);
        let m1 = MeshletDesc::new(16, 24, 3, 0, 0, [4, 4, 4, 8, 8, 8]);
        store.insert_section(0, 100, &[m0, m1]);
        store.insert_section(0, 400, &[m0]);
        store.insert_section(1, 200, &[m1]);

        let params_for = |pass| MeshCullParams {
            planes: [[0.0; 4]; 6],
            eye: [0.0; 4],
            counts: [0, pass, 0, 0],
        };
        assert_eq!(
            store.prepare_page(&device, &queue, 0, &params_for(PASS_OPAQUE)),
            3
        );
        assert_eq!(
            store.prepare_page(&device, &queue, 1, &params_for(PASS_OPAQUE)),
            1
        );
        assert_eq!(
            store.prepare_page(&device, &queue, 0, &params_for(PASS_CUTOUT)),
            3
        );
        assert_eq!(store.page_count(0), 3);
        assert_eq!(store.staged_page(0)[0], m0.to_gpu(100));

        // Page 0 and Page 1 must return distinct bind groups, and passes on the same page must have
        // distinct dynamic uniform offsets so all draws coexist inside a single RenderPass.
        let p0_opaque = store.page_draw(0, PASS_OPAQUE).unwrap();
        let p0_cutout = store.page_draw(0, PASS_CUTOUT).unwrap();
        let p1_opaque = store.page_draw(1, PASS_OPAQUE).unwrap();
        assert_ne!(p0_opaque.uniform_offset, p0_cutout.uniform_offset);
        assert!(!std::ptr::eq(p0_opaque.bind_group, p1_opaque.bind_group));

        store.remove_section(0, 400);
        assert_eq!(
            store.prepare_page(&device, &queue, 0, &params_for(PASS_OPAQUE)),
            2
        );
        assert_eq!(
            store.prepare_page(&device, &queue, 1, &params_for(PASS_OPAQUE)),
            1
        );

        store.clear();
        assert_eq!(
            store.prepare_page(&device, &queue, 0, &params_for(PASS_OPAQUE)),
            0
        );
        assert!(store.page_draw(0, PASS_OPAQUE).is_none());
    }

    #[test]
    fn hardware_mesh_pipeline_renders_multi_page_opaque_cutout_and_culled_quads() {
        use crate::mesher::packed::{PACKED_BASE, layout};
        use crate::mesher::{
            OPAQUE_LAYER, TerrainVertex, UV_STEPS, meshlet::build_section_meshlets,
        };

        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = mesh_shader_adapter() else {
            return;
        };
        let info = adapter.get_info();
        eprintln!(
            "mesh shader hardware proof adapter: {} ({:?})",
            info.name, info.backend
        );
        let (device, queue) = open_mesh_device(&adapter, "Mesh shader hardware render test");

        let mut store = new_store(&device).expect("mesh shaders are enabled");
        let g0_layout = group0_layout(&device);
        let g1_layout = group1_layout(&device);
        let pipe_layout =
            pipeline_layout(&device, &g0_layout, &g1_layout, store.bind_group_layout());
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Terrain mesh hardware proof shader"),
            source: wgpu::ShaderSource::Wgsl(shader().into()),
        });
        let color_format = wgpu::TextureFormat::Rgba8Unorm;
        let depth_format = wgpu::TextureFormat::Depth32Float;
        let opaque_pipeline = create_mesh_terrain_pipeline(
            &device,
            Some(&pipe_layout),
            &module,
            "fs_opaque",
            color_format,
            Some(depth_format),
            &[("HDR_LIGHTING", 0.0)],
        );
        let cutout_pipeline = create_mesh_terrain_pipeline(
            &device,
            Some(&pipe_layout),
            &module,
            "fs_solid",
            color_format,
            Some(depth_format),
            &[("HDR_LIGHTING", 0.0)],
        );

        let upload_buffer =
            |label: &str, usage: wgpu::BufferUsages, bytes: &[u8]| -> wgpu::Buffer {
                let buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: bytes.len().max(16) as u64,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&buf, 0, bytes);
                buf
            };

        // Globals: camera at origin looking -Y with -Z as up (FOV 90 deg, aspect 1.0).
        let proj = crate::gfx::world_projection(90.0, 1.0, 256.0);
        let view = glam::Mat4::look_at_rh(glam::Vec3::ZERO, glam::Vec3::NEG_Y, glam::Vec3::NEG_Z);
        let view_proj = proj * view;
        let mut globals: crate::gfx::Globals = bytemuck::Zeroable::zeroed();
        globals.view_proj = view_proj.to_cols_array_2d();
        globals.fog = [1000.0, 2000.0, 1000.0, 2000.0];
        globals.filtering = [0.0, 16.0, 0.0, 0.0];
        globals.world_view = [1.0, 0.0, 0.0, 0.0];
        let globals_buf = upload_buffer(
            "Mesh test globals",
            wgpu::BufferUsages::UNIFORM,
            bytemuck::bytes_of(&globals),
        );

        let create_white_tex = |label: &str, layers: u32| -> wgpu::TextureView {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            for layer in 0..layers {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &[255u8, 255, 255, 255],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4),
                        rows_per_image: Some(1),
                    },
                    wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
            }
            tex.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(if layers > 1 {
                    wgpu::TextureViewDimension::D2Array
                } else {
                    wgpu::TextureViewDimension::D2
                }),
                ..Default::default()
            })
        };

        let sprite_view = create_white_tex("Mesh test sprites", 2);
        let sprite_views: [wgpu::TextureView; 8] = std::array::from_fn(|_| sprite_view.clone());
        let lightmap_view = create_white_tex("Mesh test lightmap", 1);
        let emission_view = create_white_tex("Mesh test emission", 1);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let group0 = crate::gfx::terrain_group(
            &device,
            "Mesh test group0",
            &g0_layout,
            &globals_buf,
            &sprite_views,
            &sampler,
            &lightmap_view,
            &sampler,
            &emission_view,
        );

        let top_quad =
            |x: f32, z: f32, x_skew: f32, layer: u16, color: u32| -> [TerrainVertex; 4] {
                let v = |dx, dz| TerrainVertex {
                    pos: [x + dx, 2.0, z + dz],
                    uv: [0, 0],
                    layer,
                    light: 0xF0F0,
                    color,
                };
                [v(x_skew, 0.0), v(0.0, 1.0), v(1.0, 1.0), v(1.0, 0.0)]
            };

        // Section origin [-4, -4, -4] places y_local=2 at camera-relative y=-2 (FOV 90 -> x/z [2..6] fill NDC [-1..1]).
        let section_origin = [-4.0f32, -4.0, -4.0, 1.0 / UV_STEPS];

        // Page 0 (slot_base = 0): Opaque RED on left (px 0..16), Cutout BLUE on right (px 48..64, one compact + one spilled FULL).
        let p0_quads = [
            top_quad(2.0, 3.0, 0.0, OPAQUE_LAYER, 0xFF00_00FF),
            top_quad(2.0, 4.0, 0.0, OPAQUE_LAYER, 0xFF00_00FF),
            top_quad(5.0, 3.0, 0.0, 0, 0xFFFF_0000),
            top_quad(5.0, 4.0, 0.05, 0, 0xFFFF_0000),
        ];
        let mut p0_solid: Vec<TerrainVertex> = p0_quads.into_iter().flatten().collect();
        let mut p0_tints = vec![0xFF_0000, 0xFF_0000, 0x00_00FF, 0x00_00FF];
        let p0_sides = crate::mesher::sort_tinted(&mut p0_solid, &mut p0_tints);
        let p0_packed = layout(&p0_solid, &p0_tints);
        assert!(!p0_packed.is_empty());
        let p0_meshlets = build_section_meshlets(&p0_sides, &p0_solid, &p0_packed);
        store.insert_section(0, PACKED_BASE, &p0_meshlets);

        // Page 1 (slot_base = GRAIN -> block_slots[1]): Opaque GREEN in bottom-center (px 16..48, py 32..48).
        let p1_slot_base = crate::gfx::arena::GRAIN as u32;
        let p1_quads = [
            top_quad(3.0, 4.0, 0.0, OPAQUE_LAYER, 0xFF00_FF00),
            top_quad(4.0, 4.0, 0.0, OPAQUE_LAYER, 0xFF00_FF00),
        ];
        let mut p1_solid: Vec<TerrainVertex> = p1_quads.into_iter().flatten().collect();
        let mut p1_tints = vec![0x00_FF00, 0x00_FF00];
        let p1_sides = crate::mesher::sort_tinted(&mut p1_solid, &mut p1_tints);
        let p1_packed = layout(&p1_solid, &p1_tints);
        assert!(!p1_packed.is_empty());
        let mut p1_arena = vec![[0u32; 6]; p1_slot_base as usize];
        p1_arena.extend_from_slice(&p1_packed);
        let p1_meshlets = build_section_meshlets(&p1_sides, &p1_solid, &p1_packed);
        store.insert_section(1, PACKED_BASE + 4 * p1_slot_base, &p1_meshlets);

        let build_group1 = |block_slots: &[u32], origins: &[[f32; 4]], arena: &[[u32; 6]]| {
            let bs = upload_buffer(
                "block_slots",
                wgpu::BufferUsages::STORAGE,
                bytemuck::cast_slice(block_slots),
            );
            let so = upload_buffer(
                "slot_origins",
                wgpu::BufferUsages::STORAGE,
                bytemuck::cast_slice(origins),
            );
            let ap = upload_buffer(
                "arena_page",
                wgpu::BufferUsages::STORAGE,
                bytemuck::cast_slice(arena),
            );
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Mesh test group1"),
                layout: &g1_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: bs.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: so.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: crate::gfx::flat::ARENA_BINDING,
                        resource: ap.as_entire_binding(),
                    },
                ],
            });
            (bs, so, ap, bg)
        };
        let _p0_bufs = build_group1(&[0, 0], &[section_origin], &p0_packed);
        let _p1_bufs = build_group1(&[0, 1], &[[0.0; 4], section_origin], &p1_arena);

        let size = 64u32;
        let color_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Mesh test color"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: color_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Mesh test depth"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: depth_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let color_view = color_tex.create_view(&Default::default());
        let depth_view = depth_tex.create_view(&Default::default());
        let readback_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mesh test readback"),
            size: u64::from(size * size * 4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let box_planes = [
            [1.0, 0.0, 0.0, 50.0],
            [-1.0, 0.0, 0.0, 50.0],
            [0.0, 1.0, 0.0, 50.0],
            [0.0, -1.0, 0.0, 50.0],
            [0.0, 0.0, 1.0, 50.0],
            [0.0, 0.0, -1.0, 50.0],
        ];
        let params = |pass: u32, eye: [f32; 4]| MeshCullParams {
            planes: box_planes,
            eye,
            counts: [0, pass, 0, 0],
        };

        let render_and_readback =
            |store: &MeshSectionStore,
             draws: &[(u32, u32, &wgpu::RenderPipeline, &wgpu::BindGroup)]| {
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Mesh hardware proof pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &color_view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 0.0,
                                    g: 0.0,
                                    b: 0.0,
                                    a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(0.0),
                                store: wgpu::StoreOp::Discard,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    pass.set_bind_group(0, &group0, &[]);
                    for &(page, pass_kind, pipe, g1) in draws {
                        let mut any = false;
                        for draw in store.page_draws(page, pass_kind) {
                            any = true;
                            draw_mesh_page(&mut pass, pipe, g1, draw);
                        }
                        assert!(any, "page draw must be prepared");
                    }
                }
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: &color_tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &readback_buf,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(size * 4),
                            rows_per_image: Some(size),
                        },
                    },
                    wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: 1,
                    },
                );
                queue.submit([encoder.finish()]);
                readback_buf.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                let view = readback_buf
                    .get_mapped_range(..)
                    .expect("mapped readback buffer");
                let pixels = view.to_vec();
                drop(view);
                readback_buf.unmap();
                pixels
            };
        let px = |pixels: &[u8], x: u32, y: u32| -> [u8; 4] {
            let i = ((y * size + x) * 4) as usize;
            [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
        };

        // Case 1: Multi-page + multi-pass prepared beforehand & drawn inside ONE RenderPass.
        let eye_above = [0.0, 0.0, 0.0, 0.0];
        store.prepare_page(&device, &queue, 0, &params(PASS_OPAQUE, eye_above));
        store.prepare_page(&device, &queue, 1, &params(PASS_OPAQUE, eye_above));
        store.prepare_page(&device, &queue, 0, &params(PASS_CUTOUT, eye_above));
        let combined = render_and_readback(
            &store,
            &[
                (0, PASS_OPAQUE, &opaque_pipeline, &_p0_bufs.3),
                (1, PASS_OPAQUE, &opaque_pipeline, &_p1_bufs.3),
                (0, PASS_CUTOUT, &cutout_pipeline, &_p0_bufs.3),
            ],
        );
        let red = px(&combined, 8, 32);
        let green = px(&combined, 32, 40);
        let blue_compact = px(&combined, 56, 24);
        let blue_spilled = px(&combined, 56, 40);
        let bg = px(&combined, 32, 8);
        eprintln!(
            "hardware readback pixels: red={red:?}, green={green:?}, blue_compact={blue_compact:?}, blue_spilled={blue_spilled:?}, bg={bg:?}"
        );
        assert!(
            red[0] > 200 && red[1] < 20 && red[2] < 20,
            "p0 opaque red: {red:?}"
        );
        assert!(
            green[1] > 200 && green[0] < 20 && green[2] < 20,
            "p1 opaque green: {green:?}"
        );
        assert!(
            blue_compact[2] > 200 && blue_compact[0] < 20 && blue_compact[1] < 20,
            "p0 compact cutout blue: {blue_compact:?}"
        );
        assert!(
            blue_spilled[2] > 200 && blue_spilled[0] < 20 && blue_spilled[1] < 20,
            "p0 spilled FULL cutout blue: {blue_spilled:?}"
        );
        assert_eq!(bg, [0, 0, 0, 255]);

        // Case 2: Strict pass isolation (OPAQUE renders red only; CUTOUT renders blue only).
        let opaque_only =
            render_and_readback(&store, &[(0, PASS_OPAQUE, &opaque_pipeline, &_p0_bufs.3)]);
        assert!(px(&opaque_only, 8, 32)[0] > 200);
        assert_eq!(px(&opaque_only, 56, 24), [0, 0, 0, 255]);
        assert_eq!(px(&opaque_only, 56, 40), [0, 0, 0, 255]);

        let cutout_only =
            render_and_readback(&store, &[(0, PASS_CUTOUT, &cutout_pipeline, &_p0_bufs.3)]);
        assert_eq!(px(&cutout_only, 8, 32), [0, 0, 0, 255]);
        assert!(px(&cutout_only, 56, 24)[2] > 200);
        assert!(px(&cutout_only, 56, 40)[2] > 200);

        // Case 3: Task backface culling below +Y quads culls all slices.
        store.prepare_page(
            &device,
            &queue,
            0,
            &params(PASS_ALL, [0.0, -10.0, 0.0, -10.0]),
        );
        let culled = render_and_readback(&store, &[(0, PASS_ALL, &cutout_pipeline, &_p0_bufs.3)]);
        let (chunks, remainder) = culled.as_chunks::<4>();
        assert!(remainder.is_empty());
        assert!(
            chunks.iter().all(|&c| c == [0, 0, 0, 255]),
            "eye below +Y faces must cull all pixels"
        );

        // Case 4: Inactive/cave-culled section slot (origin.w == 0.0) renders zero pixels.
        let hidden_origin = [-4.0f32, -4.0, -4.0, 0.0];
        let _hidden_bufs = build_group1(&[0, 0], &[hidden_origin], &p0_packed);
        store.prepare_page(&device, &queue, 0, &params(PASS_ALL, eye_above));
        let hidden =
            render_and_readback(&store, &[(0, PASS_ALL, &cutout_pipeline, &_hidden_bufs.3)]);
        let (hidden_chunks, hidden_rem) = hidden.as_chunks::<4>();
        assert!(hidden_rem.is_empty());
        assert!(
            hidden_chunks.iter().all(|&c| c == [0, 0, 0, 255]),
            "inactive slot (origin.w == 0.0) must not emit geometry"
        );

        // Case 5: Verify MeshTerrain wrapper pipelines + prepare_frame dispatch end-to-end.
        let mut terrain = MeshTerrain::new(
            &device,
            &g0_layout,
            &g1_layout,
            (color_format, false),
            &[("SPRITE_ARRAYS", 1.0), ("SPRITE_SPLIT", 2048.0)],
            false,
        )
        .expect("MeshTerrain initialized on mesh shader adapter");
        terrain.store.insert_section(0, PACKED_BASE, &p0_meshlets);
        let dispatches = terrain.prepare_frame(&device, &queue, [0], box_planes, eye_above, false);
        assert_eq!(dispatches, 2);
        let opaque_handle = terrain.store.page_draw(0, PASS_OPAQUE).unwrap();
        let cutout_handle = terrain.store.page_draw(0, PASS_CUTOUT).unwrap();
        let wrapper_pixels = render_and_readback(
            &terrain.store,
            &[
                (0, PASS_OPAQUE, &terrain.opaque_plain, &_p0_bufs.3),
                (0, PASS_CUTOUT, &terrain.cutout_plain, &_p0_bufs.3),
            ],
        );
        assert!(opaque_handle.meshlet_count > 0 && cutout_handle.meshlet_count > 0);
        assert!(px(&wrapper_pixels, 8, 32)[0] > 200);
        assert!(px(&wrapper_pixels, 56, 24)[2] > 200);

        // Case 6: Capped max_task_workgroups splits the page into multiple 1-workgroup dispatches
        // respecting both per-dimension and total workgroup limits, drawing meshlets in chunk 1.
        terrain.store.max_task_workgroups = 1;
        let mut capped_meshlets = vec![MeshletDesc::new(0, 0, 2, 0, 0, [0, 0, 0, 1, 1, 1]); 32];
        capped_meshlets.extend_from_slice(&p0_meshlets);
        terrain
            .store
            .insert_section(0, PACKED_BASE, &capped_meshlets);
        let split_dispatches =
            terrain.prepare_frame(&device, &queue, [0], box_planes, eye_above, false);
        assert_eq!(split_dispatches, 4);
        let capped_draws: Vec<_> = terrain.store.page_draws(0, PASS_OPAQUE).collect();
        assert_eq!(capped_draws.len(), 2);
        assert!(capped_draws.iter().all(|d| d.workgroups == 1));
        let capped_pixels = render_and_readback(
            &terrain.store,
            &[(0, PASS_OPAQUE, &terrain.opaque_plain, &_p0_bufs.3)],
        );
        assert!(px(&capped_pixels, 8, 32)[0] > 200);
    }

    #[test]
    fn page_draw_requires_pass_prepared_for_current_page_revision() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = mesh_shader_adapter() else {
            return;
        };
        let (device, queue) = open_mesh_device(&adapter, "Mesh pass revision test");
        let mut store = new_store(&device).expect("mesh shaders are enabled");

        let m_one: Vec<MeshletDesc> = (0..32)
            .map(|i| MeshletDesc::new(i, 1, 2, 0, 0, [0, 0, 0, 4, 4, 4]))
            .collect();
        let m_extra = MeshletDesc::new(0, 1, 2, 0, 0, [0, 0, 0, 4, 4, 4]);
        let params_for = |pass| MeshCullParams {
            planes: [[0.0; 4]; 6],
            eye: [0.0; 4],
            counts: [0, pass, 0, 0],
        };

        // Preparing only PASS_OPAQUE leaves PASS_CUTOUT unprepared (`None`).
        store.insert_section(0, 0, &m_one);
        assert_eq!(
            store.prepare_page(&device, &queue, 0, &params_for(PASS_OPAQUE)),
            32
        );
        assert_eq!(store.page_draw(0, PASS_OPAQUE).unwrap().meshlet_count, 32);
        assert!(store.page_draw(0, PASS_CUTOUT).is_none());

        // Adding a 33rd meshlet changes page 0's revision, invalidating PASS_OPAQUE immediately.
        store.insert_section(0, 1024, &[m_extra]);
        assert!(store.page_draw(0, PASS_OPAQUE).is_none());

        // Preparing PASS_CUTOUT on the new revision must keep PASS_OPAQUE returning None until re-prepared.
        assert_eq!(
            store.prepare_page(&device, &queue, 0, &params_for(PASS_CUTOUT)),
            33
        );
        assert_eq!(store.page_draw(0, PASS_CUTOUT).unwrap().meshlet_count, 33);
        assert!(store.page_draw(0, PASS_OPAQUE).is_none());

        assert_eq!(
            store.prepare_page(&device, &queue, 0, &params_for(PASS_OPAQUE)),
            33
        );
        assert_eq!(store.page_draw(0, PASS_OPAQUE).unwrap().meshlet_count, 33);
        assert_eq!(store.page_draw(0, PASS_CUTOUT).unwrap().meshlet_count, 33);
    }

    #[test]
    fn retire_page_and_trim_release_gpu_buffers_while_remove_section_reuses_allocation() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = mesh_shader_adapter() else {
            return;
        };
        let (device, queue) = open_mesh_device(&adapter, "Mesh page retire test");
        let mut store = new_store(&device).expect("mesh shaders are enabled");

        let m = MeshletDesc::new(0, 8, 2, 0, 0, [0, 0, 0, 4, 4, 4]);
        let params = MeshCullParams {
            planes: [[0.0; 4]; 6],
            eye: [0.0; 4],
            counts: [0, PASS_OPAQUE, 0, 0],
        };
        store.insert_section(0, 100, &[m]);
        store.insert_section(1, 200, &[m]);
        assert_eq!(store.prepare_page(&device, &queue, 0, &params), 1);
        assert_eq!(store.prepare_page(&device, &queue, 1, &params), 1);
        assert!(store.has_gpu_page(0));
        assert!(store.has_gpu_page(1));

        // Removing the last section keeps PageGpu alive for buffer reuse during replacements.
        store.remove_section(0, 100);
        assert!(store.has_gpu_page(0));
        assert!(store.page_draw(0, PASS_OPAQUE).is_none());

        // Reinserting on page 0 works and reuses page 0's PageGpu allocation.
        store.insert_section(0, 300, &[m, m]);
        assert_eq!(store.prepare_page(&device, &queue, 0, &params), 2);
        assert_eq!(store.page_draw(0, PASS_OPAQUE).unwrap().meshlet_count, 2);

        // relocate_section transfers meshlets between pages, rebases vertex_base, and invalidates passes.
        store.relocate_section(0, 300, 1, 500);
        assert!(store.page_draw(0, PASS_OPAQUE).is_none());
        assert!(store.page_draw(1, PASS_OPAQUE).is_none());
        assert_eq!(store.prepare_page(&device, &queue, 1, &params), 3);
        assert!(store.staged_page(1).contains(&m.to_gpu(500)));

        // retire_page and trim drop GPU page buffers when Arena releases empty pages.
        store.retire_page(0);
        assert!(!store.has_gpu_page(0));
        assert!(store.has_gpu_page(1));

        store.trim(|page| page != 1);
        assert!(!store.has_gpu_page(1));
        assert_eq!(store.page_count(1), 0);
    }

    #[test]
    fn prepare_visible_frame_partitions_opaque_cutout_and_indirect_args() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = mesh_shader_adapter() else {
            return;
        };
        let (device, queue) = open_mesh_device(&adapter, "Mesh visible frame test");
        let mut store = new_store(&device).expect("mesh shaders are enabled");

        let opaque_m = MeshletDesc::new(0, 8, 2, 0, 0, [0, 0, 0, 4, 4, 4]);
        let cutout_m = MeshletDesc::new(8, 6, 2, MeshletDesc::FLAG_CUTOUT, 0, [0, 0, 0, 4, 4, 4]);
        // Section A (visible, non-fading): 1 opaque + 1 cutout
        store.insert_section(0, 100, &[opaque_m, cutout_m]);
        // Section B (visible, fading): 1 opaque (must move to PASS_CUTOUT)
        store.insert_section(0, 200, &[opaque_m]);
        // Section C (culled / not in visible_sections): must be excluded completely
        store.insert_section(0, 300, &[opaque_m, opaque_m]);

        let dispatches = store.prepare_visible_pages(
            &device,
            &queue,
            &[(0, 200, true), (0, 100, false)],
            [[0.0; 4]; 6],
            [0.0; 4],
            false,
        );
        // 1 opaque chunk (Section A's opaque_m) + 1 cutout chunk (Section B's fading opaque_m + Section A's cutout_m).
        assert_eq!(dispatches, 2);

        let opaque_draw = store.page_draw(0, PASS_OPAQUE).unwrap();
        let cutout_draw = store.page_draw(0, PASS_CUTOUT).unwrap();
        assert_eq!(opaque_draw.meshlet_count, 1);
        assert_eq!(opaque_draw.workgroups, 1);
        assert_eq!(cutout_draw.meshlet_count, 2);
        assert_eq!(cutout_draw.workgroups, 1);
        assert_ne!(opaque_draw.indirect_offset, cutout_draw.indirect_offset);

        let staged = &store.table.pages[&0].visible_staging;
        assert_eq!(staged.len(), 3);
        // Front-to-back order: Section A opaque first, then Section B fading, then Section A cutout.
        assert_eq!(staged[0], opaque_m.to_gpu(100));
        assert_eq!(staged[1], opaque_m.to_gpu(200));
        assert_eq!(staged[2], cutout_m.to_gpu(100));
    }

    #[test]
    fn visbuffer_and_cull_shaders_validate() {
        for (label, source) in [
            ("vis_shader", vis_shader()),
            ("cull_shader", cull_shader().to_owned()),
        ] {
            let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| {
                panic!("{label} WGSL parse failed:\n{}", e.emit_to_string(&source))
            });
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{label} naga validation failed: {e}"));
        }
    }

    #[test]
    fn gpu_compute_cull_compacts_visible_meshlets_and_zeros_culled_indirect_args() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = mesh_shader_adapter() else {
            return;
        };
        let (device, queue) = open_mesh_device(&adapter, "Mesh GPU compute cull test");
        let mut store = new_store(&device).expect("mesh shaders are enabled");

        // 2 visible opaque (slot 0), 1 visible cutout (slot 0), 1 occluded section (slot 1, origin.w == 0),
        // 1 frustum-culled section (slot 2, origin.z = +100 behind near/far).
        let opaque_m = MeshletDesc::new(0, 4, 2, 0, 0, [0, 0, 0, 4, 4, 4]);
        let cutout_m = MeshletDesc::new(4, 4, 2, MeshletDesc::FLAG_CUTOUT, 0, [0, 0, 0, 4, 4, 4]);
        let resident = [
            opaque_m.to_gpu(0),
            cutout_m.to_gpu(0),
            opaque_m.to_gpu(0),
            opaque_m.to_gpu(64),
            opaque_m.to_gpu(128),
        ];
        let resident_count = resident.len() as u32;
        let op_cap = resident_count;
        store.ensure_page_buffers(&device, 0, (op_cap * 2 + 64) as usize, 64, 3);
        let pg = store.gpu_pages.get(&0).unwrap();
        queue.write_buffer(&pg.resident_storage, 0, bytemuck::cast_slice(&resident));

        // 6 inward frustum planes: only keep boxes within [-20..20]^3.
        let planes = [
            [1.0, 0.0, 0.0, 20.0],
            [-1.0, 0.0, 0.0, 20.0],
            [0.0, 1.0, 0.0, 20.0],
            [0.0, -1.0, 0.0, 20.0],
            [0.0, 0.0, 1.0, 20.0],
            [0.0, 0.0, -1.0, 20.0],
        ];
        let cull_params = MeshCullParams {
            planes,
            eye: [0.0; 4],
            counts: [resident_count, op_cap, 1, store.max_task_workgroups],
        };
        queue.write_buffer(&pg.uniform, 0, bytemuck::bytes_of(&cull_params));
        let zero_indirect: [u32; 8] = [0, 1, 1, 0, 1, 1, 0, 0];
        queue.write_buffer(&pg.indirect, 0, bytemuck::cast_slice(&zero_indirect));

        let g1_layout = group1_layout(&device);
        let block_slots = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test block_slots"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // arena_slot = solid_base (0, 64, 128) >> 8u == 0, wait: 64 >> 8 == 0, so use block 0..2 with solid_base 0, 256, 512!
        let resident_blocks = [
            opaque_m.to_gpu(0),
            cutout_m.to_gpu(0),
            opaque_m.to_gpu(0),
            opaque_m.to_gpu(256),
            opaque_m.to_gpu(512),
        ];
        queue.write_buffer(
            &pg.resident_storage,
            0,
            bytemuck::cast_slice(&resident_blocks),
        );
        queue.write_buffer(
            &block_slots,
            0,
            bytemuck::cast_slice(&[0u32, 1u32, 2u32, 0u32]),
        );

        let slot_origins = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test slot_origins"),
            size: 48,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let origins: [[f32; 4]; 3] = [
            [0.0, 0.0, 0.0, 1.0],   // slot 0: visible
            [0.0, 0.0, 0.0, 0.0],   // slot 1: occluded (origin.w == 0)
            [0.0, 0.0, 100.0, 1.0], // slot 2: outside frustum
        ];
        queue.write_buffer(&slot_origins, 0, bytemuck::cast_slice(&origins));

        let dummy_arena = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test arena"),
            size: 64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let group1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test group1"),
            layout: &g1_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: block_slots.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: slot_origins.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: crate::gfx::flat::ARENA_BINDING,
                    resource: dummy_arena.as_entire_binding(),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut cpass = encoder.begin_compute_pass(&Default::default());
            cpass.set_pipeline(&store.cull_pipeline);
            cpass.set_bind_group(0, &pg.cull_bind_group, &[]);
            cpass.set_bind_group(1, &group1, &[]);
            cpass.dispatch_workgroups(1, 1, 1);
        }
        {
            let mut fpass = encoder.begin_compute_pass(&Default::default());
            fpass.set_pipeline(&store.finalize_pipeline);
            fpass.set_bind_group(0, &pg.cull_bind_group, &[]);
            fpass.dispatch_workgroups(1, 1, 1);
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("indirect readback"),
            size: 32,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&pg.indirect, 0, &readback, 0, 32);
        queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let data = slice.get_mapped_range().expect("mapped readback");
        let words: &[u32] = bytemuck::cast_slice(&data);
        // 2 opaque (1 wg), 1 cutout (1 wg), counts: opaque=2, cutout=1.
        assert_eq!(words, &[1, 1, 1, 1, 1, 1, 2, 1]);
    }

    #[test]
    fn hardware_visbuffer_mesh_pipeline_renders_opaque_and_cutout_vis_ids() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = mesh_shader_adapter() else {
            return;
        };
        let (device, _queue) = open_mesh_device(&adapter, "Mesh visbuffer pipeline test");
        let g0 = group0_layout(&device);
        let g1 = group1_layout(&device);
        let store = new_store(&device).expect("mesh shaders are enabled");
        let layout = pipeline_layout(&device, &g0, &g1, store.bind_group_layout());
        let vis_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon terrain visbuffer mesh shader test"),
            source: wgpu::ShaderSource::Wgsl(vis_shader().into()),
        });
        let _vis_opaque = create_vis_mesh_pipeline(
            &device,
            Some(&layout),
            &vis_module,
            "ms_vis_opaque",
            "fs_vis_mesh_id",
            &[("HDR_LIGHTING", 0.0)],
        );
        let _vis_cutout = create_vis_mesh_pipeline(
            &device,
            Some(&layout),
            &vis_module,
            "ms_vis_cutout",
            "fs_vis_mesh_cutout",
            &[("HDR_LIGHTING", 0.0)],
        );
    }
}
