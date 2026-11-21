// GPU culling (gfx/gpu_culling.rs). `apply` writes the records that changed.
// `cull`: one invocation per section id keeps the sections whose column is
// within the render distance, that cave culling's graph reaches (occlusion.rs)
// and whose box is in the frustum, writes each one's origin and the runs of its
// solid quads that can face the eye, a draw each, and counts them per (page,
// distance band) and block of 64 ids. `scan`: one workgroup per page turns the
// blocks' counts into where each block's draws start in the page's draw region
// (front-to-back by distance band, `BANDS`), and the page's count. `scatter`:
// each block writes its sections' draws there, ordered per page and phase by
// distance band, section id, and run. Within a band section order stays
// deterministic rather than depending on GPU scheduling (where faces of two
// sections meet at the same depth the later draw keeps the pixel), while moving
// the camera across a band boundary can reorder sections whose bands change.
// Nothing here computes with floats: the origins are the CPU's bits, copied,
// the frustum test adds integers the CPU computed, and facing compares keys
// that order as the floats the CPU compares. `cull` leaves sections fading in
// or out to the CPU (`FADING`).
// Frames that occlude (`OCCLUDE`, gpu_occlusion.wgsl) draw in two phases, each
// ordered per page by distance band, section id, and run: `cull` leaves to
// phase 2 the sections whose box the last test did not see and lists the boxes
// to test; after the test, `count2`, `scan2` and `scatter2` write the draws of
// those it saw into the same regions. Which boxes count as seen untested
// (`near_or_big`) is float arithmetic of the shader's own: it only sorts
// sections between the phases, and doubt counts as seen.

struct Params {
    // Ids to test, records `apply` writes, the arena pages the counts and
    // regions cover, and where the regions start in `tables`.
    ids: u32,
    updates: u32,
    pages: u32,
    regions: u32,
    // Bits 3j, 3j + 1 and 3j + 2: frustum plane j's x, y and z are at least 0.
    signs: u32,
    // The render distance in columns, and the camera's column.
    near: i32,
    cam_x: i32,
    cam_z: i32,
    // The tables' first column: x, 0, z, 0.
    first: vec4<i32>,
    // Their entries along x, y and z, and 0.
    cells: vec4<u32>,
    // Each frustum plane's w term, its margin in, then 0, 0.
    w: array<vec4<i32>, 2>,
    // Cave culling's graph (occlusion.rs `Reach`): from x, from z, radius, lowest
    // section y; its height in sections and the level's; REACH, SKY, EYE,
    // OCCLUDE and PROBE; where the eye's keys start in `tables`.
    reach: vec4<i32>,
    reach_height: i32,
    reach_sections: i32,
    flags: u32,
    eyes: u32,
    // Occlusion (OCCLUDE): this frame's stamp (never 0), where the boxes to
    // test start in `counts`, 0, 0.
    occl: vec4<u32>,
    // The world pass's view projection, TAA's jitter included (gfx.rs
    // `globals.view_proj`).
    view_proj: mat4x4<f32>,
}

// gpu_culling.rs `Record`.
struct Section {
    // Section x, y (from the level's bottom), z, and its arena page (-1: a
    // free id).
    key: vec4<i32>,
    // The first vertex of its solid quads, its solid quads (where its last
    // bucket by facing side ends), its box (gpu_culling.rs `rounded_box`), then
    // its flags (`FADING`).
    a: vec4<u32>,
    // Where its first six buckets end (mesher/sides.rs `BUCKETS`): +X, +Z, the
    // other bucket, +Y, -Z, -X.
    ends: array<u32, 6>,
    // What its buckets facing +X, +Z, +Y, -Z, -X and -Y compare the eye's keys
    // with (gpu_culling.rs `Record::facing`).
    keys: array<u32, 6>,
}

