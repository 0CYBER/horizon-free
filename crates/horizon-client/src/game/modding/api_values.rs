// game-module-visibility: pub(crate)
//! The game-owned conversion boundary between API records and engine values.
//! Rust's orphan rule prevents foreign From/TryFrom implementations here.
//! The traits and the conversions that need no `Game` live in `horizon_gameplay::api_values`; what stays
//! here names `Game`, `GameLevel` or a client type.
#[cfg(test)]
use horizon_core::nbt::{JavaString, Tag};
pub(crate) use horizon_gameplay::api_values::{
    BlockEntitySnapshotError, StackReadError, TryIntoApi, TryIntoEngine,
    TryIntoEnginePreservingCount, block_metadata_table, finish_block_entity_writeback, stack_value,
};
use horizon_items::items::{ItemStack as EngineStack, Items};
use mod_api::item::item_stack::{ItemStack as ApiStack, StackWriteError};
#[path = "api_values/aggregates.rs"]
pub(in crate::game) mod aggregates;
#[path = "api_values/block_edit.rs"]
pub(in crate::game) mod block_edit;
#[path = "api_values/block_state.rs"]
pub(in crate::game) mod block_state;

pub(crate) fn block_jump_factor(
    context: &horizon_world::native_load_context::LoopContext,
    state: horizon_world::registry::BlockStateId,
    factor: &mut f32,
) {
    if !block_state::block_listens(mod_api::HookId::BlockJumpFactor, state) {
        return;
    }
    let Ok(state) = state.try_into_api() else {
        return;
    };
    mod_api::dispatch_block_jump_factor(
        &horizon_client_render::api_context::LoopReader(context).context(),
        state,
        factor,
    );
}
pub(crate) fn block_speed_factor(
    context: &horizon_world::native_load_context::LoopContext,
    state: horizon_world::registry::BlockStateId,
    factor: &mut f32,
) {
    if !block_state::block_listens(mod_api::HookId::BlockSpeedFactor, state) {
        return;
    }
    let Ok(state) = state.try_into_api() else {
        return;
    };
    mod_api::dispatch_block_speed_factor(
        &horizon_client_render::api_context::LoopReader(context).context(),
        state,
        factor,
    );
}
pub(crate) fn is_ladder(
    context: &horizon_world::native_load_context::LoopContext,
    entity: u32,
    pos: horizon_core::position::BlockPos,
    state: horizon_world::registry::BlockStateId,
    ladder: &mut bool,
) {
    if !block_state::block_listens(mod_api::HookId::IsLadder, state) {
        return;
    }
    let Ok(state) = state.try_into_api() else {
        return;
    };
    mod_api::dispatch_is_ladder(
        &horizon_client_render::api_context::LoopReader(context).context(),
        entity,
        pos,
        state,
        ladder,
    );
}

/// Resolve the metadata default through the sole numeric reverse index, without allocation.
pub(crate) fn default_state(
    block: mod_api::BlockKey,
) -> Option<horizon_world::registry::BlockStateId> {
    use mod_api::BlockMetadata;
    let metadata = mod_api::__private::block_definitions()?;
    let value = mod_api::BlockState::from(metadata.of(block, &[]).ok()?);
    block_state::current().ok()?.to_engine(value).ok()
}

pub(crate) fn behavior_block(
    block: mod_api::BlockKey,
    behaviours: &horizon_world::level::Behaviours,
) -> Option<horizon_world::level::states::BlockId> {
    Some(behaviours.block(default_state(block)?))
}
pub(crate) use super::native_ground::alter_ground;
pub(in crate::game) use super::native_worldgen::prepare as prepare_block_states;
pub(in crate::game) use block_state::install as install_block_states;
#[cfg(test)]
#[path = "api_values/inside_write.rs"]
mod inside_write;
#[cfg(test)]
#[path = "api_values/landing_write.rs"]
mod landing_write;
#[path = "api_values/recipe_inputs.rs"]
pub(in crate::game) mod recipe_inputs;

/// Publish a loaded item table's registry identities: the item keys in the table's
/// command-registry order, then the enchantment keys in the loaded registry's order.
/// Both tables load together, so one call keeps the two orderings from drifting.
pub(in crate::game) fn resolve_item_registry(items: &crate::items::Items) -> Result<(), String> {
    mod_api::__private::resolve_items(items.registry_order.iter().map(|id| &**id)).map_err(
        |errors| {
            errors
                .into_iter()
                .map(|error| format!("mod {} requested unknown item {}", error.mod_id, error.id))
                .collect::<Vec<_>>()
                .join("\n")
        },
    )?;
    resolve_enchantment_registry(items)
}

/// Publish the loaded enchantment registry's keys in `RegistryDataLoader`'s order,
/// separately for a path that reloads only the data-pack tables.
pub(in crate::game) fn resolve_enchantment_registry(
    items: &crate::items::Items,
) -> Result<(), String> {
    mod_api::__private::resolve_enchantments(items.enchantments.list.iter().map(|entry| &*entry.id))
        .map_err(|errors| errors.join("\n"))
}

/// Publish immutable metadata with completed keys before any mesh tables or callbacks use it.
/// Registry definitions do not change on pack reload; world-number mappings are separate.
pub(in crate::game) fn resolve_block_definitions(
    registry: &horizon_world::registry::BlockRegistry,
) -> Result<(), String> {
    // Without installed parts the completed keys land in one process-wide table that keeps the
    // first registry resolved; in a test process that is whichever test got there first, so a
    // test that installed no parts gets its own empty installation, as tests do their mods.
    #[cfg(test)]
    if !mod_api::__private::parts_installation::parts_installed() {
        mod_platform::__private::part_tests::install_parts_fixture(&[])
            .map_err(|errors| errors.join("\n"))?;
    }
    block_state::clear();
    if mod_api::__private::block_definitions().is_some() {
        return bind_worker_blocks(registry);
    }
    let mut blocks = std::collections::BTreeMap::<_, Vec<_>>::new();
    // The map is sorted for lookup, but keys publish in registry order: the
    // first state that names a block is that block's registration position.
    let mut ordered = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for index in 0..registry.len() {
        let state = registry
            .state(horizon_world::registry::BlockStateId(index as u32))
            .expect("registry index is in bounds");
        if seen.insert(state.name.as_str()) {
            ordered.push(state.name.as_str());
        }
        blocks
            .entry(state.name.as_str())
            .or_default()
            .push((index, &state.properties));
    }
    mod_api::__private::resolve_block_keys(ordered, |name| {
        let states = &blocks[name];
        let default = registry
            .default_state(name)
            .expect("registry owns block defaults");
        let ordinal = states
            .iter()
            .position(|(index, _)| *index == default.0 as usize)
            .expect("default belongs to block");
        let states: Vec<std::collections::BTreeMap<_, _>> = states
            .iter()
            .map(|(_, properties)| {
                properties
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect()
            })
            .collect();
        let states: Vec<_> = states.iter().collect();
        horizon_mod_runtime::block_definitions::BlockDefinition::new(&states, ordinal as u16)
            .map_err(|error| format!("block {name}: {error:?}"))
    })
    .map_err(|errors| errors.join("\n"))?;
    bind_worker_blocks(registry)
}

fn bind_worker_blocks(registry: &horizon_world::registry::BlockRegistry) -> Result<(), String> {
    block_state::bind(registry)?;
    let index = block_state::current().map_err(|error| format!("worker index: {error:?}"))?;
    let retained =
        super::native_worldgen::retain(registry, index).inspect_err(|_| block_state::clear())?;
    block_state::install(retained);
    Ok(())
}

#[cfg(test)]
#[test]
fn a_test_without_parts_resolves_its_own_blocks_after_another_resolved_fewer() {
    // Another test thread, with no parts installed, resolves a registry of air alone first.
    std::thread::spawn(|| {
        let air =
            r#"{"defaults":{"minecraft:air":0},"states":[["air",true,false,"","minecraft:air"]]}"#;
        let air = horizon_world::registry::BlockRegistry::from_facts_json(air.as_bytes()).unwrap();
        resolve_block_definitions(&air).unwrap();
    })
    .join()
    .unwrap();
    // This one, with no parts installed either, still resolves its own block.
    let own = r#"{"defaults":{"minecraft:air":0,"regression:own_block":1},"states":[
        ["air",true,false,"","minecraft:air"],
        ["b",false,false,"","regression:own_block"]
    ]}"#;
    let own = horizon_world::registry::BlockRegistry::from_facts_json(own.as_bytes()).unwrap();
    resolve_block_definitions(&own).unwrap();
    let state = own
        .state(horizon_world::registry::BlockStateId(1))
        .unwrap()
        .try_into_api();
    assert!(state.is_ok(), "{state:?}");
}

