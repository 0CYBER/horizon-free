//! Entity capabilities (arismoko/horizon#41 §7): what a mod registers for an entity type to
//! answer a capability, such as [`ENTITY_ITEMS`], beside the game's own (NeoForge's
//! `Capabilities.Item.ENTITY`). A player's inventory is the game's own answer for a player.
use crate::{
    Capability, ComponentId, Direction, EnergyHandler, EntityRef, EquipmentSlot, FluidResource,
    Host, ItemPlace, ItemResource, PREVENT_ARMOR_CHANGE, ProvidedBy, ResourceHandler, ServerHost,
    Tag, Transaction, backend_of,
};
use std::marker::PhantomData;

// no wire: process-local capability declaration carries a provider function pointer and type identity.
/// An entity capability: its name, the handler trait `T` it answers with, the context `C` a
/// lookup passes its providers (NeoForge's `EntityCapability<T, C>`: `()` where NeoForge's is
/// `Void`, the side asked about where it is `@Nullable Direction`), and the game's own provider,
/// if the game has one.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct EntityCapabilityKey<T: ?Sized + 'static, C: 'static = ()> {
    /// The name.
    pub name: &'static str,
    default: Option<fn(&mut dyn ServerHost, EntityRef, C) -> Option<Box<T>>>,
    handler: PhantomData<fn(C) -> Box<T>>,
}

impl<T: ?Sized + 'static, C: 'static> EntityCapabilityKey<T, C> {
    /// A mod's own entity capability, answered only by the providers registered for it.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            default: None,
            handler: PhantomData,
        }
    }
}

/// An entity's items, as NeoForge 26.3's `Capabilities.Item.ENTITY`. The game answers it for a
/// player with their inventory (`PlayerInventoryWrapper`): indices as vanilla's `Inventory`
/// numbers them, 0..36 the main inventory (0..9 the hotbar), 36..40 the armour from feet to
/// head, 40 the off hand. For a chest or hopper minecart, a chest boat, or a chested donkey,
/// mule or llama, it answers with their container's slots ([`ItemPlace::EntitySlot`]).
pub const ENTITY_ITEMS: EntityCapabilityKey<dyn ResourceHandler<ItemResource>> =
    EntityCapabilityKey {
        name: "horizon:entity_items",
        default: Some(|host, entity, ()| {
            if !builtin_player_inventory_type(&entity.kind(host)?.id(host)) {
                if let Some(size) = backend_of(host)?.entity_container_size(entity.id()) {
                    return Some(Box::new(ContainerItems { entity, size }));
                }
                // A living entity without a container: its equipment (a mob's
                // or an armor stand's).
                let size = backend_of(host)?.entity_equipment_size(entity.id())?;
                return Some(Box::new(LivingEquipment { entity, size }));
            }
            let player = entity.uuid(host)?;
            Some(Box::new(PlayerItems { entity, player }))
        }),
        handler: PhantomData,
    };

/// An entity's items as automation reaches them, as NeoForge 26.3's
/// `Capabilities.Item.ENTITY_AUTOMATION`, asked for the side of the entity a hopper reaches it
/// from. The game answers it for a chest or hopper minecart and a chest boat, the container
/// entities `EntitySelector.CONTAINER_ENTITY_SELECTOR` picks, with the slots [`ENTITY_ITEMS`]
/// gives them; a player and a chested mount are not reached by automation.
pub const ENTITY_AUTOMATION: EntityCapabilityKey<
    dyn ResourceHandler<ItemResource>,
    Option<Direction>,
> = EntityCapabilityKey {
    name: "horizon:entity_automation",
    default: Some(|host, entity, _| {
        let size = backend_of(host)?.entity_automation_size(entity.id())?;
        Some(Box::new(ContainerItems { entity, size }))
    }),
    handler: PhantomData,
};

/// An entity's energy, as NeoForge 26.3's `Capabilities.Energy.ENTITY`, asked for the side of
/// the entity a lookup reaches it from, or none for no particular side. Vanilla has no entity
/// that holds energy, so only mods answer it.
pub const ENTITY_ENERGY: EntityCapabilityKey<dyn EnergyHandler, Option<Direction>> =
    EntityCapabilityKey::new("horizon:entity_energy");
