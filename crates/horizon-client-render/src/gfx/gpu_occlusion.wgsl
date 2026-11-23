// GPU culling's occlusion test (gpu_culling.rs `BoxTest`), after
// gpu_culling.wgsl in one module. On compute Hi-Z backends (`prepare_boxes` +
// `test_boxes`), one compute thread per queued box tests its 8 corners against
// the reversed-Z min-depth Hi-Z pyramid (`hiz_depth`); on the `Early` raster
// fallback (`vs_box` + `EARLY_FS_BOX` in gpu_culling.rs), boxes are drawn front
// faces only against phase 1's depth. Then `count2`, `scan2` and `scatter2`
// write phase 2's draws.
@group(0) @binding(9) var<storage, read> box_list: array<u32>;
@group(0) @binding(10) var<storage, read> box_origins: array<vec4<u32>>;
@group(0) @binding(11) var<storage, read_write> box_runs: array<u32>;
@group(0) @binding(12) var<storage, read> box_sections: array<Section>;
@group(1) @binding(0) var hiz_depth: texture_2d<f32>;

// The corners (bit 0: x, bit 1: y, bit 2: z) of the box's 12 triangles,
// counter-clockwise seen from outside (cull.wgsl's `box_triangle_corner`).
// Packed four bits each, eight a word, read through a switch: a constant array indexed by the
// vertex would be a private array, which Adreno's Vulkan compiler (build 0676.41) fails to link.
fn box_triangle_corner(v: u32) -> u32 {
    var word: u32;
    switch v / 8u {
        case 0u: { word = 0x31640260u; }
        case 1u: { word = 0x05105717u; }
        case 2u: { word = 0x76237245u; }
        case 3u: { word = 0x54320130u; }
        default: { word = 0x00006747u; }
    }
    return (word >> (v % 8u * 4u)) & 0xfu;
}

struct BoxOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) id: u32,
}

@vertex
fn vs_box(@builtin(vertex_index) v: u32, @builtin(instance_index) t: u32) -> BoxOut {
    let id = box_list[params.occl.y + t];
    let o = bitcast<vec3<f32>>(box_origins[id].xyz);
    var out: BoxOut;
    out.id = id;
    out.position = params.view_proj * vec4(box_corner(o, box_sections[id].a.z, box_triangle_corner(v)), 1.0);
    return out;
}

// Compute the indirect dispatch dimensions for `test_boxes` from `counts[BOX + 1u]`
// (queued box count), storing (x, y, z) into `counts[BOX + 2u..5u]`.
@compute @workgroup_size(1)
fn prepare_boxes() {
    let n = atomicLoad(&counts[BOX + 1u]);
    let wg = (n + BLOCK - 1u) / BLOCK;
    let x = min(wg, ROW);
    let y = select(0u, (wg + ROW - 1u) / ROW, wg > 0u);
    atomicStore(&counts[BOX + 2u], x);
    atomicStore(&counts[BOX + 3u], y);
    atomicStore(&counts[BOX + 4u], select(0u, 1u, wg > 0u));
}