#[cfg(test)]
#[test]
fn block_definition_import_preserves_defaults_and_is_immutable_after_load() {
    let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":2},"states":[
        ["air",true,false,"","minecraft:air"],
        ["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north"}}],
        ["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east"}}]
    ]}"#;
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    let registry =
        horizon_world::registry::BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
    resolve_block_definitions(&registry).unwrap();
    let blocks = mod_api::__private::block_definitions().unwrap();
    let key = blocks.names.slots["minecraft:stairs"] as usize;
    assert_eq!(blocks.definitions[key].of(&[]), Ok(1));
    assert_eq!(
        blocks.definitions[key].property(0, "facing"),
        Ok(Some("north"))
    );
    assert_eq!(
        blocks.definitions[key].property(1, "facing"),
        Ok(Some("east"))
    );
    let changed = facts.replace("east", "south");
    let registry =
        horizon_world::registry::BlockRegistry::from_facts_json(changed.as_bytes()).unwrap();
    assert!(resolve_block_definitions(&registry).is_err());
    assert_eq!(
        mod_api::__private::block_definitions().unwrap().definitions[key].property(1, "facing"),
        Ok(Some("east"))
    );
}

#[path = "api_values/item_snapshot.rs"]
mod item_snapshot;
pub(in crate::game) use item_snapshot::ItemBoundary;

#[cfg(test)]
#[test]
fn block_values_keep_frozen_ordinals_when_world_state_ids_reorder() {
    use horizon_world::registry::{BlockRegistry, BlockStateId};
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    let first = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":2},"states":[["air",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east"}}]]}"#;
    let first = BlockRegistry::from_facts_json(first.as_bytes()).unwrap();
    resolve_block_definitions(&first).unwrap();
    let reordered = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":1},"states":[["air",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north"}}]]}"#;
    let reordered = BlockRegistry::from_facts_json(reordered.as_bytes()).unwrap();
    resolve_block_definitions(&reordered).unwrap();
    let east = reordered
        .state(BlockStateId(1))
        .unwrap()
        .try_into_api()
        .unwrap();
    let north = reordered
        .state(BlockStateId(2))
        .unwrap()
        .try_into_api()
        .unwrap();
    let metadata = block_metadata_table().unwrap();
    assert_eq!(east.ordinal(), 1);
    assert_eq!(north.ordinal(), 0);
    assert_eq!(east.property(metadata, "facing"), Ok(Some("east")));
    assert_eq!(north.property(metadata, "facing"), Ok(Some("north")));
    let mut invalid = reordered.state(BlockStateId(1)).unwrap().clone();
    invalid.properties.insert("facing".into(), "south".into());
    assert_eq!(
        invalid.try_into_api(),
        Err(mod_api::BlockStateError::InvalidValue(0))
    );
    invalid.name = "missing:block".into();
    assert_eq!(
        invalid.try_into_api(),
        Err(mod_api::BlockStateError::UnknownBlock)
    );
}

/// Prepare callback data before mesh construction; workers never convert engine records.
pub(in crate::game) fn mod_tint_values(
    _registry: &horizon_world::registry::BlockRegistry,
) -> Result<Option<crate::mesher::ModTintValues>, String> {
    if !crate::mesher::has_custom_block_tint_sources() && !crate::mesher::has_section_geometry() {
        return Ok(None);
    }
    let metadata =
        block_metadata_table().map_err(|error| format!("block tint metadata: {error:?}"))?;
    let states = block_state::snapshot().map_err(|error| format!("block tint state: {error:?}"))?;
    Ok(Some(crate::mesher::ModTintValues { states, metadata }))
}

impl super::Game<'_> {
    pub(in crate::game) fn block_metadata_name(
        &self,
        block: mod_api::BlockKey,
    ) -> Result<&str, mod_api::BlockStateError> {
        mod_api::BlockMetadata::name(block_metadata_table()?, block)
    }
    pub(in crate::game) fn block_metadata_property(
        &self,
        state: mod_api::BlockStateRecord,
        name: &str,
    ) -> Result<Option<&str>, mod_api::BlockStateError> {
        mod_api::BlockMetadata::property(block_metadata_table()?, state, name)
    }
    pub(in crate::game) fn block_metadata_with(
        &self,
        state: mod_api::BlockStateRecord,
        name: &str,
        value: &str,
    ) -> Result<mod_api::BlockStateRecord, mod_api::BlockStateError> {
        mod_api::BlockMetadata::with(block_metadata_table()?, state, name, value)
    }
    pub(in crate::game) fn block_metadata_of(
        &self,
        block: mod_api::BlockKey,
        properties: &[mod_api::StateConstraint],
    ) -> Result<mod_api::BlockStateRecord, mod_api::BlockStateError> {
        mod_api::BlockMetadata::of(block_metadata_table()?, block, properties)
    }
}

/// Normalize an existing game write's answer without changing its implementation.
/// Generated API forwarders add the checked-input refusal around successful native answers.
pub(in crate::game) trait IntoWriteResult {
    type Value;
    type Error;
    fn into_write_result(self) -> Result<Self::Value, Self::Error>;
}
impl<T> IntoWriteResult for Option<T> {
    type Value = Self;
    type Error = StackWriteError;
    fn into_write_result(self) -> Result<Self, Self::Error> {
        Ok(self)
    }
}
impl<T, E> IntoWriteResult for Result<T, E> {
    type Value = T;
    type Error = E;
    fn into_write_result(self) -> Self {
        self
    }
}
macro_rules! write_answers {
    ($($ty:ty),* $(,)?) => { $(
        impl IntoWriteResult for $ty {
            type Value = Self;
            type Error = StackWriteError;
            fn into_write_result(self) -> Result<Self, Self::Error> { Ok(self) }
        }
    )* };
}
write_answers!(bool, u32, ());

/// Tests use the production snapshot conversion, never a second fixture codec.
#[cfg(test)]
pub(in crate::game) fn test_snapshot<T: TryIntoApi + ?Sized>(value: &T) -> T::Api
where
    T::Error: std::fmt::Debug,
{
    value
        .try_into_api()
        .expect("fixture snapshot has resolved item keys and a nonempty count")
}

#[cfg(test)]
#[path = "api_values/content_declarations.rs"]
mod content_declarations;
pub(in crate::game) use horizon_gameplay::api_values::content_declarations::push_reaction;

#[cfg(test)]
#[path = "api_values/stack_count.rs"]
mod stack_count;

/// Execute a native write and preserve its answer, including unit and original errors.
pub(in crate::game) fn call_write<W: IntoWriteResult>(
    call: impl FnOnce() -> W,
) -> Result<W::Value, W::Error> {
    call().into_write_result()
}

/// Convert a successful stack-bearing write answer through its original error channel.
pub(in crate::game) fn call_stack_write<W>(
    call: impl FnOnce() -> W,
) -> Result<<W::Value as TryIntoApi>::Api, W::Error>
where
    W: IntoWriteResult,
    W::Value: TryIntoApi<Error = StackReadError>,
    W::Error: From<StackWriteError>,
{
    call_write(call)?
        .try_into_api()
        .map_err(|error| error.into_write_error().into())
}

impl TryIntoApi for super::spill::BlockItemDrop {
    type Api = mod_api::ItemDrop;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(self.state.to_api(mod_api::ItemDrop::new(
            self.stack.try_into_api()?,
            self.position,
            self.velocity,
            self.pickup_delay,
        )))
    }
}

/// Edited drops back as engine drops, as `Vec<ItemDrop>::try_into_engine` would. The engine type is
/// the game's own, so this cannot be a `TryIntoEngine` impl on `ItemDrop`: the trait and the type
/// both live in other crates.
pub(in crate::game) fn item_drops_into_engine(
    drops: &[mod_api::ItemDrop],
    items: &Items,
) -> Result<Vec<super::spill::BlockItemDrop>, StackWriteError> {
    drops
        .iter()
        .map(|drop| {
            let stack = drop.stack.try_into_engine(items)?;
            Ok(super::spill::BlockItemDrop::from_api(drop, stack))
        })
        .collect()
}

/// Install the engine-facing callbacks; snapshots never enter engine storage.
pub(crate) fn install_stack_hooks() {
    crate::native_stack_hooks::install(crate::native_stack_hooks::StackHooks {
        ingredient_test: |context, kind, data, stack, accept| {
            if !mod_api::__private::listened(mod_api::HookId::IngredientTest) {
                return;
            }
            if let Ok(stack) = stack.try_into_api() {
                horizon_client::crafting::native_hooks::native_ingredient_test(
                    context, kind, data, &stack, accept,
                );
            }
        },
        enchantments: |stack, target, value| {
            if !mod_api::__private::listened(mod_api::HookId::GetEnchantmentLevel) {
                return;
            }
            if let Ok(stack) = stack.try_into_api() {
                mod_api::item::enchantments::native_get_enchantment_level(&stack, target, value);
            }
        },
        attributes: |stack, value| {
            if !mod_api::__private::listened(mod_api::HookId::ItemAttributeModifier) {
                return;
            }
            if let Ok(stack) = stack.try_into_api() {
                mod_api::item::item_attributes::native_item_attribute_modifier(&stack, value);
            }
        },
    });
}

