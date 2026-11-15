//! Stock storage a mod attaches to an entity, a player, a chunk or the level, as NeoForge 26.3
//! mods keep an `ItemStacksResourceHandler`, a `FluidStacksResourceHandler` or a
//! `SimpleEnergyHandler` in an attachment: handlers over an [`AttachmentType`] the mod declares,
//! saved by its codec. Every write is an attachment write, so the game's transaction journal,
//! which keeps each holder's attachment map, puts it back on an abort.
use crate::content::registry_keys::{fluid_id, fluid_key, item_id, item_key_by_identifier};
use crate::{
    AttachmentCodec, AttachmentError, AttachmentType, Components, Compound, EnergyHandler,
    FluidIdentity, FluidResource, FluidStack, Holder, Identifier, ItemResource, ItemStack,
    ResourceHandler, ServerHost, Tag, Transaction,
};
use horizon_core::nbt::JavaString;

/// What each index holds and how much, none for an empty one: the saved value of an
/// [`AttachedStacks`] (NeoForge's `ResourceStacksResourceHandler` contents).
// no wire: the saved value of an attachment, written by its codec, never sent.
pub type StackList<R> = Vec<Option<(R, u32)>>;

/// A resource an [`AttachedStacks`] holds: how much of it one index takes at most, given the
/// handler's own capacity. Items stack to their stack size; anything else to the capacity.
pub trait StackedResource: Clone + PartialEq + Send + Sync + 'static {
    /// The most of it one index of capacity `capacity` holds.
    fn limit(&self, host: &mut dyn ServerHost, capacity: u32) -> u32 {
        let _ = host;
        capacity
    }
}

impl StackedResource for ItemResource {
    fn limit(&self, host: &mut dyn ServerHost, capacity: u32) -> u32 {
        match self.value_id(&*host, "minecraft:max_stack_size") {
            Some(Tag::Int(size)) => capacity.min(u32::try_from(size).unwrap_or(0)),
            _ => capacity,
        }
    }
}

impl StackedResource for FluidResource {}

/// Indices of a resource kept in an attachment of a holder (`StacksResourceHandler`): `size`
/// indices of up to `capacity` each, taking only what `valid` accepts. The attachment holds a
/// [`StackList`]; one emptied whole is removed.
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct AttachedStacks<R: 'static> {
    attachment: &'static AttachmentType<StackList<R>>,
    holder: Holder,
    size: usize,
    capacity: u32,
    valid: fn(&R) -> bool,
}

/// Item slots in an attachment (`ItemStacksResourceHandler`); its codec is [`ITEM_STACKS`].
// no wire: an alias of AttachedStacks, a mod-side handler.
pub type AttachedItems = AttachedStacks<ItemResource>;
/// Fluid tanks in an attachment (`FluidStacksResourceHandler`); its codec is [`FLUID_STACKS`].
// no wire: an alias of AttachedStacks, a mod-side handler.
pub type AttachedFluids = AttachedStacks<FluidResource>;

impl<R: StackedResource> AttachedStacks<R> {
    /// `size` indices of `attachment` on `holder`, each holding up to `capacity` of what `valid`
    /// accepts. Returns `None` if `size` exceeds the codecs' 4096-index limit.
    pub fn new(
        attachment: &'static AttachmentType<StackList<R>>,
        holder: Holder,
        size: usize,
        capacity: u32,
        valid: fn(&R) -> bool,
    ) -> Option<Self> {
        if size > MAX_SLOTS {
            return None;
        }
        Some(Self {
            attachment,
            holder,
            size,
            capacity,
            valid,
        })
    }
    /// One index (`ItemStackResourceHandler`, a single tank).
    pub fn single(
        attachment: &'static AttachmentType<StackList<R>>,
        holder: Holder,
        capacity: u32,
        valid: fn(&R) -> bool,
    ) -> Self {
        Self::new(attachment, holder, 1, capacity, valid)
            .expect("one index is within the codec limit")
    }
    /// What index `index` holds; none past the handler's `size`, or where the holder has none
    /// or is gone. One borrowed read of the stored list: only the entry is cloned.
    fn at(&self, host: &mut dyn ServerHost, index: usize) -> Option<(R, u32)> {
        if index >= self.size {
            return None;
        }
        let borrowed = self.attachment.with_decoded(host, self.holder, |list| {
            list.map(|list| list.get(index).and_then(|held| held.as_ref()).cloned())
        });
        match borrowed {
            // The holder keeps a decoded list: the entry is read without a clone.
            Ok(Some(held)) => held,
            // Nothing decoded yet: `get_existing` decodes a saved value once, and the store
            // keeps its decoded copy for the borrowed reads after it.
            _ => self
                .attachment
                .get_existing(host, self.holder)
                .ok()
                .flatten()
                .and_then(|list| list.into_iter().nth(index).flatten()),
        }
    }
    /// Index `index` set to `now`: whether the write went through. Indices past the handler's
    /// `size` that the holder keeps are preserved, so a write through a smaller view never
    /// deletes them.
    fn write(&self, host: &mut dyn ServerHost, index: usize, now: Option<(R, u32)>) -> bool {
        let mut list = self
            .attachment
            .get_existing(host, self.holder)
            .ok()
            .flatten()
            .unwrap_or_default();
        if list.len() <= index {
            list.resize(index + 1, None);
        }
        list[index] = now.filter(|(_, amount)| *amount > 0);
        if list.iter().all(Option::is_none) {
            self.attachment.remove(host, self.holder).is_ok()
        } else {
            self.attachment.set(host, self.holder, list).is_ok()
        }
    }
}

