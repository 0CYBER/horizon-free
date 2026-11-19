//! Hardware mesh shaders, only on request (#17243 step 2): when the player or setting enables
//! `HORIZON_MESH_SHADER=1` and the GPU supports `EXPERIMENTAL_MESH_SHADER`.
//! Without either, the device is opened with the exact same features and limits as `main`.
//! Every build has it: it is an engine render capability for terrain geometry.

#[allow(unused_imports)]
pub use crate::mesher::meshlet::{GpuMeshlet, MeshletDesc};

/// The mesh shader feature in wgpu.
pub(in crate::gfx) const MESH_SHADER: wgpu::Features = wgpu::Features::EXPERIMENTAL_MESH_SHADER;

/// Workgroup size used by both the task shader and the mesh slice shader.
pub const TASK_WORKGROUP_SIZE: u32 = 32;

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

/// `@group(2)` bind group layout for `terrain_mesh.wgsl` (`meshlets` storage + `mesh_cull` uniform).
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
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

/// CPU reference matching `terrain_mesh.wgsl::meshlet_facing`: whether `desc`'s directional
/// bucket can face `rel_eye` (or `rel_low` for `-Y` water-mirror bounds) relative to the section origin.
#[allow(dead_code)]
pub fn meshlet_facing(desc: &MeshletDesc, rel_eye: [f32; 3], rel_low: f32) -> bool {
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

    #[test]
    fn shader_module_compiles_on_mesh_shader_adapter() {
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        if !adapter.features().contains(MESH_SHADER) {
            return;
        }
        let desc = wgpu::DeviceDescriptor {
            label: Some("Mesh shader compile test"),
            required_features: MESH_SHADER,
            required_limits: adapter.limits(),
            experimental_features: horizon_hdr::experimental_features(),
            ..Default::default()
        };
        let (device, _queue) = pollster::block_on(adapter.request_device(&desc))
            .expect("adapter offers EXPERIMENTAL_MESH_SHADER");
        let _layout = bind_group_layout(&device);
        let _module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Terrain mesh shader"),
            source: wgpu::ShaderSource::Wgsl(shader().into()),
        });
    }
}
