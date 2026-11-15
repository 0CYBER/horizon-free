//! API-owned health and damage values.
/// How a modifier changes what a reduction took (NeoForge's `IReductionsModifier`, as plain
/// data so it also crosses into a wasm mod).
// no wire: preserves a u8 change tag followed by f32, rather than the structural enum derive u32 tag.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ReductionChange {
    /// Take this much more off (less when negative).
    Add(f32),
    /// Take this many times as much.
    Scale(f32),
    /// Take at least this much.
    AtLeast(f32),
    /// Take at most this much.
    AtMost(f32),
}
#[allow(unused_imports)]
use crate::*;
/// What took a share of a hit before the health, as NeoForge's `DamageContainer.Reduction`
/// names it.
// no wire: preserves the compact u8 reduction tag; the structural enum derive uses u32 tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reduction {
    /// The damage cooldown: what the last hit already dealt (`lastHurt`), taken off a hit
    /// that lands within it.
    Invulnerability,
    /// Armor and toughness (`getDamageAfterArmorAbsorb`).
    Armor,
    /// Protection enchantments (`EnchantmentHelper.getDamageProtection`).
    Enchantments,
    /// Resistance (`getDamageAfterMagicAbsorb`'s mob effect step).
    MobEffects,
    /// The absorption hearts.
    Absorption,
    /// What the entity's own kind resists: a witch's magic resistance.
    InnateResistance,
}
/// A hit's damage sequence as plain values, field for field: the form it crosses into a wasm
/// mod as. Reduction storage is crate-visible only for the generated wasm codec; mods use
/// the reduction accessors so future reduction kinds do not expose a fixed array layout.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct DamageValues {
    /// The damage sequence's `original` value.
    pub original: f32,
    /// The damage sequence's `new_damage` value.
    pub new_damage: f32,
    /// The damage sequence's `blocked` value.
    pub blocked: f32,
    /// The damage sequence's `shield_damage` value.
    pub shield_damage: f32,
    /// The damage sequence's `post_attack_invulnerability_ticks` value.
    pub post_attack_invulnerability_ticks: i32,
    /// The damage sequence's `should_cause_side_effects` value.
    pub should_cause_side_effects: bool,
    pub(crate) reductions: [f32; 6],
    /// Ordered reduction adjustments for this hit.
    pub reduction_modifiers: Vec<(Reduction, ReductionChange)>,
    /// The damage sequence's `entered_pre` value.
    pub entered_pre: f32,
    /// The damage sequence's `inflicted` value.
    pub inflicted: f32,
    /// The damage sequence's `health_damage` value.
    pub health_damage: f32,
}