impl<R: StackedResource> ResourceHandler<R> for AttachedStacks<R> {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.size
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<R> {
        self.at(host, index).map(|(resource, _)| resource)
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.at(host, index).map_or(0, |(_, amount)| amount)
    }
    fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: &R) -> u32 {
        // The physical limit of an in-range index; capacity is not permission to insert
        // (`is_valid` is), and extraction does not look at it.
        if index < self.size {
            resource.limit(host, self.capacity)
        } else {
            0
        }
    }
    fn is_valid(&self, _: &mut dyn ServerHost, index: usize, resource: &R) -> bool {
        index < self.size && (self.valid)(resource)
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        _: &Transaction,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> u32 {
        if !self.is_valid(host, index, resource) {
            return 0;
        }
        let room = self.capacity(host, index, resource);
        let have = match self.at(host, index) {
            Some((held, _)) if held != *resource => return 0,
            Some((_, have)) => have,
            None => 0,
        };
        match amount.min(room.saturating_sub(have)) {
            0 => 0,
            went if self.write(host, index, Some((resource.clone(), have + went))) => went,
            _ => 0,
        }
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        _: &Transaction,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> u32 {
        let Some((held, have)) = self.at(host, index).filter(|(held, _)| held == resource) else {
            return 0;
        };
        match amount.min(have) {
            0 => 0,
            came if self.write(host, index, Some((held, have - came))) => came,
            _ => 0,
        }
    }
}

/// Energy kept in an int attachment of a holder (`SimpleEnergyHandler`): up to `capacity`,
/// moving at most `max_insert` in and `max_extract` out per call.
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct AttachedEnergy {
    attachment: &'static AttachmentType<u32>,
    holder: Holder,
    capacity: u32,
    max_insert: u32,
    max_extract: u32,
}

impl AttachedEnergy {
    /// `attachment` on `holder`, holding up to `capacity`, at most `max_insert` in and
    /// `max_extract` out per call.
    pub fn new(
        attachment: &'static AttachmentType<u32>,
        holder: Holder,
        capacity: u32,
        max_insert: u32,
        max_extract: u32,
    ) -> Self {
        Self {
            attachment,
            holder,
            capacity,
            max_insert,
            max_extract,
        }
    }
    /// Sets the energy: `moved` when the write went through, else 0.
    fn write(&self, host: &mut dyn ServerHost, energy: u32, moved: u32) -> u32 {
        match self.attachment.set(host, self.holder, energy) {
            Ok(()) => moved,
            Err(_) => 0,
        }
    }
}

impl EnergyHandler for AttachedEnergy {
    fn amount(&self, host: &mut dyn ServerHost) -> u32 {
        self.attachment
            .get_existing(host, self.holder)
            .ok()
            .flatten()
            .unwrap_or(0)
    }
    fn capacity(&self, _: &mut dyn ServerHost) -> u32 {
        self.capacity
    }
    fn insert(&self, host: &mut dyn ServerHost, _: &Transaction, amount: u32) -> u32 {
        let held = self.amount(host);
        match amount
            .min(self.max_insert)
            .min(self.capacity.saturating_sub(held))
        {
            0 => 0,
            went => self.write(host, held + went, went),
        }
    }
    fn extract(&self, host: &mut dyn ServerHost, _: &Transaction, amount: u32) -> u32 {
        let held = self.amount(host);
        match amount.min(self.max_extract).min(held) {
            0 => 0,
            came => self.write(host, held - came, came),
        }
    }
}

/// The codec of an [`AttachedItems`] attachment: `{stacks: [{Slot, id, count, components}]}`,
/// each held index as vanilla saves a container's items. An amount past what a signed `Int`
/// saves (`i32::MAX`) fails the encode rather than wrapping; the handler's capacity is far
/// below it.
/// At most 4096 indices (slots 0 through 4095) can be encoded or decoded.
pub const ITEM_STACKS: AttachmentCodec<StackList<ItemResource>> = AttachmentCodec {
    encode: |list| {
        encode_stacks(list, |item, amount| {
            let stack = item.to_stack(amount)?;
            let mut saved = Compound::new();
            saved.insert("id".into(), Tag::String(item_id(stack.item())?.into()));
            saved.insert("count".into(), Tag::Int(i32::try_from(amount).ok()?));
            let mut patch = stack
                .components()
                .map(|patch| patch.as_nbt().clone())
                .unwrap_or_default();
            if let Some(damage) = stack.damage_component() {
                patch.insert("minecraft:damage".into(), Tag::Int(damage as i32));
            }
            if !patch.is_empty() {
                saved.insert("components".into(), Tag::Compound(patch));
            }
            Some(saved)
        })
    },
    decode: |saved| {
        decode_stacks(saved, |entry| {
            let Some(Tag::String(id)) = entry.get(&"id".into()) else {
                return None;
            };
            let item = item_key_by_identifier(&Identifier::parse(&id.to_string_lossy()).ok()?)?;
            let Some(&Tag::Int(count)) = entry.get(&"count".into()) else {
                return None;
            };
            let count = u32::try_from(count).ok()?;
            let mut stack = ItemStack::new(item, 1)?;
            if let Some(Tag::Compound(patch)) = entry.get(&"components".into()) {
                stack.set_components(patch.clone()).ok()?;
            }
            Some((ItemResource::of(&stack), count))
        })
    },
};

/// The codec of an [`AttachedFluids`] attachment: `{stacks: [{Slot, id, amount, components}]}`.
/// At most 4096 indices (slots 0 through 4095) can be encoded or decoded.
/// An amount past what a signed `Int` saves (`i32::MAX`) fails the encode, as the items codec
/// does; a fluid tank that can hold more carries it in a mod's own codec (NeoForge saves the
/// same `Int`).
pub const FLUID_STACKS: AttachmentCodec<StackList<FluidResource>> = AttachmentCodec {
    encode: |list| {
        encode_stacks(list, |fluid, amount| {
            let stack = fluid.to_stack(amount);
            let mut saved = Compound::new();
            saved.insert("id".into(), Tag::String(fluid_id(stack.key()?)?.into()));
            saved.insert("amount".into(), Tag::Int(i32::try_from(amount).ok()?));
            if stack.has_components()
                && let Some(components) = &stack.components
            {
                saved.insert(
                    "components".into(),
                    Tag::Compound(components.as_nbt().clone()),
                );
            }
            Some(saved)
        })
    },
    decode: |saved| {
        decode_stacks(saved, |entry| {
            let Some(Tag::String(id)) = entry.get(&"id".into()) else {
                return None;
            };
            let Some(&Tag::Int(amount)) = entry.get(&"amount".into()) else {
                return None;
            };
            let components = match entry.get(&"components".into()) {
                Some(Tag::Compound(patch)) => Some(Box::new(Components::from(patch.clone()))),
                _ => None,
            };
            let stack = FluidStack {
                fluid: FluidIdentity::Resolved(fluid_key(&id.to_string_lossy())?),
                amount: 1,
                components,
            };
            Some((FluidResource::of(&stack), u32::try_from(amount).ok()?))
        })
    },
};

/// `list`'s held indices, each saved by `entry` with its `Slot`; a resource `entry` can't save
/// (an id the registry lacks) fails the encode, so nothing is lost silently.
fn encode_stacks<R>(
    list: &StackList<R>,
    entry: impl Fn(&R, u32) -> Option<Compound>,
) -> Result<Compound, AttachmentError> {
    if list.len() > MAX_SLOTS {
        return Err(AttachmentError::Encode);
    }
    let mut stacks = Vec::new();
    for (slot, (resource, amount)) in list
        .iter()
        .enumerate()
        .filter_map(|(slot, held)| Some((slot, held.as_ref()?)))
    {
        let mut saved = entry(resource, *amount).ok_or(AttachmentError::Encode)?;
        let slot = i32::try_from(slot).map_err(|_| AttachmentError::Encode)?;
        saved.insert("Slot".into(), Tag::Int(slot));
        stacks.push(Tag::Compound(saved));
    }
    let mut saved = Compound::new();
    saved.insert(JavaString::from("stacks"), Tag::List(stacks));
    Ok(saved)
}

/// The most indices a handler or saved stack list may span. Checked at construction and
/// at both codec boundaries, before a hostile saved slot can drive allocation.
const MAX_SLOTS: usize = 4096;

/// The list `saved` holds, each index read by `entry`; one it can't read fails the decode. A
/// `Slot` past [`MAX_SLOTS`] fails the decode before anything is allocated to its size.
fn decode_stacks<R: Clone>(
    saved: &Compound,
    entry: impl Fn(&Compound) -> Option<(R, u32)>,
) -> Result<StackList<R>, AttachmentError> {
    let mut list = StackList::new();
    let Some(Tag::List(stacks)) = saved.get(&JavaString::from("stacks")) else {
        return Ok(list);
    };
    for stack in stacks {
        let Tag::Compound(stack) = stack else {
            return Err(AttachmentError::Decode);
        };
        let Some(&Tag::Int(slot)) = stack.get(&"Slot".into()) else {
            return Err(AttachmentError::Decode);
        };
        let slot = usize::try_from(slot).map_err(|_| AttachmentError::Decode)?;
        if slot >= MAX_SLOTS {
            return Err(AttachmentError::Decode);
        }
        let held = entry(stack).ok_or(AttachmentError::Decode)?;
        if list.len() <= slot {
            list.resize(slot + 1, None);
        }
        list[slot] = Some(held);
    }
    Ok(list)
}

#[cfg(test)]
#[path = "attached_storage_tests.rs"]
mod tests;
