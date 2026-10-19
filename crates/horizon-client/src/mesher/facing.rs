//! Solid quads grouped by the side they face, as Sodium's `ModelQuadFacing`
//! does. A quad in the plane `x = p` facing +X is front-facing only while the
//! eye is at `x > p`, and terrain culls back faces, so when the eye is on the
//! back side of every quad of a group the renderer skips the group's draw:
//! the GPU would have discarded all of it after shading its vertices. The
//! picture is unchanged; the vertex work for those quads is gone.
use super::TerrainVertex;
use glam::Vec3;

/// Groups in buffer order: the negative sides, quads not in an axis plane
/// (rotated model elements, sloped fluid tops), then the positive sides.
/// With the eye outside a section on every axis, one side per axis is
/// visible, so the drawn groups form one to three runs.
pub const GROUPS: usize = 7;
const UNALIGNED: usize = 3;
/// The group of a quad facing `-axis` (0 x, 1 y, 2 z) is `NEGATIVE[axis]`, of
/// one facing `+axis` `GROUPS - 1 - NEGATIVE[axis]`.
const NEGATIVE: [usize; 3] = [0, 2, 1];
/// How far (blocks) the eye must be behind every plane of a group before
/// the group is skipped: rounding on the GPU never makes such a quad
/// front-facing.
const MARGIN: f32 = 1.0 / 16.0;

/// The solid quads' groups, filled by `group`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceGroups {
    /// Quads per group, in buffer order.
    pub quads: [u32; GROUPS],
    /// Per axis group, the plane nearest the side the group faces: the
    /// lowest plane of a positive group and the highest of a negative one.
    planes: [f32; GROUPS],
}

impl Default for FaceGroups {
    /// Everything unaligned: always drawn.
    fn default() -> Self {
        Self {
            quads: [0; GROUPS],
            planes: [0.0; GROUPS],
        }
    }
}

impl FaceGroups {
    /// Group arbitrary-sized terrain quads, including distant LOD tiles.
    pub fn group(solid: &mut Vec<TerrainVertex>) -> Self {
        group(solid)
    }

    /// Whether group `g` can hold a front-facing quad for an eye at `eye`,
    /// relative to the section's minimum corner.
    pub fn visible(&self, g: usize, eye: Vec3) -> bool {
        if g == UNALIGNED {
            return true;
        }
        let (axis, positive) = axis(g);
        if positive {
            eye[axis] > self.planes[g] - MARGIN
        } else {
            eye[axis] < self.planes[g] + MARGIN
        }
    }
    /// The quad ranges (first quad, quad count) to draw for an eye at `eye`
    /// relative to the section's minimum corner, adjacent groups merged.
    pub fn ranges(&self, eye: Vec3, mut out: impl FnMut(u32, u32)) {
        let mut first = 0;
        let mut run: Option<(u32, u32)> = None;
        for g in 0..GROUPS {
            let quads = self.quads[g];
            if quads > 0 {
                if self.visible(g, eye) {
                    run = Some(match run {
                        Some((start, len)) => (start, len + quads),
                        None => (first, quads),
                    });
                } else if let Some((start, len)) = run.take() {
                    out(start, len);
                }
            }
            first += quads;
        }
        if let Some((start, len)) = run {
            out(start, len);
        }
    }
}

/// The axis and sign of each axis group (`UNALIGNED`'s entry is unused).
const AXES: [(usize, bool); GROUPS] = {
    let mut axes = [(0, false); GROUPS];
    let mut axis = 0;
    while axis < 3 {
        axes[NEGATIVE[axis]] = (axis, false);
        axes[GROUPS - 1 - NEGATIVE[axis]] = (axis, true);
        axis += 1;
    }
    axes
};

/// The axis and sign of axis group `g`.
fn axis(g: usize) -> (usize, bool) {
    debug_assert_ne!(g, UNALIGNED);
    AXES[g]
}

/// The group of one quad, and its plane: an axis group only when all four
/// corners lie exactly in one axis plane.
fn facing(quad: &[TerrainVertex]) -> (usize, f32) {
    let p = |i: usize| Vec3::from(quad[i].pos);
    let normal = (p(1) - p(0)).cross(p(2) - p(0));
    for axis in 0..3 {
        let [a, b] = [(axis + 1) % 3, (axis + 2) % 3];
        let plane = quad[0].pos[axis];
        if normal[axis] != 0.0
            && normal[a] == 0.0
            && normal[b] == 0.0
            && quad.iter().all(|v| v.pos[axis] == plane)
        {
            let negative = NEGATIVE[axis];
            let g = if normal[axis] > 0.0 {
                GROUPS - 1 - negative
            } else {
                negative
            };
            return (g, plane);
        }
    }
    (UNALIGNED, 0.0)
}