/// An entity's fluids, as NeoForge 26.3's `Capabilities.Fluid.ENTITY`, asked for the side of
/// the entity a lookup reaches it from, or none for no particular side. Vanilla has no entity
/// that holds a fluid, so only mods answer it.
pub const ENTITY_FLUIDS: EntityCapabilityKey<
    dyn ResourceHandler<FluidResource>,
    Option<Direction>,
> = EntityCapabilityKey::new("horizon:entity_fluids");

/// The player inventory default's type domain, shared with registration-time coverage.
pub(crate) fn builtin_player_inventory_type(kind: &str) -> bool {
    kind == "minecraft:player"
}

impl EntityRef {
    /// `key`'s handler for this entity (`Entity.getCapability`), its providers given `context`
    /// (`()` for [`ENTITY_ITEMS`], the side asked about for [`ENTITY_AUTOMATION`],
    /// [`ENTITY_ENERGY`] and [`ENTITY_FLUIDS`]): the first provider to answer, of those mods registered for its type
    /// `before_game`, the game's own, and the mods' others, each in registration order. None for
    /// a stale handle and for an entity outside the level in place: no provider is asked for one.
    /// Disabled mods' providers are skipped until their mod is re-enabled.
    pub fn capability<T: ?Sized + 'static, C: Copy + 'static>(
        self,
        host: &mut dyn ServerHost,
        key: &EntityCapabilityKey<T, C>,
        context: C,
    ) -> Option<Capability<T>> {
        if !self.level(host)?.in_place(host) {
            return None;
        }
        let mut game = None;
        let table = host.capability_providers();
        let kind = table
            .provided(ProvidedBy::Entity, key.name)
            .then(|| self.kind(host).map(|kind| kind.id(host)))
            .flatten();
        crate::answered(
            crate::ask_providers(
                host,
                |index| table.entity_provider(key.name, kind.as_deref()?, index),
                crate::EntityProviderOf::get::<T, C>,
                |provider, host| provider.provide(host, self, context),
                |host| {
                    game = key.default.and_then(|default| default(host, self, context));
                    game.is_some()
                },
            ),
            game,
        )
    }
}

/// Whether `entity` is still there, in the level in place: a handler kept past the entity's
/// removal or its world's close reaches nothing, as a new lookup would answer none.
fn live(host: &dyn Host, entity: EntityRef) -> bool {
    entity.level(host).is_some_and(|level| level.in_place(host))
}

/// The game's own [`ENTITY_ITEMS`] for a container entity: its `size` slots, each an
/// [`ItemPlace::EntitySlot`], taking a stack of anything.
struct ContainerItems {
    entity: EntityRef,
    size: usize,
}

impl ContainerItems {
    /// Index `index` as the entity's slot, while the entity is there.
    fn place(&self, host: &dyn Host, index: usize) -> Option<ItemPlace> {
        (index < self.size && live(host, self.entity)).then_some(ItemPlace::EntitySlot {
            entity: self.entity.id(),
            slot: index,
        })
    }
}

impl ResourceHandler<ItemResource> for ContainerItems {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.size
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
        self.place(host, index)?.resource(host)
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.place(host, index)
            .map_or(0, |place| place.amount(host))
    }
    fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: &ItemResource) -> u32 {
        match self.place(host, index) {
            Some(_) => max_stack(host, resource),
            None => 0,
        }
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        let Some(place) = self.place(host, index) else {
            return 0;
        };
        let limit = max_stack(host, resource);
        fill(host, tx, &place, resource, amount, limit)
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        self.place(host, index)
            .map_or(0, |place| place.extract(host, tx, resource, amount))
    }
}

/// The game's own [`ENTITY_ITEMS`] for a living non-player entity: its equipment slots, by
/// `EquipmentSlot`'s ordinal (`LivingEntityEquipmentWrapper`), each an [`ItemPlace::Equipment`]
/// whose write is the equipment write `set_equipment` makes. The armour and body slots take
/// only what their `equippable` names for this entity's type; the hands take a stack of
/// anything.
struct LivingEquipment {
    entity: EntityRef,
    size: usize,
}

