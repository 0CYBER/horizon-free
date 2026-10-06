//! Resident blockstate model meshing. Greedy rectangles preserve the texture
//! and signed UV basis of eligible unit-cube faces; other geometry stays baked.
use crate::{
    Result,
    assets::Assets,
    models::{self, CubeStyle, ModelInstance},
    world::TestWorld,
};
use horizon_core::{geometry::Direction, position::BlockPos};
use horizon_world::block_render::FaceVisibility;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Quad {
    pub origin: [f32; 3],
    pub extent: [f32; 2],
    pub face: u32,
    pub layer: u32,
    pub uv_origin: [f32; 2],
    pub uv_u: [f32; 2],
    pub uv_v: [f32; 2],
    pub shade: f32,
}
pub struct Mesh {
    pub cubes: Vec<Quad>,
    pub models: Vec<ModelInstance>,
    pub visible_faces: usize,
    pub culled_faces: usize,
    pub model_blocks: usize,
}
fn position(face: usize, plane: usize, u: usize, v: usize) -> [usize; 3] {
    match face {
        0 | 1 => [u, plane, v],
        2 | 3 => [u, v, plane],
        _ => [plane, v, u],
    }
}
pub fn build(
    world: &TestWorld<'_>,
    assets: &Assets,
    visibility: &mut FaceVisibility,
) -> Result<Mesh> {
    let mut mesh = Mesh {
        cubes: Vec::new(),
        models: Vec::new(),
        visible_faces: 0,
        culled_faces: 0,
        model_blocks: 0,
    };
    for &(cx, cz) in world.chunks.keys() {
        let mut masks = vec![[None; 256]; 6 * 16];
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    let pos = BlockPos::new(cx * 16 + x, y, cz * 16 + z);
                    let state = world.block(pos.x, pos.y, pos.z);
                    if world.is_air(state) {
                        continue;
                    }
                    mesh.model_blocks += 1;
                    let selected = assets.model_catalog.collect(state, pos)?;
                    let mut visible = [false; 6];
                    for direction in Direction::ALL {
                        let [dx, dy, dz] = direction.normal();
                        let neighbor = world.block(pos.x + dx, pos.y + dy, pos.z + dz);
                        visible[direction as usize] =
                            visibility.should_render_face(state, neighbor, direction, true)?;
                    }
                    if selected.len() == 1
                        && let Some(styles) = selected[0].1.cube
                    {
                        for (face, style) in styles.into_iter().enumerate() {
                            if !visible[face] {
                                mesh.culled_faces += 1;
                                continue;
                            }
                            mesh.visible_faces += 1;
                            let (plane, u, v) = match face {
                                0 | 1 => (y, x, z),
                                2 | 3 => (z, x, y),
                                _ => (x, z, y),
                            };
                            masks[face * 16 + plane as usize][(u + 16 * v) as usize] = Some(style);
                        }
                    } else {
                        for (_, model) in selected {
                            for face in &model.faces {
                                if face.quad.cull.is_some_and(|d| !visible[d as usize]) {
                                    mesh.culled_faces += 1;
                                    continue;
                                }
                                mesh.visible_faces += 1;
                                mesh.models.push(models::instance(face, pos));
                            }
                        }
                    }
                }
            }
        }
        for (index, mask) in masks.iter_mut().enumerate() {
            merge(mask, index / 16, index % 16, cx, cz, &mut mesh.cubes);
        }
    }
    Ok(mesh)
}
fn merge(
    mask: &mut [Option<CubeStyle>; 256],
    face: usize,
    plane: usize,
    cx: i32,
    cz: i32,
    quads: &mut Vec<Quad>,
) {
    for v in 0..16 {
        let mut u = 0;
        while u < 16 {
            let Some(style) = mask[u + v * 16] else {
                u += 1;
                continue;
            };
            let mut width = 1;
            while u + width < 16 && mask[u + width + v * 16] == Some(style) {
                width += 1;
            }
            let mut height = 1;
            while v + height < 16
                && (u..u + width).all(|x| mask[x + (v + height) * 16] == Some(style))
            {
                height += 1;
            }
            for y in v..v + height {
                for x in u..u + width {
                    mask[x + y * 16] = None;
                }
            }
            let p = position(face, plane + usize::from(face % 2 == 1), u, v);
            let [a, b, c, d, e, f] = style.uv.map(f32::from);
            quads.push(Quad {
                origin: [
                    (cx * 16 + p[0] as i32) as f32,
                    p[1] as f32,
                    (cz * 16 + p[2] as i32) as f32,
                ],
                extent: [width as f32, height as f32],
                face: face as u32,
                layer: style.layer,
                uv_origin: [a, b],
                uv_u: [c, d],
                uv_v: [e, f],
                shade: f32::from_bits(style.shade_bits),
            });
            u += width;
        }
    }
}

