//! Stack property calculations shared by gameplay, display and mod queries.
use horizon_core::{color::hsv_to_argb, nbt::Tag};

pub mod attack_range;
pub mod pet_variants;
pub mod resolvable;
pub mod swing_animation;
pub mod tooltip_display;

/// The effective rarity of an item stack, after its enchantment upgrade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ItemRarity {
    Common,
    Uncommon,
    Rare,
    Epic,
}

impl ItemRarity {
    /// `ItemStack.getRarity`: enchantments upgrade common/uncommon to rare and rare to epic.
    pub fn of(base: &str, enchanted: bool) -> Self {
        match (base, enchanted) {
            ("epic", _) | ("rare", true) => Self::Epic,
            ("rare", false) | (_, true) => Self::Rare,
            ("uncommon", false) => Self::Uncommon,
            _ => Self::Common,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Common => "common",
            Self::Uncommon => "uncommon",
            Self::Rare => "rare",
            Self::Epic => "epic",
        }
    }
}

/// A visible item bar: its width out of thirteen pixels and its RGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a plain drawing value, also constructed by the bundle calculation.
pub struct ItemBar {
    pub width: i32,
    pub color: u32,
}

/// `ItemStack.isDamageableItem`, using effective component presence.
pub fn damageable(mut has: impl FnMut(&str) -> bool) -> bool {
    has("minecraft:max_damage") && !has("minecraft:unbreakable") && has("minecraft:damage")
}

/// `ItemStack.hasFoil`: the effective override wins over the item-specific fallback.
pub fn foil(override_glint: Option<bool>, item_foil: bool) -> bool {
    override_glint.unwrap_or(item_foil)
}

/// `Item.isFoil`, including the compass override before enchantments.
pub fn item_foil(item: &str, lodestone_tracker: bool, enchanted: bool) -> bool {
    (item == "minecraft:compass" && lodestone_tracker) || enchanted
}

/// Compare registry identifiers with vanilla's default minecraft namespace.
pub fn same_item_id(a: &str, b: &str) -> bool {
    fn path(id: &str) -> &str {
        id.strip_prefix("minecraft:")
            .or_else(|| id.strip_prefix(':'))
            .unwrap_or(id)
    }
    path(a) == path(b)
}

/// `Item.getBarWidth` and `getBarColor`, when the durability bar is visible.
pub fn damage_bar(damage: i32, max_damage: i32) -> Option<ItemBar> {
    if max_damage <= 0 || damage <= 0 {
        return None;
    }
    let damage = damage.min(max_damage);
    let width = (13.0 - damage as f32 * 13.0 / max_damage as f32).round() as i32;
    let health = ((max_damage as f32 - damage as f32) / max_damage as f32).max(0.0);
    Some(ItemBar {
        width,
        color: hsv_to_argb(health / 3.0, 1.0, 1.0, 0),
    })
}

/// A registry holder or named tag in an item's holder set.
pub fn item_matches(spec: &str, item: &str, mut in_tag: impl FnMut(&str) -> bool) -> bool {
    match spec.strip_prefix('#') {
        Some(tag) => in_tag(tag),
        None => same_item_id(spec, item),
    }
}

/// `Repairable.isValidRepairItem`: the effective component's item holder set.
pub fn repairable(component: &Tag, item: &str, in_tag: impl FnMut(&str) -> bool) -> bool {
    match component.get("items") {
        Some(Tag::String(spec)) => item_matches(&spec.to_string_lossy(), item, in_tag),
        Some(Tag::List(ids)) => ids
            .iter()
            .any(|id| matches!(id, Tag::String(id) if same_item_id(&id.to_string_lossy(), item))),
        _ => false,
    }
}

/// An NBT numeric component converted with Java's integer conversion rules.
pub fn component_int(value: &Tag) -> Option<i32> {
    Some(match value {
        Tag::Byte(n) => i32::from(*n),
        Tag::Short(n) => i32::from(*n),
        Tag::Int(n) => *n,
        Tag::Long(n) => *n as i32,
        Tag::Float(n) => f32::from_bits(*n) as i32,
        Tag::Double(n) => f64::from_bits(*n) as i32,
        _ => return None,
    })
}

/// An NBT number read as a component's float value.
pub fn component_float(value: &Tag) -> Option<f32> {
    Some(match value {
        Tag::Byte(n) => f32::from(*n),
        Tag::Short(n) => f32::from(*n),
        Tag::Int(n) => *n as f32,
        Tag::Long(n) => *n as f32,
        Tag::Float(n) => f32::from_bits(*n),
        Tag::Double(n) => f64::from_bits(*n) as f32,
        _ => return None,
    })
}

/// `Weapon.CODEC`: optional nonnegative attack damage and finite blocking duration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weapon {
    pub item_damage_per_attack: i32,
    pub disable_blocking_for_seconds: f32,
}

impl Weapon {
    /// Decode borrowed NBT with Java numeric conversions, ignoring unknown fields.
    pub fn from_tag(tag: &Tag) -> Result<Self, &'static str> {
        let Tag::Compound(fields) = tag else {
            return Err("Weapon must be a map");
        };
        // Compare UTF-16 keys directly: even a first candidate needs no lookup-key allocation.
        let field = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| key.0.iter().copied().eq(name.encode_utf16()))
                .map(|(_, value)| value)
        };
        let item_damage_per_attack = match field("item_damage_per_attack") {
            None => 1,
            Some(value) => component_int(value).ok_or("item_damage_per_attack: Not a number")?,
        };
        if item_damage_per_attack < 0 {
            return Err("item_damage_per_attack: Value must be non-negative");
        }
        let disable_blocking_for_seconds = match field("disable_blocking_for_seconds") {
            None => 0.0,
            Some(value) => {
                component_float(value).ok_or("disable_blocking_for_seconds: Not a number")?
            }
        };
        // Float.compare distinguishes -0.0 from +0.0 and rejects NaN above MAX_VALUE.
        if !disable_blocking_for_seconds.is_finite()
            || disable_blocking_for_seconds.is_sign_negative()
        {
            return Err("disable_blocking_for_seconds: Value must be non-negative and finite");
        }
        Ok(Self {
            item_damage_per_attack,
            disable_blocking_for_seconds,
        })
    }

    /// Canonical decoded payload; defaults are explicit and unknown fields are absent.
    pub fn to_tag(self) -> Tag {
        Tag::compound([
            (
                "item_damage_per_attack",
                Tag::Int(self.item_damage_per_attack),
            ),
            (
                "disable_blocking_for_seconds",
                Tag::Float(self.disable_blocking_for_seconds.to_bits()),
            ),
        ])
    }
}
