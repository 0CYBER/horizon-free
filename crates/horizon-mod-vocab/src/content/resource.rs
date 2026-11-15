//! NeoForge 26.3's transfer API ( §7): a resource is what a handler holds, without an amount
//! ([`ItemResource`]), and a [`ResourceHandler`] is the indexed contents of one holder, written
//! only inside a transaction.
use crate::{
    Capability, Host, HostTransaction, ItemStack, ServerHost, Tag, Transaction, backend_of,
};
use horizon_core::nbt::JavaString;
use std::fmt;

/// A data component's id, `namespace:path` (`minecraft:damage`, `mymod:charge`), checked once
/// where a mod names it: what [`ItemResource`]'s component methods take, as NeoForge's take a
/// `DataComponentType`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ComponentId(Box<str>);

impl ComponentId {
    /// `id` as a component id, or None where it isn't a namespaced identifier
    /// (`Identifier.parse`'s characters, a namespace and a path): `Damage`, `!minecraft:food`
    /// and `my mod:charge` aren't.
    pub fn new(id: &str) -> Option<Self> {
        horizon_core::identifier::Identifier::parse_namespaced(id)?;
        Some(Self(id.into()))
    }
    /// The id, as saved (`minecraft:damage`).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// no wire: static registration identity; mod-local storage cannot be reconstructed from a wire string.
/// An enchantment effect component type (`EnchantmentEffectComponents`): what an enchantment's
/// `effects` are keyed by, and what [`ItemResource::has_enchantment_effect`] asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct EnchantmentEffect(&'static str);

impl EnchantmentEffect {
    /// `id` as an effect type (a mod's own, as `mymod:soulbound`), or None where it isn't a
    /// namespaced identifier.
    pub fn new(id: &'static str) -> Option<Self> {
        horizon_core::identifier::Identifier::parse_namespaced(id)?;
        Some(Self(id))
    }
    /// The id (`minecraft:prevent_armor_change`).
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

/// `EnchantmentEffectComponents.PREVENT_ARMOR_CHANGE`: worn armour with it stays on outside
/// creative (Curse of Binding's effect, and any data pack enchantment's that carries it).
pub const PREVENT_ARMOR_CHANGE: EnchantmentEffect =
    EnchantmentEffect("minecraft:prevent_armor_change");

/// The effective rarity of an item stack, after its enchantment upgrade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ItemRarity {
    /// The ordinary rarity.
    Common,
    /// An uncommon item.
    Uncommon,
    /// A rare item, including enchanted common and uncommon items.
    Rare,
    /// An epic item, including enchanted rare items.
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
    /// The serialized vanilla rarity name.
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
#[derive(horizon_mod_api_macros::Wire)]
pub struct ItemBar {
    /// Width out of thirteen pixels.
    pub width: i32,
    /// RGB color of the filled bar.
    pub color: u32,
}

/// Effective attack range for an actor, including the component's hitbox margin.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: a value record
#[derive(horizon_mod_api_macros::Wire)]
pub struct ItemAttackRange {
    /// Minimum reach before the margin.
    pub min: f32,
    /// Maximum reach before the margin.
    pub max: f32,
    /// Margin added to either end of the range.
    pub hitbox_margin: f32,
}
impl ItemAttackRange {
    /// Inclusive vanilla float bounds compared with a double distance and optional server buffer.
    pub fn contains(self, distance: f64, buffer: f64) -> bool {
        distance >= f64::from(self.min - self.hitbox_margin) - buffer
            && distance <= f64::from(self.max + self.hitbox_margin) + buffer
    }
}

/// Owned damage source information for a stack attack or a source-specific item query.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a value record
#[derive(horizon_mod_api_macros::Wire)]
pub struct ItemDamageSource {
    /// Registry identifier of the damage type.
    pub damage_type: crate::Identifier,
    /// Entity directly delivering the damage, if any.
    pub direct: Option<crate::EntityRef>,
    /// Entity responsible for the damage, if any.
    pub causing: Option<crate::EntityRef>,
    /// Explicit source position, if any (melee sources use their direct entity's position).
    pub position: Option<[f64; 3]>,
}

/// An item and its components, without a count, as NeoForge's `ItemResource`: what a slot holds.
/// An empty slot holds none.
// no wire: decoding calls ItemResource::of to normalize the incoming stack count to one; a structural derive would preserve a non-unit wire count.
#[derive(Clone)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ItemResource(pub(crate) ItemStack);

impl ItemResource {
    /// What `stack` is, whatever its count.
    pub fn of(stack: &ItemStack) -> Self {
        let mut stack = stack.clone();
        stack.set_count(1).expect("one item is nonempty");
        Self(stack)
    }
    /// The item's process-local identity.
    pub fn item(&self) -> crate::ItemKey {
        self.0.item()
    }
    /// Borrow the resource's single-item stack without cloning its component patch.
    pub fn as_stack(&self) -> &ItemStack {
        &self.0
    }
    /// `count` of it as a stack, or none for an empty result.
    pub fn to_stack(&self, count: u32) -> Option<ItemStack> {
        let mut stack = self.0.clone();
        stack.set_count(count).ok()?;
        Some(stack)
    }
    /// Whether `stack` is this item with these components.
    pub fn matches(&self, stack: &ItemStack) -> bool {
        self.0.is_stackable_with(stack)
    }
    /// This item's own value of component `id` in its patch, as saved (`minecraft:damage` on a
    /// worn sword); none where it keeps the item's default or has the component removed.
    pub fn component(&self, id: &ComponentId) -> Option<Tag> {
        if id.as_str() == "minecraft:damage" {
            return self
                .0
                .damage_component()
                .map(crate::item::item_stack::unsigned_tag);
        }
        self.0
            .components()?
            .as_nbt()
            .get(&JavaString::from(id.as_str()))
            .cloned()
    }
    /// This item's value of component `id` (`ItemResource.get`): its own, else the item's default
    /// (`host.item_default_component`), none where it has the component removed or the item
    /// hasn't one.
    pub fn value(&self, host: &dyn Host, id: &ComponentId) -> Option<Tag> {
        self.value_id(host, id.as_str())
    }
    pub(crate) fn value_id(&self, host: &dyn Host, id: &str) -> Option<Tag> {
        if let Some(patch) = self.0.components().map(|patch| patch.as_nbt()) {
            if let Some(own) = patch.get(&JavaString::from(id)) {
                return Some(own.clone());
            }
            if patch.contains_key(&JavaString::from(format!("!{id}").as_str())) {
                return None;
            }
        }
        if id == "minecraft:damage"
            && let Some(damage) = self.0.damage_component()
        {
            return Some(crate::item::item_stack::unsigned_tag(damage));
        }
        host.item_default_component(self.item(), id)
    }
    /// `ItemStack.isDamageableItem`: maximum damage and damage components are
    /// present, and the unbreakable component is absent, after this stack's patch.
    pub fn damageable(&self, host: &dyn Host) -> bool {
        backend_of(host).is_some_and(|backend| backend.item_resource_damageable(&self.0))
    }
    fn enchanted(&self, host: &dyn Host) -> bool {
        matches!(self.value_id(host, "minecraft:enchantments"), Some(Tag::Compound(levels)) if !levels.is_empty())
    }
    /// `ItemStack.hasFoil`: the effective glint override wins, otherwise a
    /// item-specific foil applies (lodestone compass or nonempty enchantments).
    pub fn foil(&self, host: &dyn Host) -> bool {
        backend_of(host).is_some_and(|backend| backend.item_resource_foil(&self.0))
    }
    /// `ItemStack.getRarity`: the effective rarity, upgraded for a nonempty
    /// enchantments component; removing rarity restores common before the upgrade.
    pub fn rarity(&self, host: &dyn Host) -> ItemRarity {
        let base = match self.value_id(host, "minecraft:rarity") {
            Some(Tag::String(value)) => value.to_string_lossy(),
            _ => String::new(),
        };
        ItemRarity::of(&base, self.enchanted(host))
    }
    /// `ItemStack.isEnchantable`: the effective enchantable component is
    /// present and the enchantments component is present but empty.
    pub fn enchantable(&self, host: &dyn Host) -> bool {
        self.value_id(host, "minecraft:enchantable").is_some()
            && matches!(self.value_id(host, "minecraft:enchantments"), Some(Tag::Compound(levels)) if levels.is_empty())
    }
    /// `ItemStack.isValidRepairItem`: whether `material` belongs to the
    /// effective repairable component's item holder set. The comparison is by
    /// item identity/tag, not its count, damage or components.
    pub fn repairable(&self, host: &dyn Host, material: &ItemResource) -> bool {
        backend_of(host)
            .is_some_and(|backend| backend.item_resource_repairable(&self.0, material.item()))
    }
    /// `Item.getUseDuration` and its item overrides, in ticks, including the
    /// same use-duration Part as gameplay. Uses the host's current acting-player
    /// context for Parts; vanilla's current implementations need no player argument.
    /// None for an unknown item or a host without the engine backend.
    pub fn use_duration(&self, host: &dyn Host) -> Option<i32> {
        backend_of(host)?.item_resource_use_duration(&self.0)
    }
    /// `IItemExtension.getCreatorModId`: the mod that made this stack, for a
    /// recipe viewer or a tooltip to name. The item id's namespace, unless the
    /// item's `creator_mod_id` Part answers another mod or none. None for an
    /// unknown item or a host without the engine backend.
    pub fn creator_mod_id(&self, host: &dyn Host) -> Option<String> {
        backend_of(host)?.item_resource_creator_mod_id(&self.0)
    }
    /// `ItemStack.isBarVisible`, `getBarWidth` and `getBarColor`: the visible
    /// vanilla durability or bundle bar, after effective components. None when
    /// no bar is visible. Client drawing hooks can later replace this bar.
    pub fn bar(&self, host: &dyn Host) -> Option<ItemBar> {
        backend_of(host)?.item_resource_bar(&self.0)
    }
    /// `AttackRange.defaultFor` or this stack's effective attack-range component for `attacker`.
    /// Returns None for an unavailable actor or item. Does not equip this stack.
    pub fn entity_attack_range(
        &self,
        host: &dyn Host,
        attacker: crate::LivingRef,
    ) -> Option<ItemAttackRange> {
        backend_of(host)?.item_resource_entity_attack_range(&self.0, attacker)
    }
    /// `ItemStack.getDamageSource`: component override, item-specific source (mace smash),
    /// then the attacker's ordinary melee source. None for an unavailable actor or damage type.
    pub fn damage_source(
        &self,
        host: &dyn Host,
        attacker: crate::LivingRef,
    ) -> Option<ItemDamageSource> {
        backend_of(host)?.item_resource_damage_source(&self.0, attacker)
    }
    /// `ItemStack.canBeHurtBy`: whether this item itself can be hurt by `source`, after
    /// effective damage-resistant components. This is not wearer protection.
    /// None for an unknown item or damage type.
    pub fn can_be_hurt_by(&self, host: &dyn Host, source: &ItemDamageSource) -> Option<bool> {
        backend_of(host)?.item_resource_can_be_hurt_by(&self.0, source)
    }
    /// Preview the stack's attack bonus against `target`, including enchantments and mace
    /// fall damage. Returns the additional damage above `base_damage`. Uses a cloned RNG
    /// snapshot: a read never consumes world RNG. This is a preview under the current state; a later attack may
    /// roll differently. The queried stack is never equipped. None for unavailable handles
    /// or actors and targets outside the host's active level.
    pub fn attack_damage_bonus(
        &self,
        host: &dyn Host,
        attacker: crate::LivingRef,
        target: crate::EntityRef,
        base_damage: f32,
    ) -> Option<f32> {
        backend_of(host)?.item_resource_attack_damage_bonus(&self.0, attacker, target, base_damage)
    }
    /// Standard tooltip text, with explicit advanced and creative flags and the client's
    /// current viewer context. Uses the same line builder as inventory tooltips and skips
    /// mod callbacks, so a tooltip hook can safely request another item's base lines.
    /// Returns None without a client backend, or when the item cannot be resolved.
    pub fn tooltip(
        &self,
        host: &dyn crate::ClientHost,
        advanced: bool,
        creative: bool,
    ) -> Option<Vec<crate::TooltipElement>> {
        backend_of(host)?.item_resource_tooltip(&self.0, advanced, creative)
    }
    /// Roll compatible enchantments from the named enchantment tag with the active server
    /// level's random source. Returns a detached resource, converting a book to an enchanted
    /// book when selected. None means the request was refused or the roll chose nothing.
    /// A valid roll consumes random draws even if the caller discards the result.
    /// To store the result, use the normal transactional item-place or capability write.
    pub fn enchant_randomly(
        &self,
        host: &mut dyn ServerHost,
        level: crate::LevelRef,
        power: i32,
        pool: &crate::Identifier,
    ) -> Result<Option<Self>, crate::StackWriteError> {
        let Some(backend) = crate::backend_mut_of(host) else {
            return Ok(None);
        };
        Ok(backend
            .item_resource_enchant_randomly(&self.0, level, power, pool)?
            .map(Self))
    }
    /// Whether an enchantment on this item has effect `effect` (`EnchantmentHelper.has`), by the
    /// enchantments' definitions, data packs' included, not their ids.
    pub fn has_enchantment_effect(&self, host: &dyn Host, effect: EnchantmentEffect) -> bool {
        backend_of(host)
            .is_some_and(|backend| backend.item_has_enchantment_effect(&self.0, effect.as_str()))
    }
    /// The slot this item is equipped to (`Equippable.slot`): `head`, `chest`, `legs`, `feet`,
    /// `body`, `mainhand`, `offhand` or `saddle`, by its `equippable` component after the stack's
    /// own patch; none where it is not equippable. Whether a given entity may wear it is another
    /// question, not answered here.
    pub fn equippable_slot(&self, host: &dyn Host) -> Option<String> {
        backend_of(host)?.item_equippable_slot(&self.0)
    }
    /// This item with component `id` set to `value` (`ItemResource.with`, as vanilla's
    /// `PatchedDataComponentMap.set`): a removal of it dropped, and the stack's own value too
    /// where `value` is the item's default (`host.item_default_component`), so the stack is
    /// the plain item again. A stack changes where it lives by an exchange of one of it for
    /// this: `place.exchange(host, tx, &changed, 1)`, which splits it off a bigger stack, in the
    /// transaction.
    pub fn with_component(
        &self,
        host: &dyn Host,
        id: &ComponentId,
        value: Tag,
    ) -> Result<Self, crate::StackError> {
        let mut stack = self.0.clone();
        let own = !host
            .item_default_component(self.item(), id.as_str())
            .is_some_and(|default| same_value(id, &default, &value));
        if own || id.as_str() == "minecraft:damage" {
            stack.set_component_value(id.as_str(), value)?;
        }
        if !own {
            stack.remove_component_value(id.as_str(), false);
        }
        Ok(Self(stack))
    }
    /// This item without component `id` (`ItemResource.without`, as vanilla's
    /// `PatchedDataComponentMap.remove`): its own value dropped and, where the item has the
    /// component by default (`host.item_default_component`), a removal recorded in its patch
    /// (`"!minecraft:max_damage": {}`), saved and sent as vanilla's are.
    pub fn without_component(&self, host: &dyn Host, id: &ComponentId) -> Self {
        let mut stack = self.0.clone();
        stack.remove_component_value(
            id.as_str(),
            host.item_default_component(self.item(), id.as_str())
                .is_some(),
        );
        Self(stack)
    }
}

/// Components the game keeps as the NBT given (`CustomData`, `TypedEntityData`), equal only
/// type for type (`CompoundTag.equals`).
const RAW_NBT: [&str; 4] = [
    "minecraft:custom_data",
    "minecraft:bucket_entity_data",
    "minecraft:entity_data",
    "minecraft:block_entity_data",
];

/// Whether `a` and `b` are one value of component `id` (`Objects.equals` on the decoded
/// values, `PatchedDataComponentMap.set`). A vanilla component other than [`RAW_NBT`] is
/// compared as its codec reads the item report's JSON beside a mod's NBT ([`same_coded`]);
/// a raw NBT one, or a mod's, whose codec the game doesn't know, type for type.
fn same_value(id: &ComponentId, a: &Tag, b: &Tag) -> bool {
    let coded = id.as_str().starts_with("minecraft:") && !RAW_NBT.contains(&id.as_str());
    if coded { same_coded(a, b) } else { a == b }
}

/// Whether `a` and `b` read as one value through a codec (`NbtOps`): integers of any width
/// by exact value, a `Float` beside any number at `f32` precision and a `Double` at `f64`,
/// an array as the list of its numbers, and lists and compounds entry by entry.
fn same_coded(a: &Tag, b: &Tag) -> bool {
    enum Number {
        Integer(i64),
        Float(f32),
        Double(f64),
    }
    let number = |tag: &Tag| match *tag {
        Tag::Byte(v) => Some(Number::Integer(v.into())),
        Tag::Short(v) => Some(Number::Integer(v.into())),
        Tag::Int(v) => Some(Number::Integer(v.into())),
        Tag::Long(v) => Some(Number::Integer(v)),
        Tag::Float(bits) => Some(Number::Float(f32::from_bits(bits))),
        Tag::Double(bits) => Some(Number::Double(f64::from_bits(bits))),
        _ => None,
    };
    if let (Some(a), Some(b)) = (number(a), number(b)) {
        let float = |n: &Number| match *n {
            Number::Integer(v) => v as f32,
            Number::Float(v) => v,
            Number::Double(v) => v as f32,
        };
        let double = |n: &Number| match *n {
            Number::Integer(v) => v as f64,
            Number::Float(v) => v.into(),
            Number::Double(v) => v,
        };
        return match (&a, &b) {
            (Number::Integer(a), Number::Integer(b)) => a == b,
            (Number::Float(_), _) | (_, Number::Float(_)) => float(&a) == float(&b),
            _ => double(&a) == double(&b),
        };
    }
    let list = |tag: &Tag| -> Option<Vec<Tag>> {
        Some(match tag {
            Tag::List(items) => items.clone(),
            Tag::ByteArray(items) => items.iter().map(|&v| Tag::Byte(v as i8)).collect(),
            Tag::IntArray(items) => items.iter().map(|&v| Tag::Int(v)).collect(),
            Tag::LongArray(items) => items.iter().map(|&v| Tag::Long(v)).collect(),
            _ => return None,
        })
    };
    if let (Some(a), Some(b)) = (list(a), list(b)) {
        return a.len() == b.len() && a.iter().zip(&b).all(|(a, b)| same_coded(a, b));
    }
    match (a, b) {
        (Tag::Compound(a), Tag::Compound(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_some_and(|b| same_coded(a, b)))
        }
        _ => a == b,
    }
}

impl PartialEq for ItemResource {
    fn eq(&self, other: &Self) -> bool {
        self.matches(&other.0)
    }
}
impl Eq for ItemResource {}

/// By what [`PartialEq`] compares: the item, its damage and its component patch, so a resource
/// keys a map (a store's kinds) without a count ever entering the key.
impl std::hash::Hash for ItemResource {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.item().hash(state);
        self.0.damage_component().hash(state);
        self.0.components().map(|c| c.as_nbt()).hash(state);
    }
}

impl fmt::Debug for ItemResource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ItemResource").field(&self.0.item()).finish()
    }
}

