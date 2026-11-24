//! What a living entity wears and holds, as a mod's `render_worn_equipment` lets it be drawn
//! this frame: each slot's base, glint and trim hidden or shown, and its colour tinted, without
//! replacing the entity's model or its other layers.

/// A slot of what a living entity wears or holds, as [`EquipmentRender`] names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum EquipmentRenderSlot {
    /// The helmet (`HumanoidArmorLayer`), or the item `CustomHeadLayer` draws on the head in its
    /// place (a carved pumpkin, a skull, a block).
    Head,
    /// The chestplate.
    Chest,
    /// The leggings.
    Legs,
    /// The boots.
    Feet,
    /// What the body slot draws: a wolf's, horse's or nautilus's armour, a llama's carpet, a
    /// happy ghast's harness.
    Body,
    /// The elytra's wings (`WingsLayer`), worn in the chest slot but a slot of its own here: a
    /// hidden chestplate leaves them.
    Elytra,
    /// The item in the main hand (`ItemInHandLayer`).
    MainHand,
    /// The item in the off hand.
    OffHand,
}

impl EquipmentRenderSlot {
    /// Every slot.
    pub const ALL: [Self; 8] = [
        Self::Head,
        Self::Chest,
        Self::Legs,
        Self::Feet,
        Self::Body,
        Self::Elytra,
        Self::MainHand,
        Self::OffHand,
    ];
}

/// What of a slot's drawing [`EquipmentRender::hide`] leaves out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum EquipmentRenderPart {
    /// The piece itself, with all that is drawn over it: the armour's model and its dyed and
    /// emissive layers, the wings, the held item, and their glint and trim.
    Base,
    /// The enchantment glint, and nothing else.
    Glint,
    /// The armour trim, and nothing else.
    Trim,
}

/// White: a tint that changes nothing.
const UNTINTED: u32 = 0xFF_FFFF;

/// How a mod lets a living entity's worn and held items draw this frame
/// (`render_worn_equipment`): by [`EquipmentRenderSlot`], a [`EquipmentRenderPart`] hidden or a
/// colour multiplied in. It starts with everything shown and untinted; what one mod hides or
/// tints, the next sees and adds to. Nothing allocated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct EquipmentRender {
    /// One bit for each slot and part: `slot * 3 + part`.
    hidden: u32,
    /// Each slot's tint, `0xRRGGBB`, white when untinted.
    tints: [u32; 8],
}

impl Default for EquipmentRender {
    fn default() -> Self {
        Self::new()
    }
}

impl EquipmentRender {
    /// Everything shown and untinted.
    pub const fn new() -> Self {
        Self {
            hidden: 0,
            tints: [UNTINTED; 8],
        }
    }

    fn bit(slot: EquipmentRenderSlot, part: EquipmentRenderPart) -> u32 {
        1 << (slot as u32 * 3 + part as u32)
    }

    /// Leaves `part` of `slot` out this frame. [`EquipmentRenderPart::Base`] takes the whole
    /// piece, its glint and trim with it; hiding twice keeps it hidden.
    pub fn hide(&mut self, slot: EquipmentRenderSlot, part: EquipmentRenderPart) {
        self.hidden |= Self::bit(slot, part);
    }

    /// Whether `part` of `slot` is left out: hidden itself, or with the base.
    pub fn hides(&self, slot: EquipmentRenderSlot, part: EquipmentRenderPart) -> bool {
        let bits = Self::bit(slot, part) | Self::bit(slot, EquipmentRenderPart::Base);
        (self.hidden & bits) != 0
    }

    /// Multiplies the colour `slot` draws in by `rgb` (`0xRRGGBB`, each channel times its
    /// value over 255; the high byte is ignored, and the alpha stays): the piece, its dyed and
    /// emissive layers and its trim, not its glint. A second tint multiplies the first.
    pub fn tint(&mut self, slot: EquipmentRenderSlot, rgb: u32) {
        let old = self.tints[slot as usize];
        let channel =
            |shift: u32| ((((old >> shift) & 0xFF) * ((rgb >> shift) & 0xFF)) / 255) << shift;
        self.tints[slot as usize] = channel(16) | channel(8) | channel(0);
    }

    /// The colour `slot` is multiplied by, `0xRRGGBB`: `0xFFFFFF` when untinted.
    pub fn tint_color(&self, slot: EquipmentRenderSlot) -> u32 {
        self.tints[slot as usize] & UNTINTED
    }

    /// Nothing hidden and nothing tinted: every slot draws as vanilla draws it.
    pub fn is_empty(&self) -> bool {
        (self.hidden & 0xFF_FFFF) == 0 && self.tints.iter().all(|t| (t & UNTINTED) == UNTINTED)
    }
}

#[cfg(test)]
mod tests {
    use super::{EquipmentRender, EquipmentRenderPart as Part, EquipmentRenderSlot as Slot};

    /// A slot's base takes its glint and trim; a glint or trim hides alone, slot by slot.
    #[test]
    fn hiding_a_base_takes_its_glint_and_trim_and_nothing_else() {
        let mut shown = EquipmentRender::new();
        assert!(shown.is_empty());
        assert_eq!(shown, EquipmentRender::default());
        shown.hide(Slot::Head, Part::Base);
        for part in [Part::Base, Part::Glint, Part::Trim] {
            assert!(shown.hides(Slot::Head, part), "{part:?}");
            assert!(!shown.hides(Slot::Chest, part), "{part:?}");
        }
        shown.hide(Slot::Feet, Part::Trim);
        assert!(shown.hides(Slot::Feet, Part::Trim));
        assert!(!shown.hides(Slot::Feet, Part::Base));
        assert!(!shown.hides(Slot::Feet, Part::Glint));
        shown.hide(Slot::OffHand, Part::Glint);
        assert!(shown.hides(Slot::OffHand, Part::Glint));
        assert!(!shown.hides(Slot::OffHand, Part::Base));
        assert!(!shown.hides(Slot::MainHand, Part::Glint));
        // Each slot's bits are its own, the last one's too.
        let mut last = EquipmentRender::new();
        last.hide(Slot::OffHand, Part::Trim);
        assert!(!last.is_empty());
        for slot in Slot::ALL {
            assert_eq!(
                last.hides(slot, Part::Trim),
                slot == Slot::OffHand,
                "{slot:?}"
            );
        }
    }

    /// A tint multiplies each channel (ARGB.multiply), a second tint multiplies the first, and
    /// the high byte is ignored.
    #[test]
    fn tints_multiply_by_channel_and_ignore_the_high_byte() {
        let mut shown = EquipmentRender::new();
        assert_eq!(shown.tint_color(Slot::MainHand), 0xFF_FFFF);
        shown.tint(Slot::MainHand, 0x80_FF_40);
        assert!(!shown.is_empty());
        assert_eq!(shown.tint_color(Slot::MainHand), 0x80_FF_40);
        assert_eq!(shown.tint_color(Slot::OffHand), 0xFF_FFFF);
        // 0x80 * 0x80 / 255 = 64, 0xFF * 0x10 / 255 = 16, 0x40 * 0xFF / 255 = 64.
        shown.tint(Slot::MainHand, 0x12_80_10_FF);
        assert_eq!(shown.tint_color(Slot::MainHand), 0x40_10_40);
        let mut white = EquipmentRender::new();
        white.tint(Slot::Body, 0x00FF_FFFF);
        assert!(white.is_empty(), "white tints nothing");
    }
}
