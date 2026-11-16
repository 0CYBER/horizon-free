//! Owned item values. Registry-dependent validation belongs to the game's write boundary.
use crate::ItemKey;
use horizon_core::nbt::{Compound, JavaString, Tag};
use std::num::NonZeroU32;
use std::sync::Arc;

/// A typed descriptor for one component in an item's component patch.
/// Implementations own the conversion between their value and Minecraft NBT.
pub trait Component {
    /// The value returned by this descriptor.
    type Value;
    /// Canonical component name, including its namespace.
    fn id(&self) -> &str;
    /// Decode a component value, refusing an incompatible NBT representation.
    fn decode(&self, value: &Tag) -> Option<Self::Value>;
    /// Encode a component value.
    fn encode(&self, value: Self::Value) -> Tag;
}

/// An owned component patch, independent of the engine's component representation.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Components {
    patch: Compound,
}

impl Components {
    /// Borrow the raw component patch at an NBT boundary.
    pub fn as_nbt(&self) -> &Compound {
        &self.patch
    }
}
impl From<Compound> for Components {
    fn from(patch: Compound) -> Self {
        Self { patch }
    }
}
impl From<&Components> for Compound {
    fn from(value: &Components) -> Self {
        value.patch.clone()
    }
}

/// A nonempty, owned item value. Maxima are checked when the game accepts a write,
/// not by this record: item definitions and effective component overrides belong to the game.
/// The component patch is shared copy-on-write: `Clone` counts a reference instead of
/// copying it, the first write through a shared stack copies the patch once, and equality
/// of two stacks that still share it is a pointer comparison. An absent patch allocates nothing.
///
/// An untouched mutable stack hook argument retains its engine value without
/// write validation, even when the engine value exceeds effective maxima. Equality
/// with the snapshot passed to the listener determines whether it changed; a changed
/// argument is validated in full before committing any hook outputs.
/// A refused hook output retains its captured engine value. The clicked-slot bridge also
/// stages the slot, cursor and cancellation flags together. Independent host effects,
/// including storage writes, commands and messages, are not undone by output validation.
// no wire: decoding rejects zero counts through new and validates component patches through set_components; damage is applied through set_damage.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ItemStack {
    item: ItemKey,
    count: NonZeroU32,
    damage: Option<u32>,
    components: Option<Arc<Components>>,
}

/// A stack operation could not represent the requested count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum StackError {
    /// Empty stacks are represented by `None`, not by a zero-count item.
    Empty,
    /// Adding items would overflow the record's count.
    CountOverflow,
    /// Damage must be a nonnegative integer representable by u32.
    InvalidDamage,
    /// A patch and the damage field (or a patch value and removal) disagree.
    ConflictingDamage,
}
impl std::fmt::Display for StackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("an item stack must be nonempty"),
            Self::CountOverflow => f.write_str("item stack count overflow"),
            Self::InvalidDamage => f.write_str("invalid item damage"),
            Self::ConflictingDamage => f.write_str("conflicting item damage"),
        }
    }
}
impl std::error::Error for StackError {}

/// Why the game refused to accept an owned stack at a write boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum StackWriteError {
    /// A text field failed component conversion before this write.
    Text(crate::text::TextFormError),
    /// The host has no game backend for this operation.
    Unavailable,
    /// The process-local key is not present in this game's item registry.
    UnknownItem,
    /// The count is above the effective stack limit.
    CountExceeded {
        /// Effective maximum, including component overrides.
        maximum: u32,
    },
    /// The damage is above the effective durability limit.
    DamageExceeded {
        /// Effective maximum, including component overrides.
        maximum: u32,
    },
    /// A maximum is malformed or negative, or a stack limit lies outside 1..=99.
    InvalidMaximum,
    /// An engine result contained a zero-count stack instead of an empty slot.
    EmptyEngineStack,
    /// An engine snapshot's component patch was malformed or conflicted with its damage.
    InvalidComponents {
        /// The record-level normalization refusal.
        reason: StackError,
    },
    /// A consume or death-protection component could not decode its opaque effect payload.
    InvalidEffectPayload,
    /// A stack carries both maximum durability and a stack limit above one.
    StackableDamageable,
}