/// The contents of one holder (a block entity as one side reaches it, an item where it lives),
/// numbered from 0, as NeoForge's `ResourceHandler`: a provider makes one for its holder, so it
/// knows where its contents live. It holds no borrow of the game: each call is given the host,
/// and its state is the game's (slots, a saved compound, attachments, a stack's components), so
/// the transaction `tx` a write runs in puts it back on an abort. Amounts never exceed what was
/// asked; a handler answering more is a mod bug, and the move it was part of is aborted.
pub trait ResourceHandler<R> {
    /// Set the container limit transactionally, accepting 1..=99 (vanilla's upper bound).
    /// Existing oversized stacks remain extractable. The game persists this with block entity data.
    /// Unsupported handlers return false without mutation.
    fn set_max_stack_size(
        &self,
        _host: &mut dyn ServerHost,
        _tx: &Transaction,
        _limit: u32,
    ) -> bool {
        false
    }

    /// How many indices it has.
    fn size(&self, host: &mut dyn ServerHost) -> usize;
    /// What index `index` holds, if anything.
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<R>;
    /// How much of it.
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32;
    /// How much of `resource` index `index` holds when it holds that, whatever it holds now
    /// (`getCapacityAsLong`). Capacity is not free space or permission to insert: validity,
    /// current contents and access rules can still refuse it.
    fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: &R) -> u32;
    /// Whether `resource` may go in index `index` at all (`isValid`). The default uses capacity;
    /// a handler with separate acceptance rules overrides this and enforces them in `insert`.
    fn is_valid(&self, host: &mut dyn ServerHost, index: usize, resource: &R) -> bool {
        self.capacity(host, index, resource) > 0
    }
    /// Puts up to `amount` of `resource` into index `index`: how much went in.
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> u32;
    /// Takes up to `amount` of `resource` out of index `index`: how much came out. None of
    /// another resource.
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> u32;
    /// Puts up to `amount` of `resource` in wherever it goes (NeoForge's `insert` without an
    /// index): by default index by index from 0. How much went in.
    fn insert_any(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &R,
        amount: u32,
    ) -> u32 {
        let mut went = 0u32;
        for index in 0..self.size(host) {
            if went >= amount {
                break;
            }
            went = went.saturating_add(self.insert(host, tx, index, resource, amount - went));
        }
        went
    }
    /// Takes up to `amount` of `resource` out from wherever it is (`extract` without an index):
    /// by default index by index from 0. How much came out.
    fn extract_any(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &R,
        amount: u32,
    ) -> u32 {
        let mut came = 0u32;
        for index in 0..self.size(host) {
            if came >= amount {
                break;
            }
            came = came.saturating_add(self.extract(host, tx, index, resource, amount - came));
        }
        came
    }
}