impl LivingEquipment {
    /// Index `index` as the entity's equipment slot, while the entity is there.
    fn place(&self, host: &dyn Host, index: usize) -> Option<ItemPlace> {
        Some(ItemPlace::Equipment {
            entity: self.entity.id(),
            slot: self.slot(host, index)?,
        })
    }
    /// The equipment slot `index` is, while the entity is there.
    fn slot(&self, host: &dyn Host, index: usize) -> Option<EquipmentSlot> {
        if index >= self.size || !live(host, self.entity) {
            return None;
        }
        EquipmentSlot::ALL.get(index).copied()
    }
    /// The armour slots' names, by `EquipmentSlot`'s ordinal, `None` for a hand.
    fn armour(slot: EquipmentSlot) -> Option<&'static str> {
        match slot {
            EquipmentSlot::Feet => Some("feet"),
            EquipmentSlot::Legs => Some("legs"),
            EquipmentSlot::Chest => Some("chest"),
            EquipmentSlot::Head => Some("head"),
            EquipmentSlot::Body => Some("body"),
            _ => None,
        }
    }
}

impl ResourceHandler<ItemResource> for LivingEquipment {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.size
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
        self.place(host, index)?.resource(host)
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.place(host, index)
            .map_or(0, |place| place.amount(host))
    }
    fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: &ItemResource) -> u32 {
        let Some(slot) = self.slot(host, index) else {
            return 0;
        };
        match Self::armour(slot) {
            // `Mob.canEquip`: only what its `equippable` names for this kind,
            // one of it (`ArmorSlot.getMaxStackSize`).
            Some(slot) if !equips(host, resource, slot, &kind_of(host, self.entity)) => 0,
            Some(_) => max_stack(host, resource).min(1),
            None => max_stack(host, resource),
        }
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        let Some(place) = self.place(host, index) else {
            return 0;
        };
        let limit = self.capacity(host, index, resource);
        fill(host, tx, &place, resource, amount, limit)
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        self.place(host, index)
            .map_or(0, |place| place.extract(host, tx, resource, amount))
    }
}

/// The entity type `entity` is (`minecraft:zombie`), empty where the handle no longer reads.
fn kind_of(host: &dyn Host, entity: EntityRef) -> String {
    entity
        .kind(host)
        .map(|kind| kind.id(host))
        .unwrap_or_default()
}

/// The player inventory's slots, by `Inventory`'s numbers (`PlayerInventoryWrapper`).
const MAIN: usize = 36;
const ARMOUR: [&str; 4] = ["feet", "legs", "chest", "head"];
const OFF_HAND: usize = MAIN + ARMOUR.len();

/// The game's own [`ENTITY_ITEMS`] for `player`, the player `entity`: each index is the
/// player's [`ItemPlace::PlayerSlot`], written in place and journaled, and takes only what
/// fits there.
struct PlayerItems {
    entity: EntityRef,
    player: u128,
}

impl PlayerItems {
    /// Index `index` as the player's slot, while the player is there.
    fn place(&self, host: &dyn Host, index: usize) -> Option<ItemPlace> {
        live(host, self.entity).then(|| self.slot(index)).flatten()
    }
    /// Index `index` as the player's slot, numbered as `inventory_tick` numbers them.
    fn slot(&self, index: usize) -> Option<ItemPlace> {
        let slot = match index {
            OFF_HAND => -106,
            index if index > OFF_HAND => return None,
            index if index >= MAIN => 100 + (index - MAIN) as i32,
            index => index as i32,
        };
        Some(ItemPlace::PlayerSlot {
            player: self.player,
            slot,
        })
    }
}

/// Puts up to `amount` of `resource` at `place`, into what fits there and no further: an
/// empty place or one with the same item, up to `limit`.
fn fill(
    host: &mut dyn ServerHost,
    tx: &Transaction,
    place: &ItemPlace,
    resource: &ItemResource,
    amount: u32,
    limit: u32,
) -> u32 {
    let held = match place.stack(host) {
        Some(held) if !resource.matches(&held) => return 0,
        held => held.map_or(0, |held| held.count()),
    };
    match amount.min(limit.saturating_sub(held)) {
        0 => 0,
        room => place.insert(host, tx, resource, room),
    }
}

