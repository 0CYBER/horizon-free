//! Pinned world-border extent geometry and tick-driven size interpolation.
//! Saved-data codecs, listeners, damage and synchronization are separate layers.
use super::{Aabb, ShapeError, ShapeOp, Vec3, VoxelShape, clamp, java_max, java_min};

#[derive(Debug)]
pub enum BorderError {
    InvalidParameters,
    Shape(ShapeError),
}
impl std::fmt::Display for BorderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {Self::InvalidParameters=>f.write_str("border requires finite nonnegative sizes, finite centers and nonnegative limits/duration"),Self::Shape(e)=>e.fmt(f)}
    }
}
impl std::error::Error for BorderError {}
impl From<ShapeError> for BorderError {
    fn from(e: ShapeError) -> Self {
        Self::Shape(e)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderStatus {
    Stationary,
    Growing,
    Shrinking,
}
#[derive(Clone, Debug)]
enum Extent {
    Static {
        size: f64,
        bounds: [f64; 4],
        shape: VoxelShape,
    },
    Moving {
        from: f64,
        to: f64,
        duration: i64,
        remaining: i64,
        size: f64,
        previous: f64,
    },
}
#[derive(Clone, Debug)]
pub struct BorderGeometry {
    center: [f64; 2],
    absolute_max: i32,
    extent: Extent,
}
impl BorderGeometry {
    pub fn new(center: [f64; 2], size: f64, absolute_max: i32) -> Result<Self, BorderError> {
        if !center.into_iter().all(f64::is_finite)
            || !size.is_finite()
            || size < 0.0
            || absolute_max < 0
        {
            return Err(BorderError::InvalidParameters);
        }
        Ok(Self {
            center,
            absolute_max,
            extent: Self::stationary(center, size, absolute_max)?,
        })
    }
    fn stationary(center: [f64; 2], size: f64, absolute_max: i32) -> Result<Extent, BorderError> {
        let bounds = Self::extent_bounds(center, size, absolute_max);
        Ok(Extent::Static {
            size,
            bounds,
            shape: Self::shape_for(bounds)?,
        })
    }
    fn extent_bounds(center: [f64; 2], size: f64, absolute_max: i32) -> [f64; 4] {
        let limit = f64::from(absolute_max);
        let lower = f64::from(-absolute_max);
        [
            clamp(center[0] - size / 2.0, lower, limit),
            clamp(center[1] - size / 2.0, lower, limit),
            clamp(center[0] + size / 2.0, lower, limit),
            clamp(center[1] + size / 2.0, lower, limit),
        ]
    }
    fn shape_for([min_x, min_z, max_x, max_z]: [f64; 4]) -> Result<VoxelShape, ShapeError> {
        let infinity = VoxelShape::from_box(Aabb::new(
            Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
            Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
        ))?;
        let inside = VoxelShape::from_box(Aabb::new(
            Vec3::new(min_x.floor(), f64::NEG_INFINITY, min_z.floor()),
            Vec3::new(max_x.ceil(), f64::INFINITY, max_z.ceil()),
        ))?;
        infinity.join(&inside, ShapeOp::ONLY_FIRST)
    }
    pub fn center(&self) -> [f64; 2] {
        self.center
    }
    pub fn absolute_max(&self) -> i32 {
        self.absolute_max
    }
    pub fn set_center(&mut self, center: [f64; 2]) -> Result<(), BorderError> {
        if !center.into_iter().all(f64::is_finite) {
            return Err(BorderError::InvalidParameters);
        }
        if let Extent::Static { size, .. } = self.extent {
            self.extent = Self::stationary(center, size, self.absolute_max)?;
        }
        self.center = center;
        Ok(())
    }
    pub fn set_absolute_max(&mut self, limit: i32) -> Result<(), BorderError> {
        if limit < 0 {
            return Err(BorderError::InvalidParameters);
        }
        if let Extent::Static { size, .. } = self.extent {
            self.extent = Self::stationary(self.center, size, limit)?;
        }
        self.absolute_max = limit;
        Ok(())
    }
    pub fn set_size(&mut self, size: f64) -> Result<(), BorderError> {
        if !size.is_finite() || size < 0.0 {
            return Err(BorderError::InvalidParameters);
        }
        self.extent = Self::stationary(self.center, size, self.absolute_max)?;
        Ok(())
    }
    /// Native start/end tick timestamps cancel in the exposed speed calculation;
    /// progress itself advances by one per tick, independently of wall time.
    pub fn lerp_size_between(
        &mut self,
        from: f64,
        to: f64,
        duration: i64,
    ) -> Result<(), BorderError> {
        if !from.is_finite() || !to.is_finite() || from < 0.0 || to < 0.0 || duration < 0 {
            return Err(BorderError::InvalidParameters);
        }
        if from == to {
            return self.set_size(to);
        }
        let size = Self::moving_size(from, to, duration, duration);
        self.extent = Extent::Moving {
            from,
            to,
            duration,
            remaining: duration,
            size,
            previous: size,
        };
        Ok(())
    }
    fn moving_size(from: f64, to: f64, duration: i64, remaining: i64) -> f64 {
        let fraction = (duration as f64 - remaining as f64) / duration as f64;
        if fraction < 1.0 {
            from + fraction * (to - from)
        } else {
            to
        }
    }
    pub fn tick(&mut self) -> Result<(), BorderError> {
        if let Extent::Moving {
            from,
            to,
            duration,
            remaining,
            size,
            previous,
        } = &mut self.extent
        {
            *remaining = remaining.wrapping_sub(1);
            *previous = *size;
            *size = Self::moving_size(*from, *to, *duration, *remaining);
            if *remaining <= 0 {
                self.extent = Self::stationary(self.center, *to, self.absolute_max)?;
            }
        }
        Ok(())
    }
    pub fn size(&self) -> f64 {
        match self.extent {
            Extent::Static { size, .. } | Extent::Moving { size, .. } => size,
        }
    }
    pub fn lerp_target(&self) -> f64 {
        match self.extent {
            Extent::Static { size, .. } => size,
            Extent::Moving { to, .. } => to,
        }
    }
    pub fn lerp_time(&self) -> i64 {
        match self.extent {
            Extent::Static { .. } => 0,
            Extent::Moving { remaining, .. } => remaining,
        }
    }
    pub fn lerp_speed(&self) -> f64 {
        match self.extent {
            Extent::Static { .. } => 0.0,
            Extent::Moving {
                from, to, duration, ..
            } => (from - to).abs() / duration as f64,
        }
    }
    pub fn status(&self) -> BorderStatus {
        match self.extent {
            Extent::Static { .. } => BorderStatus::Stationary,
            Extent::Moving { from, to, .. } => {
                if to < from {
                    BorderStatus::Shrinking
                } else {
                    BorderStatus::Growing
                }
            }
        }
    }
    /// [minX, minZ, maxX, maxZ]. Ordinary collision bounds use partial_tick=0,
    /// hence the previous size while moving. Static extents ignore partial_tick.
    pub fn bounds(&self, partial_tick: f32) -> [f64; 4] {
        match self.extent {
            Extent::Static { bounds, .. } => bounds,
            Extent::Moving { size, previous, .. } => Self::extent_bounds(
                self.center,
                previous + f64::from(partial_tick) * (size - previous),
                self.absolute_max,
            ),
        }
    }
    pub fn collision_shape(&self) -> Result<VoxelShape, ShapeError> {
        match &self.extent {
            Extent::Static { shape, .. } => Ok(shape.clone()),
            Extent::Moving { .. } => Self::shape_for(self.bounds(0.0)),
        }
    }
    pub fn distance_to_border(&self, x: f64, z: f64) -> f64 {
        let [min_x, min_z, max_x, max_z] = self.bounds(0.0);
        java_min(
            java_min(java_min(x - min_x, max_x - x), z - min_z),
            max_z - z,
        )
    }
    pub fn is_within_bounds(&self, x: f64, z: f64, margin: f64) -> bool {
        let [min_x, min_z, max_x, max_z] = self.bounds(0.0);
        x >= min_x - margin && x < max_x + margin && z >= min_z - margin && z < max_z + margin
    }
    pub fn is_inside_close_to_border(&self, entity_x: f64, entity_z: f64, bounds: Aabb) -> bool {
        let size = bounds.size();
        let margin = java_max(java_max(size.x.abs(), size.z.abs()), 1.0);
        self.distance_to_border(entity_x, entity_z) < margin * 2.0
            && self.is_within_bounds(entity_x, entity_z, margin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_parameters_leave_existing_extent_unchanged() {
        let mut border = BorderGeometry::new([1.0, 2.0], 10.0, 100).unwrap();
        let bounds = border.bounds(0.0);
        assert!(border.set_center([f64::NAN, 0.0]).is_err());
        assert!(border.set_size(-1.0).is_err());
        assert!(border.set_size(f64::INFINITY).is_err());
        assert!(border.set_absolute_max(-1).is_err());
        assert!(border.lerp_size_between(10.0, 20.0, -1).is_err());
        assert!(border.lerp_size_between(10.0, f64::NAN, 1).is_err());
        assert_eq!(border.bounds(0.0), bounds);
        assert_eq!(border.center(), [1.0, 2.0]);
        assert_eq!(border.size(), 10.0);
        assert_eq!(border.status(), BorderStatus::Stationary);
    }
}