struct ApiSlot<'a> {
    engine: &'a mut dyn crate::menu::SlotView,
    invalid: bool,
}
impl mod_api::item::menu::SlotView for ApiSlot<'_> {
    fn item(&self) -> Option<ApiStack> {
        self.engine.item().and_then(|s| s.try_into_api().ok())
    }
    fn may_place(&self, stack: &ApiStack) -> bool {
        stack_value(stack).is_ok_and(|s| self.engine.may_place(&s))
    }
    fn may_pickup(&self) -> bool {
        self.engine.may_pickup()
    }
    fn allow_modification(&self) -> bool {
        self.engine.allow_modification()
    }
    fn safe_insert(&mut self, stack: Option<ApiStack>, count: u32) -> Option<ApiStack> {
        let value = match stack
            .as_ref()
            .map(|s| s.try_into_engine(self.engine.items()))
            .transpose()
        {
            Ok(s) => s,
            Err(_) => {
                self.invalid = true;
                return stack;
            }
        };
        match self.engine.safe_insert(value, count).try_into_api() {
            Ok(value) => value,
            Err(_) => {
                self.invalid = true;
                None
            }
        }
    }
    fn safe_take(&mut self, count: u32, max: u32) -> Option<ApiStack> {
        match self.engine.safe_take(count, max).try_into_api() {
            Ok(value) => value,
            Err(_) => {
                self.invalid = true;
                None
            }
        }
    }
}
/// Adapt a player's block menu `stillValid` check; the game asks only for a menu
/// whose type has a part answering it (`menu_still_valid_listens`, when it opened).
pub(crate) fn menu_still_valid(
    context: &horizon_world::native_load_context::LoopContext,
    player: u128,
    menu: &str,
    pos: [i32; 3],
    eye: [f64; 3],
    valid: &mut bool,
) {
    let reader = horizon_client_render::api_context::LoopReader(context);
    let context = reader.context();
    mod_api::dispatch_menu_still_valid(&context, player, menu, pos, eye, valid);
}

pub(crate) fn stacked_on_other(
    context: &horizon_world::native_load_context::LoopContext,
    event: &mut crate::menu::StackedOnOther<'_>,
    canceled: &mut bool,
) {
    if !mod_api::__private::listened(mod_api::HookId::ItemStackedOnOther) {
        return;
    }
    let Ok(mut carried) = event.carried.0.try_into_api() else {
        return;
    };
    let mut candidate_canceled = *canceled;
    let mut cancellation_result = event.cancellation_result;
    let mut admitted = None;
    let mut run = |clicked: &mut dyn crate::menu::SlotView| {
        let mut slot = ApiSlot {
            engine: clicked,
            invalid: false,
        };
        let mut api = mod_api::item::menu::StackedOnOther {
            player: event.player,
            player_entity_id: event.player_entity_id,
            slot: event.slot,
            container: event.container,
            container_slot: event.container_slot,
            primary: event.primary,
            carried: mod_api::item::menu::CarriedAccess(&mut carried),
            clicked: &mut slot,
            cancellation_result,
        };
        mod_api::dispatch_item_stacked_on_other(
            &horizon_client_render::api_context::LoopReader(context).context(),
            &mut api,
            &mut candidate_canceled,
        );
        cancellation_result = api.cancellation_result;
        if slot.invalid {
            return false;
        }
        let Ok(slot_value) = slot.engine.item().try_into_api() else {
            return false;
        };
        if slot_value.try_into_engine(slot.engine.items()).is_err() {
            return false;
        }
        match carried
            .as_ref()
            .map(|stack| stack.try_into_engine(slot.engine.items()))
            .transpose()
        {
            Ok(value) => {
                admitted = Some(value);
                true
            }
            Err(_) => false,
        }
    };
    let accepted = event.clicked.with_staged(&mut run);
    if accepted {
        *event.carried.0 = admitted.expect("an accepted cursor");
        event.cancellation_result = cancellation_result;
        *canceled = candidate_canceled;
    }
}

impl super::Game<'_> {
    pub(in crate::game) fn api_anvil_update(
        &mut self,
        player: u128,
        entity: i32,
        pos: [i32; 3],
        value: &mut crate::menu::AnvilUpdate,
        cancel: &mut bool,
    ) {
        let Ok(mut api) = value.try_into_api() else {
            return;
        };
        let original = api.clone();
        self.native_anvil_update(player, entity, pos, &mut api, cancel);
        if api == original {
            return;
        }
        match api.try_into_engine(self.items()) {
            Ok(result) => *value = result,
            Err(_) => *cancel = true,
        }
    }
    pub(in crate::game) fn api_grindstone_place(
        &mut self,
        player: u128,
        entity: i32,
        pos: [i32; 3],
        value: &mut crate::menu::GrindstonePlace,
        cancel: &mut bool,
    ) {
        let Ok(mut api) = value.try_into_api() else {
            return;
        };
        let original = api.clone();
        self.native_grindstone_place(player, entity, pos, &mut api, cancel);
        if api == original {
            return;
        }
        match api.try_into_engine(self.items()) {
            Ok(result) => *value = result,
            Err(_) => *cancel = true,
        }
    }
    pub(in crate::game) fn api_grindstone_take(
        &mut self,
        player: u128,
        entity: i32,
        pos: [i32; 3],
        value: &mut crate::menu::GrindstoneTake,
        cancel: &mut bool,
    ) {
        let Ok(mut api) = value.try_into_api() else {
            return;
        };
        let original = api.clone();
        self.native_grindstone_take(player, entity, pos, &mut api, cancel);
        if api == original {
            return;
        }
        match api.try_into_engine(self.items()) {
            Ok(result) => *value = result,
            Err(_) => *cancel = true,
        }
    }
    pub(in crate::game) fn api_anvil_craft_pre(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        value: &crate::menu::AnvilCraft,
        cancel: &mut bool,
    ) {
        if let Ok(api) = value.try_into_api() {
            self.native_anvil_craft_pre(player, name, pos, &api, cancel);
        }
    }
    pub(in crate::game) fn api_anvil_craft_post(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        value: &crate::menu::AnvilCraft,
    ) {
        if let Ok(api) = value.try_into_api() {
            self.native_anvil_craft_post(player, name, pos, &api);
        }
    }
}

#[cfg(test)]
mod attribute_boundary_tests {
    use super::EngineStack as ItemStack;
    use crate::item_attributes::attribute_modifiers_or;
    use horizon_items::item_data::ModifierRow;
    use mod_api::{
        AttributeModifier, AttributeOperation, EquipmentSlotGroup, Hooks, Identifier,
        ItemAttributeModifierEvent, ModEntry,
    };
    use mod_platform::testing::with_mods;
    use std::borrow::Cow;
    /// Makes a netherite chestplate heavier: +0.4 knockback resistance in
    /// the chest slot, and no armor toughness.
    struct Heavy;
    #[mod_api::__subscriptions]
    impl Hooks for Heavy {
        fn item_attribute_modifier(&self, event: &mut ItemAttributeModifierEvent<'_>) {
            if mod_api::__private::item_key_id(event.stack.item())
                != Some("minecraft:netherite_chestplate")
            {
                return;
            }
            let id = |text: &str| Identifier::parse(text).unwrap();
            let heavy = |amount| {
                AttributeModifier::new(id("test:heavy"), amount, AttributeOperation::AddValue)
            };
            let resistance = id("minecraft:knockback_resistance");
            assert!(
                event
                    .modifiers
                    .add(resistance.clone(), heavy(0.4), EquipmentSlotGroup::Chest)
            );
            // The id is taken on that attribute.
            assert!(
                !event
                    .modifiers
                    .add(resistance, heavy(1.0), EquipmentSlotGroup::Chest)
            );
            assert!(
                event
                    .modifiers
                    .remove_all_for(&id("minecraft:armor_toughness"))
            );
        }
    }

    fn chestplate() -> Vec<ModifierRow> {
        ["minecraft:armor", "minecraft:armor_toughness"]
            .map(|attribute| {
                ModifierRow::new(
                    attribute,
                    "minecraft:armor.chestplate",
                    3.0,
                    "add_value",
                    "chest",
                )
            })
            .to_vec()
    }

