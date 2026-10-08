//! FoodData fields in a vanilla player NBT compound.
use crate::{
    food::FoodData,
    nbt::{Compound, JavaString, Tag},
};
fn int(tag: Option<&Tag>, default: i32) -> i32 {
    match tag {
        Some(Tag::Byte(v)) => i32::from(*v),
        Some(Tag::Short(v)) => i32::from(*v),
        Some(Tag::Int(v)) => *v,
        Some(Tag::Long(v)) => *v as i32,
        Some(Tag::Float(v)) => f32::from_bits(*v).floor() as i32,
        Some(Tag::Double(v)) => f64::from_bits(*v).floor() as i32,
        _ => default,
    }
}
fn float(tag: Option<&Tag>, default: f32) -> f32 {
    match tag {
        Some(Tag::Byte(v)) => f32::from(*v),
        Some(Tag::Short(v)) => f32::from(*v),
        Some(Tag::Int(v)) => *v as f32,
        Some(Tag::Long(v)) => *v as f32,
        Some(Tag::Float(v)) => f32::from_bits(*v),
        Some(Tag::Double(v)) => f64::from_bits(*v) as f32,
        _ => default,
    }
}
impl FoodData {
    /// Missing or nonnumeric values reset each field to its native default.
    /// Numeric values are converted without clamping the resulting food state.
    pub fn read_nbt(&mut self, tag: &Compound) {
        let get = |s: &str| tag.get(&JavaString::from(s));
        self.food = int(get("foodLevel"), 20);
        self.timer = int(get("foodTickTimer"), 0);
        self.saturation = float(get("foodSaturationLevel"), 5.0);
        self.exhaustion = float(get("foodExhaustionLevel"), 0.0);
    }
    /// Writes these four fields while preserving other player data. Java float
    /// tag factories normalize both signed zeros to the shared positive zero.
    pub fn write_nbt(&self, tag: &mut Compound) {
        let float = |v: f32| Tag::Float(if v == 0.0 { 0 } else { v.to_bits() });
        tag.insert("foodLevel".into(), Tag::Int(self.food));
        tag.insert("foodTickTimer".into(), Tag::Int(self.timer));
        tag.insert("foodSaturationLevel".into(), float(self.saturation));
        tag.insert("foodExhaustionLevel".into(), float(self.exhaustion));
    }
}