// Conservative Hi-Z compute occlusion test for GPU culling: 1 thread per queued box,
// reading `box_list` (`counts`, read-only) while dispatched indirectly from `counts[BOX + 2u]`.
@compute @workgroup_size(64)
fn test_boxes(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    let t = (group.y * ROW + group.x) * BLOCK + lane;
    let n = box_list[BOX + 1u];
    if t >= n {
        return;
    }
    let id = box_list[params.occl.y + t];
    let o = bitcast<vec3<f32>>(box_origins[id].xyz);
    let b = box_sections[id].a.z;

    var lo = vec2(1e30);
    var hi = vec2(-1e30);
    var max_z = -1e30;
    for (var c = 0u; c < 8u; c++) {
        let p = params.view_proj * vec4(box_corner(o, b, c), 1.0);
        if !(p.w > 1e-6 && p.z < p.w) {
            box_runs[id * RUN_WORDS + SEEN_WORD] = params.occl.x;
            return;
        }
        let inv_w = 1.0 / p.w;
        let ndc = p.xy * inv_w;
        let z = p.z * inv_w;
        lo = min(lo, ndc);
        hi = max(hi, ndc);
        max_z = max(max_z, z);
    }

    if hi.x < -1.0 || lo.x > 1.0 || hi.y < -1.0 || lo.y > 1.0 || max_z < 0.0 {
        return;
    }

    let ndc_lo = max(lo, vec2(-1.0));
    let ndc_hi = min(hi, vec2(1.0));
    let uv_min = vec2(ndc_lo.x * 0.5 + 0.5, 0.5 - ndc_hi.y * 0.5);
    let uv_max = vec2(ndc_hi.x * 0.5 + 0.5, 0.5 - ndc_lo.y * 0.5);

    let max_mip = i32(textureNumLevels(hiz_depth)) - 1;
    let hiz_base = textureDimensions(hiz_depth, 0);
    let hiz_size = vec2<f32>(hiz_base);
    let rect = max((uv_max - uv_min) * hiz_size, vec2(0.0));
    let max_extent = max(rect.x, rect.y);
    var mip = clamp(i32(ceil(log2(max(max_extent, 1.0)))), 0, max_mip);

    // Derive mip dimensions from mip 0 by integer shift rather than dynamic
    // textureDimensions(hiz_depth, mip) to avoid extra descriptor queries and
    // non-uniform OpImageQuerySizeLod lane miscompiles on Mesa llvmpipe.
    if mip > 0 {
        let fine = mip - 1;
        let fine_size = vec2<i32>(max(hiz_base >> vec2<u32>(u32(fine)), vec2<u32>(1u)));
        let f_lo = clamp(vec2<i32>(floor(uv_min * vec2<f32>(fine_size))), vec2(0), fine_size - vec2(1));
        let f_hi = clamp(vec2<i32>(floor(uv_max * vec2<f32>(fine_size))), vec2(0), fine_size - vec2(1));
        if (f_hi.x - f_lo.x <= 1) && (f_hi.y - f_lo.y <= 1) {
            mip = fine;
        }
    }

    var mip_size = vec2<i32>(max(hiz_base >> vec2<u32>(u32(mip)), vec2<u32>(1u)));
    var lo_px = clamp(vec2<i32>(floor(uv_min * vec2<f32>(mip_size))), vec2(0), mip_size - vec2(1));
    var hi_px = clamp(vec2<i32>(floor(uv_max * vec2<f32>(mip_size))), vec2(0), mip_size - vec2(1));
    if ((hi_px.x - lo_px.x > 1) || (hi_px.y - lo_px.y > 1)) && mip < max_mip {
        mip += 1;
        mip_size = vec2<i32>(max(hiz_base >> vec2<u32>(u32(mip)), vec2<u32>(1u)));
        lo_px = clamp(vec2<i32>(floor(uv_min * vec2<f32>(mip_size))), vec2(0), mip_size - vec2(1));
        hi_px = clamp(vec2<i32>(floor(uv_max * vec2<f32>(mip_size))), vec2(0), mip_size - vec2(1));
    }

    let d00 = textureLoad(hiz_depth, vec2<i32>(lo_px.x, lo_px.y), mip).r;
    let d10 = textureLoad(hiz_depth, vec2<i32>(hi_px.x, lo_px.y), mip).r;
    let d01 = textureLoad(hiz_depth, vec2<i32>(lo_px.x, hi_px.y), mip).r;
    let d11 = textureLoad(hiz_depth, vec2<i32>(hi_px.x, hi_px.y), mip).r;
    let occluder_min_z = min(min(d00, d10), min(d01, d11));

    if !(max_z < occluder_min_z) {
        box_runs[id * RUN_WORDS + SEEN_WORD] = params.occl.x;
    }
}

var<workgroup> tally: array<atomic<u32>, 3>;

// After the test, one workgroup per block of ids: the quads of the sections
// with runs, the tested boxes not seen and their quads, and per (page, band)
// each block's draws that phase 2 makes (`drawn_in`'s condition).
@compute @workgroup_size(64)
fn count2(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    if lane < 3u {
        atomicStore(&tally[lane], 0u);
    }
    if lane < FAST_BUCKETS {
        atomicStore(&wg_buckets[lane], 0u);
    }
    workgroupBarrier();
    let block = group.y * ROW + group.x;
    let id = block * BLOCK + lane;
    if id < params.ids {
        let at = id * RUN_WORDS;
        let w = runs[at];
        let n = w & 7u;
        if n > 0u {
            var quads = 0u;
            for (var k = 0u; k < n; k++) {
                quads += runs[at + 2u + 2u * k];
            }
            atomicAdd(&tally[0], quads);
            let saw = runs[at + SEEN_WORD] == params.occl.x;
            if (w & TESTED) != 0u && !saw {
                atomicAdd(&tally[1], 1u);
                atomicAdd(&tally[2], quads);
            }
            if (w & PHASE2) != 0u && saw {
                let page = w >> PAGE_SHIFT;
                let band = (w >> BAND_SHIFT) & BAND_MASK;
                if page < FAST_PAGES {
                    atomicAdd(&wg_buckets[page * BANDS + band], n);
                } else {
                    atomicAdd(&counts[block_count(block, page, band, 1u)], n);
                }
            }
        }
    }
    workgroupBarrier();
    if lane == 0u {
        let q = atomicLoad(&tally[0]);
        if q > 0u {
            atomicAdd(&counts[QUADS], q);
        }
        let h = atomicLoad(&tally[1]);
        if h > 0u {
            atomicAdd(&counts[HIDDEN], h);
        }
        let hq = atomicLoad(&tally[2]);
        if hq > 0u {
            atomicAdd(&counts[HIDDEN_QUADS], hq);
        }
    }
    if lane < FAST_BUCKETS {
        let count = atomicLoad(&wg_buckets[lane]);
        if count > 0u {
            let page = lane / BANDS;
            let band = lane % BANDS;
            let blocks = (params.ids + BLOCK - 1u) / BLOCK;
            atomicStore(&counts[block_base(page, band, 1u, blocks) + block], count);
        }
    }
}

@compute @workgroup_size(256)
fn scan2(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    scan_page(group.x, lane, 1u);
}

@compute @workgroup_size(64)
fn scatter2(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    scatter_block(group.y * ROW + group.x, lane, 1u);
}
