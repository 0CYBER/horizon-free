//! A mob the game has made but not yet added to a level: its typed refusal, the scoped editor a
//! hook gets beside its [`UnspawnedMobRef`](crate::UnspawnedMobRef), and the spawn group data
//! (`SpawnGroupData`) the mobs of one pack share.

use crate::{AttributeModifier, EntityRef, EquipmentSlot, Identifier, ItemStack, Text};

/// Why a read or an edit of an unspawned mob was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum UnspawnedMobError {
    /// The retained custom name exceeds the component boundary.
    Text(crate::text::TextFormError),
    /// The hook that lent the handle has returned, the handle belongs to another lend or world,
    /// or there is no game.
    Expired,
    /// This mob's type has no such attribute, or the game keeps no mob behind this lend (the
    /// ender dragon, which it stores apart from mobs, or a position check where the game creates
    /// the mob later from saved data or the world generator).
    Unsupported,
    /// The value is refused: an unknown attribute, a non-finite number, a modifier id the
    /// attribute already has, an empty or full tag list, or invalid item data.
    Invalid,
}

impl From<crate::text::TextFormError> for UnspawnedMobError {
    fn from(error: crate::text::TextFormError) -> Self {
        Self::Text(error)
    }
}

impl From<crate::StackWriteError> for UnspawnedMobError {
    fn from(_: crate::StackWriteError) -> Self {
        Self::Invalid
    }
}

impl crate::NoBackend for UnspawnedMobError {
    fn no_backend() -> Self {
        Self::Expired
    }
}

/// The edits a hook may make to the mob it was lent, beside the handle's reads.
///
/// The game hands this out with the [`UnspawnedMobRef`](crate::UnspawnedMobRef) for one hook
/// call and keeps neither after it returns. Each method takes that handle and reaches only that
/// mob: another lend's handle, or one kept from an earlier call, is refused with
/// [`UnspawnedMobError::Expired`]. An edit applies at once, so the handle's reads, and the next
/// listener, see it. The editor works where the hook's host is the read-only one: it changes
/// nothing in the level, only the mob that is about to join it.
pub trait UnspawnedMobEdits {
    /// Puts `item` in `slot` (`Mob.setItemSlot`): an API-owned stack; `None` empties the slot. Invalid
    /// item keys, counts, damage or components are refused with `Invalid`, without changing the
    /// slot. The stack's attribute modifiers apply to the mob at once.
    fn set_equipment(
        &self,
        mob: crate::UnspawnedMobRef,
        slot: EquipmentSlot,
        item: Option<ItemStack>,
    ) -> Result<(), UnspawnedMobError>;

    /// Sets an attribute's base value (`AttributeInstance.setBaseValue`), kept within the
    /// attribute's range.
    fn set_attribute_base(
        &self,
        mob: crate::UnspawnedMobRef,
        attribute: &Identifier,
        base: f64,
    ) -> Result<(), UnspawnedMobError>;

    /// Adds a permanent modifier (`AttributeInstance.addPermanentModifier`), saved with the mob.
    fn add_attribute_modifier(
        &self,
        mob: crate::UnspawnedMobRef,
        attribute: &Identifier,
        modifier: AttributeModifier,
    ) -> Result<(), UnspawnedMobError>;

    /// Removes the modifier `id` from the attribute; `false` when it had none.
    fn remove_attribute_modifier(
        &self,
        mob: crate::UnspawnedMobRef,
        attribute: &Identifier,
        id: &Identifier,
    ) -> Result<bool, UnspawnedMobError>;

    /// Adds a tag (`Entity.addTag`); `false` when the mob has it already.
    fn add_tag(&self, mob: crate::UnspawnedMobRef, tag: &str) -> Result<bool, UnspawnedMobError>;

    /// Removes a tag (`Entity.removeTag`); `false` when the mob did not have it.
    fn remove_tag(&self, mob: crate::UnspawnedMobRef, tag: &str)
    -> Result<bool, UnspawnedMobError>;

