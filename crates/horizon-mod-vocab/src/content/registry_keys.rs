//! Load-owned declarations and resolved mappings for registration-slot keys.
use crate::{
    BlockEntityTypeKey, BlockKey, DimensionKey, EffectKey, EnchantmentKey, EntityTypeKey, FluidKey,
    ItemKey, MenuKey,
};

impl BlockKey {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let known =
            crate::parts::registration_membership(crate::parts::RegistrationKeyKind::Block, slot)
                .unwrap_or_else(|| (block_names()).id(slot).is_some());
        known.then_some(Self(slot)).ok_or("unknown BlockKey slot")
    }
}

impl FluidKey {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let known =
            crate::parts::registration_membership(crate::parts::RegistrationKeyKind::Fluid, slot)
                .unwrap_or_else(|| {
                    (crate::parts::resolved_fluids()
                        .get()
                        .unwrap_or(&crate::parts::requested_keys().fluids))
                    .id(slot)
                    .is_some()
                });
        known.then_some(Self(slot)).ok_or("unknown FluidKey slot")
    }
}

impl ItemKey {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let known =
            crate::parts::registration_membership(crate::parts::RegistrationKeyKind::Item, slot)
                .unwrap_or_else(|| {
                    (resolved_items()
                        .get()
                        .unwrap_or(&crate::parts::requested_keys().items))
                    .id(slot)
                    .is_some()
                });
        known.then_some(Self(slot)).ok_or("unknown ItemKey slot")
    }
}

fn resolved_items() -> &'static std::sync::OnceLock<horizon_mod_runtime::registry_keys::Names> {
    static EMPTY: std::sync::OnceLock<horizon_mod_runtime::registry_keys::Names> =
        std::sync::OnceLock::new();
    crate::parts::installed().map_or(&EMPTY, |installed| &installed.resolved_items)
}

/// Complete the load-owned item table with all registry entries, preserving requested slots.
/// Missing forward references are refused before publishing any keys.
pub fn resolve_items<'a>(
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<(), Vec<crate::parts::ItemKeyError>> {
    let names = crate::parts::requested_keys()
        .items
        .complete(ids, |mod_id, id| crate::parts::ItemKeyError {
            mod_id,
            id: id.to_owned(),
        })?;
    let _ = resolved_items().set(names);
    Ok(())
}

fn resolved_block_keys()
-> &'static std::sync::OnceLock<horizon_mod_runtime::block_definitions::Blocks> {
    static EMPTY: std::sync::OnceLock<horizon_mod_runtime::block_definitions::Blocks> =
        std::sync::OnceLock::new();
    crate::parts::installed().map_or(&EMPTY, |installed| &installed.resolved_blocks)
}

fn block_names() -> &'static horizon_mod_runtime::registry_keys::Names {
    resolved_block_keys()
        .get()
        .map(|blocks| &blocks.names)
        .unwrap_or(&crate::parts::requested_keys().blocks)
}

/// Publish completed identities and immutable definitions together before constructing mesh tables.
/// World-number mappings remain owning-thread data.
pub fn resolve_block_keys<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    definition: impl FnMut(
        &str,
    )
        -> Result<horizon_mod_runtime::block_definitions::BlockDefinition, String>,
) -> Result<(), Vec<String>> {
    let names = crate::parts::requested_keys()
        .blocks
        .complete(ids, |mod_id, id| {
            format!("mod {mod_id} requested unknown block {id}")
        })?;
    let blocks = horizon_mod_runtime::block_definitions::Blocks::prepare(names, definition)?;
    let _ = resolved_block_keys().set(blocks);
    Ok(())
}

/// Find a block in the completed registry without allocating at a hook boundary.
pub fn block_key(id: &str) -> Option<BlockKey> {
    Some(BlockKey(*resolved_block_keys().get()?.names.slots.get(id)?))
}

/// Borrow the immutable definitions published with completed block identities.
pub fn block_definitions() -> Option<&'static horizon_mod_runtime::block_definitions::Blocks> {
    resolved_block_keys().get()
}

