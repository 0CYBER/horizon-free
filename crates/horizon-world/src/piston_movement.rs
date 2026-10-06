//! Per-tick Entity piston movement accumulation and displacement limiting.
use horizon_core::geometry::Vec3;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PistonMovement {
    pub game_time: i64,
    pub deltas: [f64; 3],
}
impl PistonMovement {
    /// Native queries game time only after the length threshold. A failed time
    /// query leaves this accumulator unchanged. Tiny motion passes
    /// through before either the query or per-tick reset.
    pub fn limit<E>(
        &mut self,
        movement: Vec3,
        game_time: impl FnOnce() -> Result<i64, E>,
    ) -> Result<Vec3, E> {
        let length = movement.x * movement.x + movement.y * movement.y + movement.z * movement.z;
        if length <= 1.0e-7 {
            return Ok(movement);
        }
        let time = game_time()?;
        if self.game_time != time {
            self.deltas = [0.0; 3];
            self.game_time = time;
        }
        for (axis, value) in [movement.x, movement.y, movement.z].into_iter().enumerate() {
            if value != 0.0 {
                let sum = value + self.deltas[axis];
                // Math.min in native Mth.clamp preserves NaN.
                let next = if sum < -0.51 {
                    -0.51
                } else if sum.is_nan() {
                    sum
                } else {
                    sum.min(0.51)
                };
                let change = next - self.deltas[axis];
                self.deltas[axis] = next;
                if change.abs() <= f64::from(1.0e-5f32) {
                    return Ok(Vec3::new(0.0, 0.0, 0.0));
                }
                let mut result = [0.0; 3];
                result[axis] = change;
                return Ok(Vec3::new(result[0], result[1], result[2]));
            }
        }
        Ok(Vec3::new(0.0, 0.0, 0.0))
    }
}
