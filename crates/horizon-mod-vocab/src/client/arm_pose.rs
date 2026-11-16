//! `HumanoidModel.ArmPose`: how a humanoid model poses an arm, and the names an item declaration gives them.

/// `HumanoidModel.ArmPose`'s constants, the arm poses a humanoid model has.
#[derive(
    horizon_mod_api_macros::Wire,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Debug,
    serde::Deserialize,
    serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
// exhaustive: vanilla constants and a registered custom pose
pub enum ArmPose {
    /// No held item pose.
    Empty,
    /// Ordinary held item.
    Item,
    /// Blocking with a shield.
    Block,
    /// Drawing a bow.
    #[serde(rename = "bow_and_arrow")]
    Bow,
    /// Holding a loaded crossbow.
    CrossbowHold,
    /// Charging a crossbow.
    CrossbowCharge,
    /// Raising a trident to throw it (or to launch with Riptide).
    ThrowTrident,
    /// Playing a goat horn.
    TootHorn,
    /// Looking through a spyglass (`HumanoidModel.ArmPose.SPYGLASS`).
    Spyglass,
    /// A spear held out (`HumanoidModel.ArmPose.SPEAR`), at rest or charging.
    Spear,
    /// A brush being used (`HumanoidModel.ArmPose.BRUSH`).
    Brush,
    /// A pose registered once through `ArmPosesRegistrar`.
    Custom(CustomArmPose),
}

impl ArmPose {
    /// Both hands hold the item (`BOW_AND_ARROW`'s and the crossbow's two).
    pub fn two_handed(self) -> bool {
        matches!(self, Self::Bow | Self::CrossbowHold | Self::CrossbowCharge)
            || matches!(self, Self::Custom(pose) if pose.both)
    }

    /// The other hand's pose gives way to this one.
    pub fn affects_offhand(self) -> bool {
        match self {
            Self::Custom(pose) => pose.offhand,
            _ => self.two_handed() || self == Self::ThrowTrident,
        }
    }
}

/// A load-owned custom pose slot. Invalid or stale slots are ignored by rendering.
// exhaustive: private fields; only the registrar constructs tokens
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    horizon_mod_api_macros::Wire,
)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct CustomArmPose {
    pub(crate) slot: u32,
    pub(crate) both: bool,
    pub(crate) offhand: bool,
}

pub(crate) struct PoseInfo {
    pub(crate) id: String,
    pub(crate) both: bool,
    pub(crate) offhand: bool,
}

impl CustomArmPose {
    /// Resolve a slot against the current load's registrations.
    // hidden: the game's renderer resolves the load-owned slot; not a mod-facing call
    #[doc(hidden)]
    pub fn from_slot(slot: u32) -> Option<Self> {
        table().get(slot as usize).map(|pose| Self {
            slot,
            both: pose.both,
            offhand: pose.offhand,
        })
    }
    /// Resolved flags for the renderer; absent for a refused or unknown registration.
    // hidden: the game's renderer reads the resolved flags; not a mod-facing call
    #[doc(hidden)]
    pub fn flags(self) -> Option<(bool, bool)> {
        let table = table();
        table
            .get(self.slot as usize)
            .map(|pose| (pose.both, pose.offhand))
    }
    /// Load-owned slot for the renderer.
    // hidden: the game's renderer reads the load-owned slot; not a mod-facing call
    #[doc(hidden)]
    pub fn slot(self) -> u32 {
        self.slot
    }
}

/// Absolute model arm rotations in radians. Each non-finite component leaves
/// that component of the ordinary humanoid pose unchanged.
// exhaustive: mods write either arm's three model rotation components
#[derive(Clone, Copy, Debug, PartialEq, horizon_mod_api_macros::Wire)]
pub struct ArmAngles {
    /// Right arm pitch, yaw and roll.
    pub right: [f32; 3],
    /// Left arm pitch, yaw and roll.
    pub left: [f32; 3],
}

fn table() -> &'static [PoseInfo] {
    #[cfg(any(test, feature = "testing"))]
    if let Some(table) = TEST_POSES.get() {
        return table;
    }
    crate::parts::installed().map_or(&[], |installed| installed.arm_poses.as_slice())
}

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static TEST_POSES: std::cell::Cell<Option<&'static [PoseInfo]>> = const { std::cell::Cell::new(None) };
}
#[cfg(any(test, feature = "testing"))]
pub(crate) struct RestorePoses(Option<&'static [PoseInfo]>);
#[cfg(any(test, feature = "testing"))]
impl Drop for RestorePoses {
    fn drop(&mut self) {
        TEST_POSES.set(self.0);
    }
}
#[cfg(any(test, feature = "testing"))]
pub(crate) fn test_table(poses: Vec<PoseInfo>) -> RestorePoses {
    RestorePoses(TEST_POSES.replace(Some(Box::leak(poses.into_boxed_slice()))))
}
