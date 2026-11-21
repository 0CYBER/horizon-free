//! The compact layout of a section's solid quads in the arena: most of them are whole faces
//! of a block's cube on the block grid, and those take one 24-byte record each instead of
//! four 24-byte vertices. terrain.wgsl's `section_corner` expands a record into the very
//! vertex words `TerrainVertex` would hold (`packed_vertex` here is the same on the CPU, and
//! a record is kept only when it gives back the quad's four vertices bit for bit), so every
//! pass draws the same picture from either layout.
//!
//! A mesh's solid range is then one record per quad in mesh order (so `Sides` and the draws
//! count quads as before), padding to a multiple of 4 slots, and the quads no record fits as
//! their four vertices: their record holds the distance to those (`FULL`). A record:
//!
//! - word 0: the tint (0xRRGGBB as the mesher multiplies it) and the colour's alpha byte;
//! - word 1: each corner's grey level before the tint;
//! - word 2: the UV rectangle in 1/32 of the sprite (u0, u1, v0, v1, 6 bits each) and, per
//!   corner, which edge of it its u and v take;
//! - word 3: the layer, corner 0's light (sky and block light over 4, 6 bits each), and
//!   corner 1's alpha code (`ALPHA_STEP` times it; 0: word 0's alpha);
//! - word 4: corners 1 and 2's light, and corners 2 and 3's alpha codes;
//! - word 5: corner 3's light, the face (block x, y, z, the axis it faces along, which side
//!   of the block, and where and which way its corners go round), and `FULL` clear.
use super::TerrainVertex;
use std::sync::atomic::{AtomicBool, Ordering};

/// A solid quad's tint the mesher does not know (fluids): only untinted colours pack.
pub const UNKNOWN_TINT: u32 = u32::MAX;
/// The tint of an untinted quad, whose colour is its grey level.
pub const WHITE: u32 = 0xFF_FFFF;
/// Word 5's bit of a quad kept as four vertices; word 0 is then the slots from the record
/// to its first vertex.
pub const FULL: u32 = 1 << 31;
/// The step of a corner's own alpha (words 3 and 4's codes): 255 / 5.
pub const ALPHA_STEP: u32 = 51;
/// The draws' base vertex of a compact mesh: `PACKED_BASE + 4 * slot + corner` is a corner of
/// the quad whose record is at `slot`; below it, vertex indices name vertices.
pub const PACKED_BASE: u32 = 1 << 28;

/// Whether meshes sorted from now on get the compact layout: set by the renderer while its
/// passes read it (no shader pack).
static PACK: AtomicBool = AtomicBool::new(false);

pub fn pack_quads(on: bool) {
    PACK.store(on, Ordering::Relaxed);
}

pub fn packing() -> bool {
    PACK.load(Ordering::Relaxed)
}

/// The order corners go round a face: (du, dw) of each step.
const ROUND: [[u32; 2]; 4] = [[0, 0], [1, 0], [1, 1], [0, 1]];

