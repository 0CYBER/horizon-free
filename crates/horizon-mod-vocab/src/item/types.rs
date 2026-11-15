// Types that lived in the API root before the domain split.
use crate::*;

/// A baked standalone model resolved through [`RenderAssets::model`].
/// Retain it between frames; reacquire it after a successful resource reload.
// no wire: decoding from_parts rejects generation zero (an unpublished model handle).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: private fields; only the engine resolves model handles
pub struct ModelRef(pub(crate) u64, pub(crate) u32);

impl ModelRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished model generation");
        }
        Ok(Self(generation, slot))
    }
}

/// Why a standalone model could not be resolved or drawn. Errors append no geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ModelError {
    /// The name is not a registered, successfully baked standalone model, or its slot is absent.
    UnknownModel,
    /// A successful resource replacement superseded the handle's generation.
    StaleModel,
}

/// An item requested at registration; its load-owned slot identifies the canonical name.
// no wire: decoding validates the u32 slot with ItemKey::from_slot on the host; wasm carries the host-issued opaque token without consulting guest tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only registration creates a key
pub struct ItemKey(pub(crate) u32);

/// A menu type (`minecraft:generic_9x3`) in the built-in menu registry, in
/// registration order. A key stays valid until the game exits.
// no wire: decoding validates the u32 slot with MenuKey::from_slot on the host; wasm carries the host-issued opaque token without consulting guest tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the load-owned table creates keys
pub struct MenuKey(pub(crate) u32);

/// An enchantment (`minecraft:sharpness`) in the data-loaded enchantment
/// registry, in registry order. Keys keep their identity across reloads; a
/// reload changes which entries the table lists, not what an existing slot names.
// no wire: decoding validates the u32 slot with EnchantmentKey::from_slot on the host; wasm carries the host-issued opaque token without consulting guest tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the load-owned table creates keys
pub struct EnchantmentKey(pub(crate) u32);

/// The registry facts a recipe viewer needs for an enchantment.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct EnchantmentInfo {
    /// Registry name, including namespace.
    pub id: String,
    /// Largest supported level.
    pub max_level: u32,
    /// Whether the enchantment is in the curse tag.
    pub curse: bool,
}

impl EnchantmentInfo {
    /// Construct registry facts for an enchantment.
    pub fn new(id: String, max_level: u32, curse: bool) -> Self {
        Self {
            id,
            max_level,
            curse,
        }
    }
}

/// One armor slot of an `ArmorHurtEvent`: the piece in `slot`, the durability
/// damage vanilla works out for it (`damage`, `max(1, hit / 4)`) and the damage it
/// takes (`new_damage`, starts equal; a mod may change it, below 1 for none; the
/// game casts it to a whole number as vanilla does).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ArmorHurt {
    /// The equipment slot the armor piece sits in.
    pub slot: EquipmentSlot,
    /// The armor piece being damaged.
    pub stack: ItemStack,
    /// The durability damage vanilla computed.
    pub damage: f32,
    /// The durability damage to apply instead; starts equal to `damage`.
    pub new_damage: f32,
}

// no wire: type alias inherits the Wire policy of its tuple or Result target.
/// A dispenser hook's answer: the slot's remainder (None empties it) and the
/// level event to play, `CLICK` (1000), `FAIL` (1001) or 0 for none.
pub type Dispensed = (Option<ItemStack>, i32);