impl From<crate::text::TextFormError> for StackWriteError {
    fn from(error: crate::text::TextFormError) -> Self {
        Self::Text(error)
    }
}

impl crate::NoBackend for StackWriteError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// The two nonempty portions of a consuming split. Either portion can be absent.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct StackSplit {
    /// Up to the requested number of items.
    pub taken: Option<ItemStack>,
    /// Items left after taking that portion.
    pub remainder: Option<ItemStack>,
}

impl ItemStack {
    /// Construct a nonempty stack. Registry maxima are validated by the game on writes.
    pub fn new(item: ItemKey, count: u32) -> Option<Self> {
        Some(Self {
            item,
            count: NonZeroU32::new(count)?,
            damage: None,
            components: None,
        })
    }
    /// Construct a single item without a component patch.
    pub fn of(item: ItemKey) -> Self {
        Self {
            item,
            count: NonZeroU32::MIN,
            damage: None,
            components: None,
        }
    }
    /// The process-local identity of this item.
    pub fn item(&self) -> ItemKey {
        self.item
    }
    /// The nonzero item count.
    pub fn count(&self) -> u32 {
        self.count.get()
    }
    /// Change the count without consulting the game. A refusal leaves the value unchanged.
    pub fn set_count(&mut self, count: u32) -> Result<(), StackError> {
        self.count = NonZeroU32::new(count).ok_or(StackError::Empty)?;
        Ok(())
    }
    /// Stored damage, or zero when no numeric damage override is present.
    /// A plain stack inherits its item's default. Some(0) is an explicit present zero;
    /// [`Self::clear_damage`] records removal of the default component instead.
    pub fn damage(&self) -> u32 {
        self.damage.unwrap_or(0)
    }
    /// The numeric damage override: None inherits the prototype unless the patch
    /// contains an explicit removal; Some(0) is a present zero-damage component.
    pub fn damage_component(&self) -> Option<u32> {
        self.damage
    }
    /// Set damage. The game checks the effective maximum when accepting this value.
    pub fn set_damage(&mut self, damage: u32) {
        self.damage = Some(damage);
        self.remove_patch_entries(&[JavaString::from("!minecraft:damage")]);
    }
    /// Remove the effective damage component, including an inherited default.
    /// The patch stores a real NBT removal, never a second numeric damage value.
    pub fn clear_damage(&mut self) {
        self.remove_component_value("minecraft:damage", true);
    }
    /// Whether a nonzero numeric damage override is stored on the stack.
    pub fn is_damaged(&self) -> bool {
        self.damage.is_some_and(|damage| damage != 0)
    }
    /// Add items. Overflow leaves the stack unchanged; registry maxima are checked on writes.
    pub fn grow(&mut self, count: u32) -> Result<(), StackError> {
        let count = self
            .count()
            .checked_add(count)
            .ok_or(StackError::CountOverflow)?;
        self.set_count(count)
    }
    /// Remove items, consuming the value so removing everything needs no empty sentinel.
    pub fn shrink(mut self, count: u32) -> Option<Self> {
        self.count = NonZeroU32::new(self.count().saturating_sub(count))?;
        Some(self)
    }
    /// Take up to `count` items. A full split moves the patch without cloning it.
    pub fn split(mut self, count: u32) -> StackSplit {
        if count == 0 {
            return StackSplit {
                taken: None,
                remainder: Some(self),
            };
        }
        if count >= self.count() {
            return StackSplit {
                taken: Some(self),
                remainder: None,
            };
        }
        let mut taken = self.clone();
        taken.count = NonZeroU32::new(count).expect("partial split is nonzero");
        self.count = NonZeroU32::new(self.count() - count).expect("partial split leaves items");
        StackSplit {
            taken: Some(taken),
            remainder: Some(self),
        }
    }
    /// Compare item identities, independent of count and components.
    pub fn same_item(&self, other: &Self) -> bool {
        self.item == other.item
    }
    /// Compare component patches and damage, independent of count and item identity.
    pub fn same_components(&self, other: &Self) -> bool {
        self.damage == other.damage && self.components == other.components
    }
    /// Whether these values have the same item and component patch.
    /// Capacity is a registry-dependent write-boundary check, not a promise of this predicate.
    pub fn is_stackable_with(&self, other: &Self) -> bool {
        self.same_item(other) && self.same_components(other)
    }
    /// Read a component with its typed descriptor.
    pub fn get<C: Component>(&self, component: &C) -> Option<C::Value> {
        if component.id() == "minecraft:damage" {
            return self
                .damage
                .and_then(|damage| component.decode(&unsigned_tag(damage)));
        }
        component.decode(
            self.components
                .as_ref()?
                .patch
                .get(&JavaString::from(component.id()))?,
        )
    }
    /// Replace a component value. Only stacks with a patch allocate component storage.
    pub fn set<C: Component>(&mut self, component: &C, value: C::Value) -> Result<(), StackError> {
        self.set_component_value(component.id(), component.encode(value))
    }
    pub(crate) fn set_component_value(&mut self, id: &str, value: Tag) -> Result<(), StackError> {
        if id == "minecraft:damage" {
            self.set_damage(damage_value(&value)?);
            return Ok(());
        }
        if id == "!minecraft:damage" {
            if !matches!(&value, Tag::Compound(value) if value.is_empty()) {
                return Err(StackError::InvalidDamage);
            }
            if self.damage.is_some() {
                return Err(StackError::ConflictingDamage);
            }
        }
        let patch = &mut Arc::make_mut(self.components.get_or_insert_with(Default::default)).patch;
        patch.remove(&JavaString::from(format!("!{id}").as_str()));
        patch.insert(JavaString::from(id), value);
        Ok(())
    }
    /// Remove an override, releasing the patch allocation when its last entry is removed.
    pub fn remove<C: Component>(&mut self, component: &C) {
        self.remove_component_value(component.id(), false);
    }
    pub(crate) fn remove_component_value(&mut self, id: &str, removal: bool) {
        if id == "minecraft:damage" {
            self.damage = None;
        }
        let removed = JavaString::from(format!("!{id}").as_str());
        if removal {
            let patch =
                &mut Arc::make_mut(self.components.get_or_insert_with(Default::default)).patch;
            patch.remove(&JavaString::from(id));
            patch.insert(removed, Tag::Compound(Compound::new()));
            return;
        }
        self.remove_patch_entries(&[JavaString::from(id), removed]);
    }
    /// Remove entries from an existing patch. A shared patch is copied only when one of
    /// the keys is present; the patch is released when its last entry is removed.
    fn remove_patch_entries(&mut self, keys: &[JavaString]) {
        let Some(components) = &mut self.components else {
            return;
        };
        if !keys.iter().any(|key| components.patch.contains_key(key)) {
            return;
        }
        let patch = &mut Arc::make_mut(components).patch;
        for key in keys {
            patch.remove(key);
        }
        if patch.is_empty() {
            self.components = None;
        }
    }
    /// The component patch, if this stack has overrides.
    pub fn components(&self) -> Option<&Components> {
        self.components.as_deref()
    }
    /// `ItemStack.applyComponents`: merge a patch without discarding other overrides.
    /// Incoming values replace removals and incoming removals replace values. If both
    /// are supplied for one component, removal wins (conflicting damage is refused).
    /// Damage is normalized before any write; a malformed patch leaves this stack unchanged.
    pub fn apply_components(&mut self, patch: &Compound) -> Result<(), StackError> {
        let damage = patch_damage(patch)?;
        for (id, value) in patch {
            let id = id.to_string_lossy();
            if id == "minecraft:damage" {
                continue;
            }
            if let Some(id) = id.strip_prefix('!') {
                self.remove_component_value(id, true);
            } else if !patch.contains_key(&JavaString::from(format!("!{id}").as_str())) {
                self.set_component_value(&id, value.clone())?;
            }
        }
        if let Some(damage) = damage {
            self.set_damage(damage);
        }
        Ok(())
    }
    /// Copy this component's own value or explicit removal from `other`, without
    /// consulting item defaults. If `other` inherits it, clear this stack's override too.
    /// An `id` that is not a namespaced component id (`Damage`, or a removal marker such as
    /// `!minecraft:damage`) changes nothing.
    pub fn copy_component_from(&mut self, other: &Self, id: &str) {
        if crate::ComponentId::new(id).is_none() {
            return;
        }
        if id == "minecraft:damage"
            && let Some(damage) = other.damage
        {
            self.set_damage(damage);
            return;
        }
        let patch = other.components().map(Components::as_nbt);
        let removed = JavaString::from(format!("!{id}").as_str());
        if patch.is_some_and(|patch| patch.contains_key(&removed)) {
            self.remove_component_value(id, true);
        } else if let Some(value) = patch.and_then(|patch| patch.get(&JavaString::from(id))) {
            self.set_component_value(id, value.clone())
                .expect("stored component is normalized");
        } else {
            self.remove_component_value(id, false);
        }
    }
    /// Replace the component patch at an NBT conversion boundary.
    /// Damage is normalized into the scalar field. Malformed or conflicting values
    /// refuse the entire replacement and leave the stack unchanged.
    pub fn set_components(&mut self, mut patch: Compound) -> Result<(), StackError> {
        let damage = patch_damage(&patch)?;
        let removed = patch.contains_key(&JavaString::from("!minecraft:damage"));
        if damage.is_some_and(|value| self.damage.is_some_and(|damage| damage != value))
            || (removed && self.damage.is_some())
        {
            return Err(StackError::ConflictingDamage);
        }
        let next = damage.or(self.damage);
        patch.remove(&JavaString::from("minecraft:damage"));
        self.damage = next;
        self.components = (!patch.is_empty()).then(|| Arc::new(Components::from(patch)));
        Ok(())
    }
    /// Decode the item NBT escape using this host's load-resolved item ids.
    /// Unknown ids, zero counts and malformed/conflicting damage return None.
    /// Effective maxima remain the game's write-boundary responsibility.
    pub fn from_nbt(host: &dyn crate::Host, value: &Compound) -> Option<Self> {
        let Tag::String(id) = value.get(&JavaString::from("id"))? else {
            return None;
        };
        let id = crate::Identifier::parse(&id.to_string_lossy()).ok()?;
        let item = host.item_key(&id)?;
        let count = match value.get(&JavaString::from("count")) {
            None => 1,
            Some(value) => damage_value(value).ok()?,
        };
        let mut stack = Self::new(item, count)?;
        match value.get(&JavaString::from("components")) {
            Some(Tag::Compound(patch)) => stack.set_components(patch.clone()).ok()?,
            None => {}
            _ => return None,
        }
        Some(stack)
    }
    /// Encode the item NBT escape using this host's canonical id, never a process-local key.
    /// Returns None when id metadata is unavailable. Only components present on the
    /// stack are written; effective defaults and maxima belong to the items host area.
    pub fn to_nbt(&self, host: &dyn crate::Host) -> Option<Compound> {
        let id = self.item.id(host);
        if id.is_empty() {
            return None;
        }
        let mut value = Compound::new();
        value.insert(JavaString::from("id"), Tag::String(id.as_str().into()));
        value.insert(JavaString::from("count"), unsigned_tag(self.count()));
        let mut patch = self
            .components()
            .map(|patch| patch.as_nbt().clone())
            .unwrap_or_default();
        if let Some(damage) = self.damage {
            patch.insert(JavaString::from("minecraft:damage"), unsigned_tag(damage));
        }
        if !patch.is_empty() {
            value.insert(JavaString::from("components"), Tag::Compound(patch));
        }
        Some(value)
    }
}