/// Find an item in the completed registry without allocating at a hook boundary.
pub fn item_key(id: &str) -> Option<ItemKey> {
    Some(ItemKey(*resolved_items().get()?.slots.get(id)?))
}

/// Resolve a typed identifier at a load boundary without formatting it as a string.
pub fn item_key_by_identifier(id: &crate::Identifier) -> Option<ItemKey> {
    Some(ItemKey(*resolved_items().get()?.identifiers.get(id)?))
}

/// Entity types use the same requested-slot and immutable resolved-table lifecycle as items.
fn resolved_entity_types() -> &'static std::sync::OnceLock<horizon_mod_runtime::registry_keys::Names>
{
    static EMPTY: std::sync::OnceLock<horizon_mod_runtime::registry_keys::Names> =
        std::sync::OnceLock::new();
    crate::parts::installed().map_or(&EMPTY, |installed| &installed.resolved_entity_types)
}
static MENUS: std::sync::LazyLock<
    std::sync::RwLock<Option<horizon_mod_runtime::registry_keys::Names>>,
> = std::sync::LazyLock::new(|| std::sync::RwLock::new(None));
static ENCHANTMENTS: std::sync::LazyLock<
    std::sync::RwLock<Option<horizon_mod_runtime::registry_keys::Names>>,
> = std::sync::LazyLock::new(|| std::sync::RwLock::new(None));

fn resolve_ordered<'a>(
    table: &std::sync::RwLock<Option<horizon_mod_runtime::registry_keys::Names>>,
    ids: impl IntoIterator<Item = &'a str>,
    kind: &str,
) -> Result<(), Vec<String>> {
    let mut table = table.write().unwrap_or_else(|error| error.into_inner());
    let names = table.get_or_insert_with(Default::default);
    let mut order = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        let slot = names
            .request("horizon", id)
            .map_err(|error| vec![format!("invalid {kind} registry key {id:?}: {error}")])?;
        order.push(slot);
    }
    names.order = order;
    Ok(())
}

fn ordered_keys<K>(
    table: &std::sync::RwLock<Option<horizon_mod_runtime::registry_keys::Names>>,
    key: impl Fn(u32) -> K,
) -> Vec<K> {
    let order = table
        .read()
        .ok()
        .and_then(|names| names.as_ref().map(|names| names.order.clone()))
        .unwrap_or_default();
    order.into_iter().map(key).collect()
}

fn ordered_id(
    table: &std::sync::RwLock<Option<horizon_mod_runtime::registry_keys::Names>>,
    slot: u32,
) -> Option<String> {
    table.read().ok()?.as_ref()?.id(slot).map(str::to_owned)
}

#[cfg(any(not(target_arch = "wasm32"), test))]
fn ordered_slot_is_live(
    table: &std::sync::RwLock<Option<horizon_mod_runtime::registry_keys::Names>>,
    slot: u32,
) -> bool {
    table
        .read()
        .ok()
        .and_then(|table| table.as_ref().map(|names| names.id(slot).is_some()))
        .unwrap_or(false)
}

/// Complete the built-in entity-type registry in `ids`' registration order.
pub fn resolve_entity_types<'a>(ids: impl IntoIterator<Item = &'a str>) -> Result<(), Vec<String>> {
    let names = crate::parts::requested_keys()
        .entity_types
        .complete(ids, |mod_id, id| {
            format!("mod {mod_id} requested unknown entity type {id}")
        })?;
    let _ = resolved_entity_types().set(names);
    Ok(())
}

/// Complete the built-in menu registry in `ids`' registration order.
pub fn resolve_menus<'a>(ids: impl IntoIterator<Item = &'a str>) -> Result<(), Vec<String>> {
    resolve_ordered(&MENUS, ids, "menu")
}

/// Complete the data-loaded enchantment registry in `ids`' registry order.
pub fn resolve_enchantments<'a>(ids: impl IntoIterator<Item = &'a str>) -> Result<(), Vec<String>> {
    resolve_ordered(&ENCHANTMENTS, ids, "enchantment")
}

