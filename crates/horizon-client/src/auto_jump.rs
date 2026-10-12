//! Auto-jump (`LocalPlayer.updateAutoJump` and `canAutoJump`): after the
//! local player's own move, look ahead along the movement for a step that
//! the player can clear with a jump and is not walled in above; if found, the
//! next `aiStep` presses jump for one tick (`autoJumpTime = 1`).
use horizon_core::{
    geometry::{Aabb, Axis, Vec3, VoxelShape, trig},
    position::BlockPos,
};

/// `Float.MIN_VALUE`, the "no obstacle" marker.
const NO_OBSTACLE: f32 = f32::from_bits(1);

/// What `updateAutoJump` reads from the player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoJumper {
    pub position: Vec3,
    /// `getBoundingBox().maxY`.
    pub max_y: f64,
    pub x_rot: f32,
    pub y_rot: f32,
    /// `getSpeed()`.
    pub speed: f32,
    /// `input.getMoveVector()` (x: strafe, y: forward).
    pub move_vector: [f32; 2],
    /// `getBbWidth()`, `getBbHeight()`.
    pub width: f32,
    pub height: f32,
    /// The jump boost amplifier, when the effect is active.
    pub jump_boost: Option<i32>,
}

/// The level queries, with the player's collision context.
pub trait AutoJumpWorld {
    type Error;
    /// `getBlockState(p).getCollisionShape(level, p, CollisionContext.of(player))`.
    fn collision_shape(&mut self, p: BlockPos) -> Result<VoxelShape, Self::Error>;
    /// `level.getCollisions(player, bounds)` flattened with `toAabbs`, in order.
    fn collision_boxes(&mut self, bounds: Aabb) -> Result<Vec<Aabb>, Self::Error>;
}

/// `LocalPlayer.canAutoJump`. `staying_on_surface` is
/// `isStayingOnGroundSurface()` (the shift key).
#[allow(clippy::too_many_arguments)]
pub fn can_auto_jump(
    enabled: bool,
    auto_jump_time: i32,
    on_ground: bool,
    staying_on_surface: bool,
    passenger: bool,
    move_vector: [f32; 2],
    block_jump_factor: f32,
) -> bool {
    enabled
        && auto_jump_time <= 0
        && on_ground
        && !staying_on_surface
        && !passenger
        && move_vector[0] * move_vector[0] + move_vector[1] * move_vector[1] > 0.0
        && f64::from(block_jump_factor) >= 1.0
}

/// `Vec3.directionFromRotation(rotX, rotY)`.
pub fn direction_from_rotation(rot_x: f32, rot_y: f32) -> Vec3 {
    let rad = trig::DEG_TO_RAD;
    let pi = std::f32::consts::PI;
    let y_cos = trig::cos(f64::from(-rot_y * rad - pi));
    let y_sin = trig::sin(f64::from(-rot_y * rad - pi));
    let x_cos = -trig::cos(f64::from(-rot_x * rad));
    let x_sin = trig::sin(f64::from(-rot_x * rad));
    Vec3::new(
        f64::from(y_sin * x_cos),
        f64::from(x_sin),
        f64::from(y_cos * x_cos),
    )
}

/// `AABB.intersects(Vec3, Vec3)`: the box overlaps the segment's bounds.
fn intersects_segment(b: Aabb, from: Vec3, to: Vec3) -> bool {
    b.intersects(Aabb::new(from, to))
}

