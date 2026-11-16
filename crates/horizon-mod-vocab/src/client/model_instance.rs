//! Instanced copies of one resolved model, for a single world view.
//!
//! `ModelInstance::tint` is `0xAARRGGBB`, the same packed colour as [`crate::WorldQuad::color`].
//! [`LightCoords`] is block and sky light, each `0..=15`. The engine converts tint and light
//! to the frame format. Mods never assume that format.

/// Block and sky light, each `0..=15`, in vanilla's convention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct LightCoords {
    /// Block light, `0..=15`.
    pub block: u8,
    /// Sky light, `0..=15`.
    pub sky: u8,
}

impl LightCoords {
    /// Both channels at 15.
    pub const FULL_BRIGHT: Self = Self { block: 15, sky: 15 };

    /// `block` and `sky` each in `0..=15`, or `None`.
    pub fn new(block: u8, sky: u8) -> Option<Self> {
        (block <= 15 && sky <= 15).then_some(Self { block, sky })
    }
}

/// White-flash column and the hurt tint. Hurt hides the white flash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct Overlay {
    /// White flash column, `0..=15`. Zero is no flash.
    pub white: u8,
    /// The hurt red.
    pub hurt: bool,
}

impl Overlay {
    /// No white flash and no hurt tint.
    pub const NONE: Self = Self {
        white: 0,
        hurt: false,
    };

    /// `white` in `0..=15`, or `None`.
    pub fn new(white: u8, hurt: bool) -> Option<Self> {
        (white <= 15).then_some(Self { white, hurt })
    }
}

/// Row-major 3×4 identity, local to a copy's origin.
pub const IDENTITY_TRANSFORM: [f32; 12] =
    [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// One copy. `origin` is world blocks; `transform` is row-major 3×4 in that local space.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ModelInstance {
    /// World-space origin, in blocks. The camera is subtracted in `f64`.
    pub origin: [f64; 3],
    /// Row-major 3×4 affine transform, applied before `origin`.
    pub transform: [f32; 12],
    /// Packed `0xAARRGGBB`, the same convention as [`crate::WorldQuad::color`].
    pub tint: u32,
    /// Block and sky light in vanilla's `0..=15` convention.
    pub light: LightCoords,
    /// Hurt tint and white flash, in vanilla's overlay convention.
    pub overlay: Overlay,
}

impl ModelInstance {
    /// One copy. The draw call checks finiteness, light range and the view budget.
    #[inline]
    pub fn new(
        origin: [f64; 3],
        transform: [f32; 12],
        tint: u32,
        light: LightCoords,
        overlay: Overlay,
    ) -> Self {
        Self {
            origin,
            transform,
            tint,
            light,
            overlay,
        }
    }

    /// Identity transform at `origin`, full bright, with no overlay.
    #[inline]
    pub fn at(origin: [f64; 3], tint: u32) -> Self {
        Self::new(
            origin,
            IDENTITY_TRANSFORM,
            tint,
            LightCoords::FULL_BRIGHT,
            Overlay::NONE,
        )
    }
}

/// Why a whole instance batch was refused. Nothing from that batch is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum InstanceError {
    /// The handle is stale or not a resolved standalone model.
    Model(crate::ModelError),
    /// An origin or transform component is not finite.
    NonFinite,
    /// A light channel is outside `0..=15`.
    InvalidLight,
    /// The batch would exceed this view's 16,384 instance budget.
    CapacityExceeded,
    /// The model has a translucent face and is not drawn as solid.
    Translucent,
}

/// Persistent copies of one resolved model, retired on reload, world close, mod disable or destroy.
/// Only the creating mod may access or submit the group; others receive [`GroupError::Stale`].
/// Retired handles never become valid again, even when their storage slot is reused.
/// Native and wasm mods use the same storage, ownership and drawing rules.
/// Static groups cache CPU conversion; all visible GPU-routed copies still upload every frame.
/// Frequent group pose changes rebuild the live cache and can cost more than transient copies.
// no wire: decoding from_parts rejects generation zero (an unpublished instance group handle).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the engine creates handles
pub struct InstanceGroupRef(pub(crate) u64, pub(crate) u32);

