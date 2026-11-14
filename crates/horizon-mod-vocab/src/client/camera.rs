//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// `Mth.lerp(float, float, float)`.
pub fn lerp(alpha: f32, p0: f32, p1: f32) -> f32 {
    p0 + alpha * (p1 - p0)
}
/// `Mth.wrapDegrees(float)`.
pub fn wrap_degrees(angle: f32) -> f32 {
    let mut normalized = angle % 360.0;
    if normalized >= 180.0 {
        normalized -= 360.0;
    }
    if normalized < -180.0 {
        normalized += 360.0;
    }
    normalized
}
/// What `AbstractClientPlayer.getFieldOfViewModifier` reads.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct FovPlayer {
    /// `Abilities.flying`.
    pub flying: bool,
    /// `Abilities.getWalkingSpeed()`.
    pub walking_speed: f32,
    /// `getAttributeValue(MOVEMENT_SPEED)`.
    pub movement_speed: f64,
    /// The item in use and its use ticks, when using one.
    pub using: Option<(UseItem, i32)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum UseItem {
    Bow,
    Spyglass,
    Other,
}
/// Why `ClientHost::set_local_rotation` changed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum LocalRotationError {
    /// No client world is open.
    NoWorld,
    /// The yaw or the pitch is NaN or infinite.
    NotFinite,
}
/// `CameraType`: where the camera sits for the local player. `key.togglePerspective` (F5) cycles
/// it first person, third person behind, third person in front.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum CameraType {
    #[default]
    FirstPerson,
    ThirdPersonBack,
    ThirdPersonFront,
}
impl CameraType {
    /// `CameraType.isFirstPerson`.
    pub fn is_first_person(self) -> bool {
        self == Self::FirstPerson
    }
    /// `CameraType.isMirrored`: third person in front.
    pub fn is_mirrored(self) -> bool {
        self == Self::ThirdPersonFront
    }
    /// `CameraType.cycle` (F5).
    pub fn cycle(self) -> Self {
        match self {
            Self::FirstPerson => Self::ThirdPersonBack,
            Self::ThirdPersonBack => Self::ThirdPersonFront,
            Self::ThirdPersonFront => Self::FirstPerson,
        }
    }
}
/// The camera entity as `Camera.alignWithEntity` and the field-of-view
/// death and hurt effects see it.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct CameraEntity {
    pub position: [f64; 3],
    /// `xo`, `yo`, `zo`.
    pub previous: [f64; 3],
    pub x_rot: f32,
    pub y_rot: f32,
    pub x_rot_o: f32,
    pub y_rot_o: f32,
    /// `Entity.getEyeHeight()`.
    pub eye_height: f32,
    /// A living entity's `getScale()` and camera distance attribute.
    pub living: Option<LivingCamera>,
    /// The vehicle's scale and camera distance when it is living.
    pub mount: Option<(f32, f64)>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct LivingCamera {
    pub scale: f32,
    pub camera_distance: f64,
    /// `isSleeping()` with `getBedOrientation()` as its `toYRot()`.
    pub sleeping: Option<Option<f32>>,
    /// `isDeadOrDying()` and `deathTime`.
    pub dead: Option<i32>,
}
impl CameraEntity {
    /// `Entity.getXRot(partialTicks)`.
    pub fn view_x_rot(&self, partial: f32) -> f32 {
        if partial == 1.0 {
            self.x_rot
        } else {
            lerp(partial, self.x_rot_o, self.x_rot)
        }
    }
    /// `LocalPlayer.getViewYRot` (`getYRot(partialTicks)`, a `Mth.rotLerp`).
    pub fn view_y_rot(&self, partial: f32) -> f32 {
        if partial == 1.0 {
            self.y_rot
        } else {
            self.y_rot_o + partial * wrap_degrees(self.y_rot - self.y_rot_o)
        }
    }
}