/// Reorder `solid` by group, keeping the order of quads within a group (so
/// coplanar quads such as a grass block's overlay still draw after their
/// base), and return the groups.
pub fn group(solid: &mut Vec<TerrainVertex>) -> FaceGroups {
    let mut groups = FaceGroups::default();
    for (g, plane) in groups.planes.iter_mut().enumerate() {
        if g != UNALIGNED {
            *plane = if axis(g).1 { f32::MAX } else { f32::MIN };
        }
    }
    let faces: Vec<u8> = solid
        .as_chunks::<4>()
        .0
        .iter()
        .map(|quad| {
            let (g, plane) = facing(quad);
            groups.quads[g] += 1;
            if g != UNALIGNED {
                let nearest = &mut groups.planes[g];
                *nearest = if axis(g).1 {
                    nearest.min(plane)
                } else {
                    nearest.max(plane)
                };
            }
            g as u8
        })
        .collect();
    if faces.windows(2).all(|w| w[0] <= w[1]) {
        return groups;
    }
    let mut next = [0usize; GROUPS];
    let mut start = 0;
    for g in 0..GROUPS {
        next[g] = start;
        start += groups.quads[g] as usize;
    }
    let mut sorted = vec![solid[0]; solid.len()];
    for (quad, &g) in solid.as_chunks::<4>().0.iter().zip(&faces) {
        let at = next[g as usize] * 4;
        sorted[at..at + 4].copy_from_slice(quad);
        next[g as usize] += 1;
    }
    *solid = sorted;
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(pos: [f32; 3], layer: u16) -> TerrainVertex {
        TerrainVertex::new(pos, [0.0; 2], layer, 0, 0)
    }
    /// A unit quad in the plane `axis = plane`, wound to face `sign`, tagged
    /// with `layer` so tests can follow it.
    fn quad(axis: usize, plane: f32, sign: f32, layer: u16) -> Vec<TerrainVertex> {
        let corner = |u: f32, v: f32| {
            let mut p = [0.0; 3];
            p[axis] = plane;
            p[(axis + 1) % 3] = u;
            p[(axis + 2) % 3] = v;
            vertex(p, layer)
        };
        let mut q = vec![
            corner(0.0, 0.0),
            corner(1.0, 0.0),
            corner(1.0, 1.0),
            corner(0.0, 1.0),
        ];
        if sign < 0.0 {
            q.reverse();
        }
        q
    }

    #[test]
    fn quads_group_by_the_side_their_winding_faces() {
        for a in 0..3 {
            for sign in [-1.0, 1.0] {
                let q = quad(a, 4.0, sign, 0);
                let (g, plane) = facing(&q);
                assert_eq!(axis(g), (a, sign > 0.0), "axis {a} sign {sign}");
                assert_eq!(plane, 4.0);
                let normal = (Vec3::from(q[1].pos) - Vec3::from(q[0].pos))
                    .cross(Vec3::from(q[2].pos) - Vec3::from(q[0].pos));
                assert_eq!(normal[a].signum(), sign);
            }
        }
        // A cross plant's diagonal quad faces no axis.
        let diagonal = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
        ]
        .map(|p| vertex(p, 0));
        assert_eq!(facing(&diagonal).0, UNALIGNED);
    }

    #[test]
    fn grouping_is_stable_and_skips_only_back_facing_groups() {
        // +X at x = 3 and x = 7 (layers 1, 2), -X at x = 5 (3), an unaligned
        // quad (4), a coplanar overlay on the x = 3 quad (5).
        let diagonal = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
        ]
        .map(|p| vertex(p, 4));
        let mut solid = [
            quad(0, 3.0, 1.0, 1),
            quad(0, 5.0, -1.0, 3),
            diagonal.to_vec(),
            quad(0, 7.0, 1.0, 2),
            quad(0, 3.0, 1.0, 5),
        ]
        .concat();
        let groups = group(&mut solid);
        let layers: Vec<u16> = solid.chunks(4).map(|q| q[0].layer).collect();
        assert_eq!(layers, [3, 4, 1, 2, 5]);
        let ranges = |eye: Vec3| {
            let mut out = Vec::new();
            groups.ranges(eye, |first, count| out.push((first, count)));
            out
        };
        // Between the planes: every group can face the eye.
        assert_eq!(ranges(Vec3::new(4.0, 8.0, 8.0)), [(0, 5)]);
        // Beyond x = 7: the -X quad at x = 5 faces away.
        assert_eq!(ranges(Vec3::new(20.0, 8.0, 8.0)), [(1, 4)]);
        // Before x = 3: every +X quad faces away.
        assert_eq!(ranges(Vec3::new(-10.0, 8.0, 8.0)), [(0, 2)]);
        // Within the margin of the nearest plane: still drawn.
        assert_eq!(ranges(Vec3::new(3.0, 8.0, 8.0)), [(0, 5)]);
    }
}