/// `quad`'s record, if one gives back its four vertices exactly. `tint`: what the mesher
/// multiplied its grey levels by (`UNKNOWN_TINT` when it does not say).
pub fn pack_quad(quad: &[TerrainVertex; 4], tint: u32) -> Option<[u32; 6]> {
    let layer = quad[0].layer;
    if quad[1].layer != layer || quad[2].layer != layer || quad[3].layer != layer {
        return None;
    }
    // Corners on the block grid of the section (rejects -0.0 and non-integers without libm).
    let mut p = [[0u32; 3]; 4];
    for (to, v) in p.iter_mut().zip(quad) {
        for (c, &x) in to.iter_mut().zip(&v.pos) {
            if !(0.0..=16.0).contains(&x) {
                return None;
            }
            let n = x as u32;
            if x.to_bits() != (n as f32).to_bits() {
                return None;
            }
            *c = n;
        }
    }
    let axis = if p[1][0] == p[0][0] && p[2][0] == p[0][0] && p[3][0] == p[0][0] {
        0
    } else if p[1][1] == p[0][1] && p[2][1] == p[0][1] && p[3][1] == p[0][1] {
        1
    } else if p[1][2] == p[0][2] && p[2][2] == p[0][2] && p[3][2] == p[0][2] {
        2
    } else {
        return None;
    };
    let (u, w) = match axis {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let lo_u = p[0][u].min(p[1][u]).min(p[2][u]).min(p[3][u]);
    let lo_w = p[0][w].min(p[1][w]).min(p[2][w]).min(p[3][w]);
    if lo_u > 15 || lo_w > 15 {
        return None;
    }
    let mut steps = [0u32; 4];
    for (step, c) in steps.iter_mut().zip(&p) {
        let du = c[u] - lo_u;
        let dw = c[w] - lo_w;
        if (du | dw) > 1 {
            return None;
        }
        *step = (dw << 1) | (du ^ dw);
    }
    let start = steps[0];
    let dir = if steps[1] == ((start + 1) & 3)
        && steps[2] == ((start + 2) & 3)
        && steps[3] == ((start + 3) & 3)
    {
        0u32
    } else if steps[1] == ((start + 3) & 3)
        && steps[2] == ((start + 2) & 3)
        && steps[3] == ((start + 1) & 3)
    {
        1u32
    } else {
        return None;
    };
    let plane = p[0][axis];
    let mut block = [0u32; 3];
    block[axis] = plane.min(15);
    block[u] = lo_u;
    block[w] = lo_w;
    let side = plane - block[axis];
    let orient = block[0]
        | block[1] << 4
        | block[2] << 8
        | (axis as u32) << 12
        | side << 14
        | start << 15
        | dir << 17;
    // UVs in 1/32 of the sprite, at most two values each way.
    let mut u_min = u32::MAX;
    let mut u_max = 0u32;
    let mut v_min = u32::MAX;
    let mut v_max = 0u32;
    let mut uvs = [[0u32; 2]; 4];
    for (uv, v) in uvs.iter_mut().zip(quad) {
        let ru = u32::from(v.uv[0]);
        let rv = u32::from(v.uv[1]);
        if (ru | rv) & 1023 != 0 {
            return None;
        }
        let u0 = ru >> 10;
        let v0 = rv >> 10;
        uv[0] = u0;
        uv[1] = v0;
        u_min = u_min.min(u0);
        u_max = u_max.max(u0);
        v_min = v_min.min(v0);
        v_max = v_max.max(v0);
    }
    let mut select = 0u32;
    for (i, &[u0, v0]) in uvs.iter().enumerate() {
        if u0 == u_max && u0 != u_min {
            select |= 1 << (2 * i);
        } else if u0 != u_min {
            return None;
        }
        if v0 == v_max && v0 != v_min {
            select |= 1 << (2 * i + 1);
        } else if v0 != v_min {
            return None;
        }
    }
    let rect = u_min | u_max << 6 | v_min << 12 | v_max << 18;
    // Light in steps of 4 (smooth light averages four levels of 16).
    let mut light = [0u32; 4];
    for (l, v) in light.iter_mut().zip(quad) {
        let raw = u32::from(v.light);
        if raw & 0x0303 != 0 {
            return None;
        }
        *l = ((raw & 0xFF) >> 2) | ((raw >> 10) << 6);
    }
    // The colour: each channel the grey level times the tint's, over 255.
    let tint = if tint == UNKNOWN_TINT {
        WHITE
    } else {
        tint & WHITE
    };
    let alpha = quad[0].color & 0xFF00_0000;
    let mut grays = 0u32;
    if tint == WHITE {
        for (i, v) in quad.iter().enumerate() {
            let r = v.color & 255;
            let g = (v.color >> 8) & 255;
            let b = (v.color >> 16) & 255;
            if r != g || g != b {
                return None;
            }
            grays |= r << (8 * i);
        }
    } else {
        let t = channels(tint);
        let k = if t[1] > t[0] {
            if t[2] > t[1] { 2 } else { 1 }
        } else if t[2] > t[0] {
            2
        } else {
            0
        };
        let mut prev_rgb = u32::MAX;
        let mut prev_gray = 0u32;
        for (i, v) in quad.iter().enumerate() {
            let rgb = v.color & 0x00FF_FFFF;
            let g = if rgb == prev_rgb {
                prev_gray
            } else {
                let g = gray_of_precomputed(rgb, t, k)?;
                prev_rgb = rgb;
                prev_gray = g;
                g
            };
            grays |= g << (8 * i);
        }
    }
    // Corners 1 to 3's alpha (a shader pack's `separateAo`: each corner's ambient occlusion,
    // multiples of 51 on a whole face): `ALPHA_STEP` times the code, 0 for corner 0's.
    let mut codes = [0u32; 3];
    for (code, v) in codes.iter_mut().zip(&quad[1..]) {
        let a = v.color >> 24;
        if v.color & 0xFF00_0000 != alpha {
            if a == 0 || a % ALPHA_STEP != 0 {
                return None;
            }
            *code = a / ALPHA_STEP;
        }
    }
    let words = [
        tint | alpha,
        grays,
        rect | select << 24,
        u32::from(layer) | light[0] << 16 | codes[0] << 28,
        light[1] | light[2] << 12 | codes[1] << 24 | codes[2] << 27,
        light[3] | orient << 12,
    ];
    debug_assert!((0..4).all(|c| {
        let (a, b) = (packed_vertex(&words, c), quad[c]);
        bytemuck::bytes_of(&a) == bytemuck::bytes_of(&b)
    }));
    Some(words)
}

/// The tint's channels, as the mesher takes them for red, green and blue.
#[inline]
fn channels(tint: u32) -> [u32; 3] {
    [tint >> 16 & 255, tint >> 8 & 255, tint & 255]
}

/// A grey level that `t` (precomputed `channels(tint)`, dominant channel `k`) turns into `rgb`.
#[inline]
fn gray_of_precomputed(rgb: u32, t: [u32; 3], k: usize) -> Option<u32> {
    let want = [rgb & 255, (rgb >> 8) & 255, (rgb >> 16) & 255];
    let tk = t[k];
    if tk == 0 {
        return (rgb == 0).then_some(0);
    }
    let (k1, k2) = match k {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let prod = want[k] * 255;
    let lo = prod.div_ceil(tk);
    let hi = ((prod + 254) / tk).min(255);
    (lo..=hi).find(|&g| g * t[k1] / 255 == want[k1] && g * t[k2] / 255 == want[k2])
}

/// Corner `corner` of the quad `words` holds, as terrain.wgsl's `packed_corner` expands it.
pub fn packed_vertex(words: &[u32; 6], corner: usize) -> TerrainVertex {
    let [w0, w1, w2, w3, w4, w5] = *words;
    let orient = w5 >> 12 & 0x3_FFFF;
    let block = [orient & 15, orient >> 4 & 15, orient >> 8 & 15];
    let axis = (orient >> 12 & 3) as usize;
    let side = orient >> 14 & 1;
    let (start, dir) = (orient >> 15 & 3, orient >> 17 & 1);
    let i = corner as u32;
    let step = (start + if dir == 1 { 4 - i } else { i }) & 3;
    let [du, dw] = ROUND[step as usize];
    let mut pos = block;
    pos[axis] += side;
    pos[(axis + 1) % 3] += du;
    pos[(axis + 2) % 3] += dw;
    let select = w2 >> 24;
    let us = select >> (2 * i) & 1;
    let vs = select >> (2 * i + 1) & 1;
    let uv = [w2 >> (6 * us) & 63, w2 >> (12 + 6 * vs) & 63].map(|k| (k << 10) as u16);
    let light = [w3 >> 16, w4, w4 >> 12, w5][corner] & 0xFFF;
    let light = ((light & 63) << 2 | (light >> 6 & 63) << 10) as u16;
    let g = w1 >> (8 * i) & 255;
    let [r, gr, b] = channels(w0).map(|t| g * t / 255);
    let code = [0, w3 >> 28 & 7, w4 >> 24 & 7, w4 >> 27 & 7][corner];
    let alpha = if code == 0 {
        w0 & 0xFF00_0000
    } else {
        (code * ALPHA_STEP) << 24
    };
    TerrainVertex {
        pos: pos.map(|c| c as f32),
        uv,
        layer: (w3 & 0xFFFF) as u16,
        light,
        color: r | gr << 8 | b << 16 | alpha,
    }
}

/// The compact layout of sorted solid quads with their tints (`SectionMesh::packed`), or
/// nothing when it would be no smaller than their vertices.
pub fn layout(solid: &[TerrainVertex], tints: &[u32]) -> Vec<[u32; 6]> {
    let mut out = Vec::new();
    layout_into(solid, tints, &mut out);
    out
}

/// Writes the compact layout of sorted solid quads with their tints directly into `out`,
/// reusing `out`'s allocation with no intermediate scratch buffer and aborting early once
/// too many quads spill to beat the uncompressed vertex layout.
pub fn layout_into(solid: &[TerrainVertex], tints: &[u32], out: &mut Vec<[u32; 6]>) {
    out.clear();
    let quads = solid.as_chunks::<4>().0;
    let n = quads.len().min(tints.len());
    let pad = n.next_multiple_of(4) - n;
    if n + pad >= 4 * n {
        return;
    }
    let max_full = (3 * n - pad - 1) / 4;
    let had_cap = out.capacity() > 0;
    out.reserve(n + pad);
    let mut full = 0usize;
    for (q, &t) in quads[..n].iter().zip(&tints[..n]) {
        match pack_quad(q, t) {
            Some(words) => out.push(words),
            None => {
                full += 1;
                if full > max_full {
                    if had_cap {
                        out.clear();
                    } else {
                        *out = Vec::new();
                    }
                    return;
                }
                out.push([0, 0, 0, 0, 0, FULL]);
            }
        }
    }
    out.resize(n + pad, [0; 6]);
    if full == 0 {
        return;
    }
    out.reserve_exact(4 * full);
    let mut next = n + pad;
    for (i, q) in quads[..n].iter().enumerate() {
        if out[i][5] == FULL {
            out[i][0] = (next - i) as u32;
            next += 4;
            for v in q {
                out.push(bytemuck::cast::<TerrainVertex, [u32; 6]>(*v));
            }
        }
    }
}

/// A mesh's solid range from a record per quad, `None` where no record fits:
/// the records in mesh order, padded to a multiple of 4 slots, then the four
/// vertices of each quad without one, whose record holds the distance to them
/// (`FULL`). Empty where that would not be smaller than the vertices, which is
/// what makes the layout never a loss.
///
/// Shared with `lod::packed`, whose records hold a far tile's quads instead but
/// spill and pad the same way.
pub fn place(records: &[Option<[u32; 6]>], quads: &[[TerrainVertex; 4]]) -> Vec<[u32; 6]> {
    let mut out = Vec::new();
    place_into(records, quads, &mut out);
    out
}

pub fn place_into(
    records: &[Option<[u32; 6]>],
    quads: &[[TerrainVertex; 4]],
    out: &mut Vec<[u32; 6]>,
) {
    out.clear();
    let n = quads.len();
    let packed = records.iter().filter(|r| r.is_some()).count();
    let full = n - packed;
    // Whole quads of slots: what follows (a quad's own vertices, the translucent ones after
    // the solid range) then starts a quad, as a shader pack's vertex stage reads its quad's
    // corners from the vertex index (`pack_atlas::PULL_FLAT`).
    let pad = n.next_multiple_of(4) - n;
    if n + pad + 4 * full >= 4 * n {
        return;
    }
    out.reserve(n + pad + 4 * full);
    let mut next = n + pad;
    for (i, record) in records.iter().enumerate() {
        out.push(record.unwrap_or_else(|| {
            let at = next;
            next += 4;
            [(at - i) as u32, 0, 0, 0, 0, FULL]
        }));
    }
    out.resize(n + pad, [0; 6]);
    if full == 0 {
        return;
    }
    for (q, record) in quads.iter().zip(records) {
        if record.is_none() {
            out.extend(
                q.iter()
                    .map(|v| bytemuck::cast::<TerrainVertex, [u32; 6]>(*v)),
            );
        }
    }
}

/// Quads `layout` gave a record of their own.
pub fn packed_quads(layout: &[[u32; 6]], quads: usize) -> usize {
    layout
        .iter()
        .take(quads)
        .filter(|w| w[5] & FULL == 0)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TINT: u32 = 0x7F_B238;

    /// An opaque colour the mesher makes of grey level `g` times `tint`.
    fn tinted(g: u32, tint: u32) -> u32 {
        let [r, gr, b] = channels(tint).map(|t| g * t / 255);
        r | gr << 8 | b << 16 | 0xFF00_0000
    }

    /// The top face of block (3, 15, 5), corners in `order`: tinted, smooth lit, with
    /// ambient occlusion darkening its corners differently.
    fn top(order: [usize; 4]) -> [TerrainVertex; 4] {
        let pos = [
            [3.0, 16.0, 5.0],
            [3.0, 16.0, 6.0],
            [4.0, 16.0, 6.0],
            [4.0, 16.0, 5.0],
        ];
        let uv = [[0, 0], [16 << 10, 0], [16 << 10, 16 << 10], [0, 16 << 10]];
        let grays = [200, 180, 160, 200];
        order.map(|i| TerrainVertex {
            pos: pos[i],
            uv: uv[i],
            layer: 7,
            light: 0xF0 | 0x40 << 8,
            color: tinted(grays[i], TINT),
        })
    }

    fn same(a: &TerrainVertex, b: &TerrainVertex) -> bool {
        bytemuck::bytes_of(a) == bytemuck::bytes_of(b)
    }

    #[test]
    fn a_whole_cube_face_takes_one_record_either_way_round() {
        for order in [[0, 1, 2, 3], [2, 3, 0, 1], [0, 3, 2, 1]] {
            let quad = top(order);
            let words = pack_quad(&quad, TINT).expect("a whole face on the block grid packs");
            for (corner, v) in quad.iter().enumerate() {
                assert!(
                    same(&packed_vertex(&words, corner), v),
                    "{order:?} corner {corner}"
                );
            }
        }
    }

    #[test]
    fn each_corners_ambient_occlusion_alpha_packs_on_its_steps() {
        // A shader pack's `separateAo`: the alpha byte is each corner's occlusion.
        let mut quad = top([0, 1, 2, 3]);
        for (v, a) in quad.iter_mut().zip([153u32, 255, 51, 204]) {
            v.color = v.color & 0x00FF_FFFF | a << 24;
        }
        let words = pack_quad(&quad, TINT).expect("occlusion on the 51 steps packs");
        for (corner, v) in quad.iter().enumerate() {
            assert!(same(&packed_vertex(&words, corner), v), "corner {corner}");
        }
        // A partial face's blended occlusion falls between the steps.
        quad[2].color = quad[2].color & 0x00FF_FFFF | 140 << 24;
        assert!(pack_quad(&quad, TINT).is_none());
    }

    #[test]
    fn quads_off_the_grid_or_the_steps_keep_their_vertices() {
        // Half a block up: a slab's top.
        let mut slab = top([0, 1, 2, 3]);
        for v in &mut slab {
            v.pos[1] = 8.5;
        }
        assert!(pack_quad(&slab, TINT).is_none());
        // A UV between the 1/32 steps.
        let mut uv = top([0, 1, 2, 3]);
        uv[2].uv[0] += 1;
        assert!(pack_quad(&uv, TINT).is_none());
        // Light between the steps of 4.
        let mut light = top([0, 1, 2, 3]);
        light[1].light += 1;
        assert!(pack_quad(&light, TINT).is_none());
    }

    #[test]
    fn a_quad_without_a_record_points_at_its_own_vertices() {
        let mut odd = top([0, 1, 2, 3]);
        for v in &mut odd {
            v.pos[1] = 8.5;
        }
        let mut solid: Vec<TerrainVertex> = Vec::new();
        for _ in 0..4 {
            solid.extend(top([0, 1, 2, 3]));
        }
        solid.extend(odd);
        let out = layout(&solid, &[TINT; 5]);
        // Five records, padded to eight slots, then the odd quad's four vertices.
        assert_eq!(out.len(), 12);
        assert_eq!(packed_quads(&out, 5), 4);
        assert_eq!(out[4][5], FULL);
        let at = 4 + out[4][0] as usize;
        for (slot, v) in out[at..at + 4].iter().zip(&odd) {
            assert!(same(&bytemuck::cast::<[u32; 6], TerrainVertex>(*slot), v));
        }
        // Nothing smaller to gain: no layout at all.
        assert!(layout(&odd, &[TINT]).is_empty());
    }

    #[test]
    fn the_records_end_on_a_whole_quad_of_slots() {
        // Five whole faces, every one a record: the translucent vertices after them start a quad.
        let solid: Vec<TerrainVertex> = (0..5).flat_map(|_| top([0, 1, 2, 3])).collect();
        let out = layout(&solid, &[TINT; 5]);
        assert_eq!(packed_quads(&out, 5), 5);
        assert_eq!(out.len(), 8);
    }

    #[test]
    fn rejected_layout_drops_fresh_capacity_but_keeps_reused_buffer() {
        let mut odd = top([0, 1, 2, 3]);
        for v in &mut odd {
            v.pos[1] = 8.5;
        }
        let solid: Vec<TerrainVertex> = (0..16).flat_map(|_| odd).collect();
        let tints = [TINT; 16];
        let mut fresh = Vec::new();
        layout_into(&solid, &tints, &mut fresh);
        assert!(fresh.is_empty());
        assert_eq!(fresh.capacity(), 0);
        let mut reused = Vec::with_capacity(64);
        layout_into(&solid, &tints, &mut reused);
        assert!(reused.is_empty());
        assert!(reused.capacity() >= 64);
    }

    #[test]
    fn spilled_quads_reserve_exact_slots() {
        let quad = top([0, 1, 2, 3]);
        let mut odd = quad;
        for v in &mut odd {
            v.pos[1] = 8.5;
        }
        let mut solid: Vec<TerrainVertex> = Vec::with_capacity(4096 * 4);
        for _ in 0..4095 {
            solid.extend_from_slice(&quad);
        }
        solid.extend_from_slice(&odd);
        let tints = vec![TINT; 4096];
        let mut out = Vec::new();
        layout_into(&solid, &tints, &mut out);
        assert_eq!(out.len(), 4100);
        assert_eq!(out.capacity(), 4100);
        assert_eq!(packed_quads(&out, 4096), 4095);
        assert_eq!(out[4095][5], FULL);
    }
}
