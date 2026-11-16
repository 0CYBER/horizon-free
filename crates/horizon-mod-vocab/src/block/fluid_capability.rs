//! The fluid capability (arismoko/horizon#41 §7): [`FLUIDS`], as NeoForge 26.3's
//! `Capabilities.Fluid.BLOCK`, a [`ResourceHandler`] of [`FluidResource`]s as
//! [`ITEMS`](crate::ITEMS) is of items. Amounts are millibuckets: a bucket is 1000.
use crate::__private::ingredient_parser::WalkedIngredient;
use crate::Components;
use crate::{
    BlockSide, CapabilityKey, DataLoadContext, FluidIdentity, FluidKey, ItemCapabilityKey,
    ItemPlace, ItemResource, ItemStack, ResourceHandler, ServerHost, Transaction,
};
use std::{fmt, marker::PhantomData};

/// An amount of one fluid, as NeoForge's `FluidStack`: its registered or unresolved
/// `FluidIdentity`, millibuckets, and data components, kept as an item stack's are (a potion fluid's
/// `minecraft:potion_contents`). Water and lava have none.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct FluidStack {
    /// The fluid.
    pub fluid: FluidIdentity,
    /// The amount.
    pub amount: u32,
    /// The components.
    pub components: Option<Box<Components>>,
}

impl FluidStack {
    /// `amount` of `fluid`, with no components.
    pub fn new(fluid: FluidKey, amount: u32) -> Self {
        Self {
            fluid: FluidIdentity::Resolved(fluid),
            amount,
            components: None,
        }
    }
    /// The registered key, or none for an unresolved saved fluid.
    pub fn key(&self) -> Option<FluidKey> {
        self.fluid.key()
    }
    /// The same fluid with the same components, `amount` of it.
    pub fn with_amount(&self, amount: u32) -> Self {
        Self {
            amount,
            ..self.clone()
        }
    }
    /// Whether it carries any component.
    pub fn has_components(&self) -> bool {
        self.components
            .as_deref()
            .is_some_and(|components| !components.as_nbt().is_empty())
    }
    /// NeoForge's `FluidStack.isSameFluidSameComponents`: the same id and the same components,
    /// an empty patch counting as none. Fills, merges and capacities ask it.
    pub fn same_fluid_same_components(&self, other: &Self) -> bool {
        let none = Components::default();
        self.fluid == other.fluid
            && self.components.as_deref().unwrap_or(&none)
                == other.components.as_deref().unwrap_or(&none)
    }
}

/// A fluid and its components, without an amount, as NeoForge's `FluidResource`: what a tank
/// holds. An empty tank holds none.
// no wire: decoding calls FluidResource::of to normalize the incoming fluid amount to one; a structural derive would preserve a non-unit wire amount.
#[derive(Clone)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct FluidResource(pub(crate) FluidStack);

impl FluidResource {
    /// What `stack` is, whatever its amount.
    pub fn of(stack: &FluidStack) -> Self {
        Self(stack.with_amount(1))
    }
    /// The resolved key or unresolved saved identity.
    pub fn fluid(&self) -> &FluidIdentity {
        &self.0.fluid
    }
    /// `amount` millibuckets of it as a stack.
    pub fn to_stack(&self, amount: u32) -> FluidStack {
        self.0.with_amount(amount)
    }
    /// Whether `stack` is this fluid with these components.
    pub fn matches(&self, stack: &FluidStack) -> bool {
        self.0.same_fluid_same_components(stack)
    }
}

impl PartialEq for FluidResource {
    fn eq(&self, other: &Self) -> bool {
        self.matches(&other.0)
    }
}
impl Eq for FluidResource {}

impl fmt::Debug for FluidResource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("FluidResource").field(&self.0.fluid).finish()
    }
}

/// Why a fluid ingredient could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum FluidIngredientError {
    /// Invalid or unsupported ingredient shape.
    Invalid,
    /// A fluid name is not registered.
    UnknownFluid(String),
}