    /// A mod's edits are what the stack grants; a stack it leaves alone, and
    /// any stack with no mod listening, keeps the item's list borrowed.
    #[test]
    fn a_mod_edits_what_a_stack_grants() {
        super::prepare_stack_boundary_tests();
        let types = chestplate();
        let netherite = ItemStack::new("minecraft:netherite_chestplate", 1);
        let diamond = ItemStack::new("minecraft:diamond_chestplate", 1);
        with_mods(Vec::new().into(), || {
            assert!(matches!(
                attribute_modifiers_or(&netherite, &types),
                Cow::Borrowed(_)
            ));
        });
        with_mods(vec![ModEntry::new("heavy", Box::new(Heavy))].into(), || {
            assert_eq!(
                &*attribute_modifiers_or(&netherite, &types),
                [
                    types[0].clone(),
                    ModifierRow::new(
                        "minecraft:knockback_resistance",
                        "test:heavy",
                        0.4,
                        "add_value",
                        "chest"
                    ),
                ]
            );
            assert!(matches!(
                attribute_modifiers_or(&diamond, &types),
                Cow::Borrowed(_)
            ));
        });
    }

    /// A chestplate a server sent with modifiers of its own, worn for the first
    /// time: the first equipment pass (`collectEquipmentChanges` with nothing
    /// worn before) asks the mods about the decoded list, so the map's first
    /// values are the mod's answer.
    #[test]
    fn a_servers_stack_takes_the_mods_answer_on_first_equip() {
        use horizon_client::living_attributes::LivingAttributes;
        use horizon_items::equipment::{CHEST, Equipment};
        use horizon_protocol::buf::{Codec, Reader, Writer};
        use horizon_protocol::item::{Slot, component};
        use horizon_protocol::registry::{Builtin, Dynamic, Ids};
        use horizon_world::{attribute_map::ValueKind, attributes::entity::Catalog};
        use std::sync::Arc;
        super::prepare_stack_boundary_tests();
        let attributes = [
            "minecraft:armor",
            "minecraft:armor_toughness",
            "minecraft:knockback_resistance",
        ];
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut components = vec![String::new(); 17];
        components[component::ATTRIBUTE_MODIFIERS as usize] =
            "minecraft:attribute_modifiers".into();
        let builtin = Builtin {
            registries: [
                (
                    "minecraft:item".to_owned(),
                    Ids::from_names(names(&["minecraft:air", "minecraft:netherite_chestplate"])),
                ),
                (
                    "minecraft:data_component_type".to_owned(),
                    Ids::from_names(components),
                ),
                (
                    "minecraft:attribute".to_owned(),
                    Ids::from_names(names(&attributes)),
                ),
            ]
            .into(),
        };
        // The server's patch: 10 armor and 4 toughness in the chest slot group (6).
        let mut wire = Writer::new();
        for v in [1, 1, 1, 0, component::ATTRIBUTE_MODIFIERS, 2] {
            wire.varint(v);
        }
        for (attribute, amount) in [(0, 10.0), (1, 4.0)] {
            wire.varint(attribute);
            wire.string("test:forged");
            wire.f64(amount);
            for v in [0, 6, 0] {
                wire.varint(v);
            }
        }
        let sent = Slot::read(&mut Reader::new(&wire.data)).unwrap().unwrap();
        let chestplate =
            horizon_client::item_sync::from_network(&sent, &builtin, &Dynamic::default());
        let bits = |n: f64| format!("{:016x}", n.to_bits());
        let catalog = Arc::new(
            Catalog::from_json(&serde_json::json!({
                "data_version": horizon_world::DATA_VERSION,
                "attributes": attributes.map(|id| serde_json::json!({
                    "id": id, "default": bits(0.0), "min": bits(0.0), "max": bits(30.0),
                    "syncable": true
                })),
                "entities": {
                    "minecraft:armor_stand": serde_json::Map::from_iter(
                        attributes.map(|id| (id.to_owned(), serde_json::json!(bits(0.0))))
                    )
                }
            }))
            .unwrap(),
        );
        let items = super::stack_limit_tests::catalog();
        let mut equipment = Equipment::default();
        equipment.set(CHEST, Some(chestplate));
        with_mods(vec![ModEntry::new("heavy", Box::new(Heavy))].into(), || {
            let mut live = LivingAttributes::default();
            live.collect_equipment_changes(
                &catalog,
                "minecraft:armor_stand",
                &equipment,
                &items,
                Vec::new,
            )
            .unwrap();
            let value = |name| {
                live.read(
                    &catalog,
                    "minecraft:armor_stand",
                    name,
                    ValueKind::Computed,
                    Vec::new,
                )
            };
            // The server's armor, the mod's knockback resistance, no toughness.
            assert_eq!(value("minecraft:armor"), Some(10.0));
            assert_eq!(value("minecraft:knockback_resistance"), Some(0.4));
            assert_eq!(value("minecraft:armor_toughness"), Some(0.0));
        });
    }
}

#[cfg(test)]
mod enchantment_boundary_tests {
    use super::EngineStack as ItemStack;
    use crate::enchantment_levels::get_enchantment_level;
    use mod_api::{GetEnchantmentLevelEvent, Hooks, Identifier, ModEntry};
    use mod_platform::testing::with_mods;
    use std::borrow::Cow;
    use std::sync::Arc;
    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }
    /// An iron sword fights as if it had Looting II and no Sharpness.
    struct Keen;
    #[mod_api::__subscriptions]
    impl Hooks for Keen {
        fn get_enchantment_level(&self, event: &mut GetEnchantmentLevelEvent<'_>) {
            if mod_api::__private::item_key_id(event.stack.item()) != Some("minecraft:iron_sword") {
                return;
            }
            let looting = id("minecraft:looting");
            if event.target.is_none_or(|target| *target == looting) {
                event.enchantments.upgrade(looting, 2);
            }
            event.enchantments.set(id("minecraft:sharpness"), 0);
        }
    }

    /// Gameplay reads see the mod's levels; the stack's own component, a
    /// stack the mod leaves alone and every stack with no mod listening
    /// keep theirs, borrowed.
    #[test]
    fn a_mod_changes_the_levels_gameplay_reads() {
        super::prepare_stack_boundary_tests();
        let _ = horizon_items::native_enchantment_hooks::install(get_enchantment_level);
        let mut iron = ItemStack::new("minecraft:iron_sword", 1);
        iron.enchant("minecraft:sharpness", 3);
        let mut diamond = ItemStack::new("minecraft:diamond_sword", 1);
        diamond.enchant("minecraft:sharpness", 3);
        with_mods(Vec::new().into(), || {
            assert!(matches!(iron.all_enchantments(), Cow::Borrowed(_)));
            assert_eq!(iron.enchantment_level("minecraft:sharpness"), 3);
        });
        with_mods(vec![ModEntry::new("keen", Box::new(Keen))].into(), || {
            assert_eq!(
                &*iron.all_enchantments(),
                [(Arc::from("minecraft:looting"), 2)]
            );
            assert_eq!(iron.enchantment_level("minecraft:looting"), 2);
            assert_eq!(iron.enchantment_level("minecraft:sharpness"), 0);
            assert_eq!(iron.tag_enchantment_level("minecraft:sharpness"), 3);
            assert_eq!(iron.enchantments(), [(Arc::from("minecraft:sharpness"), 3)]);
            assert!(matches!(diamond.all_enchantments(), Cow::Borrowed(_)));
            assert_eq!(diamond.enchantment_level("minecraft:sharpness"), 3);
        });
    }
}

#[cfg(test)]
pub(in crate::game) fn prepare_stack_boundary_tests() {
    install_stack_hooks();
    install_parts(&[]).unwrap();
    mod_api::__private::resolve_items([
        "minecraft:iron_sword",
        "minecraft:stick",
        "minecraft:diamond_sword",
        "minecraft:netherite_chestplate",
        "minecraft:diamond_chestplate",
        "minecraft:potion",
    ])
    .unwrap();
}

#[cfg(test)]
mod brewed_boundary_tests {
    use super::*;
    struct NoHost;
    impl mod_api::Host for NoHost {}
    mod_api::impl_host_areas!(NoHost);
    impl mod_api::ServerHost for NoHost {}
    mod_api::impl_server_host_areas!(NoHost);
    struct Watch(std::sync::Arc<std::sync::Mutex<Vec<(u128, i32, String, u32)>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Watch {
        fn player_brewed_potion(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::PlayerBrewedPotionEvent<'_>,
        ) {
            self.0.lock().unwrap().push((
                event.player,
                event.player_entity_id,
                mod_api::__private::item_key_id(event.stack.item())
                    .unwrap()
                    .to_owned(),
                event.stack.count(),
            ));
        }
    }
    #[test]
    fn brewed_event_reports_player_and_converted_stack() {
        prepare_stack_boundary_tests();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "watch",
                Box::new(Watch(seen.clone())),
            )]
            .into(),
            || {
                let stack = EngineStack::new("minecraft:potion", 1)
                    .try_into_api()
                    .unwrap();
                mod_api::dispatch_player_brewed_potion(&mut NoHost, 7, 41, &stack);
            },
        );
        assert_eq!(
            *seen.lock().unwrap(),
            [(7, 41, "minecraft:potion".to_owned(), 1)]
        );
    }
}