/// A handler answered more than it was asked (`move_resources`): a mod bug, warned once naming
/// the mod. Returned from a `host.transaction` closure with `?`, it aborts the whole move, so
/// neither end keeps any of it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct Overanswered {
    /// The mod whose handler answered, or "the game".
    pub owner: String,
}

/// Moves up to `max` of what `filter` accepts out of `from` into wherever `to` takes it, index by
/// index from 0 (NeoForge's `ResourceHandlerUtil.move`), in transaction `tx`: how much moved.
/// Each move is all or nothing, in a transaction of its own: what `to` won't take stays in
/// `from`, and a move `to` takes only part of is tried again with that part (a cauldron gives
/// whole thirds). A handler answering more than it was asked has its move undone and ends the
/// moving with [`Overanswered`].
pub fn move_resources<R>(
    host: &mut dyn ServerHost,
    from: &Capability<dyn ResourceHandler<R>>,
    to: &Capability<dyn ResourceHandler<R>>,
    filter: impl Fn(&R) -> bool,
    max: u32,
    tx: &Transaction,
) -> Result<u32, Overanswered> {
    let _ = tx;
    let mut moved = 0;
    for index in 0..from.size(host) {
        while moved < max {
            let Some(resource) = from.resource(host, index).filter(|r| filter(r)) else {
                break;
            };
            let want = (max - moved).min(from.amount(host, index));
            let went = move_once(
                host,
                Ends {
                    from,
                    to,
                    at: &index,
                },
                want,
                |host, tx, want| from.extract(host, tx, index, &resource, want),
                |host, tx, out| to.insert_any(host, tx, &resource, out),
            )?;
            if went == 0 {
                break;
            }
            moved += went;
        }
    }
    Ok(moved)
}

