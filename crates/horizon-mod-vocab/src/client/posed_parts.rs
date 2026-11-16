//! The parts of a humanoid model as vanilla posed them this frame (`HumanoidModel.setupAnim`),
//! read-only, for a mod drawing on a player or humanoid mob in `render_living_post`,
//! `render_player_post` or a layer (`register_entity_layers`). The poses are the
//! [`PartPose`](crate::PartPose)s a baked layer's parts take, so a mod copies them onto its own
//! tree (`ClientHost::bake_layer`) and draws it where vanilla drew the body.

/// A part of a humanoid model, by the name vanilla's `PlayerModel` gives it. The outer skin
/// layer's parts (`Hat`, `Jacket`, the sleeves and the pants) are children of the part they cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum HumanoidPart {
    /// The head.
    Head,
    /// The hat layer, over the head.
    Hat,
    /// The body.
    Body,
    /// The jacket layer, over the body.
    Jacket,
    /// The right arm.
    RightArm,
    /// The right sleeve, over the right arm.
    RightSleeve,
    /// The left arm.
    LeftArm,
    /// The left sleeve, over the left arm.
    LeftSleeve,
    /// The right leg.
    RightLeg,
    /// The right trouser leg, over the right leg.
    RightPants,
    /// The left leg.
    LeftLeg,
    /// The left trouser leg, over the left leg.
    LeftPants,
}

impl HumanoidPart {
    /// Every part, parents before their children.
    pub const ALL: [Self; 12] = [
        Self::Head,
        Self::Hat,
        Self::Body,
        Self::Jacket,
        Self::RightArm,
        Self::RightSleeve,
        Self::LeftArm,
        Self::LeftSleeve,
        Self::RightLeg,
        Self::RightPants,
        Self::LeftLeg,
        Self::LeftPants,
    ];

    /// Its name in the model (`right_sleeve`).
    pub fn model_name(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Hat => "hat",
            Self::Body => "body",
            Self::Jacket => "jacket",
            Self::RightArm => "right_arm",
            Self::RightSleeve => "right_sleeve",
            Self::LeftArm => "left_arm",
            Self::LeftSleeve => "left_sleeve",
            Self::RightLeg => "right_leg",
            Self::RightPants => "right_pants",
            Self::LeftLeg => "left_leg",
            Self::LeftPants => "left_pants",
        }
    }
}

/// How many parts [`HiddenParts`] holds.
pub const MAX_HIDDEN_PARTS: usize = 8;
/// The longest part name [`HiddenParts`] takes, in bytes.
pub const MAX_PART_NAME: usize = 24;

/// The parts a mod hid from a vanilla model this frame (`render_player_pre`, `render_living_pre`), by their
/// names in the model (`head`, `nose`, `right_front_leg`): at most [`MAX_HIDDEN_PARTS`] names of
/// at most [`MAX_PART_NAME`] bytes, nothing allocated. A hidden part takes its children with it
/// (the hat goes with the head). An unknown name refuses the draw with the mob type and name.
/// Typed humanoid parts have separate storage and always fit alongside named parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[wire(via = HiddenPartsWire)]
#[non_exhaustive]
pub struct HiddenParts {
    names: [[u8; MAX_PART_NAME]; MAX_HIDDEN_PARTS],
    lens: [u8; MAX_HIDDEN_PARTS],
    count: u8,
    typed: u16,
}

/// Why [`HiddenParts::hide_named`] took no name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum HideError {
    /// [`MAX_HIDDEN_PARTS`] names are hidden already.
    Full,
    /// The name is empty or longer than [`MAX_PART_NAME`] bytes.
    BadName,
}

impl HiddenParts {
    /// Leaves the humanoid `part` out of the vanilla model this frame.
    pub fn hide(&mut self, part: HumanoidPart) {
        self.typed |= 1 << (part as u16);
    }

    /// Leaves the part named `name` out of the vanilla model this frame; hiding a name twice
    /// keeps one.
    pub fn hide_named(&mut self, name: &str) -> Result<(), HideError> {
        if name.is_empty() || name.len() > MAX_PART_NAME {
            return Err(HideError::BadName);
        }
        if self.contains_named(name) {
            return Ok(());
        }
        let slot = usize::from(self.count);
        if slot == MAX_HIDDEN_PARTS {
            return Err(HideError::Full);
        }
        self.names[slot][..name.len()].copy_from_slice(name.as_bytes());
        self.lens[slot] = name.len() as u8;
        self.count += 1;
        Ok(())
    }

