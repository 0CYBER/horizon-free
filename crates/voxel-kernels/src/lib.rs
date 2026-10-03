//! Original occupancy kernels for synthetic 16³ voxel sections.
//! A set bit means an opaque full cube; all neighbors outside the section are air.
//! This is not a Minecraft renderer or a model of partial/transparent blocks.

pub const SIDE: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    rows: [u16; SIDE * SIDE],
}

impl Default for Section {
    fn default() -> Self {
        Self {
            rows: [0; SIDE * SIDE],
        }
    }
}

impl Section {
    pub fn from_fn(mut occupied: impl FnMut(usize, usize, usize) -> bool) -> Self {
        let mut section = Self::default();
        for z in 0..SIDE {
            for y in 0..SIDE {
                for x in 0..SIDE {
                    if occupied(x, y, z) {
                        section.rows[z * SIDE + y] |= 1 << x;
                    }
                }
            }
        }
        section
    }

    pub fn occupied(&self, x: usize, y: usize, z: usize) -> bool {
        assert!(x < SIDE && y < SIDE && z < SIDE);
        self.rows[z * SIDE + y] & (1 << x) != 0
    }

    /// Simple per-voxel reference. Count each opaque cube's exposed unit faces.
    pub fn exposed_faces_scalar(&self) -> u32 {
        let mut faces = 0;
        for z in 0..SIDE {
            for y in 0..SIDE {
                for x in 0..SIDE {
                    if !self.occupied(x, y, z) {
                        continue;
                    }
                    faces += u32::from(x == 0 || !self.occupied(x - 1, y, z));
                    faces += u32::from(x + 1 == SIDE || !self.occupied(x + 1, y, z));
                    faces += u32::from(y == 0 || !self.occupied(x, y - 1, z));
                    faces += u32::from(y + 1 == SIDE || !self.occupied(x, y + 1, z));
                    faces += u32::from(z == 0 || !self.occupied(x, y, z - 1));
                    faces += u32::from(z + 1 == SIDE || !self.occupied(x, y, z + 1));
                }
            }
        }
        faces
    }

    /// Evaluate sixteen voxels at once with row masks and population counts.
    pub fn exposed_faces_bitset(&self) -> u32 {
        let mut faces = 0;
        for z in 0..SIDE {
            for y in 0..SIDE {
                let i = z * SIDE + y;
                let row = self.rows[i];
                let below = if y == 0 { 0 } else { self.rows[i - 1] };
                let above = if y + 1 == SIDE { 0 } else { self.rows[i + 1] };
                let back = if z == 0 { 0 } else { self.rows[i - SIDE] };
                let front = if z + 1 == SIDE {
                    0
                } else {
                    self.rows[i + SIDE]
                };
                faces += (row & !(row << 1)).count_ones();
                faces += (row & !(row >> 1)).count_ones();
                faces += (row & !below).count_ones();
                faces += (row & !above).count_ones();
                faces += (row & !back).count_ones();
                faces += (row & !front).count_ones();
            }
        }
        faces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(section: &Section, expected: u32) {
        assert_eq!(section.exposed_faces_scalar(), expected);
        assert_eq!(section.exposed_faces_bitset(), expected);
    }

    #[test]
    fn known_geometry() {
        check(&Section::default(), 0);
        check(&Section::from_fn(|_, _, _| true), 6 * 16 * 16);
        check(&Section::from_fn(|x, y, z| x == 3 && y == 4 && z == 5), 6);
        check(&Section::from_fn(|x, y, z| x < 2 && y == 4 && z == 5), 10);
        check(
            &Section::from_fn(|x, y, z| (x + y + z) % 2 == 0),
            4096 / 2 * 6,
        );
    }

    #[test]
    fn every_single_voxel_including_boundaries() {
        for i in 0..SIDE.pow(3) {
            let mut section = Section::default();
            section.rows[i / SIDE] = 1 << (i % SIDE);
            check(&section, 6);
        }
    }

    #[test]
    fn random_occupancy_matches_reference_at_multiple_densities() {
        let mut state = 0x1234_5678_9abc_def0_u64;
        for threshold in [0, 1, 16, 64, 128, 192, 254, 255, 256] {
            for _ in 0..32 {
                let section = Section::from_fn(|_, _, _| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    (state & 255) < threshold
                });
                assert_eq!(
                    section.exposed_faces_scalar(),
                    section.exposed_faces_bitset()
                );
            }
        }
    }
}