// no wire: borrowed call-scoped view; transport uses owned snapshots.
/// The table, location and initiating player of a container fill.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ContainerLootFilled<'a> {
    /// The rolled loot table id.
    pub table: &'a str,
    /// The dimension containing the target.
    pub dimension: &'a str,
    /// The block centre or container entity position.
    pub origin: [f64; 3],
    /// The container block position, for a block target.
    pub block: Option<[i32; 3]>,
    /// The container entity id, for a vehicle target.
    pub entity: Option<u32>,
    /// The UUID of the player whose open starts the fill; None otherwise.
    pub opener: Option<u128>,
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// What dropped the items heard by the `loot_drops` hook.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct LootDrops<'a> {
    /// The block's or mob's own loot table (`minecraft:blocks/stone`, `minecraft:entities/zombie`);
    /// empty when none rolled: the block or mob has none, the `mob_drops` rule is off, a baby
    /// animal. A mob's drops may hold more: a charged creeper's head, a fox's mouth item.
    pub table: &'a str,
    /// The level they drop in (`minecraft:overworld`).
    pub dimension: &'a str,
    /// `LootContextParams.ORIGIN`: a broken block's centre, a mob's position.
    pub origin: [f64; 3],
    /// The block broken: its position and state.
    pub block: Option<([i32; 3], BlockState)>,
    /// The mob that died, or the entity a player's use dropped them from (the
    /// sheep sheared, the item frame broken): its entity ID and type
    /// (`minecraft:zombie`).
    pub entity: Option<(u32, &'a str)>,
    /// The player who broke the block or killed the mob, when known. A mob
    /// names one only while it remembers the player's hit (100 ticks), so one
    /// that dies to lava long after names none.
    pub player: Option<u128>,
    /// `LootContextParams.DAMAGE_SOURCE` of this death invocation. None for
    /// non-death drops. Environmental deaths have a source but no causing entity.
    /// `causing` and `direct` are the attacking and direct attacking entity IDs;
    /// 0 means absent or unknown, as on the other damage hooks.
    pub damage_source: Option<crate::entity::health::HurtSource<'a>>,
    /// The direct projectile's recorded stack; use `weapon_item` for the death weapon.
    // hidden: borrowed transport field; mods use the source-aware weapon_item read.
    #[doc(hidden)]
    pub weapon: Option<&'a ItemStack>,
    /// The breaking or using tool (`LootContextParams.TOOL`). Always None for
    /// entity deaths. For the attacker's live hand, resolve `damage_source.causing`
    /// through `host.entity`, then `host.living`, and read `living.hand_item(host, Hand::Main)`.
    /// This reads when called, including changes made by earlier loot listeners.
    pub tool: Option<&'a ItemStack>,
    /// The radius of the explosion that broke the block.
    pub explosion: Option<f32>,
    /// A player's use dropped them rather than a break or a death: shears on a
    /// sheep, a mooshroom, a snow golem or a bogged, a brush on an armadillo or
    /// a suspicious block, shears carving a pumpkin, picking sweet berries or
    /// cave vines, a hit taking an item frame or its item, or untying a lead
    /// or cutting it with shears. `player` names them, or is `None` when a
    /// dispenser's shears sheared a mooshroom; `block` or `entity` says what
    /// dropped them. A mob that a player killed while
    /// leashed drops its lead on its next tick as a call of its own (`entity`
    /// the mob, `player` its killer, `table` empty, `used` false).
    pub used: bool,
}

impl LootDrops<'_> {
    /// Vanilla's `DamageSource.getWeaponItem` for this death. Arrows return the
    /// bow or crossbow recorded when fired; tridents return their own original
    /// stack, preserving components even after the shooter changes hands.
    /// Otherwise reads the direct entity's weapon when called: a player's recorded
    /// auto-spin stack during Riptide, otherwise the live main hand.
    /// Returns None for non-death drops or an absent or unavailable weapon.
    pub fn weapon_item(&self, host: &dyn Host) -> Option<ItemStack> {
        let source = self.damage_source?;
        if let Some(weapon) = self.weapon {
            return Some(weapon.clone());
        }
        let entity = host.entity(u32::try_from(source.direct).ok().filter(|id| *id != 0)?)?;
        let living = host.living(entity)?;
        backend_of(host)?.living_weapon_item(living)
    }
}

/// An item that drops where a living entity died or a block broke, before it is
/// an item entity in the level (`living_drops`, `block_drops`): what `ItemEntity`'s
/// constructor was given, and the item-entity state it joins the level with.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ItemDrop {
    /// The dropped stack.
    pub stack: ItemStack,
    /// Where the item entity appears, in blocks.
    pub position: [f64; 3],
    /// Its motion in blocks per tick, as vanilla's constructor drew it.
    pub velocity: [f64; 3],
    /// Ticks before a player may pick it up.
    pub pickup_delay: u32,
    /// `ItemEntity.setThrower`: the UUID of the entity that threw it, none by default.
    pub thrower: Option<u128>,
    /// `ItemEntity.setTarget`: the UUID of the only player who may pick it up, none (anyone)
    /// by default.
    pub target: Option<u128>,
    /// `Entity.setNoGravity`: the item entity floats where it moves; false by default.
    pub no_gravity: bool,
    /// The item entity's starting `age` in ticks, 0 by default. It despawns at 6000;
    /// [`ItemDrop::UNLIMITED_LIFETIME`] (vanilla's `setUnlimitedLifetime`) never does.
    pub age: i32,
}

