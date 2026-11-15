//! `HumanoidModel.ArmPose`: how a humanoid model poses an arm, and the names an item declaration gives them.

/// `HumanoidModel.ArmPose`'s constants, the arm poses a humanoid model has.
// exhaustive: vanilla constants and a resolved client registration
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArmPose {
    Empty,
    Item,
    Block,
    #[serde(rename = "bow_and_arrow")]
    Bow,
    CrossbowHold,
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
    /// A client registration resolved before animation. Rotations are IEEE bits;
    /// non-finite components preserve the base model angle.
    #[serde(skip)]
    Custom {
        slot: u32,
        both: bool,
        offhand: bool,
        angles: [[u32; 3]; 2],
    },
}

impl ArmPose {
    /// Both hands hold the item (`BOW_AND_ARROW`'s and the crossbow's two).
    pub fn two_handed(self) -> bool {
        matches!(
            self,
            Self::Bow | Self::CrossbowHold | Self::CrossbowCharge | Self::Custom { both: true, .. }
        )
    }

    /// The other hand's pose gives way to this one.
    pub fn affects_offhand(self) -> bool {
        match self {
            Self::Custom { offhand, .. } => offhand,
            _ => self.two_handed() || self == Self::ThrowTrident,
        }
    }
}

/// Physical arm call order in `HumanoidModel.setupAnim`, omitting the arm whose
/// pose gives way. `true` is right, `false` is left.
pub fn application_order(right: ArmPose, left: ArmPose) -> [Option<bool>; 2] {
    if left.two_handed() {
        [Some(true), (!right.affects_offhand()).then_some(false)]
    } else {
        [Some(false), (!left.affects_offhand()).then_some(true)]
    }
}

#[cfg(test)]
mod tests {
    use super::ArmPose;
    #[test]
    fn custom_flags_select_only_the_arms_vanilla_applies_and_keep_their_order() {
        let custom = |both, offhand| ArmPose::Custom {
            slot: 0,
            both,
            offhand,
            angles: [[0; 3]; 2],
        };
        assert_eq!(
            super::application_order(custom(false, false), custom(false, true)),
            [Some(false), None]
        );
        assert_eq!(
            super::application_order(custom(false, true), custom(true, false)),
            [Some(true), None]
        );
        assert_eq!(
            super::application_order(custom(false, false), custom(true, false)),
            [Some(true), Some(false)]
        );
        assert_eq!(
            super::application_order(custom(false, false), custom(false, false)),
            [Some(false), Some(true)]
        );
    }

    #[test]
    fn registered_pose_flags_remain_independent_in_the_engine() {
        let pose = ArmPose::Custom {
            slot: 0,
            both: true,
            offhand: false,
            angles: [[f32::NAN.to_bits(); 3]; 2],
        };
        assert!(pose.two_handed());
        assert!(!pose.affects_offhand());
        let pose = ArmPose::Custom {
            slot: 0,
            both: false,
            offhand: true,
            angles: [[0; 3]; 2],
        };
        assert!(!pose.two_handed());
        assert!(pose.affects_offhand());
    }
}
