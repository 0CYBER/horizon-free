// Appended after terrain.wgsl + terrain_mesh.wgsl: Visbuffer ID-only mesh and fragment stages
// (`HORIZON_VISBUFFER=1` + `HORIZON_MESH_SHADER=1`).

fn vis_mesh_id(page: u32, s: SectionCorner) -> u32 {
    let at = 0x80000000u | ((page & 0x3fu) << 24u);
    if s.packed {
        return at | 0x40000000u | s.slot;
    }
    return at | (s.slot >> 2u);
}

struct VisMeshIdOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) id: u32,
}

struct VisMeshOpaqueOutput {
    @builtin(vertex_count) vertex_count: u32,
    @builtin(primitive_count) primitive_count: u32,
    @builtin(vertices) vertices: array<VisMeshIdOut, 128>,
    @builtin(primitives) primitives: array<MeshPrimitive, 64>,
}
var<workgroup> vis_opaque_out: VisMeshOpaqueOutput;

@mesh(vis_opaque_out) @payload(mesh_payload) @workgroup_size(128)
fn ms_vis_opaque(
    @builtin(local_invocation_index) lane: u32,
    @builtin(workgroup_id) wg: vec3<u32>,
) {
    let slice = mesh_payload.slices[wg.x];
    let base_vertex = slice.x;
    let quad_count = slice.y;
    let vert_count = quad_count * 4u;
    let prim_count = quad_count * 2u;
    if lane == 0u {
        vis_opaque_out.vertex_count = vert_count;
        vis_opaque_out.primitive_count = prim_count;
    }
    if lane < vert_count {
        let s = section_corner(base_vertex + lane);
        let origin = slot_origins[block_slots[s.slot >> 8u]];
        let page = (mesh_cull.counts.z >> 8u) & 0x3fu;
        let position = bitcast<vec3<f32>>(s.c.p);
        vis_opaque_out.vertices[lane] = VisMeshIdOut(
            fade_depth(globals.view_proj * vec4(position + origin.xyz, 1.0), origin.w),
            vis_mesh_id(page, s),
        );
    }
    if lane < prim_count {
        let v_out = (lane >> 1u) * 4u;
        vis_opaque_out.primitives[lane].indices = select(
            vec3(v_out, v_out + 1u, v_out + 2u),
            vec3(v_out, v_out + 2u, v_out + 3u),
            (lane & 1u) != 0u,
        );
    }
}

@fragment fn fs_vis_mesh_id(in: VisMeshIdOut) -> @location(0) u32 {
    return in.id;
}

struct VisMeshCutoutOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) id: u32,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) layer: u32,
    @location(3) alpha: f32,
    @location(4) @interpolate(flat) fade: u32,
    @location(5) local: vec3<f32>,
}

struct VisMeshCutoutOutput {
    @builtin(vertex_count) vertex_count: u32,
    @builtin(primitive_count) primitive_count: u32,
    @builtin(vertices) vertices: array<VisMeshCutoutOut, 128>,
    @builtin(primitives) primitives: array<MeshPrimitive, 64>,
}
var<workgroup> vis_cutout_out: VisMeshCutoutOutput;

@mesh(vis_cutout_out) @payload(mesh_payload) @workgroup_size(128)
fn ms_vis_cutout(
    @builtin(local_invocation_index) lane: u32,
    @builtin(workgroup_id) wg: vec3<u32>,
) {
    let slice = mesh_payload.slices[wg.x];
    let base_vertex = slice.x;
    let quad_count = slice.y;
    let vert_count = quad_count * 4u;
    let prim_count = quad_count * 2u;
    if lane == 0u {
        vis_cutout_out.vertex_count = vert_count;
        vis_cutout_out.primitive_count = prim_count;
    }
    if lane < vert_count {
        let s = section_corner(base_vertex + lane);
        let origin = slot_origins[block_slots[s.slot >> 8u]];
        let page = (mesh_cull.counts.z >> 8u) & 0x3fu;
        let layer = corner_layer(s.c);
        let position = bitcast<vec3<f32>>(s.c.p);
        let uv = vec2(s.c.r.x & 0xffffu, s.c.r.x >> 16u);
        let light = vec2((s.c.r.y >> 16u) & 0xffu, s.c.r.y >> 24u);
        let pos = position + origin.xyz;
        let block = max(f32(light.y), dynamic_block_light(pos));
        vis_cutout_out.vertices[lane] = VisMeshCutoutOut(
            fade_depth(globals.view_proj * vec4(pos, 1.0), origin.w),
            vis_mesh_id(page, s),
            terrain_uv(uv, origin.w),
            layer,
            sample_lightmap(f32(light.x) / 16.0, block / 16.0).a,
            fade_code(origin.w),
            position,
        );
    }
    if lane < prim_count {
        let v_out = (lane >> 1u) * 4u;
        vis_cutout_out.primitives[lane].indices = select(
            vec3(v_out, v_out + 1u, v_out + 2u),
            vec3(v_out, v_out + 2u, v_out + 3u),
            (lane & 1u) != 0u,
        );
    }
}

@fragment fn fs_vis_mesh_cutout(in: VisMeshCutoutOut) -> @location(0) u32 {
    let dx = dpdx(in.local);
    let dy = dpdy(in.local);
    if faded(in.fade, in.local, dx, dy) { discard; }
    let alpha = sprite_at(in.uv, in.layer & 0x7fffu, dpdx(in.uv), dpdy(in.uv)).a * in.alpha;
    if alpha < 0.5 && (in.layer & 0x8000u) == 0u { discard; }
    return in.id;
}
