//! What an experience orb's saved form can hold.
//!
//! `entity_nbt` writes `Value` as an NBT short and `Count` as an int, so an orb whose
//! `value` or `count` is outside these ranges would reload as a different total. The
//! mod API's `set_experience_orb` refuses such a write at the owning boundary, and the
//! merge refuses a count it cannot save instead of saturating it.

/// The most orbs one orb can count: `Count` is saved as an `i32`.
pub const MAX_COUNT: u32 = i32::MAX as u32;

/// Whether `value` survives a save: `Value` is saved as an `i16`.
pub fn value_fits(value: i32) -> bool {
    i16::try_from(value).is_ok()
}
