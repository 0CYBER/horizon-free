//! Greedy opaque-cube meshing for the controlled renderer scene. Model geometry,
//! weighted variants, tinting, transparency and lighting are separate work.
use crate::{Result, world::TestWorld};
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Quad {
    pub origin: [f32; 3],
    pub extent: [f32; 2],
    pub face: u32,
    pub layer: u32,
}
fn position(face: usize, plane: usize, u: usize, v: usize) -> [usize; 3] {
    match face {
        0 | 1 => [u, plane, v],
        2 | 3 => [u, v, plane],
        _ => [plane, v, u],
    }
}
pub fn build(world: &TestWorld<'_>) -> Result<Vec<Quad>> {
    let mut quads = Vec::new();
    for &(cx, cz) in world.chunks.keys() {
        let mut cells = [-1_i32; 18 * 18 * 18];
        for y in 0..18 {
            for z in 0..18 {
                for x in 0..18 {
                    cells[index(x, y, z)] = world.layer(
                        cx * 16 + x as i32 - 1,
                        y as i32 - 1,
                        cz * 16 + z as i32 - 1,
                    )?;
                }
            }
        }
        mesh_chunk(&cells, cx, cz, &mut quads);
    }
    Ok(quads)
}
fn index(x: usize, y: usize, z: usize) -> usize {
    x + 18 * (z + 18 * y)
}
fn mesh_chunk(cells: &[i32; 18 * 18 * 18], cx: i32, cz: i32, quads: &mut Vec<Quad>) {
    for face in 0..6 {
        for plane in 0..16 {
            let mut mask = [-1_i32; 256];
            for v in 0..16 {
                for u in 0..16 {
                    let p = position(face, plane + 1, u + 1, v + 1);
                    let layer = cells[index(p[0], p[1], p[2])];
                    let mut adjacent = p;
                    let axis = match face {
                        0 | 1 => 1,
                        2 | 3 => 2,
                        _ => 0,
                    };
                    if face % 2 == 0 {
                        adjacent[axis] -= 1;
                    } else {
                        adjacent[axis] += 1;
                    }
                    if layer >= 0 && cells[index(adjacent[0], adjacent[1], adjacent[2])] < 0 {
                        mask[u + v * 16] = layer;
                    }
                }
            }
            for v in 0..16 {
                let mut u = 0;
                while u < 16 {
                    let layer = mask[u + v * 16];
                    if layer < 0 {
                        u += 1;
                        continue;
                    }
                    let mut width = 1;
                    while u + width < 16 && mask[u + width + v * 16] == layer {
                        width += 1;
                    }
                    let mut height = 1;
                    while v + height < 16
                        && (u..u + width).all(|x| mask[x + (v + height) * 16] == layer)
                    {
                        height += 1;
                    }
                    for y in v..v + height {
                        for x in u..u + width {
                            mask[x + y * 16] = -1;
                        }
                    }
                    let p = position(face, plane + usize::from(face % 2 == 1), u, v);
                    quads.push(Quad {
                        origin: [
                            (cx * 16 + p[0] as i32) as f32,
                            p[1] as f32,
                            (cz * 16 + p[2] as i32) as f32,
                        ],
                        extent: [width as f32, height as f32],
                        face: face as u32,
                        layer: layer as u32,
                    });
                    u += width;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    // Compare rectangles expanded to unit faces against direct voxel adjacency.
    // The reference does not use the mesher's plane masks or coordinate helper.
    fn verify(cells: &[i32; 18 * 18 * 18], cx: i32, cz: i32) -> Vec<Quad> {
        let mut expected = BTreeSet::new();
        let directions = [
            [0, -1, 0],
            [0, 1, 0],
            [0, 0, -1],
            [0, 0, 1],
            [-1, 0, 0],
            [1, 0, 0],
        ];
        for y in 1..=16 {
            for z in 1..=16 {
                for x in 1..=16 {
                    let layer = cells[index(x, y, z)];
                    if layer < 0 {
                        continue;
                    }
                    for (face, [dx, dy, dz]) in directions.iter().enumerate() {
                        let neighbor = [
                            (x as i32 + dx) as usize,
                            (y as i32 + dy) as usize,
                            (z as i32 + dz) as usize,
                        ];
                        if cells[index(neighbor[0], neighbor[1], neighbor[2])] < 0 {
                            expected.insert((
                                x as i32 - 1 + cx * 16,
                                y as i32 - 1,
                                z as i32 - 1 + cz * 16,
                                face as u32,
                                layer as u32,
                            ));
                        }
                    }
                }
            }
        }
        let mut quads = Vec::new();
        mesh_chunk(cells, cx, cz, &mut quads);
        let mut actual = BTreeSet::new();
        for quad in &quads {
            assert!(
                quad.extent
                    .iter()
                    .all(|&e| (1.0..=16.0).contains(&e) && e.fract() == 0.0)
            );
            for v in 0..quad.extent[1] as i32 {
                for u in 0..quad.extent[0] as i32 {
                    let [x, y, z] = quad.origin.map(|n| n as i32);
                    let (x, y, z) = match quad.face {
                        0 => (x + u, y, z + v),
                        1 => (x + u, y - 1, z + v),
                        2 => (x + u, y + v, z),
                        3 => (x + u, y + v, z - 1),
                        4 => (x, y + v, z + u),
                        5 => (x - 1, y + v, z + u),
                        _ => panic!("invalid face"),
                    };
                    assert!(
                        actual.insert((x, y, z, quad.face, quad.layer)),
                        "overlapping faces"
                    );
                }
            }
        }
        assert_eq!(actual, expected);
        quads
    }

    #[test]
    fn empty_solid_and_neighbor_seams() {
        let mut cells = [-1; 18 * 18 * 18];
        assert!(verify(&cells, 0, 0).is_empty());
        for y in 1..=16 {
            for z in 1..=16 {
                for x in 1..=16 {
                    cells[index(x, y, z)] = 3;
                }
            }
        }
        assert_eq!(verify(&cells, -3, 2).len(), 6);
        // A solid neighbor removes only the shared boundary, including corners.
        for y in 1..=16 {
            for z in 1..=16 {
                cells[index(17, y, z)] = 4;
            }
        }
        assert_eq!(verify(&cells, -3, 2).len(), 5);
        cells.fill(3);
        assert!(verify(&cells, 1, -2).is_empty());
    }

    #[test]
    fn cavities_material_boundaries_and_random_neighbors() {
        let mut state = 0x183d_994e_058b_9432_u64;
        for pattern in 0..12 {
            let mut cells = [-1; 18 * 18 * 18];
            for y in 0..18 {
                for z in 0..18 {
                    for x in 0..18 {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        cells[index(x, y, z)] = match pattern {
                            0 => {
                                if x == 8 && y == 8 && z == 8 {
                                    -1
                                } else {
                                    0
                                }
                            }
                            1 => ((x + y + z) % 3) as i32,
                            _ => (state % 7) as i32 - 1,
                        };
                    }
                }
            }
            verify(&cells, pattern - 6, 3 - pattern);
        }
    }
}
