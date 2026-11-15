use super::*;
#[derive(Clone)]
pub(crate) struct PlaceProviderPart {
    pub(crate) mod_id: &'static str,
    pub(crate) mod_index: usize,
    pub(crate) provider: Arc<dyn PlaceProvider>,
}

// no wire: process-local registration and dispatch storage contains trait objects and owning-mod identities.
/// A tooltip appender a mod registered, as the game reads it ([`tooltip_appenders`]).
#[derive(Clone)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct TooltipAppenderPart {
    pub(crate) mod_id: &'static str,
    pub(crate) mod_index: usize,
    pub(crate) component: Box<str>,
    pub(crate) placement: crate::TooltipPlacement,
    pub(crate) appender: Arc<dyn crate::TooltipAppender>,
}

impl TooltipAppenderPart {
    /// The data component whose stacks it is asked about.
    pub fn component(&self) -> &str {
        &self.component
    }
    /// Where its lines go among the other components' lines.
    pub fn placement(&self) -> &crate::TooltipPlacement {
        &self.placement
    }
    /// The appender that adds the lines.
    pub fn appender(&self) -> &dyn crate::TooltipAppender {
        self
    }
    /// Whether its mod is enabled.
    pub fn enabled(&self) -> bool {
        crate::mod_index_on(self.mod_index)
    }
}

impl crate::TooltipAppender for TooltipAppenderPart {
    fn append(
        &self,
        host: &dyn crate::ClientHost,
        stack: &crate::ItemStack,
        context: &crate::TooltipAppendContext,
        lines: &mut Vec<(crate::Text, u32)>,
    ) {
        let _guard = crate::content::crash::enter_native_mod(crate::content::crash::ModCall {
            id: self.mod_id,
            hook: "tooltip_appender",
            part: Some(std::any::type_name_of_val(&*self.appender)),
        });
        self.appender.append(host, stack, context, lines);
    }
}

/// The tooltip appenders the mods registered, in registration order.
pub fn tooltip_appenders() -> &'static [TooltipAppenderPart] {
    installed().map_or(&[], |installed| &installed.keys.tooltip_appenders)
}
/// What answers an item capability for one item: given where the stack lives, its handler, or
/// none to leave it to the next provider. A closure is one.
pub trait ItemCapabilityProvider<T: ?Sized>: Send + Sync {
    /// The handler for the stack at `at`, or none.
    fn provide(&self, host: &mut dyn ServerHost, at: &ItemPlace) -> Option<Box<T>>;
}
impl<T: ?Sized, F> ItemCapabilityProvider<T> for F
where
    F: Fn(&mut dyn ServerHost, &ItemPlace) -> Option<Box<T>> + Send + Sync,
{
    fn provide(&self, host: &mut dyn ServerHost, at: &ItemPlace) -> Option<Box<T>> {
        self(host, at)
    }
}
// no wire: trait-object alias inherits its provider trait policy; callbacks are not serialized.
/// An item capability provider, as the registrar keeps it.
pub type ItemProvider<T> = dyn ItemCapabilityProvider<T>;

// no wire: process-local registration diagnostic retains a static hook or mod identity.
/// An item registration request that the loaded item registry cannot resolve.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ItemKeyError {
    /// The requesting mod.
    pub mod_id: &'static str,
    /// The canonical requested item id.
    pub id: String,
}

impl HookParts {
    pub(crate) fn item<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn ItemPart>] {
        attached(&self.items, id)
    }
}

impl dyn Registrar + '_ {
    /// Attach `part` to every item in tag `tag` (`minecraft:axes`, without `#`).
    pub fn item_part_tag<P: ItemPart>(&mut self, tag: &str, part: P) {
        self.attach_item_part_tag(tag, type_name::<P>(), Arc::new(part));
    }
    /// Attach `part` to item `item` (`mymod:wrench`).
    pub fn item_part<P: ItemPart>(&mut self, item: crate::ItemKey, part: P) {
        self.attach_item_part(item, type_name::<P>(), Arc::new(part));
    }
    /// Answer item capability `key` for item `item` (`mymod:backpack`) with `provider`, which
    /// gives a handler for the stack at one place, or none: `r.item_capability(&ITEM_ITEMS,
    /// "mymod:backpack", |_, _| Some(Box::new(Backpack)))`. Asked as
    /// `block_entity_capability`'s are.
    pub fn item_capability<T: ?Sized + 'static>(
        &mut self,
        key: &ItemCapabilityKey<T>,
        item: &str,
        provider: impl Fn(&mut dyn ServerHost, &ItemPlace) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        self.provide_item_capability(
            key.name,
            item,
            false,
            ItemProviderOf::of(Arc::new(provider)),
        );
    }
    /// `item_capability`, asked ahead of the game's own provider.
    pub fn item_capability_before_game<T: ?Sized + 'static>(
        &mut self,
        key: &ItemCapabilityKey<T>,
        item: &str,
        provider: impl Fn(&mut dyn ServerHost, &ItemPlace) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        self.provide_item_capability(key.name, item, true, ItemProviderOf::of(Arc::new(provider)));
    }
}

