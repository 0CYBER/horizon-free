// GPU-side meshlet frustum/facing culling and opaque/cutout compaction compute shader.
struct MeshCullParams {
    planes: array<vec4<f32>, 6>,
    eye: vec4<f32>,
    // x: resident_count, y: opaque_capacity, z: flags (bit 0 = skip facing cull), w: max_task_workgroups
    counts: vec4<u32>,
}

@group(0) @binding(0) var<storage, read> resident_meshlets: array<vec4<u32>>;
@group(0) @binding(1) var<storage, read_write> active_meshlets: array<vec4<u32>>;
// [0..2]: opaque indirect (wg_x, 1, 1); [3..5]: cutout indirect (wg_x, 1, 1);
// [6]: opaque_count; [7]: cutout_count.
@group(0) @binding(2) var<storage, read_write> indirect_state: array<atomic<u32>>;
@group(0) @binding(3) var<uniform> mesh_cull: MeshCullParams;

@group(1) @binding(0) var<storage, read> block_slots: array<u32>;
@group(1) @binding(1) var<storage, read> slot_origins: array<vec4<f32>>;

const MESHLET_MARGIN: f32 = 1.0 / 16.0;
const PACKED_BASE: u32 = 0x10000000u;
const ARENA_GRAIN_SHIFT: u32 = 6u;

var<workgroup> wg_offsets: array<u32, 64>;
var<workgroup> wg_group_sums: array<u32, 8>;
var<workgroup> wg_opaque_base: u32;
var<workgroup> wg_cutout_base: u32;

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

fn meshlet_in_frustum(min_world: vec3<f32>, max_world: vec3<f32>) -> bool {
    for (var i = 0u; i < 6u; i++) {
        let p = mesh_cull.planes[i];
        let v = select(min_world, max_world, p.xyz >= vec3(0.0));
        if dot(p.xyz, v) + p.w < 0.0 {
            return false;
        }
    }
    return true;
}

// Culls resident meshlets against frustum, backface and section visibility, compacting
// survivors in workgroup shared memory before performing at most 2 global atomicAdd calls
// per 64-lane workgroup.
@compute @workgroup_size(64)
fn cs_cull_meshlets(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let idx = gid.x;
    var desc = vec4<u32>(0u);
    var keep_opaque = false;
    var keep_cutout = false;
    if idx < mesh_cull.counts.x {
        desc = resident_meshlets[idx];
        let vertex_base = desc.x;
        let quad_count = desc.y & 0xffu;
        if quad_count != 0u {
            let bucket = (desc.y >> 8u) & 0xffu;
            let flags = (desc.y >> 16u) & 0xffu;
            let arena_slot = select(
                vertex_base,
                (vertex_base - PACKED_BASE) >> 2u,
                vertex_base >= PACKED_BASE,
            );
            let origin = slot_origins[block_slots[arena_slot >> ARENA_GRAIN_SHIFT]];
            if origin.w != 0.0 {
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
                let unbounded = (flags & 4u) != 0u;
                let skip_facing = (mesh_cull.counts.z & 1u) != 0u;
                var visible = true;
                if !unbounded {
                    if !skip_facing
                        && !meshlet_facing(
                            bucket,
                            min_local,
                            max_local,
                            mesh_cull.eye.xyz - origin.xyz,
                            mesh_cull.eye.w - origin.y,
                        )
                    {
                        visible = false;
                    }
                    if visible && !meshlet_in_frustum(origin.xyz + min_local, origin.xyz + max_local) {
                        visible = false;
                    }
                }
                if visible {
                    let is_cutout = (flags & 1u) != 0u || floor(max(-origin.w, 0.0)) > 0.0;
                    keep_cutout = is_cutout;
                    keep_opaque = !is_cutout;
                }
            }
        }
    }

    // Pack opaque survival into low 16 bits and cutout survival into high 16 bits.
    // A 64-lane workgroup sums to at most 64 in each half, so low bits never carry into high bits.
    wg_offsets[lid] = select(0u, 1u, keep_opaque) | select(0u, 0x10000u, keep_cutout);
    workgroupBarrier();

    if lid < 8u {
        let base = lid * 8u;
        var sum = 0u;
        for (var i = 0u; i < 8u; i++) {
            let v = wg_offsets[base + i];
            wg_offsets[base + i] = sum;
            sum += v;
        }
        wg_group_sums[lid] = sum;
    }
    workgroupBarrier();

    if lid == 0u {
        var sum = 0u;
        for (var g = 0u; g < 8u; g++) {
            let v = wg_group_sums[g];
            wg_group_sums[g] = sum;
            sum += v;
        }
        let opaque_total = sum & 0xffffu;
        let cutout_total = sum >> 16u;
        if opaque_total > 0u {
            wg_opaque_base = atomicAdd(&indirect_state[6], opaque_total);
        } else {
            wg_opaque_base = 0u;
        }
        if cutout_total > 0u {
            wg_cutout_base = atomicAdd(&indirect_state[7], cutout_total);
        } else {
            wg_cutout_base = 0u;
        }
    }
    workgroupBarrier();

    if keep_opaque || keep_cutout {
        let local_prefix = wg_offsets[lid] + wg_group_sums[lid >> 3u];
        if keep_cutout {
            let out_idx = wg_cutout_base + (local_prefix >> 16u);
            active_meshlets[mesh_cull.counts.y + out_idx] = desc;
        } else {
            let out_idx = wg_opaque_base + (local_prefix & 0xffffu);
            active_meshlets[out_idx] = desc;
        }
    }
}

@compute @workgroup_size(32)
fn cs_finalize_meshlets(@builtin(local_invocation_index) lane: u32) {
    let opaque_count = atomicLoad(&indirect_state[6]);
    let cutout_count = atomicLoad(&indirect_state[7]);
    let max_wg = mesh_cull.counts.w;
    if lane == 0u {
        atomicStore(&indirect_state[0], min((opaque_count + 31u) >> 5u, max_wg));
        atomicStore(&indirect_state[3], min((cutout_count + 31u) >> 5u, max_wg));
    }
    let opaque_pad = (32u - (opaque_count & 31u)) & 31u;
    if lane < opaque_pad && opaque_count + lane < mesh_cull.counts.y {
        active_meshlets[opaque_count + lane] = vec4<u32>(0u);
    }
    let cutout_pad = (32u - (cutout_count & 31u)) & 31u;
    if lane < cutout_pad && cutout_count + lane < mesh_cull.counts.y {
        active_meshlets[mesh_cull.counts.y + cutout_count + lane] = vec4<u32>(0u);
    }
}
