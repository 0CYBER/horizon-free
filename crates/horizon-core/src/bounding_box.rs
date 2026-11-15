/// Inclusive integer box (vanilla BoundingBox). The default is the one block at the origin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BoundingBox {
    pub min: [i32; 3],
    pub max: [i32; 3],
}
impl BoundingBox {
    pub fn new(min: [i32; 3], max: [i32; 3]) -> Self {
        Self { min, max }
    }
    pub fn from_corners(a: [i32; 3], b: [i32; 3]) -> Self {
        Self {
            min: std::array::from_fn(|i| a[i].min(b[i])),
            max: std::array::from_fn(|i| a[i].max(b[i])),
        }
    }
    pub fn moved(self, d: [i32; 3]) -> Self {
        Self {
            min: std::array::from_fn(|i| self.min[i] + d[i]),
            max: std::array::from_fn(|i| self.max[i] + d[i]),
        }
    }
    pub fn is_inside(&self, p: [i32; 3]) -> bool {
        (0..3).all(|i| p[i] >= self.min[i] && p[i] <= self.max[i])
    }
    pub fn intersects(&self, other: &Self) -> bool {
        (0..3).all(|i| self.max[i] >= other.min[i] && self.min[i] <= other.max[i])
    }
    pub fn encapsulate(&mut self, other: &Self) {
        for i in 0..3 {
            self.min[i] = self.min[i].min(other.min[i]);
            self.max[i] = self.max[i].max(other.max[i]);
        }
    }
    pub fn inflated(self, xz: i32, y: i32) -> Self {
        Self {
            min: [self.min[0] - xz, self.min[1] - y, self.min[2] - xz],
            max: [self.max[0] + xz, self.max[1] + y, self.max[2] + xz],
        }
    }
    pub fn span(&self) -> [i32; 3] {
        std::array::from_fn(|i| self.max[i] - self.min[i] + 1)
    }
    pub fn center(&self) -> [i32; 3] {
        std::array::from_fn(|i| self.min[i] + (self.max[i] - self.min[i] + 1) / 2)
    }
    pub fn corners(&self) -> [[i32; 3]; 8] {
        let (a, b) = (self.min, self.max);
        [
            [a[0], a[1], a[2]],
            [a[0], a[1], b[2]],
            [a[0], b[1], a[2]],
            [a[0], b[1], b[2]],
            [b[0], a[1], a[2]],
            [b[0], a[1], b[2]],
            [b[0], b[1], a[2]],
            [b[0], b[1], b[2]],
        ]
    }
}
