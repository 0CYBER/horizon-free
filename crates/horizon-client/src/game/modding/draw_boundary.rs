//! Shared CPU submission limits. A quad uses four vertices and six u32 indices.
//! This is the index-addressable maximum, not an arbitrary content limit.
pub(super) const MAX_DRAW_VERTICES: usize = (u32::MAX as usize / 6) * 4;

pub(super) fn quad_allowed(
    corners: [[f32; 3]; 4],
    uv: [[f32; 2]; 4],
    len: usize,
    limit: usize,
) -> bool {
    corners
        .into_iter()
        .flatten()
        .chain(uv.into_iter().flatten())
        .all(f32::is_finite)
        && len.checked_add(4).is_some_and(|end| end <= limit)
}

pub(super) fn truncate<T>(vertices: &mut Vec<T>, limit: usize) {
    vertices.truncate(limit / 4 * 4);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn non_finite_quads_are_dropped() {
        assert!(quad_allowed([[0.0; 3]; 4], [[0.0; 2]; 4], 0, 8));
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut corners = [[0.0; 3]; 4];
            corners[2][1] = bad;
            assert!(!quad_allowed(corners, [[0.0; 2]; 4], 0, 8));
            let mut uv = [[0.0; 2]; 4];
            uv[3][0] = bad;
            assert!(!quad_allowed([[0.0; 3]; 4], uv, 0, 8));
        }
    }
    #[test]
    fn oversized_submission_keeps_only_complete_quads() {
        let mut vertices: Vec<_> = (0..16).collect();
        truncate(&mut vertices, 10);
        assert_eq!(vertices, (0..8).collect::<Vec<_>>());
        assert!(!quad_allowed([[0.0; 3]; 4], [[0.0; 2]; 4], 8, 8));
    }
}