    /// Names the mob (`Entity.setCustomName`), or takes its name away with `None`.
    /// A component deeper than 512 nodes is refused with `UnspawnedMobError::Text`
    /// before changing the mob's name.
    fn set_custom_name(
        &self,
        mob: crate::UnspawnedMobRef,
        name: Option<Text>,
    ) -> Result<(), UnspawnedMobError>;
}

/// `AgeableMob.AgeableMobGroupData`: whether the pack's later members may be babies.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct AgeableGroup {
    /// How many mobs of the pack have been finalized with this group so far.
    pub size: i32,
    /// Whether a later member may spawn as a baby.
    pub should_spawn_baby: bool,
    /// The chance a later member is a baby.
    pub baby_chance: f32,
}

impl AgeableGroup {
    /// A group no mob has used yet.
    pub fn new(should_spawn_baby: bool, baby_chance: f32) -> Self {
        Self {
            size: 0,
            should_spawn_baby,
            baby_chance,
        }
    }
}

/// A tropical fish's look, as a school shares it (`TropicalFish.TropicalFishGroupData`), by the
/// names vanilla saves (`kob`, `orange`, `white`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct TropicalVariant {
    /// The pattern (`TropicalFish.Pattern`).
    pub pattern: String,
    /// The body's dye colour.
    pub base_color: String,
    /// The pattern's dye colour.
    pub pattern_color: String,
}

impl TropicalVariant {
    /// A look of `pattern`, `base_color` and `pattern_color`.
    pub fn new(pattern: String, base_color: String, pattern_color: String) -> Self {
        Self {
            pattern,
            base_color,
            pattern_color,
        }
    }
}

/// `SpawnGroupData`: what the mobs of one pack share, handed from one member's finalization to
/// the next. Variants are named as vanilla saves them (`creamy`, `snow`, `lucy`); a name the
/// game does not know drops the group.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SpawnGroup {
    /// `AgeableMob.AgeableMobGroupData`, for the ageable mobs with no group of their own.
    Ageable(AgeableGroup),
    /// `Zombie.ZombieGroupData`, and `Husk.HuskGroupData`'s camel try.
    Zombie {
        /// Whether the zombie is a baby.
        baby: bool,
        /// Whether a baby may ride a chicken.
        jockey: bool,
        /// For a husk, whether it already tried to ride a camel husk; `None` for other zombies.
        tried_camel: Option<bool>,
    },
    /// `Spider.SpiderEffectsGroupData`: the effect a spider spawns with, if any.
    Spider {
        /// The effect (`minecraft:speed`).
        effect: Option<Identifier>,
    },
    /// `Horse.HorseGroupData`: the coat colour.
    Horse {
        /// The ageable part.
        ageable: AgeableGroup,
        /// `Horse.Variant`'s name (`creamy`).
        variant: String,
    },
    /// `Llama.LlamaGroupData`.
    Llama {
        /// The ageable part.
        ageable: AgeableGroup,
        /// `Llama.Variant`'s name (`creamy`).
        variant: String,
    },
    /// `Fox.FoxGroupData`.
    Fox {
        /// The ageable part.
        ageable: AgeableGroup,
        /// `Fox.Variant`'s name (`red`, `snow`).
        variant: String,
    },
    /// `Rabbit.RabbitGroupData`.
    Rabbit {
        /// The ageable part.
        ageable: AgeableGroup,
        /// `Rabbit.Variant`'s name (`brown`, `evil`).
        variant: String,
    },
    /// `Axolotl.AxolotlGroupData`: the two variants a member picks between.
    Axolotl {
        /// The ageable part.
        ageable: AgeableGroup,
        /// `Axolotl.Variant`'s names (`lucy`).
        variants: [String; 2],
    },
    /// `Wolf.WolfPackData`.
    Wolf {
        /// The ageable part.
        ageable: AgeableGroup,
        /// The `minecraft:wolf_variant` registry entry.
        variant: Identifier,
    },
    /// `AbstractSchoolingFish.SchoolSpawnGroupData`: the school's leader, and for tropical
    /// fish the look the school shares.
    School {
        /// The leader.
        leader: EntityRef,
        /// The tropical fish look; `None` for other fish.
        tropical: Option<TropicalVariant>,
    },
}
