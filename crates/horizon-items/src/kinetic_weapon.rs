//! The pose timing projected once from a kinetic component at its load boundary.
use crate::items::ItemStack;
use serde_json::Value;
use {horizon_core::nbt::Tag, horizon_world::datafix::dynamic::as_number};

/// The fields read by `SpearAnimations.UseParams`, not combat or sound data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UseTiming {
    pub delay_ticks: i32,
    pub dismount_conditions: Option<UseDuration>,
    pub knockback_conditions: Option<UseDuration>,
    pub damage_conditions: Option<UseDuration>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UseDuration {
    pub max_duration_ticks: i32,
}

// Both codec inputs use Number.intValue before NON_NEGATIVE_INT validates it.
trait TimingValue: Sized {
    fn is_object(&self) -> bool;
    fn get(&self, key: &str) -> Option<&Self>;
    fn int(&self) -> Option<i32>;
}
impl TimingValue for Value {
    fn is_object(&self) -> bool {
        self.is_object()
    }
    fn get(&self, key: &str) -> Option<&Self> {
        self.get(key).filter(|v| !v.is_null())
    }
    fn int(&self) -> Option<i32> {
        Some(if let Some(n) = self.as_i64() {
            n as i32
        } else if let Some(n) = self.as_u64() {
            n as i32
        } else {
            // JsonParser's decimal tokens use BigDecimal.intValue: truncate
            // and wrap, unlike NBT Float/Double.intValue's saturating cast.
            (self.as_f64()?.trunc() % 4_294_967_296.0) as i64 as i32
        })
    }
}
impl TimingValue for Tag {
    fn is_object(&self) -> bool {
        matches!(self, Tag::Compound(_))
    }
    fn get(&self, key: &str) -> Option<&Self> {
        self.get(key)
    }
    fn int(&self) -> Option<i32> {
        Some(as_number(self)?.int_value())
    }
}
impl UseTiming {
    fn decode<V: TimingValue>(value: &V) -> Option<Self> {
        if !value.is_object() {
            return None;
        }
        let ticks = |v: &V| v.int().filter(|n| *n >= 0);
        let duration = |key| match value.get(key) {
            None => Some(None),
            Some(condition) => Some(Some(UseDuration {
                max_duration_ticks: ticks(condition.get("max_duration_ticks")?)?,
            })),
        };
        Some(Self {
            delay_ticks: value.get("delay_ticks").map_or(Some(0), ticks)?,
            dismount_conditions: duration("dismount_conditions")?,
            knockback_conditions: duration("knockback_conditions")?,
            damage_conditions: duration("damage_conditions")?,
        })
    }
    /// Normalize prototype timing using the vanilla numeric codec and defaults.
    pub fn from_json(value: &Value) -> Option<Self> {
        Self::decode(value)
    }

    /// Select the typed replacement, removal, or prototype without rebuilding a
    /// stack view or decoding any unrelated components in animation updates.
    pub fn of_stack(stack: &ItemStack, prototype: Option<Self>) -> Option<Self> {
        if let Some(data) = stack.data() {
            if let Some(kinetic) = data.kinetic_weapon() {
                return kinetic.timing;
            }
            if data.other("!minecraft:kinetic_weapon").is_some() {
                return None;
            }
        }
        prototype
    }
}

/// A modelled kinetic component: retain all fields for saving and transmission,
/// with a typed projection for rendering. They can only be replaced together.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct KineticComponent {
    raw: Tag,
    timing: Option<UseTiming>,
}
impl KineticComponent {
    pub(crate) fn new(raw: Tag) -> Self {
        Self {
            timing: UseTiming::decode(&raw),
            raw,
        }
    }
    pub(crate) fn raw(&self) -> &Tag {
        &self.raw
    }
}
