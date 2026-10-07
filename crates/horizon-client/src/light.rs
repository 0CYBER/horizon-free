//! Sky and block light for one chunk, flood-filled over its 3x3 neighbourhood
//! with vanilla's rules: sky sources run down each column until an edge is
//! occluded (`ChunkSkyLightSources`), and light then spreads losing
//! `max(1, dampening)` per step (`LightEngine.getOpacity`). Light can travel at
//! most 15 blocks, so the 48-wide region determines the centre exactly, apart
//! from face-shape occlusion of partial blocks (slabs, stairs), which is not
//! modelled yet.
use crate::{
    blocks::BlockFacts,
    chunk::{Chunk, HEIGHT, Light, SECTIONS, Section, index},
};
use std::{collections::VecDeque, sync::Arc};

const W: usize = 48;
#[derive(Clone, Copy)]
struct Cell {
    dampening: u8,
    emission: u8,
}
pub struct LightTables {
    cells: Vec<Cell>,
}
impl LightTables {
    pub fn new(facts: &BlockFacts) -> Self {
        Self {
            cells: facts
                .states
                .iter()
                .map(|s| Cell {
                    dampening: s.dampening.min(15),
                    emission: s.emission.min(15),
                })
                .collect(),
        }
    }
}
/// `region[dz][dx]` holds the chunks around the centre (index 1,1).
pub fn light_chunk(region: &[[Option<&Chunk>; 3]; 3], tables: &LightTables) -> Vec<Arc<Section>> {
    let center = region[1][1].expect("centre chunk");
    // Highest occupied section in the region bounds the work.
    let mut top_section = 0;
    for chunk in region.iter().flatten().flatten() {
        for (i, s) in chunk.sections.iter().enumerate().rev() {
            if !s.is_uniform(0) {
                top_section = top_section.max(i + 1);
                break;
            }
        }
    }
    let h = (top_section * 16 + 1).min(HEIGHT as usize);
    let at = |x: usize, y: usize, z: usize| (y * W + z) * W + x;
    let mut damp = vec![0u8; W * W * h];
    let mut sky = vec![0u8; W * W * h];
    let mut block = vec![0u8; W * W * h];
    let mut queue = VecDeque::new();
    let mut sky_queue = VecDeque::new();
    for (dz, row) in region.iter().enumerate() {
        for (dx, chunk) in row.iter().enumerate() {
            let Some(chunk) = chunk else { continue };
            for (si, section) in chunk.sections.iter().enumerate().take(top_section) {
                if section.is_uniform(0) {
                    continue;
                }
                for i in 0..4096 {
                    let id = section.get(i) as usize;
                    let cell = tables.cells[id];
                    let (lx, lz, ly) = (i & 15, (i >> 4) & 15, i >> 8);
                    let p = at(dx * 16 + lx, si * 16 + ly, dz * 16 + lz);
                    damp[p] = cell.dampening;
                    if cell.emission > 0 {
                        block[p] = cell.emission;
                        queue.push_back(p as u32);
                    }
                }
            }
        }
    }
    // Columns of missing neighbour chunks contribute no light.
    let present = |x: usize, z: usize| region[z / 16][x / 16].is_some();
    let mut lowest = vec![h as i32; W * W];
    for z in 0..W {
        for x in 0..W {
            if !present(x, z) {
                continue;
            }
            let mut y = h as i32 - 1;
            // The top layer (above the occupied range) is always open sky.
            while y >= 0 && damp[at(x, y as usize, z)] == 0 {
                sky[at(x, y as usize, z)] = 15;
                y -= 1;
            }
            lowest[z * W + x] = y + 1;
        }
    }
    for z in 0..W {
        for x in 0..W {
            if !present(x, z) {
                continue;
            }
            let own = lowest[z * W + x];
            let mut deepest = own;
            for (nx, nz) in [(x.wrapping_sub(1), z), (x + 1, z), (x, z.wrapping_sub(1)), (x, z + 1)] {
                if nx < W && nz < W && present(nx, nz) {
                    deepest = deepest.max(lowest[nz * W + nx]);
                }
            }
            // Seed propagation from the source cells that border darker cells.
            let from = own.max(0) as usize;
            let to = (deepest as usize).min(h - 1);
            for y in from..=to {
                if sky[at(x, y, z)] == 15 {
                    sky_queue.push_back(at(x, y, z) as u32);
                }
            }
        }
    }
    let spread = |light: &mut [u8], queue: &mut VecDeque<u32>| {
        while let Some(p) = queue.pop_front() {
            let p = p as usize;
            let level = light[p];
            if level <= 1 {
                continue;
            }
            let x = p % W;
            let z = (p / W) % W;
            let y = p / (W * W);
            let mut visit = |q: usize| {
                let next = level.saturating_sub(damp[q].max(1));
                if next > light[q] {
                    light[q] = next;
                    queue.push_back(q as u32);
                }
            };
            if x > 0 {
                visit(p - 1);
            }
            if x + 1 < W {
                visit(p + 1);
            }
            if z > 0 {
                visit(p - W);
            }
            if z + 1 < W {
                visit(p + W);
            }
            if y > 0 {
                visit(p - W * W);
            }
            if y + 1 < h {
                visit(p + W * W);
            }
        }
    };
    spread(&mut sky, &mut sky_queue);
    spread(&mut block, &mut queue);
    let mut sections = Vec::with_capacity(SECTIONS);
    for (si, section) in center.sections.iter().enumerate() {
        let mut out = (**section).clone();
        if si * 16 >= h {
            out.light = Light::Uniform(0xF0);
        } else {
            let mut data = Box::new([0u8; 4096]);
            for ly in 0..16 {
                let y = si * 16 + ly;
                if y >= h {
                    for i in 0..256 {
                        data[ly << 8 | i] = 0xF0;
                    }
                    continue;
                }
                for lz in 0..16 {
                    for lx in 0..16 {
                        let p = at(16 + lx, y, 16 + lz);
                        data[index(lx as i32, ly as i32, lz as i32)] = sky[p] << 4 | block[p];
                    }
                }
            }
            out.light = Light::Full(data);
        }
        out.compact();
        sections.push(Arc::new(out));
    }
    sections
}