// gpu_culling.rs `Update`: the record for id `id.x`.
struct Update {
    id: vec4<u32>,
    section: Section,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> sections: array<Section>;
// gpu_culling.rs `tables`: the origins' f32 bits along x, y and z (n of them);
// per frustum plane its terms for those origins, n each; per plane and axis its
// terms for the 32 box codes; with an eye (`EYE`), the keys of the eye less
// each origin, n of them; then each page's region of `draws` (its first draw
// and its size).
@group(0) @binding(2) var<storage, read> tables: array<u32>;
@group(0) @binding(3) var<storage, read> updates: array<Update>;
// The statistics (`STATS`), each page's draws in phase 1 and in phase 2, then
// per phase, page, distance band and block of ids its draws (`cull`, `count2`),
// turned into where they start (`scan`, `scan2`), then the boxes to test
// (`cull`).
@group(0) @binding(4) var<storage, read_write> counts: array<atomic<u32>>;
// DrawIndexedIndirectArgs, five words each.
@group(0) @binding(5) var<storage, read_write> draws: array<u32>;
// Per id the instance it is drawn with: origin and UV step, as f32 bits.
@group(0) @binding(6) var<storage, read_write> origins: array<vec4<u32>>;
// Per id `RUN_WORDS`: its (page << PAGE_SHIFT) | (band << BAND_SHIFT) | TESTED
// | PHASE2 | its draws when it is drawn this frame, else 0; then each draw's
// first quad and quads; then the stamps of the frames whose test last saw its
// box and last tested it, kept from frame to frame.
@group(0) @binding(7) var<storage, read_write> runs: array<u32>;
// occlusion.rs `Reach::bits`: whether the search reached each cell of its box,
// a bit each (32 a word, lowest first).
@group(0) @binding(8) var<storage, read> reached_cells: array<u32>;

// gfx.rs `section_instance`'s w for a whole section: 1 / mesher::UV_STEPS.
const STEP_BITS: u32 = 0x38000000u;
// Workgroups along x when the ids need more than one row of them.
const ROW: u32 = 65535u;
// Ids per block.
const BLOCK: u32 = 64u;
// Counts before the pages': sections kept and draws, then occlusion's: the
// quads of the sections with runs and those of the tested boxes not seen, the
// box draw (four words and a spare), those boxes, phase 2's draws, a spare.
const STATS: u32 = 12u;
const DRAWS: u32 = 1u;
const QUADS: u32 = 2u;
const HIDDEN_QUADS: u32 = 3u;
const BOX: u32 = 4u;
const HIDDEN: u32 = 9u;
const DRAWN2: u32 = 10u;
// gpu_culling.rs `rounded_box` for a box past what its codes hold: no plane
// rejects it.
const UNBOUNDED: u32 = 0x80000000u;
// In `params.flags`: cave culling's graph is in `params.reach`, and its search
// reached the box's top layer (the sky band).
const REACH: u32 = 1u;
const SKY: u32 = 2u;
// In a section's `a.w`: it fades in or out, and the CPU draws it with its fade.
const FADING: u32 = 1u;
// In `params.flags`: solid quads facing away from the eye are left out, its
// keys in `tables` from `params.eyes`.
const EYE: u32 = 4u;
// Words per id in `runs`, and where its distance band and page start in the
// first word, with its flags and draws (at most 4: gpu_culling.rs `RUNS`) below.
const RUN_WORDS: u32 = 11u;
const BANDS: u32 = 4u;
const BAND_SHIFT: u32 = 5u;
const BAND_MASK: u32 = 3u;
const PAGE_SHIFT: u32 = 7u;
// Pages reduced directly in workgroup shared memory in `cull` and `count2`;
// worlds with more pages fall back to global atomics for pages >= FAST_PAGES.
const FAST_PAGES: u32 = 8u;
const FAST_BUCKETS: u32 = 32u; // FAST_PAGES * BANDS
// In its first word: its box is tested this frame; it waits for phase 2.
const TESTED: u32 = 16u;
const PHASE2: u32 = 8u;
// Its stamps: the frames whose test last saw its box, and last tested it.
const SEEN_WORD: u32 = 9u;
const TESTED_WORD: u32 = 10u;
// In `params.flags`: the frame occludes, and tests every box (cull.rs's probe).
const OCCLUDE: u32 = 8u;
const PROBE: u32 = 16u;
// cull.rs `RETEST` and `BIG`.
const RETEST: u32 = 8u;
const BIG: f32 = 0.25;
// gpu_culling.rs `CODE_OFFSET`: the rounded box's codes hold -8..=23.
const CODE_OFFSET: i32 = 8i;
// Past the box, for rounding: far past what f32 moves a camera-relative corner.
const EPS: f32 = 0.015625;
// Clip z / w at about twice the near plane's distance: reversed depth is 1 at
// the near plane and grows toward the eye; behind it, z > 0 >= w.
const NEAR_DEPTH: f32 = 0.5;

// `page`'s count draw in `phase` (0: the first world pass; 1: after the test).
fn page_count(page: u32, phase: u32) -> u32 {
    return STATS + phase * params.pages + page;
}

// The count of `block`'s draws in `(page, band)` and `phase` (`cull`, `count2`),
// then where they start in `page`'s contiguous draw region (`scan`: band 0
// first, then band 1 .. BANDS - 1). Laid out with `block` as the innermost index
// so `scan_page` reads and writes `blocks` contiguously across warp lanes
// (stride-1 coalesced access per `(page, band)`).
fn block_base(page: u32, band: u32, phase: u32, blocks: u32) -> u32 {
    return STATS + 2u * params.pages + ((phase * params.pages + page) * BANDS + band) * blocks;
}

fn block_count(block: u32, page: u32, band: u32, phase: u32) -> u32 {
    let blocks = (params.ids + BLOCK - 1u) / BLOCK;
    return block_base(page, band, phase, blocks) + block;
}

// Distance band of section `(sx, sz)` from camera column `(cam_x, cam_z)`:
// 4 geometric bands covering 0..4, 4..8, 8..16, and >=16 chunks from the camera
// (64, 128, 256+ blocks), matching early-Z importance with half the scan work.
fn distance_band(sx: i32, sz: i32) -> u32 {
    let dx = abs(sx - params.cam_x);
    let dz = abs(sz - params.cam_z);
    let d2 = u32(dx * dx + dz * dz);
    return select(0u, 1u, d2 >= 16u) + select(0u, 1u, d2 >= 64u) + select(0u, 1u, d2 >= 256u);
}

// The id's first `runs` word, its flags cleared, if it draws in `phase` (in
// phase 2, only once this frame's test saw its box); else 0.
fn drawn_in(id: u32, phase: u32) -> u32 {
    if id >= params.ids {
        return 0u;
    }
    let w = runs[id * RUN_WORDS];
    if (w & 7u) == 0u || ((w & PHASE2) != 0u) != (phase == 1u) {
        return 0u;
    }
    if (w & PHASE2) != 0u && runs[id * RUN_WORDS + SEEN_WORD] != params.occl.x {
        return 0u;
    }
    return w & ~(PHASE2 | TESTED);
}

// `page`'s region of `draws`: its first draw and its size.
fn region(page: u32) -> vec2<u32> {
    let at = params.regions + 2u * page;
    return vec2<u32>(tables[at], tables[at + 1u]);
}

// streaming.rs `in_view_distance` around the camera's column. A gap at or past
// the distance cannot pass, so testing it first keeps the squares in range.
fn near(x: i32, z: i32) -> bool {
    let r = params.near;
    let gx = max(abs(x - params.cam_x) - 1, 0);
    let gz = max(abs(z - params.cam_z) - 1, 0);
    if gx >= r || gz >= r {
        return false;
    }
    return gx * gx + gz * gz < r * r;
}

// occlusion.rs `Occlusion::contains` (`Graph::contains`): with no graph, or with
// cave culling off, every section; in the searched box, whether the search
// reached it; above a clamped box, over its columns and within the level, the
// sky band (all of it or none); else none. `Reach::contains` on the CPU.
fn reached(key: vec3<i32>) -> bool {
    if (params.flags & REACH) == 0u {
        return true;
    }
    let r = params.reach.z;
    let dx = key.x - params.reach.x;
    let dz = key.z - params.reach.y;
    let dy = key.y - params.reach.w;
    if dx < -r || dx > r || dz < -r || dz > r || dy < 0 {
        return false;
    }
    if dy >= params.reach_height {
        return (params.flags & SKY) != 0u && key.y >= 0 && key.y <= params.reach_sections;
    }
    let i = u32(((dz + r) * (2 * r + 1) + dx + r) * params.reach_height + dy);
    return ((reached_cells[i >> 5u] >> (i & 31u)) & 1u) != 0u;
}

fn term(at: u32) -> i32 {
    return bitcast<i32>(tables[at]);
}

// gfx.rs `Frustum::intersects` for the box `b` of the section in cell `c` of the
// tables, in integers (gpu_culling.rs `tables`): it keeps every box the CPU keeps.
fn in_frustum(c: vec3<u32>, b: u32) -> bool {
    if (b & UNBOUNDED) != 0u {
        return true;
    }
    let nx = params.cells.x;
    let ny = params.cells.y;
    let n = nx + ny + params.cells.z;
    for (var j = 0u; j < 6u; j++) {
        // The box's codes at the corner the plane looks at.
        let s = params.signs >> (3u * j);
        let kx = (b >> select(0u, 15u, (s & 1u) != 0u)) & 31u;
        let ky = (b >> select(5u, 20u, (s & 2u) != 0u)) & 31u;
        let kz = (b >> select(10u, 25u, (s & 4u) != 0u)) & 31u;
        let origin_terms = n + j * n;
        let code_terms = 7u * n + 96u * j;
        let sum = params.w[j / 4u][j % 4u] + term(origin_terms + c.x)
            + term(origin_terms + nx + c.y) + term(origin_terms + nx + ny + c.z)
            + term(code_terms + kx) + term(code_terms + 32u + ky) + term(code_terms + 64u + kz);
        if sum < 0 {
            return false;
        }
    }
    return true;
}

// Where bucket `b` of section `id` ends (mesher/sides.rs `BUCKETS` order): its
// solid quads for the last.
fn bucket_end(id: u32, b: u32) -> u32 {
    if b == 6u {
        return sections[id].a.y;
    }
    return sections[id].ends[b];
}

// mesher/sides.rs `Sides::visible_from` for bucket `b` of section `id` (+X, +Z,
// the other bucket, +Y, -Z, -X, -Y), with `eye` the keys of the eye less its
// origin: whether its quads can face the eye. -Y compares the eye's own y, as a
// GPU frame has no water mirror. The keys order as the floats the CPU compares.
fn seen(id: u32, b: u32, eye: vec3<u32>) -> bool {
    if b == 2u {
        return true;
    }
    let t = sections[id].keys[b - select(0u, 1u, b > 2u)];
    switch b {
        case 0u: { return eye.x > t; }
        case 1u: { return eye.z > t; }
        case 3u: { return eye.y > t; }
        case 4u: { return eye.z < t; }
        case 5u: { return eye.x < t; }
        default: { return eye.y < t; }
    }
}

// Corner `c` (bit 0: x, bit 1: y, bit 2: z) of the box tested for the section
// at `o` whose rounded box is `b` (never UNBOUNDED here): cull.wgsl's box, the
// cell grown by a block, grown further to the section's own bounds where they
// reach past it, then by EPS.
fn box_corner(o: vec3<f32>, b: u32, c: u32) -> vec3<f32> {
    let low = vec3<i32>(vec3<u32>(b, b >> 5u, b >> 10u) & vec3(31u)) - vec3(CODE_OFFSET);
    let high = vec3<i32>(vec3<u32>(b >> 15u, b >> 20u, b >> 25u) & vec3(31u)) - vec3(CODE_OFFSET);
    let lo = min(vec3<f32>(low), vec3(-1.0)) - vec3(EPS);
    let hi = max(vec3<f32>(high), vec3(17.0)) + vec3(EPS);
    return o + select(lo, hi, (vec3(c) & vec3(1u, 2u, 4u)) != vec3(0u));
}

// Boxes that count as seen untested: one with a corner within about twice the
// near plane's distance, behind the eye, or NaN, whose front faces near
// clipping could cut away; or, as in cull.wgsl, one over BIG of the screen.
fn near_or_big(o: vec3<f32>, b: u32) -> bool {
    var lo = vec2(1e30);
    var hi = vec2(-1e30);
    for (var c = 0u; c < 8u; c++) {
        let p = params.view_proj * vec4(box_corner(o, b, c), 1.0);
        if !(p.z < NEAR_DEPTH * p.w) {
            return true;
        }
        let q = p.xy / p.w;
        lo = min(lo, q);
        hi = max(hi, q);
    }
    let size = max(min(hi, vec2(1.0)) - max(lo, vec2(-1.0)), vec2(0.0));
    return size.x * size.y > BIG * 4.0;
}

// In frames that occlude, a kept section with runs: its flags (0: phase 1,
// untested), its box listed when it is tested. Its last test saw it: phase 1,
// retested every RETEST frames (every frame with PROBE). A box that counts as
// seen: phase 1. Else phase 2, drawn if this frame's test sees its box.
fn occlusion(id: u32, at: u32, o: vec3<f32>, b: u32) -> u32 {
    let stamp = params.occl.x;
    let seen = runs[at + SEEN_WORD];
    let passed = seen != 0u && seen == runs[at + TESTED_WORD];
    if passed && (params.flags & PROBE) == 0u && (id + stamp) % RETEST != 0u {
        return 0u;
    }
    if (b & UNBOUNDED) != 0u || near_or_big(o, b) {
        runs[at + SEEN_WORD] = stamp;
        runs[at + TESTED_WORD] = stamp;
        return 0u;
    }
    runs[at + TESTED_WORD] = stamp;
    let t = atomicAdd(&counts[BOX + 1u], 1u);
    atomicStore(&counts[params.occl.y + t], id);
    return TESTED | select(PHASE2, 0u, passed);
}

// One invocation per changed record; the CPU lists each id once.
@compute @workgroup_size(64)
fn apply(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x < params.updates {
        let u = updates[gid.x];
        sections[u.id.x] = u.section;
    }
}

// Workgroup-local reductions for `cull` and `count2` (one workgroup owns one
// `block` exclusively, so its `block_count(block, ...)` slots have no cross-
// workgroup contention and need at most one atomicStore per non-zero bucket).
var<workgroup> wg_kept: atomic<u32>;
var<workgroup> wg_buckets: array<atomic<u32>, 32>;

@compute @workgroup_size(64)
fn cull(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    let block = group.y * ROW + group.x;
    let id = block * BLOCK + lane;
    if lane == 0u {
        atomicStore(&wg_kept, 0u);
        if block == 0u && (params.flags & OCCLUDE) != 0u {
            // The box draw's vertices: the cube's corners (gpu_occlusion.wgsl).
            atomicStore(&counts[BOX], 36u);
        }
    }
    if lane < FAST_BUCKETS {
        atomicStore(&wg_buckets[lane], 0u);
    }
    workgroupBarrier();

    // Past the table's end the records are stale: the CPU never writes them free.
    if id < params.ids {
        let at = id * RUN_WORDS;
        runs[at] = 0u;
        let s = sections[id];
        if s.key.w >= 0 && s.a.y != 0u && (s.a.w & FADING) == 0u
            && near(s.key.x, s.key.z) && reached(s.key.xyz)
        {
            let page = u32(s.key.w);
            // The tables hold every column within the distance; a key before their
            // first wraps past their end.
            let c = vec3<u32>(s.key.xyz - params.first.xyz);
            if page < params.pages && all(c < params.cells.xyz) && in_frustum(c, s.a.z) {
                // Kept, whether or not a quad of it can face the eye.
                atomicAdd(&wg_kept, 1u);
                let nx = params.cells.x;
                let ny = params.cells.y;
                let origin = vec4<u32>(tables[c.x], tables[nx + c.y], tables[nx + ny + c.z], STEP_BITS);
                origins[id] = origin;
                // gfx.rs `facing_runs`: the runs of its buckets that can face the eye, an
                // empty bucket breaking none, a draw each; without an eye, its solid quads.
                var n = 0u;
                if (params.flags & EYE) == 0u {
                    runs[at + 1u] = 0u;
                    runs[at + 2u] = s.a.y;
                    n = 1u;
                } else {
                    let e = params.eyes;
                    let eye = vec3<u32>(tables[e + c.x], tables[e + nx + c.y], tables[e + nx + ny + c.z]);
                    var start = 0u;
                    var first = 0u;
                    var open = false;
                    for (var b = 0u; b < 7u; b++) {
                        let end = bucket_end(id, b);
                        if end == start {
                            continue;
                        }
                        let shown = seen(id, b, eye);
                        if shown && !open {
                            open = true;
                            first = start;
                        } else if !shown && open {
                            runs[at + 1u + 2u * n] = first;
                            runs[at + 2u + 2u * n] = start - first;
                            n += 1u;
                            open = false;
                        }
                        start = end;
                    }
                    if open {
                        runs[at + 1u + 2u * n] = first;
                        runs[at + 2u + 2u * n] = start - first;
                        n += 1u;
                    }
                }
                if n > 0u {
                    var flags = 0u;
                    if (params.flags & OCCLUDE) != 0u {
                        flags = occlusion(id, at, bitcast<vec3<f32>>(origin.xyz), s.a.z);
                    }
                    let band = distance_band(s.key.x, s.key.z);
                    runs[at] = (page << PAGE_SHIFT) | (band << BAND_SHIFT) | flags | n;
                    if (flags & PHASE2) == 0u {
                        if page < FAST_PAGES {
                            atomicAdd(&wg_buckets[page * BANDS + band], n);
                        } else {
                            atomicAdd(&counts[block_count(block, page, band, 0u)], n);
                        }
                    }
                }
            }
        }
    }

    workgroupBarrier();
    if lane == 0u {
        let kept = atomicLoad(&wg_kept);
        if kept > 0u {
            atomicAdd(&counts[0], kept);
        }
    }
    if lane < FAST_BUCKETS {
        let count = atomicLoad(&wg_buckets[lane]);
        if count > 0u {
            let page = lane / BANDS;
            let band = lane % BANDS;
            let blocks = (params.ids + BLOCK - 1u) / BLOCK;
            atomicStore(&counts[block_base(page, band, 0u, blocks) + block], count);
        }
    }
}

var<workgroup> partial: array<vec4<u32>, 256>;

// One workgroup per page: scans all 4 distance bands together in a single
// Hillis-Steele workgroup scan with stride-1 coalesced global accesses
// per band. Band 0's draws land first in the page's contiguous draw region,
// then band 1's, band 2's, and band 3's; within each band draws stay in
// deterministic id order.
fn scan_page(page: u32, lane: u32, phase: u32) {
    let blocks = (params.ids + BLOCK - 1u) / BLOCK;
    let base0 = block_base(page, 0u, phase, blocks);
    let base1 = base0 + blocks;
    let base2 = base1 + blocks;
    let base3 = base2 + blocks;

    if blocks <= 256u {
        // Fast path (up to 16,384 section IDs): 1 coalesced load per lane per band,
        // 1 vectorized 4-band scan, 1 coalesced store per lane per band.
        var vals = vec4<u32>(0u);
        if lane < blocks {
            vals = vec4<u32>(
                atomicLoad(&counts[base0 + lane]),
                atomicLoad(&counts[base1 + lane]),
                atomicLoad(&counts[base2 + lane]),
                atomicLoad(&counts[base3 + lane]),
            );
        }
        partial[lane] = vals;
        workgroupBarrier();
        for (var step = 1u; step < 256u; step <<= 1u) {
            var before = vec4<u32>(0u);
            if lane >= step {
                before = partial[lane - step];
            }
            workgroupBarrier();
            partial[lane] += before;
            workgroupBarrier();
        }
        let totals = partial[255];
        let band_bases = vec4<u32>(
            0u,
            totals.x,
            totals.x + totals.y,
            totals.x + totals.y + totals.z,
        );
        if lane < blocks {
            let excl = partial[lane] - vals;
            let starts = band_bases + excl;
            if vals.x > 0u {
                atomicStore(&counts[base0 + lane], starts.x);
            }
            if vals.y > 0u {
                atomicStore(&counts[base1 + lane], starts.y);
            }
            if vals.z > 0u {
                atomicStore(&counts[base2 + lane], starts.z);
            }
            if vals.w > 0u {
                atomicStore(&counts[base3 + lane], starts.w);
            }
        }
        if lane == 255u {
            let sum_all = totals.x + totals.y + totals.z + totals.w;
            // A region has room for every draw of its page's records; were one
            // short, its draws past it go.
            let total = min(sum_all, region(page).y);
            atomicStore(&counts[page_count(page, phase)], total);
            atomicAdd(&counts[DRAWS], total);
            if phase == 1u {
                atomicAdd(&counts[DRAWN2], total);
            }
        }
        return;
    }

    // Large worlds (> 16,384 section IDs, e.g. RD 32+): tile in chunks of 256
    // blocks so every global load and store stays 100% stride-1 coalesced across
    // the 256 lanes, scanning all 4 bands simultaneously in `vec4<u32>`.
    var totals = vec4<u32>(0u);
    for (var tile = 0u; tile < blocks; tile += 256u) {
        let b = tile + lane;
        if b < blocks {
            totals += vec4<u32>(
                atomicLoad(&counts[base0 + b]),
                atomicLoad(&counts[base1 + b]),
                atomicLoad(&counts[base2 + b]),
                atomicLoad(&counts[base3 + b]),
            );
        }
    }
    // Reduce per-lane partial totals to get total draw count per band across the page.
    partial[lane] = totals;
    workgroupBarrier();
    for (var step = 128u; step > 0u; step >>= 1u) {
        if lane < step {
            partial[lane] += partial[lane + step];
        }
        workgroupBarrier();
    }
    let page_totals = partial[0];
    var running = vec4<u32>(
        0u,
        page_totals.x,
        page_totals.x + page_totals.y,
        page_totals.x + page_totals.y + page_totals.z,
    );
    workgroupBarrier();

    // Second pass over 256-block tiles: coalesced load, vectorized scan, coalesced store.
    for (var tile = 0u; tile < blocks; tile += 256u) {
        let b = tile + lane;
        var vals = vec4<u32>(0u);
        if b < blocks {
            vals = vec4<u32>(
                atomicLoad(&counts[base0 + b]),
                atomicLoad(&counts[base1 + b]),
                atomicLoad(&counts[base2 + b]),
                atomicLoad(&counts[base3 + b]),
            );
        }
        partial[lane] = vals;
        workgroupBarrier();
        for (var step = 1u; step < 256u; step <<= 1u) {
            var before = vec4<u32>(0u);
            if lane >= step {
                before = partial[lane - step];
            }
            workgroupBarrier();
            partial[lane] += before;
            workgroupBarrier();
        }
        if b < blocks {
            let starts = running + (partial[lane] - vals);
            if vals.x > 0u {
                atomicStore(&counts[base0 + b], starts.x);
            }
            if vals.y > 0u {
                atomicStore(&counts[base1 + b], starts.y);
            }
            if vals.z > 0u {
                atomicStore(&counts[base2 + b], starts.z);
            }
            if vals.w > 0u {
                atomicStore(&counts[base3 + b], starts.w);
            }
        }
        running += partial[255];
        workgroupBarrier();
    }

    if lane == 255u {
        let sum_all = page_totals.x + page_totals.y + page_totals.z + page_totals.w;
        // A region has room for every draw of its page's records; were one
        // short, its draws past it go.
        let total = min(sum_all, region(page).y);
        atomicStore(&counts[page_count(page, phase)], total);
        atomicAdd(&counts[DRAWS], total);
        if phase == 1u {
            atomicAdd(&counts[DRAWN2], total);
        }
    }
}

@compute @workgroup_size(256)
fn scan(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    scan_page(group.x, lane, 0u);
}

var<workgroup> held: array<u32, 64>;
var<workgroup> scatter_bases: array<u32, 32>;
var<workgroup> scatter_active: atomic<u32>;

// One workgroup per block of ids: each section's draws go after those of the
// block's earlier sections in the same `(page, band)`, in its runs' order, in
// `phase` (`scatter`, gpu_occlusion.wgsl `scatter2`).
fn scatter_block(block: u32, lane: u32, phase: u32) {
    let id = block * BLOCK + lane;
    let mine = drawn_in(id, phase);
    held[lane] = mine;
    if lane == 0u {
        atomicStore(&scatter_active, 0u);
    }
    workgroupBarrier();
    let n = mine & 7u;
    if n > 0u {
        let page = mine >> PAGE_SHIFT;
        if page < FAST_PAGES {
            let band = (mine >> BAND_SHIFT) & BAND_MASK;
            atomicOr(&scatter_active, 1u << (page * BANDS + band));
        }
    }
    workgroupBarrier();
    if lane < FAST_BUCKETS {
        let mask = atomicLoad(&scatter_active);
        if ((mask >> lane) & 1u) != 0u {
            let page = lane / BANDS;
            let band = lane % BANDS;
            let blocks = (params.ids + BLOCK - 1u) / BLOCK;
            scatter_bases[lane] = atomicLoad(&counts[block_base(page, band, phase, blocks) + block]);
        }
    }
    workgroupBarrier();
    // Its draws below BAND_SHIFT, its band in [BAND_SHIFT..PAGE_SHIFT), its page above.
    if n == 0u {
        return;
    }
    let page_band = mine >> BAND_SHIFT;
    let page = mine >> PAGE_SHIFT;
    let band = page_band & BAND_MASK;
    var slot: u32;
    if page < FAST_PAGES {
        slot = scatter_bases[page * BANDS + band];
    } else {
        slot = atomicLoad(&counts[block_count(block, page, band, phase)]);
    }
    for (var other = 0u; other < lane; other++) {
        // A section not drawn holds 0: no draws.
        let h = held[other];
        if (h >> BAND_SHIFT) == page_band {
            slot += h & 7u;
        }
    }
    let r = region(page);
    let at = id * RUN_WORDS;
    let base = sections[id].a.x;
    for (var k = 0u; k < n; k++) {
        if slot + k >= r.y {
            return;
        }
        // A run of its solid quads, from its first vertex, with its origin.
        let d = (r.x + slot + k) * 5u;
        draws[d] = runs[at + 2u + 2u * k] * 6u;
        draws[d + 1u] = 1u;
        draws[d + 2u] = 0u;
        draws[d + 3u] = base + runs[at + 1u + 2u * k] * 4u;
        draws[d + 4u] = id;
    }
}

@compute @workgroup_size(64)
fn scatter(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    scatter_block(group.y * ROW + group.x, lane, 0u);
}
