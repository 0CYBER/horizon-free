//! `LocalPlayer.updateAutoJump` behind `canAutoJump`, in its own module so the
//! move paths around it stay readable.
use super::*;
use crate::auto_jump::AutoJumper;
impl MotionWorld<'_, '_, '_> {
    pub fn update_auto_jump(&mut self, xa: f32, za: f32) -> Result<()> {
        let p = &*self.player;
        // canAutoJump; MotionWorld drives an unmounted player (not a passenger).
        let ready = crate::auto_jump::can_auto_jump(
            p.auto_jump_enabled,
            p.auto_jump_time,
            p.ground.on_ground,
            p.shift_key_down,
            false,
            p.move_vector,
            1.0,
        );
        if !ready || f64::from(self.block_jump_factor()?) < 1.0 {
            return Ok(());
        }
        // Player.getSpeed: the movement speed attribute as it is now.
        let speed = self.player.attribute("minecraft:movement_speed")? as f32;
        let p = &*self.player;
        let jumper = AutoJumper {
            position: p.position,
            max_y: p.bounds().max_position().y,
            x_rot: p.pitch,
            y_rot: p.yaw,
            speed,
            move_vector: p.move_vector,
            width: p.dimensions.width,
            height: p.dimensions.height,
            jump_boost: p.status.jump_boost,
        };
        if crate::auto_jump::update_auto_jump(self, &jumper, xa, za)? {
            self.player.auto_jump_time = 1;
        }
        Ok(())
    }
}