/// A set of accepted fluids, as NeoForge's `FluidIngredient`: a fluid
/// (`minecraft:water`), a `#tag`, a list of them, or the three shapes with
/// their children, parsed from a recipe's JSON by the same walk an item
/// [`Ingredient`](crate::Ingredient) parses its own with, over the fluid
/// tags the load's [`DataLoadContext`] holds. A kind outside the standard
/// shapes fails the parse: fluids have no custom ingredient hook.
/// Components are not checked — any stack of a named fluid answers.
#[derive(Clone, Debug)]
// exhaustive: its fields are private, so mods cannot build or destructure it
#[derive(horizon_mod_api_macros::Wire)]
pub struct FluidIngredient {
    fluids: Vec<FluidKey>,
}

impl FluidIngredient {
    /// Parse a fluid, tag, list or standard shape against this load's fluid registry.
    /// Invalid shapes or unknown names return a typed error.
    pub fn from_json(
        v: &serde_json::Value,
        context: &dyn DataLoadContext,
    ) -> Result<Self, FluidIngredientError> {
        let mut tags = std::collections::HashMap::new();
        if let Some(list) = context.fluid_tags() {
            tags.reserve(list.len());
            for (tag, fluids) in list.iter() {
                tags.insert(tag.clone(), fluids.iter().cloned().collect());
            }
        }
        // A custom kind is the item ingredient's hook; a fluid one fails
        // the file, as a kind outside these does.
        let keys = std::cell::RefCell::new(std::collections::HashMap::new());
        let unknown = std::cell::RefCell::new(None);
        let source =
            WalkedIngredient::walk_resolved(v, &tags, &|_| None, &|_| None, &|_| None, &|id| {
                let Some(key) = context.fluid_key(id) else {
                    *unknown.borrow_mut() = Some(id.to_owned());
                    return None;
                };
                let canonical = if id.contains(':') {
                    id.to_owned()
                } else {
                    format!("minecraft:{id}")
                };
                keys.borrow_mut().insert(canonical.clone(), key);
                Some(canonical)
            });
        fn nonempty_tree(source: &WalkedIngredient) -> bool {
            !source.order.is_empty()
                && source
                    .children
                    .as_ref()
                    .is_none_or(|(_, children)| children.iter().all(nonempty_tree))
        }
        let source = source.filter(nonempty_tree).ok_or_else(|| {
            unknown.into_inner().map_or(
                FluidIngredientError::Invalid,
                FluidIngredientError::UnknownFluid,
            )
        })?;
        let keys = keys.into_inner();
        let fluids = source.order.iter().map(|id| keys[id]).collect();
        Ok(Self { fluids })
    }
    /// `FluidIngredient.test`: the stack's fluid among the ones the source
    /// named — and for a shape, each child its own way (any for
    /// `compound`, all for `intersection`, base-but-not-subtracted for
    /// `difference`).
    pub fn test(&self, stack: &FluidStack) -> bool {
        stack.key().is_some_and(|key| self.fluids.contains(&key))
    }
    /// The fluids it accepts, in the sources' order (`HolderSet`) — what a
    /// recipe book shows.
    pub fn fluids(&self) -> &[FluidKey] {
        &self.fluids
    }
}

/// `FluidType.BUCKET_VOLUME`: the amount [`SizedFluidIngredient`]'s
/// `"amount"` defaults to.
const BUCKET_VOLUME: u32 = 1000;

/// A [`FluidIngredient`] with the amount it asks, as NeoForge's
/// `SizedFluidIngredient`: the fluids the ingredient accepts, and how many
/// millibuckets of them (a bucket is 1000).
#[derive(Clone, Debug)]
// exhaustive: plain data the mod reads out of a recipe and matches stacks with
#[derive(horizon_mod_api_macros::Wire)]
pub struct SizedFluidIngredient {
    /// The fluids it accepts, without an amount.
    pub ingredient: FluidIngredient,
    /// How many millibuckets it asks of one of them; positive.
    pub amount: u32,
}

