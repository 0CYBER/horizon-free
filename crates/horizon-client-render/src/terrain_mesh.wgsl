// Appended to terrain.wgsl (gfx/mesh_shader.rs): hardware task and mesh shader stages
// for sub-section 64-quad meshlets (#17243 step 2).
struct MeshCullParams {
    // Six frustum planes in camera-relative world space: xyz inward normal, w offset.
    planes: array<vec4<f32>, 6>,
    // xyz: camera eye (camera-relative), w: low eye y (water mirror).
    eye: vec4<f32>,
    // x: meshlet count, y: pass_kind (0 = all, 1 = opaque, 2 = cutout), z: flags (bit 0 = skip facing cull), w: meshlet base offset.
    counts: vec4<u32>,
}
// Array of 16-byte `GpuMeshlet` descriptors (rebased via `MeshletDesc::to_gpu(solid_base)`):
// - x: vertex_base (PACKED_BASE + 4 * slot or uncompressed full-vertex index)
// - y: quad_count (bits 0..7) | bucket (bits 8..15) | flags (bits 16..23) | plane_quant (bits 24..31)
// - z: min_x (bits 0..7) | min_y (bits 8..15) | min_z (bits 16..23) | max_x (bits 24..31)
// - w: max_y (bits 0..7) | max_z (bits 8..15) | padding (bits 16..31)
@group(2) @binding(0) var<storage, read> meshlets: array<vec4<u32>>;
@group(2) @binding(1) var<uniform> mesh_cull: MeshCullParams;

struct MeshTaskPayload {
    // Per spawned mesh workgroup: x = base vertex index (PACKED_BASE + 4 * slot or vertex),
    // y = quad count (1..=32). Up to 64 slices for 32 meshlets per task workgroup.
    slices: array<vec2<u32>, 64>,
}
var<task_payload> mesh_payload: MeshTaskPayload;
var<workgroup> lane_offsets: array<u32, 32>;
var<workgroup> total_slices: u32;

const MESHLET_MARGIN: f32 = 1.0 / 16.0;

// Matches `Sides::BUCKETS` (0: +X, 1: +Z, 2: Other, 3: +Y, 4: -Z, 5: -X, 6: -Y)
// against the meshlet's own quantized sub-AABB (`min_local..max_local`) inside the section.
fn meshlet_facing(
    bucket: u32,
    min_local: vec3<f32>,
    max_local: vec3<f32>,
    rel_eye: vec3<f32>,
    rel_low: f32,
) -> bool {
    switch bucket {
        case 0u: { return rel_eye.x > min_local.x - MESHLET_MARGIN; }
        case 1u: { return rel_eye.z > min_local.z - MESHLET_MARGIN; }
        case 3u: { return rel_eye.y > min_local.y - MESHLET_MARGIN; }
        case 4u: { return rel_eye.z < max_local.z + MESHLET_MARGIN; }
        case 5u: { return rel_eye.x < max_local.x + MESHLET_MARGIN; }
        case 6u: { return rel_low < max_local.y + MESHLET_MARGIN; }
        default: { return true; }
    }
}

fn meshlet_in_frustum(bmin: vec3<f32>, bmax: vec3<f32>) -> bool {
    for (var p = 0u; p < 6u; p++) {
        let plane = mesh_cull.planes[p];
        let pv = vec3(
            select(bmin.x, bmax.x, plane.x >= 0.0),
            select(bmin.y, bmax.y, plane.y >= 0.0),
            select(bmin.z, bmax.z, plane.z >= 0.0),
        );
        if dot(plane.xyz, pv) + plane.w < 0.0 {
            return false;
        }
    }
    return true;
}