impl ItemDrop {
    /// A drop of `stack` at `position` with a `velocity` and `pickup_delay`.
    pub fn new(
        stack: ItemStack,
        position: [f64; 3],
        velocity: [f64; 3],
        pickup_delay: u32,
    ) -> Self {
        Self {
            stack,
            position,
            velocity,
            pickup_delay,
            thrower: None,
            target: None,
            no_gravity: false,
            age: 0,
        }
    }
    /// The `age` that keeps an item entity from despawning (`ItemEntity.setUnlimitedLifetime`).
    pub const UNLIMITED_LIFETIME: i32 = -32768;
    /// The same drop, thrown by `thrower` (`ItemEntity.setThrower`).
    pub fn with_thrower(self, thrower: Option<u128>) -> Self {
        Self { thrower, ..self }
    }
    /// The same drop, which only `target` may pick up (`ItemEntity.setTarget`).
    pub fn with_target(self, target: Option<u128>) -> Self {
        Self { target, ..self }
    }
    /// The same drop, with or without gravity (`Entity.setNoGravity`).
    pub fn with_no_gravity(self, no_gravity: bool) -> Self {
        Self { no_gravity, ..self }
    }
    /// The same drop, starting at `age` ticks ([`ItemDrop::UNLIMITED_LIFETIME`] never despawns).
    pub fn with_age(self, age: i32) -> Self {
        Self { age, ..self }
    }
}

/// A statistic type (`BuiltInRegistries.STAT_TYPE`), for a player's
/// statistics: what the stat's key names (a block, an item, an entity type,
/// or a `minecraft:custom` stat).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum StatType {
    /// `minecraft:mined`, keyed by block.
    Mined,
    /// `minecraft:crafted`, keyed by item.
    Crafted,
    /// `minecraft:used`, keyed by item.
    Used,
    /// `minecraft:broken`, keyed by item.
    Broken,
    /// `minecraft:picked_up`, keyed by item.
    PickedUp,
    /// `minecraft:dropped`, keyed by item.
    Dropped,
    /// `minecraft:killed`, keyed by entity type.
    Killed,
    /// `minecraft:killed_by`, keyed by entity type.
    KilledBy,
    /// `minecraft:custom`, keyed by custom stat (`minecraft:jump`...).
    Custom,
}

impl StatType {
    /// The type's registry path (`mined`, `killed_by`...).
    pub fn path(self) -> &'static str {
        match self {
            Self::Mined => "mined",
            Self::Crafted => "crafted",
            Self::Used => "used",
            Self::Broken => "broken",
            Self::PickedUp => "picked_up",
            Self::Dropped => "dropped",
            Self::Killed => "killed",
            Self::KilledBy => "killed_by",
            Self::Custom => "custom",
        }
    }
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// A fishing catch, for `item_fished` (`ItemFishedEvent`).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
pub struct ItemFished {
    /// The bobber's entity id and position.
    pub hook: u32,
    /// The position.
    pub position: [f64; 3],
}

/// The slots an item's attribute modifier applies in (vanilla's
/// `EquipmentSlotGroup`).
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EquipmentSlotGroup {
    /// `any`: every slot.
    Any,
    /// `mainhand`.
    MainHand,
    /// `offhand`.
    OffHand,
    /// `hand`: either hand.
    Hand,
    /// `feet`.
    Feet,
    /// `legs`.
    Legs,
    /// `chest`.
    Chest,
    /// `head`.
    Head,
    /// `armor`: feet, legs, chest, head and body.
    Armor,
    /// `body`: an animal's armor slot.
    Body,
    /// `saddle`.
    Saddle,
}

/// One attribute modifier an item stack grants (vanilla's
/// `ItemAttributeModifiers.Entry`): the attribute (`minecraft:armor`), the
/// modifier, and the slots it applies in.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct ItemAttributeModifier {
    /// The attribute's id.
    pub attribute: Identifier,
    /// The modifier.
    pub modifier: AttributeModifier,
    /// The slots it applies in.
    pub slot: EquipmentSlotGroup,
}