    /// Whether the humanoid `part` is hidden.
    pub fn contains(&self, part: HumanoidPart) -> bool {
        self.contains_named(part.model_name())
    }

    /// Whether the part named `name` is hidden.
    pub fn contains_named(&self, name: &str) -> bool {
        self.names().any(|n| n == name)
    }

    /// No part is hidden.
    pub fn is_empty(&self) -> bool {
        self.count == 0 && self.typed == 0
    }

    /// The hidden parts' names, in the order they were hidden.
    pub fn names<'a>(&'a self) -> impl Iterator<Item = &'a str> + 'a {
        let (names, lens) = (&self.names, &self.lens);
        (0..usize::from(self.count))
            .map(move |i| {
                // Construction and Wire decoding validate the byte lengths and UTF-8 once.
                std::str::from_utf8(&names[i][..usize::from(lens[i])]).unwrap()
            })
            .chain(
                HumanoidPart::ALL
                    .into_iter()
                    .filter(move |part| self.typed & (1 << (*part as u16)) != 0)
                    .map(|part| -> &'a str { part.model_name() }),
            )
    }
}

#[derive(horizon_mod_api_macros::Wire)]
struct HiddenPartsWire {
    names: [[u8; MAX_PART_NAME]; MAX_HIDDEN_PARTS],
    lens: [u8; MAX_HIDDEN_PARTS],
    count: u8,
    typed: u16,
}
impl From<&HiddenParts> for HiddenPartsWire {
    fn from(v: &HiddenParts) -> Self {
        Self {
            names: v.names,
            lens: v.lens,
            count: v.count,
            typed: v.typed,
        }
    }
}
impl TryFrom<HiddenPartsWire> for HiddenParts {
    type Error = crate::wire::WireError;
    fn try_from(v: HiddenPartsWire) -> Result<Self, Self::Error> {
        use crate::wire::WireError;
        if usize::from(v.count) > MAX_HIDDEN_PARTS || v.typed >> 12 != 0 {
            return Err(WireError::Invalid("HiddenParts count or typed bits"));
        }
        for i in 0..usize::from(v.count) {
            let len = usize::from(v.lens[i]);
            if len == 0 || len > MAX_PART_NAME {
                return Err(WireError::Length);
            }
            std::str::from_utf8(&v.names[i][..len]).map_err(|_| WireError::Utf8)?;
        }
        Ok(Self {
            names: v.names,
            lens: v.lens,
            count: v.count,
            typed: v.typed,
        })
    }
}

/// One humanoid part as posed this frame: its pose relative to its parent, and whether it draws
/// (`ModelPart.visible` as the body drew it: after Skin Customization and the parts
/// `render_player_pre` hid).
#[derive(Clone, Copy, Debug, PartialEq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct PosedPart {
    /// Which part it is.
    pub part: HumanoidPart,
    /// Its pose relative to its parent.
    pub pose: crate::PartPose,
    /// Whether it draws.
    pub visible: bool,
}

/// The humanoid parts of a model, posed this frame: a fixed-size snapshot, nothing allocated.
#[derive(Clone, Copy, Debug, PartialEq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct PosedParts {
    /// Each of [`HumanoidPart::ALL`] at its index there: None for a part the model lacks.
    pub parts: [Option<PosedPart>; 12],
}

impl PosedParts {
    /// The parts `pose` gives `ModelPart`'s nine numbers for (x, y, z, x rot, y rot, z rot, x, y,
    /// z scale) and whether they draw; None when the model lacks a head or a body (not a humanoid
    /// model). A part the model lacks, such as a zombie's sleeves, is left out.
    pub fn new(pose: impl Fn(HumanoidPart) -> Option<([f32; 9], bool)>) -> Option<Self> {
        let parts = HumanoidPart::ALL.map(|part| {
            let (v, visible) = pose(part)?;
            let pose = crate::PartPose {
                x: v[0],
                y: v[1],
                z: v[2],
                x_rot: v[3],
                y_rot: v[4],
                z_rot: v[5],
                x_scale: v[6],
                y_scale: v[7],
                z_scale: v[8],
            };
            Some(PosedPart {
                part,
                pose,
                visible,
            })
        });
        let posed = Self { parts };
        (posed.get(HumanoidPart::Head).is_some() && posed.get(HumanoidPart::Body).is_some())
            .then_some(posed)
    }