impl DamageValues {
    /// The amount taken by one reduction kind.
    pub fn reduction(&self, reduction: Reduction) -> f32 {
        self.reductions[reduction as usize]
    }
    /// Set the amount taken by one reduction kind.
    pub fn set_reduction(&mut self, reduction: Reduction, amount: f32) {
        self.reductions[reduction as usize] = amount;
    }
}
/// A plain snapshot of one hit. Sequencing and normalization remain owned by the game.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq)]
// exhaustive: its wrapped values are private; mods use constructors and accessors
pub struct DamageContainer(DamageValues);
impl From<DamageValues> for DamageContainer {
    fn from(values: DamageValues) -> Self {
        Self(values)
    }
}
impl From<&DamageContainer> for DamageValues {
    fn from(value: &DamageContainer) -> Self {
        value.0.clone()
    }
}
impl DamageContainer {
    /// Append a per-hit modifier, applied by the engine in registration order.
    pub fn add_reduction_modifier(&mut self, reduction: Reduction, change: ReductionChange) {
        self.0.reduction_modifiers.push((reduction, change));
    }
    /// Iterate the modifiers registered for this hit in order.
    pub fn reduction_modifiers(&self) -> impl Iterator<Item = (Reduction, ReductionChange)> + '_ {
        self.0.reduction_modifiers.iter().copied()
    }

    /// Begin a snapshot with the incoming damage and vanilla's default cooldown.
    pub fn new(original: f32) -> Self {
        Self(DamageValues {
            original,
            new_damage: original,
            blocked: 0.0,
            shield_damage: 0.0,
            post_attack_invulnerability_ticks: 20,
            should_cause_side_effects: false,
            reductions: [0.0; 6],
            reduction_modifiers: Vec::new(),
            entered_pre: 0.0,
            inflicted: 0.0,
            health_damage: 0.0,
        })
    }
    /// Read the hit snapshot's `original`.
    pub fn original(&self) -> f32 {
        self.0.original
    }
    /// Read the hit snapshot's `new_damage`.
    pub fn new_damage(&self) -> f32 {
        self.0.new_damage
    }
    /// Read the hit snapshot's `blocked`.
    pub fn blocked(&self) -> f32 {
        self.0.blocked
    }
    /// Read the hit snapshot's `shield_damage`.
    pub fn shield_damage(&self) -> f32 {
        self.0.shield_damage
    }
    /// Read the hit snapshot's `post_attack_invulnerability_ticks`.
    pub fn post_attack_invulnerability_ticks(&self) -> i32 {
        self.0.post_attack_invulnerability_ticks
    }
    /// Read the hit snapshot's `should_cause_side_effects`.
    pub fn should_cause_side_effects(&self) -> bool {
        self.0.should_cause_side_effects
    }
    /// Read the hit snapshot's `inflicted`.
    pub fn inflicted(&self) -> f32 {
        self.0.inflicted
    }
    /// Read the hit snapshot's `health_damage`.
    pub fn health_damage(&self) -> f32 {
        self.0.health_damage
    }
    /// Change `new_damage`; the engine applies normalization at its original application point.
    pub fn set_new_damage(&mut self, value: f32) {
        self.0.new_damage = value;
    }
    /// Change `post_attack_invulnerability_ticks`; the engine applies normalization at its original application point.
    pub fn set_post_attack_invulnerability_ticks(&mut self, value: i32) {
        self.0.post_attack_invulnerability_ticks = value;
    }
    /// The amount taken by one reduction kind.
    pub fn reduction(&self, reduction: Reduction) -> f32 {
        self.0.reduction(reduction)
    }
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// A hit's `DamageSource` for `HURT_SERVER`: its type (`minecraft:fall`),
/// the entity ids of `getEntity` (`causing`) and `getDirectEntity`
/// (`direct`), 0 for none or not known, and where it comes from
/// (`getSourcePosition`), none when not known.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct HurtSource<'a> {
    pub damage_type: &'a str,
    pub causing: i32,
    /// The projectile's entity id when the damage type names one (an arrow, a thrown
    /// entity, an owned wither skull), including an ownerless projectile; the attacker
    /// itself for melee when known. 0 when the source names none, including `magic()`
    /// dealt during a projectile's impact (an ownerless wither skull, a tipped arrow's
    /// instant harm).
    pub direct: i32,
    pub position: Option<[f64; 3]>,
}
/// Which entry of vanilla's `LivingEntity.knockback` a push came through, as
/// `living_knockback` tells its listener. The short entry (power, direction) only delegates to
/// the full one (which also takes whether the push comes from an effect), so one push through
/// the short entry reaches the hook once, as `Short`; a call of the full entry itself is `Full`.
/// A listener that changes the strength where a mixin on both entries would change it twice
/// for a short-entry call and once for a full-entry call applies its change twice on `Short`.
// exhaustive: vanilla has exactly these two entries, `knockback(power, x, z)` and the full one
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
pub enum KnockbackEntry {
    /// The push entered through `knockback(power, x, z)`, which then calls the full entry.
    Short,
    /// The push entered the full entry directly.
    Full,
}
/// Where the entity a hit lands on stands and looks (`getViewVector`), for
/// `HURT_SERVER`.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct Victim {
    pub position: [f64; 3],
    pub view: [f64; 3],
}
/// What a player's raised blocking item does to a hit, as `living_shield_block`
/// may change it (`LivingShieldBlockEvent`).
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ShieldBlock {
    /// The player counts as blocking.
    /// The damage sequence's `blocked` value.
    pub blocked: bool,
    /// The damage the item stops, kept within 0 and the hit's damage.
    pub blocked_damage: f32,
    /// The durability the item loses (`ItemDamageFunction` of what it stops). Below 0,
    /// vanilla's `ItemDamageFunction` of the final `blocked_damage` applies.
    /// The damage sequence's `shield_damage` value.
    pub shield_damage: i32,
}