impl ItemAttributeModifier {
    /// `modifier` on `attribute`, in the slots of `slot`.
    pub fn new(
        attribute: Identifier,
        modifier: AttributeModifier,
        slot: EquipmentSlotGroup,
    ) -> Self {
        Self {
            attribute,
            modifier,
            slot,
        }
    }
}

/// The attribute modifiers of one item stack, as `item_attribute_modifier`
/// edits them (`ItemAttributeModifierEvent`'s list). It starts as the
/// stack's own `minecraft:attribute_modifiers`, or its item's, in order.
// exhaustive: its fields are private, so mods only edit it through its methods
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemAttributeModifiers {
    pub(crate) entries: Vec<ItemAttributeModifier>,
    pub(crate) changed: bool,
}

impl ItemAttributeModifiers {
    /// The modifiers, in order.
    pub fn iter(&self) -> std::slice::Iter<'_, ItemAttributeModifier> {
        self.entries.iter()
    }
    /// How many modifiers there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// `addModifier`: adds `modifier` on `attribute` in the slots of `slot`,
    /// unless `attribute` already has a modifier with its id. Whether it
    /// was added.
    pub fn add(
        &mut self,
        attribute: Identifier,
        modifier: AttributeModifier,
        slot: EquipmentSlotGroup,
    ) -> bool {
        if self.find(&attribute, &modifier.id).is_some() {
            return false;
        }
        self.entries
            .push(ItemAttributeModifier::new(attribute, modifier, slot));
        self.changed = true;
        true
    }
    /// `replaceModifier`: puts `modifier` on `attribute` in the slots of
    /// `slot`, in place of the one with its id, or at the end.
    pub fn replace(
        &mut self,
        attribute: Identifier,
        modifier: AttributeModifier,
        slot: EquipmentSlotGroup,
    ) {
        let entry = ItemAttributeModifier::new(attribute, modifier, slot);
        match self.find(&entry.attribute, &entry.modifier.id) {
            Some(i) => self.entries[i] = entry,
            None => self.entries.push(entry),
        }
        self.changed = true;
    }
    /// `removeModifier`: removes the modifier `id` of `attribute`. Whether
    /// there was one.
    pub fn remove(&mut self, attribute: &Identifier, id: &Identifier) -> bool {
        let Some(i) = self.find(attribute, id) else {
            return false;
        };
        self.entries.remove(i);
        self.changed = true;
        true
    }
    /// `removeIf`: keeps only the modifiers `keep` accepts. Whether any went.
    pub fn retain(&mut self, mut keep: impl FnMut(&ItemAttributeModifier) -> bool) -> bool {
        let len = self.entries.len();
        self.entries.retain(|entry| keep(entry));
        let removed = self.entries.len() != len;
        self.changed |= removed;
        removed
    }
    /// `removeAllModifiersFor`: removes every modifier of `attribute`.
    /// Whether any went.
    pub fn remove_all_for(&mut self, attribute: &Identifier) -> bool {
        self.retain(|entry| entry.attribute != *attribute)
    }
    /// `clearModifiers`: removes every modifier.
    pub fn clear(&mut self) {
        self.changed |= !self.entries.is_empty();
        self.entries.clear();
    }
    pub(crate) fn find(&self, attribute: &Identifier, id: &Identifier) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.attribute == *attribute && e.modifier.id == *id)
    }
}

/// The enchantments `get_enchantment_level` (`GetEnchantmentLevelEvent`)
/// hands a mod: NeoForge's `ItemEnchantments.Mutable`, each enchantment's id
/// (`minecraft:sharpness`) with its level, in order. Private fields, edited
/// through methods, so the game knows whether a mod changed them.
// exhaustive: its fields are private, so mods only edit it through its methods
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemEnchantments {
    pub(crate) entries: Vec<(Identifier, u32)>,
    pub(crate) changed: bool,
}

impl ItemEnchantments {
    /// The highest level an enchantment can have (`ItemEnchantments.MAX_LEVEL`).
    pub const MAX_LEVEL: u32 = 255;

