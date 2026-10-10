//! The world border (`WorldBorder`): its size lerp, centre, damage and
//! warning settings, and how far a point is from its edge.

pub const MAX_SIZE: f64 = 5.999_997e7;
pub const MAX_CENTER: f64 = 2.999_998_4e7;

#[derive(Clone, Debug, PartialEq)]
pub struct WorldBorder {
    pub center: [f64; 2],
    size: f64,
    /// Size lerp: (from, to, ticks elapsed, ticks total).
    lerp: Option<(f64, f64, i64, i64)>,
    pub damage_per_block: f64,
    pub safe_zone: f64,
    pub warning_distance: i32,
    pub warning_time: i32,
}
impl Default for WorldBorder {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            size: 5.999_997e7,
            lerp: None,
            damage_per_block: 0.2,
            safe_zone: 5.0,
            warning_distance: 5,
            warning_time: 300,
        }
    }
}
impl WorldBorder {
    pub fn size(&self) -> f64 {
        match self.lerp {
            Some((from, to, elapsed, total)) if total > 0 => {
                from + (to - from) * elapsed as f64 / total as f64
            }
            _ => self.size,
        }
    }
    pub fn target_size(&self) -> f64 {
        self.lerp.map_or(self.size, |l| l.1)
    }
    /// Ticks left in a size change (0 when none).
    pub fn remaining_ticks(&self) -> i64 {
        self.lerp.map_or(0, |(_, _, e, t)| t - e)
    }
    pub fn set_size(&mut self, size: f64) {
        self.size = size.clamp(1.0, MAX_SIZE);
        self.lerp = None;
    }
    /// `lerpSizeBetween`.
    pub fn lerp_to(&mut self, to: f64, ticks: i64) {
        let from = self.size();
        if ticks <= 0 || (from - to).abs() < f64::EPSILON {
            self.set_size(to);
        } else {
            self.lerp = Some((from, to.clamp(1.0, MAX_SIZE), 0, ticks));
        }
    }
    pub fn tick(&mut self) {
        if let Some((from, to, elapsed, total)) = self.lerp {
            if elapsed + 1 >= total {
                self.size = to;
                self.lerp = None;
            } else {
                self.lerp = Some((from, to, elapsed + 1, total));
            }
        }
    }
    pub fn bounds(&self) -> [f64; 4] {
        let half = self.size() / 2.0;
        [
            (self.center[0] - half).max(-MAX_CENTER),
            (self.center[1] - half).max(-MAX_CENTER),
            (self.center[0] + half).min(MAX_CENTER),
            (self.center[1] + half).min(MAX_CENTER),
        ]
    }
    /// `getDistanceToBorder(x, z)`: negative outside.
    pub fn distance(&self, x: f64, z: f64) -> f64 {
        let [min_x, min_z, max_x, max_z] = self.bounds();
        (x - min_x).min(max_x - x).min(z - min_z).min(max_z - z)
    }
    pub fn contains(&self, x: f64, z: f64) -> bool {
        let [min_x, min_z, max_x, max_z] = self.bounds();
        x >= min_x && x < max_x && z >= min_z && z < max_z
    }
    /// Clamp a point so a body of half-width `half` stays inside.
    pub fn clamp(&self, x: f64, z: f64, half: f64) -> [f64; 2] {
        let [min_x, min_z, max_x, max_z] = self.bounds();
        [
            x.clamp(min_x + half, (max_x - half).max(min_x + half)),
            z.clamp(min_z + half, (max_z - half).max(min_z + half)),
        ]
    }
    pub fn pack(&self) -> horizon_world::nbt::Tag {
        use horizon_world::nbt::Tag;
        Tag::compound([
            ("center_x", Tag::Double(self.center[0].to_bits())),
            ("center_z", Tag::Double(self.center[1].to_bits())),
            ("size", Tag::Double(self.size().to_bits())),
            (
                "damage_per_block",
                Tag::Double(self.damage_per_block.to_bits()),
            ),
            ("safe_zone", Tag::Double(self.safe_zone.to_bits())),
            ("warning_blocks", Tag::Int(self.warning_distance)),
            ("warning_time", Tag::Int(self.warning_time)),
            ("lerp_target", Tag::Double(self.target_size().to_bits())),
            ("lerp_time", Tag::Long(self.remaining_ticks())),
        ])
    }
    pub fn unpack(tag: &horizon_world::nbt::Tag) -> Self {
        use horizon_world::nbt::Tag;
        let d = |k: &str, default: f64| match tag.get(k) {
            Some(Tag::Double(b)) => f64::from_bits(*b),
            _ => default,
        };
        let i = |k: &str, default: i32| match tag.get(k) {
            Some(Tag::Int(v)) => *v,
            _ => default,
        };
        let mut b = Self {
            center: [d("center_x", 0.0), d("center_z", 0.0)],
            size: d("size", MAX_SIZE).clamp(1.0, MAX_SIZE),
            lerp: None,
            damage_per_block: d("damage_per_block", 0.2),
            safe_zone: d("safe_zone", 5.0),
            warning_distance: i("warning_blocks", 5),
            warning_time: i("warning_time", 300),
        };
        if let Some(Tag::Long(t)) = tag.get("lerp_time")
            && *t > 0
        {
            let to = d("lerp_target", b.size);
            b.lerp = Some((b.size, to.clamp(1.0, MAX_SIZE), 0, *t));
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_and_distance() {
        let mut b = WorldBorder::default();
        b.set_size(100.0);
        assert_eq!(b.distance(0.0, 0.0), 50.0);
        assert_eq!(b.distance(60.0, 0.0), -10.0);
        assert!(b.contains(49.9, -49.9) && !b.contains(50.0, 0.0));
        b.lerp_to(200.0, 10);
        assert_eq!(b.size(), 100.0);
        for _ in 0..5 {
            b.tick();
        }
        assert_eq!(b.size(), 150.0);
        assert_eq!(b.remaining_ticks(), 5);
        for _ in 0..5 {
            b.tick();
        }
        assert_eq!((b.size(), b.remaining_ticks()), (200.0, 0));
        // An instant change and clamping a body inside.
        b.lerp_to(10.0, 0);
        assert_eq!(b.size(), 10.0);
        assert_eq!(b.clamp(100.0, -100.0, 0.3), [4.7, -4.7]);
        b.center = [10.0, 0.0];
        assert_eq!(b.bounds(), [5.0, -5.0, 15.0, 5.0]);
    }
    #[test]
    fn saved_form_round_trips() {
        let mut b = WorldBorder::default();
        b.set_size(500.0);
        b.center = [3.5, -7.0];
        b.damage_per_block = 1.5;
        b.warning_distance = 9;
        let back = WorldBorder::unpack(&b.pack());
        assert_eq!(back, b);
    }
}
