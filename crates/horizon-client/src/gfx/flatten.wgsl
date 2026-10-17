// Flat section draws (gfx/flat.rs): copies each section draw's indices into
// one index list, in draw order, offset by the draw's base vertex. Solid
// draws index quads (0 1 2 0 2 3 per quad, as Renderer::quad_indices);
// translucent ones read their sorted indices.
struct Range {
    count: u32,
    // First sorted index, or QUADS for a solid draw.
    first: u32,
    base: u32,
    dst: u32,
}
const QUADS: u32 = 0xffffffffu;
const GROUP: u32 = 64u;
@group(0) @binding(0) var<storage, read> ranges: array<Range>;
@group(0) @binding(1) var<storage, read> sorted: array<u32>;
@group(0) @binding(2) var<storage, read_write> out: array<u32>;
@compute @workgroup_size(64)
fn flatten(@builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) lane: u32) {
    let r = group.y * 65535u + group.x;
    if r >= arrayLength(&ranges) {
        return;
    }
    let d = ranges[r];
    for (var k = lane; k < d.count; k += GROUP) {
        var i: u32;
        if d.first == QUADS {
            let c = k % 6u;
            i = (k / 6u) * 4u + select(select(c - 2u, 0u, c == 3u), c, c < 3u);
        } else {
            i = sorted[d.first + k];
        }
        out[d.dst + k] = d.base + i;
    }
}