/// Every block key in vanilla's registry order, mod blocks after vanilla's.
pub fn block_keys() -> Vec<BlockKey> {
    let order = resolved_block_keys()
        .get()
        .map_or_else(Vec::new, |blocks| blocks.names.order.clone());
    order.into_iter().map(BlockKey).collect()
}

/// Every item key in vanilla's registry order, mod items after vanilla's.
pub fn item_keys() -> Vec<ItemKey> {
    resolved_items()
        .get()
        .map_or_else(Vec::new, |names| names.order.clone())
        .into_iter()
        .map(ItemKey)
        .collect()
}

/// Resolve an entity type without materializing an owned name at a read boundary.
pub fn resolved_entity_type_key(id: &str) -> Option<EntityTypeKey> {
    if !id.contains(':') {
        return resolved_entity_type_key_parts(&["minecraft:", id]);
    }
    let names = resolved_entity_types().get()?;
    Some(EntityTypeKey(*names.slots.get(id)?))
}

/// Resolve a composed engine name (notably boat/raft types) without heap formatting.
pub fn resolved_entity_type_key_parts(parts: &[&str]) -> Option<EntityTypeKey> {
    let names = resolved_entity_types().get()?;
    names.slots.iter().find_map(|(name, slot)| {
        name.bytes()
            .eq(parts.iter().flat_map(|part| part.bytes()))
            .then_some(EntityTypeKey(*slot))
    })
}

/// Every built-in entity-type key in registry order.
pub fn entity_type_keys() -> Vec<EntityTypeKey> {
    resolved_entity_types()
        .get()
        .map_or_else(Vec::new, |names| {
            names
                .order
                .iter()
                .map(|slot| EntityTypeKey(*slot))
                .collect()
        })
}

/// Every built-in menu key in registry order.
pub fn menu_keys() -> Vec<MenuKey> {
    ordered_keys(&MENUS, MenuKey)
}

/// Every loaded enchantment key in registry order.
pub fn enchantment_keys() -> Vec<EnchantmentKey> {
    ordered_keys(&ENCHANTMENTS, EnchantmentKey)
}

impl EntityTypeKey {
    /// Validate an incoming slot against the built-in registry, not a world's mapping.
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::registration_membership(crate::parts::RegistrationKeyKind::EntityType, slot)
            .unwrap_or_else(|| {
                resolved_entity_types()
                    .get()
                    .is_some_and(|names| names.id(slot).is_some())
            })
            .then_some(Self(slot))
            .ok_or("unknown EntityTypeKey slot")
    }
}

impl MenuKey {
    /// Validate an incoming slot against the built-in registry, not a world's mapping.
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        ordered_slot_is_live(&MENUS, slot)
            .then_some(Self(slot))
            .ok_or("unknown MenuKey slot")
    }
}

impl EnchantmentKey {
    /// Validate an incoming slot against the loaded registry, not a world's mapping.
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        ordered_slot_is_live(&ENCHANTMENTS, slot)
            .then_some(Self(slot))
            .ok_or("unknown EnchantmentKey slot")
    }
}

/// The canonical name of a built-in entity-type key, for diagnostics.
pub fn entity_type_key_id(key: EntityTypeKey) -> Option<String> {
    crate::parts::registration_name(crate::parts::RegistrationKeyKind::EntityType, key.0)
        .or_else(|| resolved_entity_types().get()?.id(key.0).map(str::to_owned))
}

/// The canonical name of a built-in menu key, for diagnostics.
pub fn menu_key_id(key: MenuKey) -> Option<String> {
    ordered_id(&MENUS, key.0)
}

/// The canonical name of a loaded enchantment key, for diagnostics.
pub fn enchantment_key_id(key: EnchantmentKey) -> Option<String> {
    ordered_id(&ENCHANTMENTS, key.0)
}

impl EffectKey {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let known =
            crate::parts::registration_membership(crate::parts::RegistrationKeyKind::Effect, slot)
                .unwrap_or_else(|| {
                    (crate::parts::resolved_effects()
                        .get()
                        .map_or(&crate::parts::requested_keys().effects, |effects| {
                            &effects.names
                        }))
                    .id(slot)
                    .is_some()
                });
        known.then_some(Self(slot)).ok_or("unknown EffectKey slot")
    }
}