@task @payload(mesh_payload) @workgroup_size(32)
fn ts_terrain(
    @builtin(local_invocation_index) lane: u32,
    @builtin(workgroup_id) wg: vec3<u32>,
) -> @builtin(mesh_task_size) vec3<u32> {
    var my_slices = 0u;
    var my_vertex_base = 0u;
    var my_quad_count = 0u;

    let idx = mesh_cull.counts.w + wg.x * 32u + lane;
    if idx < mesh_cull.counts.x {
        let desc = meshlets[idx];
        let vertex_base = desc.x;
        let quad_count = desc.y & 0xffu;
        let bucket = (desc.y >> 8u) & 0xffu;
        let flags = (desc.y >> 16u) & 0xffu;
        let min_local = vec3<f32>(
            f32(desc.z & 0xffu),
            f32((desc.z >> 8u) & 0xffu),
            f32((desc.z >> 16u) & 0xffu),
        );
        let max_local = vec3<f32>(
            f32((desc.z >> 24u) & 0xffu),
            f32(desc.w & 0xffu),
            f32((desc.w >> 8u) & 0xffu),
        );
        let arena_slot = select(
            vertex_base,
            (vertex_base - PACKED_BASE) >> 2u,
            vertex_base >= PACKED_BASE,
        );
        let origin = slot_origins[block_slots[arena_slot >> 8u]];
        let pass_kind = mesh_cull.counts.y;
        let is_cutout_pass = (flags & 1u) != 0u || fade_code(origin.w) != 0u;
        let pass_match = pass_kind == 0u
            || (pass_kind == 1u && !is_cutout_pass)
            || (pass_kind == 2u && is_cutout_pass);
        let unbounded = (flags & 4u) != 0u;
        let skip_facing = (mesh_cull.counts.z & 1u) != 0u;

        if pass_match
            && origin.w != 0.0
            && quad_count > 0u
            && (unbounded
                || ((skip_facing
                    || meshlet_facing(
                        bucket,
                        min_local,
                        max_local,
                        mesh_cull.eye.xyz - origin.xyz,
                        mesh_cull.eye.w - origin.y,
                    ))
                    && meshlet_in_frustum(origin.xyz + min_local, origin.xyz + max_local)))
        {
            my_slices = select(1u, 2u, quad_count > 32u);
            my_vertex_base = vertex_base;
            my_quad_count = quad_count;
        }
    }

    lane_offsets[lane] = my_slices;
    workgroupBarrier();

    // Stable lane-prefix scan preserving original descriptor and overlay order across all 32 lanes
    if lane == 0u {
        var sum = 0u;
        for (var i = 0u; i < 32u; i++) {
            let count = lane_offsets[i];
            lane_offsets[i] = sum;
            sum += count;
        }
        total_slices = sum;
    }
    workgroupBarrier();

    if my_slices > 0u {
        let slot = lane_offsets[lane];
        let first_count = min(my_quad_count, 32u);
        mesh_payload.slices[slot] = vec2(my_vertex_base, first_count);
        if my_slices == 2u {
            mesh_payload.slices[slot + 1u] = vec2(my_vertex_base + 32u * 4u, my_quad_count - 32u);
        }
    }

    workgroupBarrier();
    return vec3(total_slices, 1u, 1u);
}

struct MeshPrimitive {
    @builtin(triangle_indices) indices: vec3<u32>,
}

struct TerrainMeshOutput {
    @builtin(vertex_count) vertex_count: u32,
    @builtin(primitive_count) primitive_count: u32,
    @builtin(vertices) vertices: array<TerrainWire, 128>,
    @builtin(primitives) primitives: array<MeshPrimitive, 64>,
}
var<workgroup> terrain_mesh_out: TerrainMeshOutput;

@mesh(terrain_mesh_out) @payload(mesh_payload) @workgroup_size(32)
fn ms_terrain(
    @builtin(local_invocation_index) lane: u32,
    @builtin(workgroup_id) wg: vec3<u32>,
) {
    let slice = mesh_payload.slices[wg.x];
    let base_vertex = slice.x;
    let quad_count = slice.y;
    if lane == 0u {
        terrain_mesh_out.vertex_count = quad_count * 4u;
        terrain_mesh_out.primitive_count = quad_count * 2u;
    }
    if lane < quad_count {
        let q_base = base_vertex + lane * 4u;
        let v_out = lane * 4u;
        let p_out = lane * 2u;
        let s0 = section_corner(q_base);
        let origin = slot_origins[block_slots[s0.slot >> 8u]];
        terrain_mesh_out.vertices[v_out] = to_wire(corner_vertex(s0.c, origin));
        terrain_mesh_out.vertices[v_out + 1u] =
            to_wire(corner_vertex(section_corner(q_base + 1u).c, origin));
        terrain_mesh_out.vertices[v_out + 2u] =
            to_wire(corner_vertex(section_corner(q_base + 2u).c, origin));
        terrain_mesh_out.vertices[v_out + 3u] =
            to_wire(corner_vertex(section_corner(q_base + 3u).c, origin));
        terrain_mesh_out.primitives[p_out].indices = vec3(v_out, v_out + 1u, v_out + 2u);
        terrain_mesh_out.primitives[p_out + 1u].indices = vec3(v_out, v_out + 2u, v_out + 3u);
    }
}
