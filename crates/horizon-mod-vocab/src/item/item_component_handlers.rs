//! Stock item handlers that keep their contents in one of the stack's data components, as
//! NeoForge 26.3's `ItemAccessResourceHandler`, `ItemAccessEnergyHandler` and
//! `ItemAccessFluidHandler`: what a mod answers [`ITEM_ENERGY`](crate::ITEM_ENERGY) or
//! [`ITEM_FLUIDS`](crate::ITEM_FLUIDS) with for a battery or a canteen. Each write rewrites the
//! item where it lives with an [`exchange`](ItemPlace::exchange) of one, in the caller's
//! transaction, so from a stack of more than one the changed item splits off.
use crate::content::registry_keys::{fluid_id, fluid_key};
use crate::{
    ComponentId, Components, Compound, EnergyHandler, FluidIdentity, FluidResource, FluidStack,
    ItemPlace, ResourceHandler, ServerHost, Tag, Transaction,
};

/// One component of the item at a place, read and rewritten through the place
/// (`ItemAccessResourceHandler`'s read and write-back): what a mod builds its own handler for
/// another resource on.
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ItemComponentHandler {
    place: ItemPlace,
    component: ComponentId,
}

impl ItemComponentHandler {
    /// Component `component` of the item at `place`.
    pub fn new(place: ItemPlace, component: ComponentId) -> Self {
        Self { place, component }
    }
    /// The place the item is at.
    pub fn place(&self) -> &ItemPlace {
        &self.place
    }
    /// The item's value of the component, its own else the item's default; none where the place
    /// is empty or the item hasn't one.
    pub fn read(&self, host: &mut dyn ServerHost) -> Option<Tag> {
        self.place.resource(host)?.value(&*host, &self.component)
    }
    /// Rewrites one item at the place with the component set to `value`, or removed for none, in
    /// `tx`: whether that went through.
    pub fn write(&self, host: &mut dyn ServerHost, tx: &Transaction, value: Option<Tag>) -> bool {
        let Some(held) = self.place.resource(host) else {
            return false;
        };
        let changed = match value {
            Some(value) => match held.with_component(&*host, &self.component, value) {
                Ok(changed) => changed,
                Err(_) => return false,
            },
            None => held.without_component(&*host, &self.component),
        };
        self.place.exchange(host, tx, &changed, 1) == 1
    }
}

/// Energy kept as an int component of the item (`ItemAccessEnergyHandler`): absent reads 0. Each
/// insert and extract moves at most its limit and the room or what is held.
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ItemComponentEnergy {
    item: ItemComponentHandler,
    capacity: u32,
    max_insert: u32,
    max_extract: u32,
}

impl ItemComponentEnergy {
    /// Energy in `component` of the item at `place`, up to `capacity` (at most `i32::MAX`, an
    /// int component's range), moving at most `max_insert` in and `max_extract` out per call.
    pub fn new(
        place: ItemPlace,
        component: ComponentId,
        capacity: u32,
        max_insert: u32,
        max_extract: u32,
    ) -> Self {
        Self {
            item: ItemComponentHandler::new(place, component),
            capacity: capacity.min(i32::MAX as u32),
            max_insert,
            max_extract,
        }
    }
    /// Rewrites the item as holding `energy`: `moved` when that goes through, else 0.
    fn rewrite(&self, host: &mut dyn ServerHost, tx: &Transaction, energy: u32, moved: u32) -> u32 {
        // `energy` is at most the capacity, which fits an int.
        let value = Tag::Int(i32::try_from(energy).unwrap_or(i32::MAX));
        if self.item.write(host, tx, Some(value)) {
            moved
        } else {
            0
        }
    }
}

impl EnergyHandler for ItemComponentEnergy {
    fn amount(&self, host: &mut dyn ServerHost) -> u32 {
        match self.item.read(host) {
            Some(Tag::Int(energy)) => u32::try_from(energy).unwrap_or(0),
            _ => 0,
        }
    }
    fn capacity(&self, _: &mut dyn ServerHost) -> u32 {
        self.capacity
    }
    fn insert(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
        let held = self.amount(host);
        let room = self.capacity.saturating_sub(held);
        match amount.min(self.max_insert).min(room) {
            0 => 0,
            went => self.rewrite(host, tx, held + went, went),
        }
    }
    fn extract(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
        let held = self.amount(host);
        match amount.min(self.max_extract).min(held) {
            0 => 0,
            came => self.rewrite(host, tx, held - came, came),
        }
    }
}

/// What a tank's component holds.
enum TankContent {
    Empty,
    Fluid(FluidStack),
    /// A positive amount of a fluid the registry doesn't have: kept as it is, never overwritten.
    Unresolved,
}

