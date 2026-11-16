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
    Sleeping,
}
impl Pose {
    /// `Pose` by its network id (`Pose.id`); the poses a player never takes
    /// are standing. One id table: `mod_api::Pose`'s, narrowed to the avatar's
    /// six stances.
    pub fn from_id(id: i32) -> Self {
        mod_api::Pose::from_id(id).into()
    }
    /// Network ID for this pose (`Pose.id`).
    pub fn id(self) -> i32 {
        match self {
            Self::Standing => 0,
            Self::FallFlying => 1,
            Self::Sleeping => 2,
            Self::Swimming => 3,
            Self::SpinAttack => 4,
            Self::Crouching => 5,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Standing => "STANDING",
            Self::Crouching => "CROUCHING",
            Self::Swimming => "SWIMMING",
            Self::FallFlying => "FALL_FLYING",
            Self::SpinAttack => "SPIN_ATTACK",
            Self::Sleeping => "SLEEPING",
        }
    }
    /// `Avatar.POSES` (and `LivingEntity.SLEEPING_DIMENSIONS`): the box of a
    /// player or mannequin in this pose; sleeping ignores the scale.
    pub fn dimensions(self, scale: f32) -> Dimensions {
        let (width, height, eye_height) = match self {
            Self::Standing => (0.6 * scale, 1.8 * scale, 1.62 * scale),
            Self::Crouching => (0.6 * scale, 1.5 * scale, 1.27 * scale),
            Self::Swimming | Self::FallFlying | Self::SpinAttack => {
                (0.6 * scale, 0.6 * scale, 0.4 * scale)
            }
            Self::Sleeping => (0.2, 0.2, 0.2),
        };
        Dimensions {
            width,
            height,
            eye_height,
        }
    }
}
impl From<Pose> for mod_api::Pose {
    /// The living entity's pose (`Entity.getPose`) the avatar's stance names.
    fn from(pose: Pose) -> Self {
        match pose {
            Pose::Standing => Self::Standing,
            Pose::FallFlying => Self::FallFlying,
            Pose::Sleeping => Self::Sleeping,
            Pose::Swimming => Self::Swimming,
            Pose::SpinAttack => Self::SpinAttack,
            Pose::Crouching => Self::Crouching,
        }
    }
}
impl From<mod_api::Pose> for Pose {
    /// Narrow a living entity's pose to a movement stance. Dimension queries
    /// use `Dimensions::avatar` to preserve Avatar.POSES's fixed dying box.
    fn from(pose: mod_api::Pose) -> Self {
        match pose {
            mod_api::Pose::FallFlying => Self::FallFlying,
            mod_api::Pose::Sleeping => Self::Sleeping,
            mod_api::Pose::Swimming => Self::Swimming,
            mod_api::Pose::SpinAttack => Self::SpinAttack,
            mod_api::Pose::Crouching => Self::Crouching,
            _ => Self::Standing,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Dimensions {
    pub width: f32,
    pub height: f32,
    pub eye_height: f32,
}
/// What `entity_size` (`EntityEvent.Size`) answered for an entity whose size vanilla
/// recalculated (`Entity.refreshDimensions`) to `vanilla` (width, height, eye height) in
/// `pose`: `size` replaces it while both still hold. Any change to either goes back to
/// vanilla's until the hook is asked again; `size` is `None` when the listeners left it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeAnswer {
    pub pose: mod_api::Pose,
    pub vanilla: [f32; 3],
    pub size: Option<[f32; 3]>,
}
impl SizeAnswer {
    /// Whether this answer is for `pose` and vanilla's `vanilla`.
    pub fn asked(&self, pose: mod_api::Pose, vanilla: [f32; 3]) -> bool {
        self.pose == pose && self.vanilla == vanilla
    }
    /// The size to use instead of `vanilla` in `pose`, if this answer still holds.
    pub fn holds(&self, pose: mod_api::Pose, vanilla: [f32; 3]) -> Option<[f32; 3]> {
        self.size.filter(|_| self.asked(pose, vanilla))
    }
    /// The size the entity had with this answer: the listeners', else vanilla's.
    pub fn effective(&self) -> [f32; 3] {
        self.size.unwrap_or(self.vanilla)
    }
}
impl Dimensions {
    pub fn to_array(self) -> [f32; 3] {
        [self.width, self.height, self.eye_height]
    }
    pub fn from_array([width, height, eye_height]: [f32; 3]) -> Self {
        Self {
            width,
            height,
            eye_height,
        }
    }
    /// `Avatar.getDefaultDimensions` with the living scale applied, including
    /// the fixed DYING entry that is not a movement stance.
    pub fn avatar(pose: mod_api::Pose, scale: f32) -> Self {
        if pose == mod_api::Pose::Dying {
            Self {
                width: 0.2,
                height: 0.2,
                eye_height: 1.62,
            }
        } else {
            Pose::from(pose).dimensions(scale)
        }
    }

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
    /// `LivingEntity.aiStep` counting down `autoSpinAttackTicks` for a
    /// server's copy of a player, which runs no client tick: the spin ends
    /// after its ticks and its damage goes with it, so the pose (and the eye
    /// a bow or a trident is shot from) comes back to standing.
    pub fn count_down_server_spin(&mut self) {
        if self.auto_spin_attack_ticks > 0 {
            self.auto_spin_attack_ticks -= 1;
            if self.auto_spin_attack_ticks <= 0 {
                self.auto_spin_attack_dmg = 0.0;
            }
        }
    }
    pub fn dimensions_for_pose(&mut self, pose: Pose) -> Result<Dimensions> {
        Ok(pose.dimensions(self.attribute("minecraft:scale")? as f32))
    }
    pub fn refresh_dimensions(&mut self) -> Result<()> {
        self.scale = self.attribute("minecraft:scale")? as f32;
        self.dimensions = self.pose.dimensions(self.scale);
        // `EntityEvent.Size`: an `entity_size` answer for this pose and size still holds.
        if let Some(size) = self
            .size_answer
            .and_then(|answer| answer.holds(self.pose.into(), self.dimensions.to_array()))
        {
            self.dimensions = crate::pose::Dimensions::from_array(size);
        }
        Ok(())
    }
    pub fn set_pose(&mut self, pose: Pose) -> Result<()> {
        if self.pose != pose {
            self.pose = pose;
            self.refresh_dimensions()?;
        }
        Ok(())
    }
    /// `LivingEntity.startSleeping` and `stopSleeping` set the pose at once
    /// (the 0.2 box in the bed, standing again after).
    pub fn set_sleeping(&mut self, sleeping: bool) -> Result<()> {
        if self.sleeping != sleeping {
            self.sleeping = sleeping;
            self.set_pose(if sleeping {
                Pose::Sleeping
            } else {
                Pose::Standing
            })?;
        }
        Ok(())
    }
    /// `LocalPlayer.isMovingSlowly`: crouching, or `Entity.isVisuallyCrawling`
    /// (visually swimming out of water). `LivingEntity.isVisuallySwimming`
    /// counts the swimming pose and the fall-flying pose a glide that just
    /// ended still holds, so the tick after an elytra lands moves at sneaking
    /// speed (#8295).
    pub fn moving_slowly(&self) -> bool {
        let visually_swimming =
            self.pose == Pose::Swimming || (!self.fall_flying && self.pose == Pose::FallFlying);
        self.local_crouching || (visually_swimming && !self.fluids.was_touching_water)
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
        // Player.getDesiredPose: sleeping, swimming, gliding, Riptide's spin.
        let desired = if self.player.sleeping {
            Pose::Sleeping
        } else if self.player.swimming {
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
    /// The end of a server's copy of a player's tick (`ServerPlayer.doTick`
    /// runs `Player.tick`): `LivingEntity.aiStep` counts a Riptide spin down,
    /// `LivingEntity.tick` then runs `refreshDirtyAttributes` (a changed
    /// `minecraft:scale` rebuilds the box, `onAttributeUpdated`), and
    /// `Player.tick` ends with `updatePlayerPose`. The server checks the
    /// player's moves with that box (#3684).
    pub fn server_player_pose_tick(&mut self) -> Result<()> {
        self.player.count_down_server_spin();
        self.player.refresh_attributes()?;
        self.update_player_pose()
    }
    /// The pose part of `Player.tick` for a passenger (`Entity.rideTick` runs
    /// `tick`, not the travel): `Entity.updateSwimming` stops a swim, since it
    /// needs `!isPassenger()`, and `updatePlayerPose` follows. A player who
    /// boards a boat while swimming sits up instead of lying under it (#3308).
    /// The server's `LivingEntity.updateFallFlying` ends a glide the same way
    /// (`canGlide` needs `!isPassenger()`) and `LivingEntity.tick` resets
    /// `fallFlyTicks`: a player who boards while gliding sits up instead of
    /// keeping the glide pose (#3093).
    /// `Entity.baseTick` still samples the fluids first, in the box the vehicle
    /// allows (the boat the game stores in the player's fluid state), so a boat
    /// afloat keeps the eyes dry.
    pub fn passenger_pose_tick(&mut self) -> Result<()> {
        self.player.fluids.was_eye_in_water = self.player.is_eye_in_fluid(crate::motion::WATER);
        self.update_fluids()?;
        // Player.tick: a passenger is never on the ground (setOnGround(false)
        // clears the supporting block too), so its look packets say so.
        self.player.ground.on_ground = false;
        self.player.ground.on_ground_no_blocks = false;
        self.player.ground.supporting_block = None;
        self.player.swimming = false;
        self.player.fall_flying = false;
        self.player.fall_fly_ticks = 0;
        self.update_player_pose()?;
        // A cushion's seat puts the feet in the support block, so neither
        // standing nor crawling fits and `updatePlayerPose` keeps the old
        // pose. A prior swim would stick (#4509); sit up like a boat rider.
        if self.player.pose == Pose::Swimming && !self.player.sleeping {
            self.player.set_pose(Pose::Standing)?;
        }
        Ok(())
    }
}
