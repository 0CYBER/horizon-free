//! `SectionOcclusionGraph`: which sections the camera can see through open
//! faces. Breadth-first from the camera section through the faces each
//! section's VisGraph connects, never stepping back towards the camera.
//!
//! The search runs over dense arrays around the camera rather than hash sets,
//! so it stays cheap enough to redo whenever a section is meshed.
use super::SectionKey;
use std::collections::{HashMap, VecDeque};

const STEPS: [(i32, i32, i32); 6] = [
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
    (-1, 0, 0),
    (1, 0, 0),
];
const NONE: u8 = 6;

pub struct Occlusion {
    /// VisGraph connectivity per column, one entry per section (bottom first).
    columns: HashMap<(i32, i32), Box<[u64]>>,
    sections: i32,
    dirty: bool,
    from: Option<SectionKey>,
    radius: i32,
    /// The searched box: lowest section y and its height in sections.
    y0: i32,
    height: i32,
    reached: Vec<bool>,
    vis: Vec<u64>,
    queue: VecDeque<(u32, u8, u8)>,
}

impl Occlusion {
    pub fn new(sections: i32) -> Self {
        Self {
            columns: HashMap::new(),
            sections,
            dirty: true,
            from: None,
            radius: 0,
            y0: 0,
            height: 0,
            reached: Vec::new(),
            vis: Vec::new(),
            queue: VecDeque::new(),
        }
    }
    pub fn set(&mut self, key: SectionKey, vis: u64) {
        if !(0..self.sections).contains(&key.1) {
            return;
        }
        let sections = self.sections as usize;
        let column = self
            .columns
            .entry((key.0, key.2))
            .or_insert_with(|| vec![crate::mesher::ALL_VISIBLE; sections].into());
        let slot = &mut column[key.1 as usize];
        if *slot != vis {
            *slot = vis;
            self.dirty = true;
        }
    }
    pub fn remove(&mut self, key: SectionKey) {
        let Some(column) = self.columns.get_mut(&(key.0, key.2)) else {
            return;
        };
        if let Some(slot) = column.get_mut(key.1 as usize)
            && *slot != crate::mesher::ALL_VISIBLE
        {
            *slot = crate::mesher::ALL_VISIBLE;
            self.dirty = true;
        }
        if column.iter().all(|&v| v == crate::mesher::ALL_VISIBLE) {
            self.columns.remove(&(key.0, key.2));
        }
    }
    pub fn clear(&mut self) {
        self.columns.clear();
        self.dirty = true;
    }
    fn index(&self, key: SectionKey) -> Option<usize> {
        let from = self.from?;
        let (dx, dz, dy) = (key.0 - from.0, key.2 - from.2, key.1 - self.y0);
        let side = 2 * self.radius + 1;
        if dx.abs() > self.radius || dz.abs() > self.radius || !(0..self.height).contains(&dy) {
            return None;
        }
        Some((((dz + self.radius) * side + dx + self.radius) * self.height + dy) as usize)
    }
    fn key(&self, index: usize) -> SectionKey {
        let from = self.from.expect("searched");
        let side = 2 * self.radius + 1;
        let i = index as i32;
        let dy = i % self.height;
        let column = i / self.height;
        (
            from.0 + column % side - self.radius,
            self.y0 + dy,
            from.2 + column / side - self.radius,
        )
    }
    pub fn contains(&self, key: SectionKey) -> bool {
        self.index(key).is_some_and(|i| self.reached[i])
    }
    /// Search again when sections changed or the camera moved to another section.
    pub fn update(&mut self, from: SectionKey, radius: i32) {
        if !self.dirty && self.from == Some(from) && self.radius == radius {
            return;
        }
        self.from = Some(from);
        self.radius = radius;
        self.y0 = from.1.min(-1);
        self.height = from.1.max(self.sections) - self.y0 + 1;
        let side = (2 * radius + 1) as usize;
        let size = side * side * self.height as usize;
        self.reached.clear();
        self.reached.resize(size, false);
        self.vis.clear();
        self.vis.resize(size, crate::mesher::ALL_VISIBLE);
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let Some(column) = self.columns.get(&(from.0 + dx, from.2 + dz)) else {
                    continue;
                };
                let base = self
                    .index((from.0 + dx, 0, from.2 + dz))
                    .expect("column inside the searched box");
                self.vis[base..base + column.len()].copy_from_slice(column);
            }
        }
        let start = self.index(from).expect("camera inside the searched box");
        self.reached[start] = true;
        self.queue.clear();
        self.queue.push_back((start as u32, NONE, 0));
        while let Some((index, entered, travelled)) = self.queue.pop_front() {
            let vis = self.vis[index as usize];
            let key = self.key(index as usize);
            for (d, step) in STEPS.iter().enumerate() {
                let opposite = d ^ 1;
                if travelled & (1 << opposite) != 0 {
                    continue;
                }
                if entered != NONE && vis & (1 << (usize::from(entered) * 6 + d)) == 0 {
                    continue;
                }
                let next = (key.0 + step.0, key.1 + step.1, key.2 + step.2);
                if next.1 < -1 || next.1 > self.sections {
                    continue;
                }
                let Some(n) = self.index(next) else { continue };
                if !self.reached[n] {
                    self.reached[n] = true;
                    self.queue
                        .push_back((n as u32, opposite as u8, travelled | (1 << d)));
                }
            }
        }
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::ALL_VISIBLE;

    #[test]
    fn open_space_reaches_the_whole_box() {
        let mut o = Occlusion::new(4);
        o.update((0, 1, 0), 2);
        for x in -2..=2 {
            for z in -2..=2 {
                for y in -1..=4 {
                    assert!(o.contains((x, y, z)), "{x} {y} {z}");
                }
            }
        }
        assert!(!o.contains((3, 1, 0)));
    }

    #[test]
    fn closed_neighbours_hide_what_is_behind_them() {
        let mut o = Occlusion::new(4);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.update((0, 1, 0), 3);
        assert!(o.contains((1, 1, 0)), "a neighbour is seen");
        assert!(!o.contains((2, 1, 0)), "nothing shows through it");
        o.set((1, 1, 0), ALL_VISIBLE);
        o.update((0, 1, 0), 3);
        assert!(o.contains((2, 1, 0)));
    }
}