/// One tank of fluid kept in a compound component of the item (`ItemAccessFluidHandler` over
/// `SimpleFluidContent`): `{id: "minecraft:water", amount: 1000, components: {…}}`, the
/// `components` only where the fluid has some, and the component removed when the tank empties.
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ItemComponentFluidTank {
    item: ItemComponentHandler,
    capacity: u32,
}

impl ItemComponentFluidTank {
    /// A tank of up to `capacity` millibuckets in `component` of the item at `place`.
    pub fn new(place: ItemPlace, component: ComponentId, capacity: u32) -> Self {
        Self {
            item: ItemComponentHandler::new(place, component),
            capacity: capacity.min(i32::MAX as u32),
        }
    }
    /// What the tank holds, told apart from an empty one and from content of a fluid the
    /// registry doesn't have, which a write must keep and not overwrite.
    fn content(&self, host: &mut dyn ServerHost) -> TankContent {
        let Some(Tag::Compound(content)) = self.item.read(host) else {
            return TankContent::Empty;
        };
        let Some(Tag::String(id)) = content.get(&"id".into()) else {
            return TankContent::Empty;
        };
        let Some(&Tag::Int(amount)) = content.get(&"amount".into()) else {
            return TankContent::Empty;
        };
        let Some(amount) = u32::try_from(amount).ok().filter(|&amount| amount > 0) else {
            return TankContent::Empty;
        };
        let components = match content.get(&"components".into()) {
            Some(Tag::Compound(patch)) => Some(Box::new(Components::from(patch.clone()))),
            _ => None,
        };
        match fluid_key(&id.to_string_lossy()) {
            Some(key) => TankContent::Fluid(FluidStack {
                fluid: FluidIdentity::Resolved(key),
                amount,
                components,
            }),
            None => TankContent::Unresolved,
        }
    }
    /// What the tank holds; none for an empty one or a fluid the registry doesn't have.
    fn held(&self, host: &mut dyn ServerHost) -> Option<FluidStack> {
        match self.content(host) {
            TankContent::Fluid(held) => Some(held),
            TankContent::Empty | TankContent::Unresolved => None,
        }
    }
    /// The tank rewritten as holding `stack`, emptied for none: `moved` when that goes through,
    /// else 0.
    fn rewrite(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        stack: Option<FluidStack>,
        moved: u32,
    ) -> u32 {
        let value = match stack {
            Some(stack) => {
                let Some(id) = stack.key().and_then(fluid_id) else {
                    return 0;
                };
                let mut content = Compound::new();
                content.insert("id".into(), Tag::String(id.into()));
                content.insert("amount".into(), Tag::Int(stack.amount as i32));
                let has_components = stack.has_components();
                if let Some(components) = stack.components.filter(|_| has_components) {
                    content.insert(
                        "components".into(),
                        Tag::Compound(components.as_nbt().clone()),
                    );
                }
                Some(Tag::Compound(content))
            }
            None => None,
        };
        if self.item.write(host, tx, value) {
            moved
        } else {
            0
        }
    }
}

impl ResourceHandler<FluidResource> for ItemComponentFluidTank {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        1
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<FluidResource> {
        let held = self.held(host).filter(|_| index == 0)?;
        Some(FluidResource::of(&held))
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.held(host)
            .filter(|_| index == 0)
            .map_or(0, |held| held.amount)
    }
    fn capacity(&self, _: &mut dyn ServerHost, index: usize, _: &FluidResource) -> u32 {
        if index == 0 { self.capacity } else { 0 }
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &FluidResource,
        amount: u32,
    ) -> u32 {
        if index != 0 {
            return 0;
        }
        let held = match self.content(host) {
            TankContent::Unresolved => return 0,
            TankContent::Fluid(held) => Some(held),
            TankContent::Empty => None,
        };
        if held.as_ref().is_some_and(|held| !resource.matches(held)) {
            return 0;
        }
        let have = held.map_or(0, |held| held.amount);
        match amount.min(self.capacity.saturating_sub(have)) {
            0 => 0,
            went => self.rewrite(host, tx, Some(resource.to_stack(have + went)), went),
        }
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &FluidResource,
        amount: u32,
    ) -> u32 {
        let Some(held) = self
            .held(host)
            .filter(|held| index == 0 && resource.matches(held))
        else {
            return 0;
        };
        let came = amount.min(held.amount);
        if came == 0 {
            return 0;
        }
        let left = (held.amount > came).then(|| held.with_amount(held.amount - came));
        self.rewrite(host, tx, left, came)
    }
}