/// `LocalPlayer.updateAutoJump(xa, za)` once `canAutoJump` holds: whether
/// it sets `autoJumpTime = 1`. `xa`/`za` are the float horizontal
/// displacement of the move.
pub fn update_auto_jump<W: AutoJumpWorld>(
    world: &mut W,
    p: &AutoJumper,
    xa: f32,
    za: f32,
) -> Result<bool, W::Error> {
    let move_begin = p.position;
    let move_end = move_begin.add_vector(Vec3::new(f64::from(xa), 0.0, f64::from(za)));
    let mut move_diff = Vec3::new(f64::from(xa), 0.0, f64::from(za));
    let current_speed = p.speed;
    let mut move_dist_sq = move_diff.length_squared() as f32;
    if move_dist_sq <= 0.001 {
        let input_xa = current_speed * p.move_vector[0];
        let input_za = current_speed * p.move_vector[1];
        let sin = trig::sin(f64::from(p.y_rot * trig::DEG_TO_RAD));
        let cos = trig::cos(f64::from(p.y_rot * trig::DEG_TO_RAD));
        move_diff = Vec3::new(
            f64::from(input_xa * cos - input_za * sin),
            move_diff.y,
            f64::from(input_za * cos + input_xa * sin),
        );
        move_dist_sq = move_diff.length_squared() as f32;
        if move_dist_sq <= 0.001 {
            return Ok(false);
        }
    }
    // Mth.invSqrt: 1.0F / (float) Math.sqrt(x).
    let inverted = 1.0 / move_dist_sq.sqrt();
    let move_dir = move_diff.scale(f64::from(inverted));
    let facing = direction_from_rotation(p.x_rot, p.y_rot);
    let facing_vs_moving = (facing.x * move_dir.x + facing.z * move_dir.z) as f32;
    if facing_vs_moving < -0.15 {
        return Ok(false);
    }
    let mut ceiling = BlockPos::containing(Vec3::new(move_begin.x, p.max_y, move_begin.z));
    if !world.collision_shape(ceiling)?.is_empty() {
        return Ok(false);
    }
    ceiling = ceiling.offset(0, 1, 0);
    if !world.collision_shape(ceiling)?.is_empty() {
        return Ok(false);
    }
    let mut jump_height = 1.2f32;
    if let Some(amplifier) = p.jump_boost {
        jump_height += (amplifier + 1) as f32 * 0.75;
    }
    let look_ahead = crate::camera::java_max(current_speed * 7.0, 1.0 / inverted);
    let seg_begin = move_begin;
    let seg_end = move_end.add_vector(move_dir.scale(f64::from(look_ahead)));
    let (width, height) = (p.width, p.height);
    let test_box = Aabb::new(
        seg_begin,
        seg_end.add_vector(Vec3::new(0.0, f64::from(height), 0.0)),
    )
    .inflate(Vec3::new(f64::from(width), 0.0, f64::from(width)));
    let lift = Vec3::new(0.0, f64::from(0.51f32), 0.0);
    let seg_begin = seg_begin.add_vector(lift);
    let seg_end = seg_end.add_vector(lift);
    let right_dir = move_dir.cross(Vec3::new(0.0, 1.0, 0.0));
    let right_offset = right_dir.scale(f64::from(width * 0.5));
    let left_begin = seg_begin.subtract(right_offset);
    let left_end = seg_end.subtract(right_offset);
    let right_begin = seg_begin.add_vector(right_offset);
    let right_end = seg_end.add_vector(right_offset);
    let mut obstacle = NO_OBSTACLE;
    for b in world.collision_boxes(test_box)? {
        if intersects_segment(b, left_begin, left_end)
            || intersects_segment(b, right_begin, right_end)
        {
            obstacle = b.max_position().y as f32;
            let obstacle_pos = BlockPos::containing(b.center());
            let mut steps = 1i32;
            while (steps as f32) < jump_height {
                let above = obstacle_pos.offset(0, steps, 0);
                let shape = world.collision_shape(above)?;
                if !shape.is_empty() {
                    obstacle = shape.max(Axis::Y) as f32 + above.y as f32;
                    if f64::from(obstacle) - p.position.y > f64::from(jump_height) {
                        return Ok(false);
                    }
                }
                if steps > 1 {
                    ceiling = ceiling.offset(0, 1, 0);
                    if !world.collision_shape(ceiling)?.is_empty() {
                        return Ok(false);
                    }
                }
                steps += 1;
            }
            break;
        }
    }
    if obstacle != NO_OBSTACLE {
        let y_delta = (f64::from(obstacle) - p.position.y) as f32;
        // Negated as in LocalPlayer.updateAutoJump, so a NaN delta still jumps.
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let jumps = !(y_delta <= 0.5) && !(y_delta > jump_height);
        if jumps {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Step;
    impl AutoJumpWorld for Step {
        type Error = ();
        fn collision_shape(&mut self, p: BlockPos) -> Result<VoxelShape, ()> {
            Ok(if p.x == 1 && p.y == 0 {
                VoxelShape::block()
            } else {
                VoxelShape::empty()
            })
        }
        fn collision_boxes(&mut self, bounds: Aabb) -> Result<Vec<Aabb>, ()> {
            let b = Aabb::new(Vec3::new(1.0, 0.0, -5.0), Vec3::new(2.0, 1.0, 5.0));
            Ok(if b.intersects(bounds) {
                vec![b]
            } else {
                Vec::new()
            })
        }
    }
    #[test]
    fn walking_into_a_one_block_step_jumps() {
        let p = AutoJumper {
            position: Vec3::new(0.6, 0.0, 0.5),
            max_y: 1.8,
            x_rot: 0.0,
            y_rot: -90.0,
            speed: 0.1,
            move_vector: [0.0, 1.0],
            width: 0.6,
            height: 1.8,
            jump_boost: None,
        };
        assert!(update_auto_jump(&mut Step, &p, 0.1, 0.0).unwrap());
        // Moving away from the step does not.
        assert!(!update_auto_jump(&mut Step, &p, -0.1, 0.0).unwrap());
    }
    #[test]
    fn sneaking_or_standing_still_cannot_auto_jump() {
        assert!(can_auto_jump(true, 0, true, false, false, [0.0, 1.0], 1.0));
        assert!(!can_auto_jump(true, 0, true, true, false, [0.0, 1.0], 1.0));
        assert!(!can_auto_jump(true, 0, true, false, false, [0.0, 0.0], 1.0));
        assert!(!can_auto_jump(true, 0, true, false, false, [0.0, 1.0], 0.5));
    }
}