/// Whether this item's resolved Parts subscribe to `hook`, before stack conversion.
pub fn item_listens(hook: HookId, id: &str) -> bool {
    if !has_part_hook(hook) {
        return false;
    }
    parts(hook, "item routing")
        .for_hook(hook)
        .is_some_and(|table| !table.item(|| Some(id)).is_empty())
}

/// Prepare the subscribed item parts once, before traversing candidate slots/entities.
/// The source API stack must retain its address until these leases are dropped.
// hidden: engine-owned equipment scopes retain transport leases; not a mod callback.
#[doc(hidden)]
pub fn prepare_equip_parts(
    stack: &crate::ItemStack,
    kinds: &[&str],
    reads: &[crate::EquipReadRow],
) -> Result<Vec<Box<dyn std::any::Any>>, crate::wire::WireError> {
    let parts = parts(HookId::CanEquip, "equip preparation");
    let Some(table) = parts.for_hook(HookId::CanEquip) else {
        return Ok(Vec::new());
    };
    let mut leases = Vec::new();
    for part in table.item(|| crate::content::registry_keys::item_id(stack.item())) {
        if !crate::mod_index_on(part.mod_index) {
            continue;
        }
        if let Some(lease) = part
            .part
            .__prepare_equip(stack, kinds, reads)
            .map_err(crate::wire::WireError::Invalid)?
        {
            leases.push(lease);
        }
    }
    Ok(leases)
}

/// Registrar operations for items.
pub trait ItemsRegistrar {
    /// Request an item by registry id, validated after all declarations.
    fn item_key(&mut self, id: &str) -> crate::ItemKey;

    /// Attach `part`, of kind `kind`, to item `item`.
    fn attach_item_part(
        &mut self,
        item: crate::ItemKey,
        kind: &'static str,
        part: Arc<dyn ItemPart>,
    );

    /// Attach `part`, of kind `kind`, to every item in tag `tag`.
    fn attach_item_part_tag(&mut self, tag: &str, kind: &'static str, part: Arc<dyn ItemPart>);

    /// Answer item capability `name` for item `item` with `provider`; mods reach it through
    /// `item_capability`.
    fn provide_item_capability(
        &mut self,
        name: &'static str,
        item: &str,
        before_game: bool,
        provider: ItemProviderOf,
    );

    /// Read and write the item places `ItemPlace::Mod { provider: name, .. }` with `provider`,
    /// as an accessory mod exposes its slots. One provider a name.
    fn place_provider(&mut self, name: &'static str, provider: Arc<dyn PlaceProvider>);

    /// Register `appender` as the lines data component `component` (`mymod:charge`) adds to a
    /// stack's tooltip, where `placement` puts them (NeoForge's `RegisterTooltipAppendersEvent`).
    /// Only this mod's appender is asked, for a stack that carries the component and whose
    /// tooltip shows it, and only while the mod is enabled. A component id that isn't namespaced,
    /// or a `Before`/`After` anchor the game doesn't list as a component detail, refuses the mods
    /// at startup, naming the mod.
    fn register_tooltip_appender(
        &mut self,
        component: &str,
        placement: crate::TooltipPlacement,
        appender: Arc<dyn crate::TooltipAppender>,
    );
}

/// CapabilityProviders operations for items.
// hidden: per-kind lookup implementation, exposed for generated provider bindings
#[doc(hidden)]
pub trait ItemsCapabilityProviders {
    /// The next enabled provider of item capability `name` for item `item`, advancing `index`.
    fn item_provider(
        &self,
        name: &str,
        item: crate::ItemKey,
        index: &mut u32,
    ) -> Option<(ItemProviderOf, &'static str, bool)>;

    /// The enabled provider of the item places named `name`, with its mod.
    fn place_provider(&self, name: &str) -> Option<(Arc<dyn PlaceProvider>, &'static str)>;
}

impl ItemsCapabilityProviders for InstalledProviders {
    fn item_provider(
        &self,
        name: &str,
        item: crate::ItemKey,
        index: &mut u32,
    ) -> Option<(ItemProviderOf, &'static str, bool)> {
        let item = crate::content::registry_keys::item_id(item)?;
        next(&installed()?.keys.item_capabilities, name, item, index)
    }

    fn place_provider(&self, name: &str) -> Option<(Arc<dyn PlaceProvider>, &'static str)> {
        let part = installed()?.keys.place_providers.get(name)?;
        crate::mod_index_on(part.mod_index).then(|| (part.provider.clone(), part.mod_id))
    }
}