impl BlockEntityTypeKey {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let known = crate::parts::registration_membership(
            crate::parts::RegistrationKeyKind::BlockEntityType,
            slot,
        )
        .unwrap_or_else(|| {
            (crate::parts::resolved_block_entity_types()
                .get()
                .unwrap_or(&crate::parts::requested_keys().block_entity_types))
            .id(slot)
            .is_some()
        });
        known
            .then_some(Self(slot))
            .ok_or("unknown BlockEntityTypeKey slot")
    }
}

impl DimensionKey {
    /// `minecraft:overworld`.
    pub const OVERWORLD: Self = Self(0);
    /// `minecraft:the_nether`.
    pub const NETHER: Self = Self(1);
    /// `minecraft:the_end`.
    pub const END: Self = Self(2);

    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let key = Self(slot);
        dimension_id(key).ok_or("unknown DimensionKey slot")?;
        Ok(key)
    }
}

/// The key of dimension `id`, numbering it when the game meets it for the first time.
pub fn dimension_key(id: &str) -> DimensionKey {
    DimensionKey(horizon_mod_runtime::registry_keys::dimension_slot(id))
}

/// The name of a dimension's key, None for a slot the game never gave out.
pub fn dimension_id(key: DimensionKey) -> Option<&'static str> {
    horizon_mod_runtime::registry_keys::dimension_id(key.0)
}

// no wire: process-local registration diagnostic retains a static hook or mod identity.
/// A fluid requested by a mod but absent from the completed registry.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnknownFluid {
    /// Mod that requested the fluid.
    pub mod_id: &'static str,
    /// Canonical fluid identifier.
    pub id: String,
}

impl std::fmt::Display for UnknownFluid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mod {} requested unknown fluid {}", self.mod_id, self.id)
    }
}

/// Resolve fluid requests against the loaded behavior registry.
pub fn resolve_fluids(ids: &[&str], world_fluids: &[&str]) -> Result<(), Vec<UnknownFluid>> {
    let requests = crate::parts::requested_keys();
    let mut names = requests.fluids.clone();
    let mut errors = Vec::new();
    for entry in &names.entries {
        if !ids.contains(&entry.id.as_str()) && !requests.declared_fluids.contains_key(&entry.id) {
            errors.extend(entry.mods.iter().map(|mod_id| UnknownFluid {
                mod_id,
                id: entry.id.clone(),
            }));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    for id in ids {
        names
            .request("horizon", id)
            .expect("registry owns valid ids");
    }
    for (slot, entry) in names.entries.iter().enumerate() {
        names
            .utf16_slots
            .insert(entry.id.encode_utf16().collect(), slot as u32);
        if let Some(path) = entry.id.strip_prefix("minecraft:") {
            names
                .utf16_slots
                .insert(path.encode_utf16().collect(), slot as u32);
        }
    }
    names.world_fluids = world_fluids
        .iter()
        .map(|name| names.slots.get(*name).copied())
        .collect();
    let _ = crate::parts::resolved_fluids().set(names);
    Ok(())
}

/// Resolve an engine fluid kind through the load-owned mapping.
pub fn world_fluid_key(index: usize) -> Option<FluidKey> {
    crate::parts::resolved_fluids()
        .get()?
        .world_fluids
        .get(index)
        .copied()
        .flatten()
        .map(FluidKey)
}

/// Find a fluid key at a data-loading boundary; a bare name means `minecraft:`.
pub fn fluid_key(id: &str) -> Option<FluidKey> {
    let names = crate::parts::resolved_fluids().get()?;
    let slot = if id.contains(':') {
        names.slots.get(id)
    } else {
        names.slots.get(&format!("minecraft:{id}"))
    };
    Some(FluidKey(*slot?))
}

/// Resolve an NBT UTF-16 fluid name without allocating.
pub fn fluid_key_utf16(id: &[u16]) -> Option<FluidKey> {
    Some(FluidKey(
        *crate::parts::resolved_fluids().get()?.utf16_slots.get(id)?,
    ))
}

/// The canonical registry name of a resolved fluid key.
pub fn fluid_id(key: FluidKey) -> Option<&'static str> {
    crate::parts::resolved_fluids().get()?.id(key.0)
}

/// Resolve all block entity types after registration and content loading.
pub fn resolve_block_entity_types<'a, T>(
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<T>,
) -> Result<(), Vec<String>> {
    let names = horizon_mod_runtime::registry_keys::resolve_block_entity_types(
        &crate::parts::requested_keys().block_entity_types,
        ids,
        resolve,
    )?;
    let _ = crate::parts::resolved_block_entity_types().set(names);
    Ok(())
}