impl super::Game<'_> {
    pub(in crate::game) fn api_block_entity_update_tag(
        pos: [i32; 3],
        entity: &crate::containers::ModBlockEntity,
        tag: &mut Option<horizon_core::nbt::Compound>,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityUpdateTag) {
            return;
        }
        if let Ok(api) = entity.try_into_api() {
            Self::native_block_entity_update_tag(pos, &api, tag);
        }
    }
    pub(in crate::game) fn api_block_entity_submit(
        &self,
        pos: [i32; 3],
        block: &str,
        state: horizon_world::registry::BlockStateId,
        entity: &crate::containers::ModBlockEntity,
        draw: &mut super::native_block_entity_draw::BlockEntityDraw,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntitySubmit) {
            return;
        }
        if let Ok(api) = entity.try_into_api() {
            self.native_block_entity_submit(pos, block, state, &api, draw);
        }
    }
    pub(in crate::game) fn api_block_entity_render_bounding_box(
        &self,
        pos: [i32; 3],
        block: &str,
        state: horizon_world::registry::BlockStateId,
        entity: &crate::containers::ModBlockEntity,
        bounds: &mut [f32; 6],
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityRenderBoundingBox) {
            return;
        }
        if let Ok(api) = entity.try_into_api() {
            self.native_block_entity_render_bounding_box(pos, block, state, &api, bounds);
        }
    }
    /// Shared click metadata borrows the already-converted API snapshot for this call.
    pub(in crate::game) fn block_click<'a>(
        &mut self,
        hit: &super::Hit,
        hand: mod_api::Hand,
        stack: Option<&'a ApiStack>,
    ) -> mod_api::BlockClick<'a> {
        let (player, player_entity_id) = self.native_clicker();
        mod_api::BlockClick::from(mod_api::__private::built::BlockClick {
            side: if self.remote.is_some() {
                mod_api::LogicalSide::Client
            } else {
                mod_api::LogicalSide::Server
            },
            dimension: self.dim.level(),
            pos: hit.pos,
            hit: hit.native(),
            hand,
            stack,
            player,
            player_entity_id,
        })
    }

    pub(in crate::game) fn api_menu_button(
        &mut self,
        context: &mod_api::item::menu::MenuButtonContext,
        entity: &mut crate::containers::ModBlockEntity,
        carried: &mut Option<EngineStack>,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::MenuButton) {
            return;
        }
        let (Ok(mut api), Ok(mut cursor)) = (entity.try_into_api(), carried.try_into_api()) else {
            return;
        };
        let revision = api.revision();
        let original = cursor.clone();
        self.native_menu_button(context, &mut api, &mut cursor);
        let cursor = if cursor == original {
            Ok(None)
        } else {
            cursor
                .try_into_engine(self.items())
                .map(Some)
                .map_err(|_| BlockEntitySnapshotError::InvalidStack)
        };
        if let Ok(Some(cursor)) = finish_block_entity_writeback(
            entity,
            &api,
            revision,
            self.items(),
            "menu_button",
            cursor,
        ) {
            *carried = cursor;
        }
    }
    pub(in crate::game) fn api_menu_quick_move(
        &mut self,
        pos: [i32; 3],
        entity: &mut crate::containers::ModBlockEntity,
        container_slots: usize,
        slot: usize,
        stack: &mut EngineStack,
        handled: &mut bool,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::MenuQuickMove) {
            return;
        }
        let (Ok(mut api), Ok(mut held)) = (entity.try_into_api(), stack.try_into_api()) else {
            return;
        };
        let revision = api.revision();
        let original = held.clone();
        let mut candidate_handled = *handled;
        self.native_menu_quick_move(
            pos,
            &mut api,
            container_slots,
            slot,
            &mut held,
            &mut candidate_handled,
        );
        let held = if held == original {
            Ok(None)
        } else {
            held.try_into_engine(self.items())
                .map(Some)
                .map_err(|_| BlockEntitySnapshotError::InvalidStack)
        };
        if let Ok(held) = finish_block_entity_writeback(
            entity,
            &api,
            revision,
            self.items(),
            "menu_quick_move",
            held,
        ) {
            if let Some(held) = held {
                *stack = held;
            }
            *handled = candidate_handled;
        }
    }
}
impl super::GameLevel<'_> {
    pub(in crate::game) fn api_pre_remove_side_effects(
        &mut self,
        pos: horizon_core::position::BlockPos,
        state: horizon_world::registry::BlockStateId,
        entity: &crate::containers::ModBlockEntity,
        drops: &mut Vec<EngineStack>,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::PreRemoveSideEffects) {
            return;
        }
        let Ok(api) = entity.try_into_api() else {
            return;
        };
        let mut added = Vec::new();
        self.native_pre_remove_side_effects(pos, state, &api, &mut added);
        drops.extend(added);
    }
}

#[cfg(test)]
#[path = "api_values/stacked_slot_tests.rs"]
mod stacked_slot_tests;

#[cfg(test)]
mod block_entity_snapshot_tests {
    use super::*;
    use horizon_gameplay::api_values::block_entity_writeback::apply_block_entity_snapshot;
    #[test]
    fn rejected_writebacks_invalidate_tick() {
        assert_rejected_writeback("block_entity_tick");
    }
    #[test]
    fn rejected_writebacks_invalidate_menu_button() {
        assert_rejected_writeback("menu_button");
    }
    #[test]
    fn rejected_writebacks_invalidate_menu_quick_move() {
        assert_rejected_writeback("menu_quick_move");
    }
    fn assert_rejected_writeback(hook: &'static str) {
        let items = Items::for_tests(Default::default());
        let mut entity =
            crate::containers::ModBlockEntity::new("test:rejected".into(), Default::default());
        let mut snapshot = entity.try_into_api().unwrap();
        let before = snapshot.revision();
        snapshot.data.push(42);
        let claimed = snapshot.revision().changes();
        // Tick rejection: wrong identity. Menu rejection: invalid cursor/held output.
        let output = if hook == "block_entity_tick" {
            snapshot = mod_api::ModBlockEntity::new("other:type".into(), Default::default());
            Ok(())
        } else {
            Err(BlockEntitySnapshotError::InvalidStack)
        };
        assert!(
            finish_block_entity_writeback(&mut entity, &snapshot, before, &items, hook, output)
                .is_err()
        );
        assert!(entity.data.is_empty());
        assert_eq!(
            entity.revision().changes(),
            if hook == "block_entity_tick" {
                1
            } else {
                claimed + 1
            }
        );
    }

    #[test]
    fn unchanged_snapshots_keep_revision_and_callback_writes_advance_it() {
        let mut entity =
            crate::containers::ModBlockEntity::new("test:machine".into(), Default::default());
        let first = entity.try_into_api().unwrap();
        let mut second = entity.try_into_api().unwrap();
        assert_eq!(first.revision(), second.revision());
        let before = second.revision();
        second.data.push(7);
        second.data.push(8);
        assert_ne!(before, second.revision());
        apply_block_entity_snapshot(
            &mut entity,
            &second,
            before,
            &Items::for_tests(Default::default()),
        )
        .unwrap();
        let third = entity.try_into_api().unwrap();
        assert_ne!(first.revision(), third.revision());
        assert_eq!(second.revision(), third.revision());
        assert_eq!(third.revision(), entity.try_into_api().unwrap().revision());
        assert_eq!(entity.id(), "test:machine");
        assert_eq!(entity.data, [7, 8]);
        let other = mod_api::ModBlockEntity::new("test:machine".into(), Default::default());
        assert_ne!(other.revision(), third.revision());
        entity.data.push(9);
        let fourth = entity.try_into_api().unwrap();
        assert_ne!(fourth.revision(), second.revision());
        let items = Items::for_tests(Default::default());
        assert_eq!(
            apply_block_entity_snapshot(&mut entity, &second, before, &items),
            Err(BlockEntitySnapshotError::InvalidRevision)
        );
        assert_eq!(
            apply_block_entity_snapshot(&mut entity, &other, before, &items),
            Err(BlockEntitySnapshotError::InvalidRevision)
        );
        assert_eq!(entity.data, [7, 8, 9]);
    }

    #[test]
    fn a_callbacks_client_update_fields_reach_the_queue_only_from_a_valid_snapshot() {
        let items = Items::for_tests(Default::default());
        let mut entity =
            crate::containers::ModBlockEntity::new("test:machine".into(), Default::default());
        let batch =
            horizon_core::nbt::Compound::from([("effect".into(), horizon_core::nbt::Tag::Int(23))]);
        let made = entity.revision();
        let mut snapshot = entity.try_into_api().unwrap();
        let before = snapshot.revision();
        mod_api::ModBlockEntityView::new(&mut snapshot).set_client_update_fields(batch.clone());
        // Nothing but the queue moved, and an unchanged snapshot still delivers.
        apply_block_entity_snapshot(&mut entity, &snapshot, before, &items).unwrap();
        assert_eq!(entity.revision(), made);
        assert_eq!(entity.take_client_update_fields(), Some(batch.clone()));
        // A snapshot naming another entity is refused whole.
        let mut other = mod_api::ModBlockEntity::new("test:machine".into(), Default::default());
        let before = other.revision();
        mod_api::ModBlockEntityView::new(&mut other).set_client_update_fields(batch);
        assert_eq!(
            apply_block_entity_snapshot(&mut entity, &other, before, &items),
            Err(BlockEntitySnapshotError::InvalidRevision)
        );
        assert!(!entity.has_client_update_fields());
    }
}