    /// `part` as posed this frame, if the model has it.
    pub fn get(&self, part: HumanoidPart) -> Option<PosedPart> {
        self.parts[part as usize]
    }

    /// The parts the model has, in [`HumanoidPart::ALL`]'s order.
    pub fn iter(&self) -> impl Iterator<Item = PosedPart> + '_ {
        self.parts.iter().flatten().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_hidden_wire_is_refused() {
        use crate::wire::{Reader, Writer};
        for (count, len, byte) in [(9, 1, b'a'), (1, 25, b'a'), (1, 1, 255), (1, 0, 0)] {
            let raw = HiddenPartsWire {
                names: [[byte; MAX_PART_NAME]; MAX_HIDDEN_PARTS],
                lens: [len; MAX_HIDDEN_PARTS],
                count,
                typed: 0,
            };
            let mut w = Writer::new();
            w.put(&raw);
            assert!(Reader::new(w.bytes()).take::<HiddenParts>().is_err());
        }
    }

    #[test]
    fn a_humanoid_needs_a_head_and_a_body_and_keeps_each_parts_pose() {
        let pose = |x: f32| ([x, 0.0, 0.0, 0.5, 0.0, 0.0, 1.0, 1.0, 1.0], true);
        let parts = PosedParts::new(|part| match part {
            HumanoidPart::Head => Some(pose(1.0)),
            HumanoidPart::Body => Some(pose(2.0)),
            HumanoidPart::RightArm => Some(([3.0, 0.0, 0.0, 0.0, 0.25, 0.0, 1.0, 1.0, 1.0], false)),
            _ => None,
        })
        .expect("a head and a body make a humanoid");
        let arm = parts.get(HumanoidPart::RightArm).unwrap();
        assert_eq!(
            (arm.pose.x, arm.pose.y_rot, arm.visible),
            (3.0, 0.25, false)
        );
        assert_eq!(parts.get(HumanoidPart::RightSleeve), None);
        assert_eq!(parts.get(HumanoidPart::Head).unwrap().pose.x_rot, 0.5);
        let order: Vec<_> = parts.iter().map(|p| p.part).collect();
        assert_eq!(
            order,
            [
                HumanoidPart::Head,
                HumanoidPart::Body,
                HumanoidPart::RightArm
            ]
        );
        assert!(PosedParts::new(|part| (part == HumanoidPart::Head).then(|| pose(1.0))).is_none());
        let names: Vec<_> = HumanoidPart::ALL.iter().map(|p| p.model_name()).collect();
        assert_eq!(names.len(), 12);
        assert!(names.contains(&"left_pants") && names.contains(&"hat"));
    }

    #[test]
    fn hidden_parts_hold_names_of_any_model_up_to_their_capacity() {
        let mut hidden = HiddenParts::default();
        assert!(hidden.is_empty());
        hidden.hide(HumanoidPart::Head);
        assert_eq!(hidden.hide_named("nose"), Ok(()));
        assert_eq!(hidden.hide_named("nose"), Ok(()));
        assert!(hidden.contains(HumanoidPart::Head) && hidden.contains_named("head"));
        assert!(!hidden.contains(HumanoidPart::Hat) && !hidden.contains_named("nos"));
        assert_eq!(hidden.names().collect::<Vec<_>>(), ["nose", "head"]);
        assert_eq!(hidden.hide_named(""), Err(HideError::BadName));
        assert_eq!(
            hidden.hide_named(&"x".repeat(MAX_PART_NAME + 1)),
            Err(HideError::BadName)
        );
        for i in 0..MAX_HIDDEN_PARTS - 1 {
            assert_eq!(hidden.hide_named(&format!("part_{i}")), Ok(()));
        }
        assert_eq!(hidden.hide_named("one_more"), Err(HideError::Full));
        assert_eq!(hidden.names().count(), MAX_HIDDEN_PARTS + 1);
        for part in HumanoidPart::ALL {
            hidden.hide(part);
        }
        assert!(HumanoidPart::ALL.into_iter().all(|p| hidden.contains(p)));
    }
}
