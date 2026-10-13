//! Fancy cloud face selection (`CloudRenderer.buildExtrudedCell`).
/// Direction order is DOWN, UP, NORTH, SOUTH, WEST, EAST; inward faces use bits 6..12.
pub fn face_mask(x: i32, z: i32, bottom: f32, empty: [bool; 4]) -> u16 {
    let mut mask = 0;
    if bottom <= 0.0 {
        mask |= 1 << 1;
    }
    if bottom + 4.0 >= 0.0 {
        mask |= 1;
    }
    for (i, visible) in [z > 0, z < 0, x > 0, x < 0].into_iter().enumerate() {
        if empty[i] && visible {
            mask |= 1 << (i + 2);
        }
    }
    if x.abs() <= 1 && z.abs() <= 1 {
        mask |= 63 << 6;
    }
    mask
}

/// Exterior faces first, then reversed interior faces, in vanilla emission order.
pub fn faces(mask: u16) -> impl Iterator<Item = (u8, bool)> {
    [1, 0, 2, 3, 4, 5]
        .into_iter()
        .filter(move |d| mask & (1 << d) != 0)
        .map(|d| (d, false))
        .chain(
            (0..6)
                .filter(move |d| mask & (1 << (d + 6)) != 0)
                .map(|d| (d, true)),
        )
}

/// CloudRenderer builds cells in expanding Manhattan rings, nearest first.
pub fn cells(radius: i32) -> impl Iterator<Item = (i32, i32)> {
    (0..=2 * radius).flat_map(move |ring| {
        (-ring..=ring).flat_map(move |x| {
            let z = ring - x.abs();
            let visible = z <= radius && x * x + z * z <= radius * radius;
            (visible && z != 0)
                .then_some((x, -z))
                .into_iter()
                .chain(visible.then_some((x, z)))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cells_are_nearest_first_and_unique() {
        let cells: Vec<_> = cells(2).collect();
        assert_eq!(cells[0], (0, 0));
        assert_eq!(cells.len(), 13);
        assert!(
            cells
                .windows(2)
                .all(|w| w[0].0.abs() + w[0].1.abs() <= w[1].0.abs() + w[1].1.abs())
        );
        assert_eq!(
            cells.iter().collect::<std::collections::HashSet<_>>().len(),
            cells.len()
        );
    }
    #[test]
    fn only_camera_facing_exterior_faces() {
        assert_eq!(face_mask(2, 2, 10.0, [true; 4]), 1 | 4 | 16);
        assert_eq!(face_mask(-2, -2, -5.0, [true; 4]), 2 | 8 | 32);
        assert_eq!(face_mask(2, 2, 0.0, [false; 4]), 3);
    }
    #[test]
    fn connected_cells_have_no_exterior_walls() {
        assert_eq!(face_mask(2, 2, 10.0, [false; 4]), 1);
    }
    #[test]
    fn nearby_cells_keep_inward_faces_for_camera_inside_clouds() {
        assert_eq!(face_mask(0, 0, 0.0, [false; 4]), 3 | (63 << 6));
    }
}