pub(crate) fn unsigned_tag(value: u32) -> Tag {
    i32::try_from(value).map_or_else(|_| Tag::Long(i64::from(value)), Tag::Int)
}

fn damage_value(value: &Tag) -> Result<u32, StackError> {
    let value = match value {
        Tag::Byte(value) => i64::from(*value),
        Tag::Short(value) => i64::from(*value),
        Tag::Int(value) => i64::from(*value),
        Tag::Long(value) => *value,
        _ => return Err(StackError::InvalidDamage),
    };
    u32::try_from(value).map_err(|_| StackError::InvalidDamage)
}

/// Validate the raw damage entries before either a replacement or a merge mutates a stack.
fn patch_damage(patch: &Compound) -> Result<Option<u32>, StackError> {
    let damage = patch.get(&JavaString::from("minecraft:damage"));
    let removal = patch.get(&JavaString::from("!minecraft:damage"));
    if removal.is_some_and(|value| !matches!(value, Tag::Compound(value) if value.is_empty())) {
        return Err(StackError::InvalidDamage);
    }
    let damage = damage.map(damage_value).transpose()?;
    if damage.is_some() && removal.is_some() {
        return Err(StackError::ConflictingDamage);
    }
    Ok(damage)
}
#[cfg(test)]
mod tests {
    use super::*;