impl SizedFluidIngredient {
    /// The sized ingredient of `v`, from its JSON form:
    /// `{"ingredient": <fluid ingredient>, "amount": n}` — `amount`
    /// defaults to a bucket (1000 mB) and must be positive.
    pub fn from_json(v: &serde_json::Value, context: &dyn DataLoadContext) -> Option<Self> {
        let o = v.as_object()?;
        let ingredient = FluidIngredient::from_json(o.get("ingredient")?, context).ok()?;
        // `ExtraCodecs.POSITIVE_INT` with the bucket default.
        let amount = match o.get("amount") {
            Some(a) => u32::try_from(a.as_u64()?).ok().filter(|n| *n > 0)?,
            None => BUCKET_VOLUME,
        };
        Some(Self { ingredient, amount })
    }
    /// `SizedFluidIngredient.test`: the stack matches the ingredient and
    /// carries at least `amount` of it.
    pub fn test(&self, stack: &FluidStack) -> bool {
        self.ingredient.test(stack) && stack.amount >= self.amount
    }
}

/// Fluid tanks, as NeoForge 26.3's `Capabilities.Fluid.BLOCK`: a block's tanks as one side
/// reaches them, numbered from 0 for that side, as [`ITEMS`](crate::ITEMS)' slots. The game
/// answers it for a mod block's tanks (`Registrar::block_fluids`) and for a cauldron: one tank
/// holding the content its block has, vanilla's water (a bucket in thirds) and lava (a bucket
/// only whole) or a content a mod registered through `register_cauldron_fluid_content`, filled
/// and drained by whole levels; an empty cauldron takes any fluid that has a content. A
/// cauldron has no block entity, so it is answered by position. Powder snow is no fluid: its
/// cauldron has no content and no answer.
pub const FLUIDS: CapabilityKey<dyn ResourceHandler<FluidResource>> = CapabilityKey {
    name: "horizon:fluids",
    default: Some(|host, at| {
        crate::backend_of(host)
            .and_then(|backend| backend.fluid_tank_count(at))
            .map(|_| Box::new(GameFluids(at)) as Box<dyn ResourceHandler<FluidResource>>)
    }),
    handler: PhantomData,
};

/// The game's own [`FLUIDS`]: a mod block's tanks `.side` reaches by its table, and cauldrons.
struct GameFluids(BlockSide);
impl ResourceHandler<FluidResource> for GameFluids {
    fn size(&self, host: &mut dyn ServerHost) -> usize {
        crate::backend_of(host)
            .and_then(|backend| backend.fluid_tank_count(self.0))
            .unwrap_or(0)
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<FluidResource> {
        let held = crate::backend_of(host).and_then(|backend| backend.fluid_tank(self.0, index))?;
        Some(FluidResource::of(&held))
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        crate::backend_of(host)
            .and_then(|backend| backend.fluid_tank(self.0, index))
            .map_or(0, |held| held.amount)
    }
    fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: &FluidResource) -> u32 {
        crate::backend_of(host).map_or(0, |backend| {
            backend.fluid_tank_capacity(self.0, index, &resource.to_stack(1))
        })
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &FluidResource,
        amount: u32,
    ) -> u32 {
        let Some(backend) = crate::backend_mut_of(host) else {
            return 0;
        };
        let rest = backend.fluid_tank_insert(self.0, index, resource.to_stack(amount), tx);
        amount - rest.map_or(0, |rest| rest.amount.min(amount))
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &FluidResource,
        amount: u32,
    ) -> u32 {
        let held = crate::backend_of(host).and_then(|backend| backend.fluid_tank(self.0, index));
        if !held.is_some_and(|held| resource.matches(&held)) {
            return 0;
        }
        crate::backend_mut_of(host)
            .and_then(|backend| backend.fluid_tank_extract(self.0, index, amount, tx))
            .map_or(0, |out| out.amount.min(amount))
    }
}