    /// The enchantments and their levels, in order (`keySet` and `getLevel`).
    pub fn iter(&self) -> impl Iterator<Item = (&Identifier, u32)> {
        self.entries.iter().map(|(id, level)| (id, *level))
    }
    /// How many enchantments there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// `getLevel`: `enchantment`'s level, 0 when it isn't there.
    pub fn level(&self, enchantment: &Identifier) -> u32 {
        self.find(enchantment).map_or(0, |i| self.entries[i].1)
    }
    /// `set`: gives `enchantment` `level`, at most [`Self::MAX_LEVEL`]; 0
    /// removes it.
    pub fn set(&mut self, enchantment: Identifier, level: u32) {
        let level = level.min(Self::MAX_LEVEL);
        match (self.find(&enchantment), level) {
            (Some(i), 0) => {
                self.entries.remove(i);
            }
            (Some(i), _) if self.entries[i].1 == level => return,
            (Some(i), _) => self.entries[i].1 = level,
            (None, 0) => return,
            (None, _) => self.entries.push((enchantment, level)),
        }
        self.changed = true;
    }
    /// `upgrade`: raises `enchantment` to `level` (at most
    /// [`Self::MAX_LEVEL`]) when that is higher than its own; 0 does nothing.
    pub fn upgrade(&mut self, enchantment: Identifier, level: u32) {
        if level > self.level(&enchantment) {
            self.set(enchantment, level);
        }
    }
    /// `removeIf`: keeps only the enchantments `keep` accepts. Whether any
    /// went.
    pub fn retain(&mut self, mut keep: impl FnMut(&Identifier, u32) -> bool) -> bool {
        let len = self.entries.len();
        self.entries.retain(|(id, level)| keep(id, *level));
        let removed = self.entries.len() != len;
        self.changed |= removed;
        removed
    }
    pub(crate) fn find(&self, enchantment: &Identifier) -> Option<usize> {
        self.entries.iter().position(|(id, _)| id == enchantment)
    }
}

/// `ItemDisplayContext`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ItemDisplayContext {
    /// No transform; the model as authored.
    None,
    /// Held in the left hand, seen from outside.
    ThirdPersonLeftHand,
    /// Held in the right hand, seen from outside.
    ThirdPersonRightHand,
    /// Held in the left hand, seen by its owner.
    FirstPersonLeftHand,
    /// Held in the right hand, seen by its owner.
    FirstPersonRightHand,
    /// Worn on the head.
    Head,
    /// Drawn in an inventory slot.
    Gui,
    /// Dropped as an item entity.
    Ground,
    /// Hung in an item frame.
    Fixed,
    /// Standing on a shelf.
    OnShelf,
}

impl ItemDisplayContext {
    /// Every context, in ordinal order (vanilla's id is the index).
    pub const ALL: [Self; 10] = [
        Self::None,
        Self::ThirdPersonLeftHand,
        Self::ThirdPersonRightHand,
        Self::FirstPersonLeftHand,
        Self::FirstPersonRightHand,
        Self::Head,
        Self::Gui,
        Self::Ground,
        Self::Fixed,
        Self::OnShelf,
    ];
    /// `getSerializedName`: `none`, `thirdperson_lefthand`, ... `on_shelf`.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ThirdPersonLeftHand => "thirdperson_lefthand",
            Self::ThirdPersonRightHand => "thirdperson_righthand",
            Self::FirstPersonLeftHand => "firstperson_lefthand",
            Self::FirstPersonRightHand => "firstperson_righthand",
            Self::Head => "head",
            Self::Gui => "gui",
            Self::Ground => "ground",
            Self::Fixed => "fixed",
            Self::OnShelf => "on_shelf",
        }
    }
    /// The context with that name, or None for one vanilla doesn't have.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// `AbstractArrow.Pickup`: who may pick an arrow up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ArrowPickup {
    /// Disallowed.
    Disallowed,
    /// Allowed.
    Allowed,
    /// Creative only.
    CreativeOnly,
}

/// One item cooldown a player waits on (`ItemCooldowns.cooldowns`).
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ItemCooldown {
    /// The cooldown group: an item id, or the group an item's
    /// `use_cooldown` component names.
    pub group: String,
    /// Ticks until the cooldown ends.
    pub ticks_left: i32,
}

/// Tool action requested by a block interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ToolAbility {
    /// An axe strips a log.
    AxeStrip,
    /// An axe scrapes oxidation off copper.
    AxeScrape,
    /// An axe removes wax from copper.
    AxeWaxOff,
    /// A shovel makes a path block.
    ShovelFlatten,
    /// A shovel puts out a campfire.
    ShovelDouse,
    /// A hoe tills soil.
    HoeTill,
}