#[cfg(test)]
mod mutable_output_tests {
    use super::*;

    #[test]
    fn array_outputs_are_admitted_only_after_every_stack_passes_resident_limits() {
        prepare_stack_boundary_tests();
        let root =
            std::env::temp_dir().join(format!("horizon-api-array-output-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("items.json"),
            r#"{"minecraft:stick":{"minecraft:max_stack_size":64}}"#,
        )
        .unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items =
            Items::catalog(&root, &horizon_world::loot::PackOverlay::NONE, &defaults).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        let key = mod_api::__private::item_key("minecraft:stick").unwrap();
        let mut api = [
            ApiStack::new(key, 8).unwrap(),
            ApiStack::new(key, 65).unwrap(),
        ];
        let mut output = [
            EngineStack::new("minecraft:stick", 3),
            EngineStack::new("minecraft:stick", 4),
        ];
        let before = output.clone();
        let converted = api.try_into_engine(&items);
        assert_eq!(
            converted,
            Err(StackWriteError::CountExceeded { maximum: 64 })
        );
        if let Ok(validated) = converted {
            output = validated;
        }
        assert_eq!(
            output, before,
            "a legal first element must not be applied before a rejected second element"
        );

        api[1].set_count(64).unwrap();
        output = api.try_into_engine(&items).unwrap();
        assert_eq!(
            output,
            [
                EngineStack::new("minecraft:stick", 8),
                EngineStack::new("minecraft:stick", 64)
            ]
        );
    }
}

#[cfg(test)]
#[path = "api_values/read_stack_tests.rs"]
mod read_stack_tests;

#[cfg(test)]
mod stack_limit_tests {
    use super::*;

    pub(super) fn catalog() -> Items {
        prepare_stack_boundary_tests();
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "horizon-api-limits-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("items.json"), r#"{
            "minecraft:stick":{"minecraft:max_stack_size":64},
            "minecraft:iron_sword":{"minecraft:max_stack_size":1,"minecraft:max_damage":10,"minecraft:damage":7},
            "minecraft:diamond_sword":{"minecraft:max_stack_size":1,"minecraft:max_damage":100,"minecraft:damage":0}
        }"#).unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items =
            Items::catalog(&root, &horizon_world::loot::PackOverlay::NONE, &defaults).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        items
    }

    /// Vanilla stores damage above the maximum; only writes refuse it. Reads see the stack.
    #[test]
    fn a_write_refuses_damage_over_the_maximum_a_read_takes() {
        let items = catalog();
        let mut over = ApiStack::of(mod_api::__private::item_key("minecraft:iron_sword").unwrap());
        over.set_damage(1000);
        assert_eq!(
            over.try_into_engine(&items),
            Err(StackWriteError::DamageExceeded { maximum: 10 })
        );
        let read = stack_value(&over).unwrap();
        let definition = items.items.get("minecraft:iron_sword");
        assert_eq!(read.effective_damage(definition), 1000);
        assert!(horizon_client::stations::is_damageable(&items, &read));
    }

    #[test]
    fn strict_damage_limits_include_zero_inheritance_removal_and_nbt_int_range() {
        let mut items = catalog();
        let key = |id| mod_api::__private::item_key(id).unwrap();
        let plain = ApiStack::of(key("minecraft:stick"));
        assert!(plain.try_into_engine(&items).is_ok());
        let mut explicit = plain.clone();
        explicit.set_damage(0);
        let engine = explicit.try_into_engine(&items).unwrap();
        assert!(engine.carries_component(None, "minecraft:damage"));
        assert_eq!(
            horizon_items::item_data::encode_components(&engine)
                .unwrap()
                .get("minecraft:damage"),
            Some(&Tag::Int(0))
        );
        explicit.set_damage(u32::MAX);
        assert_eq!(
            explicit.try_into_engine(&items),
            Err(StackWriteError::DamageExceeded { maximum: 0 })
        );

        let inherited = ApiStack::of(key("minecraft:iron_sword"));
        let definition = items.items.get("minecraft:iron_sword").unwrap();
        assert_eq!(
            inherited
                .try_into_engine(&items)
                .unwrap()
                .effective_damage(Some(definition)),
            7
        );
        let mut lower = inherited.clone();
        lower
            .set_components([(JavaString::from("minecraft:max_damage"), Tag::Int(5))].into())
            .unwrap();
        assert_eq!(
            lower.try_into_engine(&items),
            Err(StackWriteError::DamageExceeded { maximum: 5 })
        );
        let mut removed = inherited;
        removed
            .set_components(
                [(
                    JavaString::from("!minecraft:max_damage"),
                    Tag::Compound(Default::default()),
                )]
                .into(),
            )
            .unwrap();
        assert_eq!(
            removed.try_into_engine(&items),
            Err(StackWriteError::DamageExceeded { maximum: 0 })
        );
        removed.clear_damage();
        let engine = removed.try_into_engine(&items).unwrap();
        assert_eq!(engine.max_damage(Some(definition)), 0);
        assert_eq!(engine.effective_damage(Some(definition)), 0);
        removed.set_damage(0);
        assert!(removed.try_into_engine(&items).is_ok());

        items
            .items
            .get_mut("minecraft:diamond_sword")
            .unwrap()
            .max_damage = u32::MAX;
        let mut oversized = ApiStack::of(key("minecraft:diamond_sword"));
        oversized.set_damage(u32::MAX);
        assert_eq!(
            oversized.try_into_engine(&items),
            Err(StackWriteError::InvalidComponents {
                reason: mod_api::StackError::InvalidDamage
            })
        );
    }

    #[test]
    fn stack_maximum_respects_the_vanilla_codec_range() {
        let items = catalog();
        let key = mod_api::__private::item_key("minecraft:stick").unwrap();
        for maximum in [i32::MIN, -1, 0, 100, i32::MAX] {
            let mut stack = ApiStack::of(key);
            stack
                .set_components(
                    [(
                        JavaString::from("minecraft:max_stack_size"),
                        Tag::Int(maximum),
                    )]
                    .into(),
                )
                .unwrap();
            assert_eq!(
                stack.try_into_engine(&items),
                Err(StackWriteError::InvalidMaximum),
                "maximum {maximum}"
            );
        }
        for maximum in [1, 99] {
            let mut stack = ApiStack::new(key, maximum).unwrap();
            stack
                .set_components(
                    [(
                        JavaString::from("minecraft:max_stack_size"),
                        Tag::Int(maximum as i32),
                    )]
                    .into(),
                )
                .unwrap();
            assert_eq!(stack.try_into_engine(&items).unwrap().count, maximum);
        }
    }