/// Lookup an engine-owned name after the registry is resolved.
pub fn block_entity_type_key(id: &str) -> Option<BlockEntityTypeKey> {
    Some(BlockEntityTypeKey(
        *crate::parts::resolved_block_entity_types()
            .get()?
            .slots
            .get(id)?,
    ))
}

/// Canonical name of a load-owned block entity type slot.
pub fn block_entity_type_name(key: BlockEntityTypeKey) -> Option<&'static str> {
    crate::parts::resolved_block_entity_types()
        .get()
        .unwrap_or(&crate::parts::requested_keys().block_entity_types)
        .id(key.0)
}

pub(crate) fn block_entity_type_display_name(key: BlockEntityTypeKey) -> String {
    crate::parts::registration_name(crate::parts::RegistrationKeyKind::BlockEntityType, key.0)
        .or_else(|| block_entity_type_name(key).map(str::to_owned))
        .unwrap_or_else(|| "<invalid>".into())
}

/// Resolve requested effects once, after every mod's content has been registered.
pub fn resolve_effects<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<usize>,
) -> Result<(), Vec<String>> {
    let effects = horizon_mod_runtime::registry_keys::resolve_effects(
        &crate::parts::requested_keys().effects,
        ids,
        resolve,
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
    })?;
    let _ = crate::parts::resolved_effects().set(effects);
    Ok(())
}

/// Resolve an effect operation to its load-owned engine registry index.
pub fn effect_index(key: EffectKey) -> Option<usize> {
    crate::parts::resolved_effects()
        .get()?
        .indices
        .get(key.0 as usize)
        .copied()
}

/// Canonical diagnostic name, unavailable until the process effect registry is resolved.
pub fn effect_id(key: EffectKey) -> Option<&'static str> {
    Some(&crate::parts::resolved_effects().get()?.names.entries[key.0 as usize].id)
}

/// Canonical diagnostic name of a trusted, load-owned slot, independent of world resolution.
pub fn block_id(key: BlockKey) -> Option<&'static str> {
    block_names().id(key.0)
}

/// Canonical diagnostic name retained by an item registration slot.
pub fn item_id(key: ItemKey) -> Option<&'static str> {
    resolved_items()
        .get()
        .unwrap_or(&crate::parts::requested_keys().items)
        .id(key.0)
}

/// Find an effect by its full registry name in the completed registry, without allocating.
pub fn effect_key(id: &str) -> Option<EffectKey> {
    Some(EffectKey(
        *crate::parts::resolved_effects()
            .get()?
            .names
            .slots
            .get(id)?,
    ))
}

/// The API key for an engine registry index, also for entries not explicitly requested by mods.
pub fn effect_key_by_index(index: usize) -> Option<EffectKey> {
    crate::parts::resolved_effects()
        .get()?
        .keys
        .get(index)
        .copied()
        .map(EffectKey)
}

impl crate::BoolGameRuleKey {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let known = crate::parts::registration_membership(
            crate::parts::RegistrationKeyKind::GameRule,
            slot,
        )
        .unwrap_or_else(|| crate::parts::requested_keys().game_rules.id(slot).is_some());
        known.then_some(Self(slot)).ok_or("unknown bool rule slot")
    }
}