/// The most of `resource` one slot holds: its `max_stack_size`.
fn max_stack(host: &dyn Host, resource: &ItemResource) -> u32 {
    let id = ComponentId::new("minecraft:max_stack_size").expect("a component id");
    match resource.value(host, &id) {
        Some(Tag::Int(size)) => u32::try_from(size).unwrap_or(0),
        _ => 0,
    }
}

/// Whether `resource` goes in the armour slot `slot` (`feet`...) on an entity of type
/// `entity_type`: its `equippable` names that slot and lets that kind wear it
/// (`ArmorSlot.mayPlace`, `Equippable.canBeEquippedBy`), its allowed entities a list, one id,
/// or a `#` tag of entity types holding it.
fn equips(host: &dyn Host, resource: &ItemResource, slot: &str, entity_type: &str) -> bool {
    let id = ComponentId::new("minecraft:equippable").expect("a component id");
    let Some(equippable) = resource.value(host, &id) else {
        return false;
    };
    let named = |tag: Option<&Tag>, id: &str| match tag {
        Some(Tag::String(s)) => s.to_string_lossy() == id,
        _ => false,
    };
    named(equippable.get("slot"), slot)
        && match equippable.get("allowed_entities") {
            None => true,
            Some(Tag::List(ids)) => ids.iter().any(|id| named(Some(id), entity_type)),
            Some(Tag::String(entities)) => match entities.to_string_lossy().strip_prefix('#') {
                Some(tag) => host.entity_type_in_tag(entity_type, tag),
                None => entities.to_string_lossy() == entity_type,
            },
            _ => false,
        }
}

impl ResourceHandler<ItemResource> for PlayerItems {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        OFF_HAND + 1
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
        self.place(host, index)?.resource(host)
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.place(host, index)
            .map_or(0, |place| place.amount(host))
    }
    fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: &ItemResource) -> u32 {
        match index
            .checked_sub(MAIN)
            .and_then(|armour| ARMOUR.get(armour))
        {
            Some(slot) if !equips(host, resource, slot, "minecraft:player") => 0,
            // `ArmorSlot.getMaxStackSize`.
            Some(_) => max_stack(host, resource).min(1),
            _ if index > OFF_HAND => 0,
            _ => max_stack(host, resource),
        }
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        let Some(place) = self.place(host, index) else {
            return 0;
        };
        let limit = self.capacity(host, index, resource);
        fill(host, tx, &place, resource, amount, limit)
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        let Some(place) = self.place(host, index) else {
            return 0;
        };
        // `ArmorSlot.mayPickup`: bound armour stays on outside creative.
        let bound = resource.has_enchantment_effect(host, PREVENT_ARMOR_CHANGE);
        if (MAIN..OFF_HAND).contains(&index) && bound {
            let player = host.player(self.player);
            if player.and_then(|player| player.has_infinite_materials(host)) != Some(true) {
                return 0;
            }
        }
        place.extract(host, tx, resource, amount)
    }
    /// Where `Inventory.add` puts it: onto the same item in the selected slot, then in the off
    /// hand, then in the main inventory in order, then into its empty slots in order; never
    /// the armour.
    fn insert_any(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        if !live(host, self.entity) {
            return 0;
        }
        let limit = max_stack(host, resource);
        // The selected slot as a slot, not a hand: a creative player's hand takes anything.
        let selected = host
            .player(self.player)
            .and_then(|player| player.selected_slot(host));
        let same = [selected, Some(OFF_HAND)]
            .into_iter()
            .flatten()
            .chain(0..MAIN)
            .filter_map(|index| self.slot(index));
        let mut went = 0;
        for place in same {
            if went < amount
                && place
                    .stack(host)
                    .is_some_and(|held| resource.matches(&held))
            {
                went += fill(host, tx, &place, resource, amount - went, limit);
            }
        }
        for place in (0..MAIN).filter_map(|index| self.slot(index)) {
            if went < amount && place.stack(host).is_none() {
                went += fill(host, tx, &place, resource, amount - went, limit);
            }
        }
        went
    }
}