/// Fluid held by an item, as NeoForge 26.3's `Capabilities.Fluid.ITEM`: a handler for the item at
/// one place, whose changes rewrite the item there with an [`exchange`](ItemPlace::exchange) of
/// one, so from a stack of more than one the changed item splits off. The game answers it for
/// vanilla's buckets (NeoForge's `BucketResourceHandler`): one tank of a bucket, filled and drained
/// whole, an empty bucket taking plain water or lava.
pub const ITEM_FLUIDS: ItemCapabilityKey<dyn ResourceHandler<FluidResource>> = ItemCapabilityKey {
    name: "horizon:item_fluids",
    default: Some(|host, place, stack| {
        let buckets = [
            host.item_key(&BUCKET_IDS[0])?,
            host.item_key(&BUCKET_IDS[1])?,
            host.item_key(&BUCKET_IDS[2])?,
        ];
        if !buckets.contains(&stack.item()) {
            return None;
        }
        Some(Box::new(GameBucket {
            place: place.clone(),
            water: host.fluid_key("minecraft:water")?,
            lava: host.fluid_key("minecraft:lava")?,
            buckets,
        }) as Box<dyn ResourceHandler<FluidResource>>)
    }),
    handler: PhantomData,
};

/// Resolve the vanilla bucket identifiers once; lookups thereafter build no strings.
static BUCKET_IDS: std::sync::LazyLock<[crate::Identifier; 3]> = std::sync::LazyLock::new(|| {
    [
        "minecraft:bucket",
        "minecraft:water_bucket",
        "minecraft:lava_bucket",
    ]
    .map(|id| crate::Identifier::parse_namespaced(id).expect("vanilla identifier"))
});

/// Type-only coverage uses the very identifiers resolved by the default bucket provider.
pub(crate) fn builtin_bucket(item: &str) -> bool {
    crate::Identifier::parse_namespaced(item).is_some_and(|item| BUCKET_IDS.contains(&item))
}

/// A bucket holds this much, all or nothing.
const BUCKET: u32 = 1000;

/// The game's own [`ITEM_FLUIDS`] for one bucket place.
struct GameBucket {
    place: ItemPlace,
    water: FluidKey,
    lava: FluidKey,
    buckets: [crate::ItemKey; 3],
}
impl GameBucket {
    /// What the bucket here holds: none for an empty one, or where no bucket is now.
    fn held(&self, host: &mut dyn ServerHost) -> Option<FluidKey> {
        let stack = self.place.stack(host)?;
        if stack.item() == self.buckets[1] {
            Some(self.water)
        } else if stack.item() == self.buckets[2] {
            Some(self.lava)
        } else {
            None
        }
    }
    /// The bucket holding `fluid`, plain water or lava; none for another fluid.
    fn bucket_of(&self, fluid: &FluidResource) -> Option<crate::ItemKey> {
        if fluid.0.has_components() {
            return None;
        }
        let key = fluid.fluid().key()?;
        if key == self.water {
            Some(self.buckets[1])
        } else if key == self.lava {
            Some(self.buckets[2])
        } else {
            None
        }
    }
    /// One bucket here exchanged for `bucket`: `BUCKET` when that goes through, else 0.
    fn exchange(&self, host: &mut dyn ServerHost, tx: &Transaction, bucket: crate::ItemKey) -> u32 {
        let bucket = ItemResource::of(&ItemStack::of(bucket));
        match self.place.exchange(host, tx, &bucket, 1) {
            1 => BUCKET,
            _ => 0,
        }
    }
}
impl ResourceHandler<FluidResource> for GameBucket {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        1
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<FluidResource> {
        let fluid = self.held(host).filter(|_| index == 0)?;
        Some(FluidResource::of(&FluidStack::new(fluid, 1)))
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        match self.resource(host, index) {
            Some(_) => BUCKET,
            None => 0,
        }
    }
    fn capacity(&self, _: &mut dyn ServerHost, index: usize, resource: &FluidResource) -> u32 {
        match self.bucket_of(resource) {
            Some(_) if index == 0 => BUCKET,
            _ => 0,
        }
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &FluidResource,
        amount: u32,
    ) -> u32 {
        let empty = self
            .place
            .stack(host)
            .is_some_and(|stack| stack.item() == self.buckets[0]);
        match self.bucket_of(resource) {
            Some(full) if index == 0 && empty && amount >= BUCKET => self.exchange(host, tx, full),
            _ => 0,
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
        let held = self.resource(host, index);
        match held {
            Some(held) if held == *resource && amount >= BUCKET => {
                self.exchange(host, tx, self.buckets[0])
            }
            _ => 0,
        }
    }
}