/// The two ends of a move and where it is, for warnings.
pub(crate) struct Ends<'a, A: ?Sized, B: ?Sized> {
    pub from: &'a Capability<A>,
    pub to: &'a Capability<B>,
    pub at: &'a dyn fmt::Debug,
}

/// One move of up to `want` by `extract` out of `ends.from` and `insert` into `ends.to`, all or
/// nothing in a transaction of its own, nested in the caller's: how much moved. A move `to` takes
/// only part of is undone and tried again with that part; a handler answering more than it was
/// asked has its move undone and gives [`Overanswered`]. The one step of `move_resources`,
/// `move_energy` and the 64-bit `transfer`.
pub(crate) fn move_once<A: ?Sized, B: ?Sized, N: Copy + PartialOrd + Default>(
    host: &mut dyn ServerHost,
    ends: Ends<'_, A, B>,
    want: N,
    extract: impl Fn(&mut dyn ServerHost, &Transaction, N) -> N,
    insert: impl Fn(&mut dyn ServerHost, &Transaction, N) -> N,
) -> Result<N, Overanswered> {
    let Ends { from, to, at } = ends;
    // `Err(Ok(took))`: `to` took only that much; `Err(Err(_))`: a handler answered more than it
    // was asked.
    let step = |host: &mut dyn ServerHost, want: N| {
        host.transaction(|host, tx| {
            let Some(out) = from.checked(extract(host, tx, want), want, at) else {
                return Err(Err(Overanswered {
                    owner: from.owner(),
                }));
            };
            match to.checked(insert(host, tx, out), out, at) {
                None => Err(Err(Overanswered { owner: to.owner() })),
                Some(took) if took == out => Ok(out),
                Some(took) => Err(Ok(took)),
            }
        })
    };
    match step(host, want) {
        Err(Ok(took)) if took > N::default() => step(host, took),
        answer => answer,
    }
    .or_else(|answer| answer.map(|_| N::default()))
}

#[cfg(test)]
mod rarity_contract_tests {
    use super::ItemRarity;
    #[test]
    fn item_rarity_case_order_and_names() {
        for (index, rarity, name) in [
            (0, ItemRarity::Common, "common"),
            (1, ItemRarity::Uncommon, "uncommon"),
            (2, ItemRarity::Rare, "rare"),
            (3, ItemRarity::Epic, "epic"),
        ] {
            assert_eq!(rarity as usize, index);
            assert_eq!(rarity.as_str(), name);
        }
    }
}