#[cfg(test)]
fn index(x: usize, y: usize, z: usize) -> usize {
    x + 18 * (z + 18 * y)
}
#[cfg(test)]
fn mesh_chunk(cells: &[i32; 18 * 18 * 18], cx: i32, cz: i32, quads: &mut Vec<Quad>) {
    for face in 0..6 {
        for plane in 0..16 {
            let mut mask = [None; 256];
            for v in 0..16 {
                for u in 0..16 {
                    let p = position(face, plane + 1, u + 1, v + 1);
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
                    let layer = cells[index(p[0], p[1], p[2])];
                    if layer >= 0 && cells[index(adjacent[0], adjacent[1], adjacent[2])] < 0 {
                        mask[u + 16 * v] = Some(CubeStyle {
                            layer: layer as u32,
                            uv: [0, 1, 1, 0, 0, -1],
                            shade_bits: 1f32.to_bits(),
                        });
                    }
                }
            }
            merge(&mut mask, face, plane, cx, cz, quads);
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
    fn merging_preserves_rotation_mirroring_shade_and_texture() {
        let bases = [
            [0, 0, 1, 0, 0, 1],
            [1, 0, 0, 1, -1, 0],
            [1, 1, -1, 0, 0, -1],
            [0, 1, 0, -1, 1, 0],
            [1, 0, -1, 0, 0, 1],
            [0, 1, 1, 0, 0, -1],
            [0, 0, 0, 1, 1, 0],
            [1, 1, 0, -1, -1, 0],
        ];
        let original: [Option<CubeStyle>; 256] = std::array::from_fn(|i| {
            (i % 17 != 0).then_some(CubeStyle {
                layer: (i / 64) as u32,
                uv: bases[(i / 4) % 8],
                shade_bits: if i % 5 == 0 { 0.5f32 } else { 1f32 }.to_bits(),
            })
        });
        for face in 0..6 {
            let mut mask = original;
            let mut quads = Vec::new();
            merge(&mut mask, face, 4, -2, 3, &mut quads);
            let mut covered = [false; 256];
            for q in quads {
                let p = [q.origin[0] + 32.0, q.origin[1], q.origin[2] - 48.0];
                let (u0, v0) = match face {
                    0 | 1 => (p[0] as usize, p[2] as usize),
                    2 | 3 => (p[0] as usize, p[1] as usize),
                    _ => (p[2] as usize, p[1] as usize),
                };
                for dv in 0..q.extent[1] as usize {
                    for du in 0..q.extent[0] as usize {
                        let index = u0 + du + 16 * (v0 + dv);
                        assert!(!covered[index]);
                        covered[index] = true;
                        let style = original[index].unwrap();
                        assert_eq!(q.layer, style.layer);
                        assert_eq!(q.shade.to_bits(), style.shade_bits);
                        let [a, b, c, d, e, f] = style.uv.map(f32::from);
                        let local = [a + c * 0.25 + e * 0.75, b + d * 0.25 + f * 0.75];
                        let merged: [f32; 2] = std::array::from_fn(|axis| {
                            q.uv_origin[axis]
                                + q.uv_u[axis] * (du as f32 + 0.25)
                                + q.uv_v[axis] * (dv as f32 + 0.75)
                        });
                        assert_eq!(
                            local.map(|v| v.rem_euclid(1.0)),
                            merged.map(|v| v.rem_euclid(1.0))
                        );
                    }
                }
            }
            assert_eq!(covered, original.map(|v| v.is_some()));
        }
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
