//! Pregeneration (`/pregen`): every chunk of a square or a circle generated,
//! lit and saved ahead of the players. The area is cut into strips of columns
//! and each strip is swept north to south by a [`Band`] a few rows deep:
//! streaming generates the band's chunks and keeps them until they are lit,
//! then the band moves on and the rows behind it unload and are saved, so
//! memory stays bounded whatever the radius.
use crate::streaming::Band;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Square,
    Circle,
}

/// The widest strip, in chunks. Strips are swept one after another; the few
/// columns along the edge between two are generated with both.
const STRIP_WIDTH: i32 = 256;
/// How many chunks a band holds at least: enough to keep every worker busy
/// while the rows before it finish.
const BAND_CHUNKS: i32 = 1024;

pub struct Pregen {
    /// The centre chunk and the radius in chunks.
    pub center: (i32, i32),
    pub radius: i32,
    pub shape: Shape,
    /// The strips' columns, west to east.
    strips: Vec<(i32, i32)>,
    strip: usize,
    /// The band's first row: every row of the strip before it is done.
    row: i32,
    /// Chunks of the area lit so far, and in all.
    pub done: u64,
    pub total: u64,
}

impl Pregen {
    pub fn new(center: (i32, i32), radius: i32, shape: Shape) -> Self {
        let radius = radius.max(0);
        let (x0, width) = (center.0 - radius, 2 * radius + 1);
        let count = (width + STRIP_WIDTH - 1) / STRIP_WIDTH;
        let strips = (0..count)
            .map(|i| (x0 + width * i / count, x0 + width * (i + 1) / count - 1))
            .collect();
        let mut pregen = Self {
            center,
            radius,
            shape,
            strips,
            strip: 0,
            row: center.1 - radius,
            done: 0,
            total: 0,
        };
        pregen.total = (center.1 - radius..=center.1 + radius)
            .map(|z| pregen.row_chunks(x0, x0 + width - 1, z))
            .sum();
        pregen
    }

    fn contains(&self, x: i32, z: i32) -> bool {
        match self.shape {
            Shape::Square => true,
            Shape::Circle => {
                let (dx, dz) = (i64::from(x - self.center.0), i64::from(z - self.center.1));
                dx * dx + dz * dz <= i64::from(self.radius) * i64::from(self.radius)
            }
        }
    }

    fn row_chunks(&self, x0: i32, x1: i32, z: i32) -> u64 {
        (x0..=x1).filter(|&x| self.contains(x, z)).count() as u64
    }

    /// The band streaming should hold now; none once every strip is done.
    pub fn band(&self) -> Option<Band> {
        let &(x0, x1) = self.strips.get(self.strip)?;
        let rows = (BAND_CHUNKS + (x1 - x0)) / (x1 - x0 + 1);
        Some(Band {
            x0,
            x1,
            z0: self.row,
            z1: (self.row + rows - 1).min(self.center.1 + self.radius),
            circle: (self.shape == Shape::Circle).then_some((self.center, self.radius)),
        })
    }

    /// Move past the band's first rows once all their chunks are `lit`.
    /// Returns whether the band moved.
    pub fn advance(&mut self, lit: impl Fn(i32, i32) -> bool) -> bool {
        let mut moved = false;
        while let Some(&(x0, x1)) = self.strips.get(self.strip) {
            let z = self.row;
            if !(x0..=x1).all(|x| !self.contains(x, z) || lit(x, z)) {
                break;
            }
            self.done += self.row_chunks(x0, x1, z);
            self.row += 1;
            if self.row > self.center.1 + self.radius {
                self.strip += 1;
                self.row = self.center.1 - self.radius;
            }
            moved = true;
        }
        moved
    }

    /// The chunks of the band's first row, which it waits for.
    pub fn first_row(&self) -> Vec<(i32, i32)> {
        let Some(&(x0, x1)) = self.strips.get(self.strip) else {
            return Vec::new();
        };
        (x0..=x1)
            .filter(|&x| self.contains(x, self.row))
            .map(|x| (x, self.row))
            .collect()
    }

    pub fn finished(&self) -> bool {
        self.strip >= self.strips.len()
    }
}
