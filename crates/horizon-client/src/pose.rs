//! Resident player pose, native dimension arithmetic and dry pose-fit selection.
//! Full synchronized entity schemas, sleeping and fluid/flight lifecycles remain
//! separate from these stored movement fields.
use crate::{
    Result,
    motion::{MotionWorld, PlayerMotion},
};
use horizon_core::geometry::{Aabb, Vec3};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pose {
    #[default]
    Standing,
    Crouching,
    Swimming,
    FallFlying,
    SpinAttack,
}
impl Pose {
    pub fn name(self) -> &'static str {
        match self {
            Self::Standing => "STANDING",
            Self::Crouching => "CROUCHING",
            Self::Swimming => "SWIMMING",
            Self::FallFlying => "FALL_FLYING",
            Self::SpinAttack => "SPIN_ATTACK",
        }
    }
    pub fn dimensions(self, scale: f32) -> Dimensions {
        let (height, eye_height) = match self {
            Self::Standing => (1.8, 1.62),
            Self::Crouching => (1.5, 1.27),
            Self::Swimming | Self::FallFlying | Self::SpinAttack => (0.6, 0.4),
        };
        Dimensions {
            width: 0.6 * scale,
            height: height * scale,
            eye_height: eye_height * scale,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Dimensions {
    pub width: f32,
    pub height: f32,
    pub eye_height: f32,
}
impl Dimensions {
    pub fn bounds(self, position: Vec3) -> Aabb {
        let half = f64::from(self.width / 2.0);
        Aabb::new(
            Vec3::new(position.x - half, position.y, position.z - half),
            Vec3::new(
                position.x + half,
                position.y + f64::from(self.height),
                position.z + half,
            ),
        )
    }
}
impl PlayerMotion {
    pub fn dimensions_for_pose(&mut self, pose: Pose) -> Result<Dimensions> {
        Ok(pose.dimensions(self.attribute("minecraft:scale")? as f32))
    }
    pub fn refresh_dimensions(&mut self) -> Result<()> {
        self.dimensions = self.dimensions_for_pose(self.pose)?;
        Ok(())
    }
    pub fn set_pose(&mut self, pose: Pose) -> Result<()> {
        if self.pose != pose {
            self.pose = pose;
            self.refresh_dimensions()?;
        }
        Ok(())
    }
    pub fn moving_slowly(&self) -> bool {
        self.local_crouching || (self.pose == Pose::Swimming && !self.fluids.was_touching_water)
    }
}
impl MotionWorld<'_, '_, '_> {
    pub fn can_fit_pose(&mut self, pose: Pose) -> Result<bool> {
        let bounds = self
            .player
            .dimensions_for_pose(pose)?
            .bounds(self.player.position);
        self.no_player_collision(bounds.deflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7)))
    }
    /// LocalPlayer.aiStep computes crouching BEFORE KeyboardInput.tick, using
    /// the old shift input. This is distinct from Entity's current pose.
    pub fn update_local_crouching(&mut self) -> Result<()> {
        self.player.local_crouching = !self.player.flying
            && !self.player.swimming
            && self.can_fit_pose(Pose::Crouching)?
            && (self.player.shift_key_down || !self.can_fit_pose(Pose::Standing)?);
        Ok(())
    }
    /// Dry Player.updatePlayerPose after movement: crawl when neither standing
    /// nor crouching fits; leave the current pose if even crawling cannot fit.
    pub fn update_player_pose(&mut self) -> Result<()> {
        if !self.can_fit_pose(Pose::Swimming)? {
            return Ok(());
        }
        // Player.getDesiredPose: swimming, gliding, Riptide's spin.
        let desired = if self.player.swimming {
            Pose::Swimming
        } else if self.player.fall_flying {
            Pose::FallFlying
        } else if self.player.auto_spin_attack_ticks > 0 {
            Pose::SpinAttack
        } else if self.player.shift_key_down && !self.player.flying {
            Pose::Crouching
        } else {
            Pose::Standing
        };
        let pose = if self.player.spectator || self.can_fit_pose(desired)? {
            desired
        } else if self.can_fit_pose(Pose::Crouching)? {
            Pose::Crouching
        } else {
            Pose::Swimming
        };
        self.player.set_pose(pose)
    }
}