    #[test]
    fn production_snapshots_preserve_component_corpus_contents() {
        let items = catalog();
        let cases = [
            ("minecraft:iron_sword", None),
            (
                "minecraft:iron_sword",
                Some(Tag::compound([("minecraft:damage", Tag::Int(0))])),
            ),
            (
                "minecraft:iron_sword",
                Some(Tag::compound([("minecraft:damage", Tag::Int(5))])),
            ),
            (
                "minecraft:iron_sword",
                Some(Tag::compound([(
                    "!minecraft:damage",
                    Tag::Compound(Default::default()),
                )])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "minecraft:custom_name",
                    Tag::String("Named".into()),
                )])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "minecraft:custom_data",
                    Tag::compound([
                        ("list", Tag::List(vec![Tag::Int(3), Tag::Int(7)])),
                        ("bytes", Tag::ByteArray(vec![1, 2, 3])),
                    ]),
                )])),
            ),
            (
                "minecraft:iron_sword",
                Some(Tag::compound([("minecraft:max_stack_size", Tag::Int(1))])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([("minecraft:max_stack_size", Tag::Int(99))])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "!minecraft:max_stack_size",
                    Tag::Compound(Default::default()),
                )])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "minecraft:container",
                    Tag::List(vec![Tag::compound([
                        ("slot", Tag::Int(0)),
                        (
                            "item",
                            Tag::compound([
                                ("id", Tag::String("minecraft:stick".into())),
                                ("count", Tag::Int(2)),
                            ]),
                        ),
                    ])]),
                )])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "minecraft:enchantments",
                    Tag::compound([("minecraft:sharpness", Tag::Int(2))]),
                )])),
            ),
        ];
        for (item, patch) in cases {
            let (damage, extra) = horizon_items::item_data::decode_components(patch.as_ref());
            let original = EngineStack {
                damage,
                extra,
                ..EngineStack::new(item, 1)
            };
            // The engine's canonical component codec is independent of the snapshot adapter.
            let expected = horizon_items::item_data::encode_extra_components(&original);
            let api = original.try_into_api().unwrap();
            assert_eq!(
                api.components()
                    .map(|components| Tag::Compound(components.as_nbt().clone())),
                expected,
                "read {patch:?}"
            );
            let back = api.try_into_engine(&items).unwrap();
            assert_eq!(
                horizon_items::item_data::encode_components(&original),
                horizon_items::item_data::encode_components(&back),
                "write {patch:?}"
            );
            assert_eq!(original, back, "round trip {patch:?}");
        }
    }

    #[test]
    fn durability_and_stackability_are_mutually_exclusive() {
        let items = catalog();
        let mut stick = ApiStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        stick
            .set_components([(JavaString::from("minecraft:max_damage"), Tag::Int(10))].into())
            .unwrap();
        assert_eq!(
            stick.try_into_engine(&items),
            Err(StackWriteError::StackableDamageable)
        );
        let mut sword = ApiStack::of(mod_api::__private::item_key("minecraft:iron_sword").unwrap());
        sword
            .set_components([(JavaString::from("minecraft:max_stack_size"), Tag::Int(2))].into())
            .unwrap();
        assert_eq!(
            sword.try_into_engine(&items),
            Err(StackWriteError::StackableDamageable)
        );
        stick
            .set_components(
                [
                    (JavaString::from("minecraft:max_damage"), Tag::Int(10)),
                    (JavaString::from("minecraft:max_stack_size"), Tag::Int(1)),
                ]
                .into(),
            )
            .unwrap();
        assert!(stick.try_into_engine(&items).is_ok());
        sword
            .set_components(
                [
                    (JavaString::from("minecraft:max_stack_size"), Tag::Int(2)),
                    (
                        JavaString::from("!minecraft:max_damage"),
                        Tag::Compound(Default::default()),
                    ),
                ]
                .into(),
            )
            .unwrap();
        sword.clear_damage();
        assert!(sword.try_into_engine(&items).is_ok());
    }

    #[test]
    fn removed_stack_maximum_uses_the_absent_component_limit_of_one() {
        let items = catalog();
        let mut stack =
            ApiStack::new(mod_api::__private::item_key("minecraft:stick").unwrap(), 2).unwrap();
        stack
            .set_components(
                [(
                    JavaString::from("!minecraft:max_stack_size"),
                    Tag::Compound(Default::default()),
                )]
                .into(),
            )
            .unwrap();
        assert_eq!(
            stack.try_into_engine(&items),
            Err(StackWriteError::CountExceeded { maximum: 1 })
        );
        stack.set_count(1).unwrap();
        assert!(!stack.try_into_engine(&items).unwrap().carries_component(
            items.items.get("minecraft:stick"),
            "minecraft:max_stack_size"
        ));
    }

    #[test]
    fn damage_only_snapshots_preserve_presence_without_an_opaque_patch() {
        catalog();
        for damage in [0, 5] {
            let (damage, extra) = horizon_items::item_data::decode_components(Some(
                &Tag::compound([("minecraft:damage", Tag::Int(damage))]),
            ));
            let engine = EngineStack {
                damage,
                extra,
                ..EngineStack::new("minecraft:iron_sword", 1)
            };
            let api = engine.try_into_api().unwrap();
            assert_eq!(api.damage_component(), Some(damage));
            assert!(api.components().is_none());
        }
    }
}

#[cfg(test)]
mod item_model_snapshot_tests {
    use super::*;
    use horizon_assets::item_model::StackView;

    #[test]
    fn model_snapshots_keep_component_patches_without_runtime_bundle_selection() {
        prepare_stack_boundary_tests();
        let mut view = StackView::new("minecraft:stick", 2);
        view.bundle_selected = 3;
        view.patch
            .insert("minecraft:damage".into(), Some(serde_json::json!(7)));
        view.patch.insert(
            "example:charge".into(),
            Some(serde_json::json!({"amount":4})),
        );
        view.patch.insert("minecraft:custom_name".into(), None);
        let snapshot = view.try_into_api().unwrap();
        assert_eq!(
            snapshot.item(),
            mod_api::__private::item_key("minecraft:stick").unwrap()
        );
        assert_eq!(snapshot.count(), 2);
        assert_eq!(snapshot.damage(), 7);
        let patch = snapshot.components().unwrap().as_nbt();
        assert_eq!(
            patch
                .get(&JavaString::from("example:charge"))
                .unwrap()
                .get("amount"),
            Some(&Tag::Int(4))
        );
        assert!(
            patch
                .get(&JavaString::from("!minecraft:custom_name"))
                .is_some()
        );
        assert!(patch.get(&JavaString::from("bundle_selected")).is_none());
        assert_eq!(view.bundle_selected, 3);
    }

    #[test]
    fn empty_model_views_do_not_become_owned_stack_sentinels() {
        prepare_stack_boundary_tests();
        for count in [-1, 0] {
            assert_eq!(
                StackView::new("minecraft:stick", count).try_into_api(),
                Err(StackReadError::Empty)
            );
        }
        assert_eq!(
            StackView::new("example:unknown", 1).try_into_api(),
            Err(StackReadError::UnknownItem)
        );
    }
}

#[cfg(test)]
mod completion_snapshot_tests {
    use super::*;
    #[test]
    fn completion_round_trip_keeps_range_tooltip_and_numeric_order() {
        let source = horizon_client_core::suggestion::Suggestion {
            range: (4, 7),
            text: "42".into(),
            tooltip: Some("count".into()),
            int_value: Some(42),
        };
        let mut api = source.try_into_api().unwrap();
        api.text = "43".into();
        api.int_value = Some(43);
        let engine = api
            .try_into_engine(&Items::for_tests(Default::default()))
            .unwrap();
        assert_eq!(engine.range, (4, 7));
        assert_eq!(engine.tooltip.as_deref(), Some("count"));
        assert_eq!(engine.int_value, Some(43));
        assert_eq!(engine.apply("foo 123 bar"), "foo 43 bar");
        assert_eq!(source.text, "42");
    }
}

pub(in crate::game) fn block_entity_declarations()
-> Vec<horizon_mod_package::content::BlockEntityType> {
    mod_api::__private::block_entity_types()
        .iter()
        .map(|decl| decl.try_into_engine(&()).unwrap())
        .collect()
}

#[cfg(test)]
mod declaration_snapshot_tests {
    use super::*;
    #[test]
    fn declarations_keep_supported_blocks_and_rule_positions() {
        static KEY: std::sync::OnceLock<mod_api::BoolGameRuleKey> = std::sync::OnceLock::new();
        // Use registration, the owner of declaration validation.
        struct Mod;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Mod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.declare_block_entity_type(
                    "machine",
                    vec!["test:second".into(), "test:first".into()],
                );
                r.game_rule_category("machines").unwrap();
                KEY.set(r.game_rule_bool("active", "test:machines", true).unwrap())
                    .unwrap();
            }
        }
        install_parts(&[mod_api::ModEntry::new("test", Box::new(Mod))]).unwrap();
        let declarations = block_entity_declarations();
        assert_eq!(declarations[0].id, "test:machine");
        assert_eq!(declarations[0].blocks, ["test:second", "test:first"]);
        let registry = game_rule_registry();
        let key = *KEY.get().unwrap();
        assert_eq!(
            mod_api::__private::game_rule_bool_slot(key),
            registry.position("test:active")
        );
        assert_eq!(
            registry.rule("test:active").unwrap().default,
            horizon_client_core::gamerules::Value::Bool(true)
        );
    }
}

#[cfg(test)]
mod gui_draw_boundary_tests {
    use super::{TryIntoApi, TryIntoEngine};
    use horizon_client::{
        hud_layout::{Blend, GuiDraw as D, PoseOp},
        text::Text,
    };
    use serde_json::json;