impl InstanceGroupRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished instance group generation");
        }
        Ok(Self(generation, slot))
    }
    /// Atomically overwrite a range, extending length up to reserved capacity.
    /// Newly exposed unwritten slots are identity, white, full-bright copies at world origin.
    /// Invalid coordinates/light or an out-of-capacity range change nothing; a retired or
    /// foreign handle returns [`GroupError::Stale`] before inspecting the range or values.
    /// That range and any newly exposed gap are converted again. Visible copies still
    /// enter the shared per-view GPU upload on every frame.
    pub fn write(
        self,
        host: &dyn crate::ClientHost,
        first: u32,
        instances: &[ModelInstance],
    ) -> Result<(), GroupError> {
        host.instance_group_write(self, first, instances)
    }
    /// Atomically change the drawn length. Unwritten slots contain identity, full-bright copies.
    /// Zero draws nothing and does not release capacity. Shrinking preserves hidden copies;
    /// growing exposes them again. A length above capacity returns [`GroupError::OutOfRange`].
    /// Growth converts the newly exposed range using the current group pose.
    pub fn set_len(self, host: &dyn crate::ClientHost, len: u32) -> Result<(), GroupError> {
        host.instance_group_set_len(self, len)
    }
    /// Atomically replace the row-major 3x4 group pose (identity by default).
    /// World position = origin + group_transform * (copy_transform * local).
    /// The origin stays in world space; transformed bounds are used for culling.
    /// Non-finite components return [`GroupError::NonFinite`]. Rebuilds the drawn
    /// copies' cached render records; it does not allocate or rewrite the API copies.
    pub fn set_group_transform(
        self,
        host: &dyn crate::ClientHost,
        transform: [f32; 12],
    ) -> Result<(), GroupError> {
        host.instance_group_set_transform(self, transform)
    }
    /// Retire this handle and release its reserved copies and CPU caches immediately.
    /// Already accepted copies remain in the current view. Later operations, including another
    /// destroy, return [`GroupError::Stale`]; a new group cannot revive this handle.
    pub fn destroy(self, host: &dyn crate::ClientHost) -> Result<(), GroupError> {
        host.instance_group_destroy(self)
    }
}

/// Why a persistent group operation was refused. Errors leave storage and view unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum GroupError {
    /// The group belongs to another mod, was destroyed, its owner was disabled,
    /// or its world/resources retired. Re-enabling or slot reuse cannot revive it.
    Stale,
    /// The model could not be resolved.
    Model(crate::ModelError),
    /// An origin or transform component is not finite.
    NonFinite,
    /// A light channel is outside 0..=15.
    InvalidLight,
    /// The range exceeds reserved capacity.
    OutOfRange,
    /// The mod's reserved capacity or the view budget would be exceeded.
    CapacityExceeded,
    /// Translucent models cannot use this route.
    Translucent,
}

impl From<InstanceError> for GroupError {
    fn from(error: InstanceError) -> Self {
        match error {
            InstanceError::Model(error) => Self::Model(error),
            InstanceError::NonFinite => Self::NonFinite,
            InstanceError::InvalidLight => Self::InvalidLight,
            InstanceError::CapacityExceeded => Self::CapacityExceeded,
            InstanceError::Translucent => Self::Translucent,
        }
    }
}

impl crate::__private::game::Fields for InstanceGroupRef {
    type Fields<'a> = crate::__private::game::ModelRefFields;
    fn fields(&self) -> Self::Fields<'_> {
        crate::__private::game::ModelRefFields {
            generation: self.0,
            slot: self.1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_and_overlay_constructors_reject_a_channel_past_15() {
        assert!(LightCoords::new(16, 0).is_none());
        assert!(Overlay::new(16, false).is_none());
        assert_eq!(
            ModelInstance::at([1.0, 2.0, 3.0], 0xFF80_C040).tint,
            0xFF80_C040
        );
    }
}