/// Read-only context for a tool modification.
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ToolUse {
    /// The acting entity ID, if present.
    pub player: Option<u32>,
    /// The hand holding the tool.
    pub hand: crate::client::first_person::Hand,
    /// The clicked block position.
    pub pos: [i32; 3],
    /// The clicked face's direction.
    pub face: [i32; 3],
    /// The click position in world coordinates.
    pub click: [f64; 3],
    /// The action being considered.
    pub ability: ToolAbility,
    /// Whether this is a simulation. Preview calls pass true; actual use passes false.
    pub simulated: bool,
}

/// A scoped action's result. The returned stacks belong to the caller, not to the fake's
/// restored inventory. World changes and normal action hooks have already run.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ScopedActionResult {
    /// What the existing player action path did, including a stopped fake-player effect.
    pub outcome: ActionOutcome,
    /// The remaining main-hand stack, if nonempty.
    pub held: Option<ItemStack>,
    /// Other nonempty inventory stacks, then offhand, carried stack and armor, in that order.
    pub other: Vec<ItemStack>,
}

impl ScopedActionResult {
    /// The outcome and inventory returned by a scoped action.
    pub fn new(outcome: ActionOutcome, held: Option<ItemStack>, other: Vec<ItemStack>) -> Self {
        Self {
            outcome,
            held,
            other,
        }
    }
}

/// The key of `InteractionKeyMappingTriggered`: what the player's click was about to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum InteractionKey {
    /// `key.attack`: an attack, or a held key digging on.
    Attack,
    /// `key.use`: once for each hand tried, the main hand first.
    Use,
    /// `key.pickItem`: only with a block or entity aimed at.
    PickBlock,
}

/// What caused a client interaction mapping to act on this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum InteractionAttempt {
    /// A queued press of the mapping.
    Press,
    /// A repeat or continuation from a mapping that is still held.
    Held,
}

/// A proposed window-side block mining operation.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game fills all fields; mods only read the snapshot
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientMiningPrediction {
    /// The targeted block.
    pub pos: [i32; 3],
    /// The outward unit normal of the hit face.
    pub face: [i32; 3],
    /// Whether this operation begins a break rather than advancing a retained target.
    pub starting: bool,
    /// The proposed resulting break fraction, from zero to one.
    pub progress: f32,
}

/// The behaviour about to run in `UseItemOnBlockEvent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum UsePhase {
    /// `IItemExtension.onItemUseFirst`, before the item's Part and the
    /// block's behaviour.
    ItemBeforeBlock,
    /// `BlockBehaviour.useItemOn`, before the block's Part and template.
    Block,
    /// `Item.useOn`, after the block passed and before the item's Part.
    ItemAfterBlock,
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// A player's item use, including after target fallthrough (`RightClickItem`).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ItemClick<'a> {
    /// The side the click is handled on.
    pub side: LogicalSide,
    /// The dimension the player is in.
    pub dimension: LevelDimension,
    /// The block containing the clicking player's feet (`getPos`).
    pub pos: [i32; 3],
    /// The hand that holds the stack.
    pub hand: Hand,
    /// The stack in that hand.
    pub stack: &'a ItemStack,
    /// The clicking player's UUID.
    pub player: u128,
    /// The clicking player's network id.
    pub player_entity_id: i32,
}

/// Vanilla's `InteractionResult`, as a hook answers it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum InteractionResult {
    /// `SUCCESS`: the action is used and the hand swings on the clicking
    /// client (`SwingSource.PREDICTED`).
    Success,
    /// `SUCCESS_SERVER`: the action is used and the server swings the hand.
    SuccessServer,
    /// `CONSUME`: the action is used, with no swing.
    Consume,
    /// `FAIL`: nothing happens and the click ends.
    Fail,
    /// `PASS`: nothing happens and the click goes on.
    #[default]
    Pass,
    /// `TRY_WITH_EMPTY_HAND`: for a block's `use_item_on`, try its
    /// `use_without_item` next, on the main hand only.
    TryWithEmptyHand,
}

impl InteractionResult {
    /// `consumesAction`: the click is used up.
    pub fn consumes_action(self) -> bool {
        matches!(self, Self::Success | Self::SuccessServer | Self::Consume)
    }
}