    #[test]
    fn gui_commands_keep_every_variant_field_order_and_text_source() {
        let text = Text::from(
            json!({"translate":"test.hud", "fallback":"%s", "with":[{"text":"name"}], "bold":false, "test:meta":{"n":1}}),
        );
        let commands = vec![
            D::Sprite {
                sprite: String::from("test:sprite").into(),
                x: -1,
                y: 2,
                w: 3,
                h: 4,
                region: Some([5, 6, 7, 8]),
                color: 0x12345678,
                blend: Blend::Crosshair,
            },
            D::AtlasSprite {
                sprite: "test:atlas".into(),
                x: 11,
                y: 12,
                w: 13,
                h: 14,
                color: 15,
            },
            D::Blit {
                texture: "test:texture".into(),
                blend: Blend::Nausea,
                x: 21,
                y: 22,
                w: 23,
                h: 24,
                uv: [0.25, 0.5],
                src: [26, 27],
                tex: [28, 29],
                color: 30,
            },
            D::Fill {
                x0: -31,
                y0: 32,
                x1: 33,
                y1: 34,
                color: 35,
            },
            D::FillGradient {
                x0: 36,
                y0: -37,
                x1: 38,
                y1: 39,
                top: 0x8000_0040,
                bottom: 0xFF00_0041,
            },
            D::Text {
                text: text.clone(),
                x: 41,
                y: 42,
                color: 43,
                shadow: false,
            },
            D::Item {
                slot: 40,
                x: 51,
                y: 52,
                seed: -53,
                scale: [0.5, 1.5],
                pivot: [55., 56.],
            },
            D::Push(vec![PoseOp::Translate(61., 62.), PoseOp::Scale(0.5, 1.5)]),
            D::Pop,
        ];
        let api = commands.try_into_api().unwrap();
        let actual: Result<Vec<D>, mod_api::text::TextFormError> = api.try_into_engine(&());
        let actual = actual.unwrap();
        assert_eq!(actual, commands);
        let D::Text { text: actual, .. } = &actual[5] else {
            panic!("text command")
        };
        assert_eq!(actual.component(), text.component());
    }

    #[test]
    fn tooltip_pairs_round_trip_without_an_item_registry() {
        let lines = vec![(horizon_client::text::Text::literal("tip"), 0xff00ff00)];
        let snapshot: Result<Vec<(mod_api::Text, u32)>, mod_api::text::TextFormError> =
            lines.try_into_api();
        let snapshot = snapshot.unwrap();
        assert_eq!(snapshot[0], (mod_api::Text::literal("tip"), 0xff00ff00));
        let restored: Result<Vec<(horizon_client::text::Text, u32)>, mod_api::text::TextFormError> =
            snapshot.try_into_engine(&());
        let restored = restored.unwrap();
        assert_eq!(restored[0].0.component(), lines[0].0.component());
        assert_eq!(restored[0].0.spans, lines[0].0.spans);
        assert_eq!(restored[0].1, lines[0].1);
    }

    #[test]
    fn nested_text_containers_need_no_item_registry_or_stack_error() {
        let pair = (
            mod_api::text::Text::literal("a"),
            mod_api::text::Text::literal("b"),
        );
        let values = Some(vec![[pair.clone(), pair]]);
        let reference = &values;
        let actual: Result<_, mod_api::text::TextFormError> = reference.try_into_engine(&());
        let actual = actual.unwrap().unwrap();
        assert_eq!(actual[0][1].0.plain(), "a");
        assert_eq!(actual[0][1].1.plain(), "b");
    }
}

#[cfg(test)]
mod placement_snapshot_tests {
    use super::*;
    #[test]
    fn placement_keeps_click_view_flags_and_empty_hand_without_fallback() {
        install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:stone"]).unwrap();
        let mut engine = horizon_world::level::PlacementContext {
            pos: horizon_core::position::BlockPos::new(1, 2, 3),
            hit_pos: horizon_core::position::BlockPos::new(4, 5, 6),
            face: horizon_core::geometry::Direction::Up,
            hit: horizon_core::geometry::Vec3::new(4.25, 5.5, 6.75),
            inside: true,
            yaw: -45.0,
            pitch: 90.0,
            sneaking: true,
            item: "minecraft:stone".into(),
            replace_clicked: true,
        };
        let snapshot = engine.try_into_api().unwrap();
        assert_eq!(snapshot.pos, horizon_core::position::BlockPos::new(1, 2, 3));
        assert_eq!(
            snapshot.hit_pos,
            horizon_core::position::BlockPos::new(4, 5, 6)
        );
        assert_eq!(snapshot.face, horizon_core::geometry::Direction::Up);
        assert_eq!(snapshot.hit, [4.25, 5.5, 6.75]);
        assert!(snapshot.inside);
        assert_eq!(snapshot.yaw, -45.0);
        assert_eq!(snapshot.pitch, 90.0);
        assert!(snapshot.sneaking && snapshot.replace_clicked);
        assert_eq!(
            snapshot.item,
            mod_api::__private::item_key("minecraft:stone")
        );
        engine.item.clear();
        let mut empty = snapshot;
        empty.item = None;
        assert_eq!(engine.try_into_api(), Ok(empty));
        engine.item = "missing:item".into();
        assert_eq!(engine.try_into_api(), Err(StackReadError::UnknownItem));
    }
}

#[test]
fn game_rule_int_snapshot_preserves_full_range() {
    for value in [i32::MIN, -1, 0, 256, i32::MAX] {
        assert_eq!(
            horizon_client_core::gamerules::Value::Int(value)
                .try_into_api()
                .unwrap(),
            mod_api::GameRuleValue::Int(value)
        );
    }
}

fn with_rule_schema<T>(f: impl FnOnce(&mod_api::__private::RuleSchema<'_>) -> T) -> T {
    let registry = horizon_client_core::gamerules::rule_registry();
    f(&mod_api::__private::RuleSchema {
        category: &|id| registry.categories().iter().any(|category| category == id),
        position: &|id| registry.position(id),
        rule_count: u32::try_from(registry.available_rules(true).count())
            .expect("vanilla rule count fits u32"),
    })
}
/// Supply the real platform when unit tests bypass executable startup.
#[cfg(test)]
pub(in crate::game) fn install_test_platform() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        let result = horizon_engine_slots::install_platform(horizon_engine_slots::Platform {
            install_packages: mod_platform::__private::install_packages,
            install_registration_data: mod_platform::__private::install_registration_data,
            install_parts: mod_platform::__private::install_parts,
            profiler_is_enabled: mod_platform::__private::profiler::is_enabled,
            profiler_start: mod_platform::__private::profiler::start,
            profiler_stop: mod_platform::__private::profiler::stop,
            profiler_summary: mod_platform::__private::profiler::summary,
            profiler_firefox_profile: mod_platform::__private::profiler::firefox_profile,
        });
        assert!(
            result.is_ok()
                || result == Err(horizon_engine_slots::PlatformRefusal::AlreadyInstalled)
        );
    });
}
pub(in crate::game) fn install_parts(mods: &[mod_api::ModEntry]) -> Result<(), Vec<String>> {
    #[cfg(test)]
    install_test_platform();
    with_rule_schema(|schema| {
        (horizon_engine_slots::platform()
            .map_err(|e| vec![e.to_string()])?
            .install_parts)(mods, schema, super::native_dimensions::valid_shape)
    })
}
#[cfg(test)]
pub(in crate::game) fn register_game_keys(ids: impl IntoIterator<Item = String>) {
    with_rule_schema(|schema| {
        mod_platform::__private::part_tests::register_game_keys(
            ids,
            schema,
            super::native_dimensions::valid_shape,
        )
    });
    mod_api::__private::resolve_effects(
        horizon_client_core::potions::all_effects()
            .map(|def| horizon_client_core::potions::namespaced(def.id)),
        |id| horizon_client_core::potions::effect(id).map(|def| def.index),
    )
    .unwrap();
    if let Some(types) = horizon_client_core::block_entity_types::current() {
        // Registration just replaced the mod snapshot, but the process-wide engine
        // table can still belong to an earlier fixture. Include this fixture's
        // declarations before resolving its requested keys, as content loading does.
        let declared = mod_api::__private::block_entity_types();
        mod_api::__private::resolve_block_entity_types(
            types
                .names()
                .chain(declared.iter().map(|kind| kind.id.as_str())),
            |id| {
                (types.key(id).is_some() || declared.iter().any(|kind| kind.id == id)).then_some(())
            },
        )
        .unwrap();
    }
}

pub(in crate::game) fn game_rule_registry() -> horizon_client_core::gamerules::RuleRegistry {
    let rules: Vec<_> = mod_api::__private::game_rules()
        .iter()
        .map(|rule| rule.try_into_engine(&()).unwrap())
        .collect();
    horizon_client_core::gamerules::RuleRegistry::with_mod_rules(
        mod_api::__private::game_rule_categories(),
        &rules,
    )
    .expect("validated at registration")
}

#[cfg(test)]
pub(in crate::game) mod damage {
    pub(in crate::game) use horizon_gameplay::api_values::damage::CONVERSIONS;
}
#[path = "api_values/loot_values.rs"]
pub(crate) mod loot_values;
#[cfg(test)]
#[path = "api_values/particle_values.rs"]
mod particle_values;
pub(in crate::game) use horizon_gameplay::api_values::package_snapshot::install_packages;
pub(crate) use loot_values::{dispatch_loot_condition, dispatch_loot_function};
#[cfg(test)]
#[path = "api_values/small_values.rs"]
mod small_values;