    include!("item_stack_damage_tests.rs");
    include!("item_stack_component_tests.rs");

    struct Number;
    impl Component for Number {
        type Value = i32;
        fn id(&self) -> &str {
            "test:number"
        }
        fn decode(&self, value: &Tag) -> Option<i32> {
            match value {
                Tag::Int(value) => Some(*value),
                _ => None,
            }
        }
        fn encode(&self, value: i32) -> Tag {
            Tag::Int(value)
        }
    }

    #[test]
    fn count_validation_is_atomic_and_registry_independent() {
        let key = ItemKey(0);
        assert!(ItemStack::new(key, 0).is_none());
        let mut stack = ItemStack::new(key, u32::MAX).unwrap();
        assert_eq!(stack.grow(1), Err(StackError::CountOverflow));
        assert_eq!(stack.set_count(0), Err(StackError::Empty));
        assert_eq!(stack.count(), u32::MAX);
        stack.set_damage(u32::MAX);
        assert_eq!(stack.damage(), u32::MAX);
    }

    #[test]
    fn splits_conserve_items_without_an_empty_sentinel() {
        for n in [0, 1, 4, 5, u32::MAX] {
            let mut stack = ItemStack::new(ItemKey(0), 4).unwrap();
            stack.set(&Number, 17).unwrap();
            let split = stack.split(n);
            assert_eq!(split.taken.as_ref().map_or(0, ItemStack::count), n.min(4));
            assert_eq!(
                split.remainder.as_ref().map_or(0, ItemStack::count),
                4 - n.min(4)
            );
            for portion in [split.taken, split.remainder].into_iter().flatten() {
                assert_eq!(portion.get(&Number), Some(17));
            }
        }
        assert!(ItemStack::of(ItemKey(0)).shrink(1).is_none());
    }

    #[test]
    fn patches_are_lazy_and_comparisons_ignore_counts() {
        let mut stack = ItemStack::of(ItemKey(0));
        assert!(stack.components().is_none());
        stack.set(&Number, 19).unwrap();
        let mut other = stack.clone();
        other.grow(3).unwrap();
        assert!(stack.is_stackable_with(&other));
        other.remove(&Number);
        assert!(!stack.same_components(&other));
        assert!(other.components().is_none());
        stack.remove(&Number);
        assert!(stack.is_stackable_with(&other));
    }
}
