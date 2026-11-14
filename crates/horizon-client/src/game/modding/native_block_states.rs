//! Block state ids and names, and item tags, for native mods:
//! registry queries with no level, so they answer anywhere a host is given.
use super::Game;
use horizon_client::command::{Reader, args::Registries, block};
use horizon_client::items::ItemStack;
use horizon_world::chunk::BlockState;
use horizon_world::fluid::{FluidKind, FluidState as EngineFluidState};
use horizon_world::level::{fluid_drip_particle, fluid_explosion_resistance, fluid_randomly_ticks};
use horizon_world::registry::{BlockRegistry, BlockStateId};
use mod_api::{
    BlockEntityTypeKey, BlockKey, BlockSoundType, BlockStateRange, EffectKey, EnchantmentKey,
    EntityTypeKey, FluidDefinition, FluidKey, FluidState, FluidStateError, FluidTypeProperties,
    ItemKey, MenuKey, StateConstraint, StateMatchError, StateProperty,
};
use std::sync::{Arc, Mutex};

impl Game<'_> {
    /// The exported definition of a world fluid, or a tank-only identity.
    pub(super) fn fluid_state_definition(&self, fluid: FluidKey) -> Option<FluidDefinition> {
        let id = mod_api::__private::fluid_id(fluid)?;
        let Some(definition) = self.res.behaviours().fluid_definitions().get(id) else {
            return Some(FluidDefinition::TankOnly);
        };
        let default = super::native_world_read::api_fluid_state(definition.default)?;
        let properties = definition
            .properties
            .iter()
            .map(|(name, values)| StateProperty::new(name.clone(), values.clone()))
            .collect();
        Some(FluidDefinition::World {
            default,
            properties,
        })
    }

    /// A registered fluid's type properties: its block's light and name for a world fluid,
    /// vanilla's `FluidTags.WATER` and `FluidTags.LAVA` behaviours by its family. None for a
    /// key the registry doesn't hold.
    pub(super) fn fluid_type_properties(&self, fluid: FluidKey) -> Option<FluidTypeProperties> {
        let id = mod_api::__private::fluid_id(fluid)?;
        let behaviours = self.res.behaviours();
        let block = behaviours.fluid_definitions().get(id).map(|definition| {
            self.res
                .facts()
                .get(behaviours.legacy_block(definition.default))
        });
        let description_id = match &block {
            Some(block) => description_id("block", &block.name)?,
            None => description_id("fluid_type", id)?,
        };
        let kind = FluidKind::from_name(id);
        let water = kind.is_some_and(|kind| kind.same_family(FluidKind::Water));
        let lava = kind.is_some_and(|kind| kind.same_family(FluidKind::Lava));
        Some(FluidTypeProperties {
            description_id,
            light_level: block.map_or(0, |block| block.emission),
            fall_distance_modifier: match (water, lava) {
                (true, _) => 0.0,
                (_, true) => 0.5,
                _ => 1.0,
            },
            can_push_entity: water || lava,
            can_swim: water,
            can_drown: water,
            can_extinguish: water,
            can_hydrate: water,
            supports_boating: water,
            explosion_resistance: kind.map_or(0.0, fluid_explosion_resistance),
            randomly_ticks: kind.is_some_and(fluid_randomly_ticks),
            drip_particle: kind.and_then(fluid_drip_particle).map(str::to_owned),
        })
    }

    /// `FluidState.createLegacyBlock` for `state`, through the one table the engine converts
    /// fluid states with. None for a state no fluid can be in: no world form, an amount above
    /// 8, a zero amount for anything but the empty fluid, or a source that isn't full.
    pub(super) fn fluid_legacy_block(&self, state: FluidState) -> Option<mod_api::BlockStateId> {
        let kind = super::native_world_read::engine_fluid_kind(state.fluid)?;
        let engine = EngineFluidState::new(kind, state.amount, state.falling);
        let consistent = match kind {
            FluidKind::Empty => state.amount == 0,
            _ if engine.is_source() => state.amount == 8,
            _ => (1..=8).contains(&state.amount),
        };
        consistent.then(|| mod_api::BlockStateId(self.res.behaviours().legacy_block(engine).0))
    }

    /// Validate fluid constraints against the exported property definition.
    pub(super) fn fluid_state_validate(
        &self,
        fluid: FluidKey,
        constraints: &[StateConstraint],
    ) -> Result<(), FluidStateError> {
        let definition = mod_api::__private::fluid_id(fluid)
            .and_then(|id| self.res.behaviours().fluid_definitions().get(id))
            .ok_or(FluidStateError::NoWorldState)?;
        for (index, constraint) in constraints.iter().enumerate() {
            if constraints[..index]
                .iter()
                .any(|previous| previous.name == constraint.name)
            {
                return Err(FluidStateError::Duplicate(index as u32));
            }
            match_state_properties(
                &definition.properties,
                true,
                |_, _| true,
                std::slice::from_ref(constraint),
            )
            .map_err(|error| match error {
                StateMatchError::UnknownProperty(_) => {
                    FluidStateError::UnknownProperty(index as u32)
                }
                StateMatchError::InvalidValue(_) => FluidStateError::InvalidValue(index as u32),
                _ => unreachable!("StateMatchError: unhandled variant"),
            })?;
        }
        Ok(())
    }
    /// Resolve a registered fluid identifier.
    pub(super) fn fluid_key(&self, id: &str) -> Option<FluidKey> {
        mod_api::__private::fluid_key(id)
    }
    /// The canonical block id retained by the load-owned registration slot. An invalid slot
    /// is a boundary error, including when a wasm guest supplies it.
    pub(super) fn block_key_id(&self, key: BlockKey) -> String {
        mod_api::__private::block_key_id(key)
            .expect("invalid block key slot")
            .to_owned()
    }
    /// The canonical item id retained by the load-owned registration slot.
    pub(super) fn item_key_id(&self, key: ItemKey) -> String {
        mod_api::__private::item_key_id(key)
            .expect("invalid item key slot")
            .to_owned()
    }
    /// The canonical effect id retained by the load-owned registration slot.
    pub(super) fn effect_key_id(&self, key: EffectKey) -> String {
        mod_api::__private::effect_key_id(key)
            .expect("invalid effect key slot")
            .to_owned()
    }

    /// The canonical fluid id retained by the completed registry.
    pub(super) fn fluid_key_id(&self, key: FluidKey) -> String {
        mod_api::__private::fluid_id(key)
            .expect("invalid fluid key slot")
            .to_owned()
    }
    /// The key of a block in the completed registry.
    pub(super) fn block_key(&self, id: &str) -> Option<BlockKey> {
        mod_api::__private::block_key(id)
    }
    /// The key of an effect in the completed registry.
    pub(super) fn effect_key(&self, id: &str) -> Option<EffectKey> {
        mod_api::__private::effect_key(id)
    }
    /// The canonical block entity type id retained by the load-owned slot.
    pub(super) fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> String {
        mod_api::__private::block_entity_type_name(key)
            .expect("invalid block entity type key slot")
            .to_owned()
    }
    /// Every block key in registry order, mod blocks after vanilla's.
    pub(super) fn block_keys(&self) -> Vec<BlockKey> {
        mod_api::__private::block_keys()
    }
    /// Every item key in registry order, mod items after vanilla's.
    pub(super) fn item_keys(&self) -> Vec<ItemKey> {
        mod_api::__private::item_keys()
    }
    /// Every entity-type key in registry order.
    pub(super) fn entity_type_keys(&self) -> Vec<EntityTypeKey> {
        mod_api::__private::entity_type_keys()
    }
    /// Every menu-type key in registry order.
    pub(super) fn menu_keys(&self) -> Vec<MenuKey> {
        mod_api::__private::menu_keys()
    }
    /// Every enchantment key in registry order.
    pub(super) fn enchantment_keys(&self) -> Vec<EnchantmentKey> {
        mod_api::__private::enchantment_keys()
    }
    /// The canonical entity type id retained by the load-owned slot.
    pub(super) fn entity_type_key_id(&self, key: EntityTypeKey) -> String {
        mod_api::__private::entity_type_key_id(key).expect("invalid entity type key slot")
    }
    /// The canonical menu type id retained by the load-owned slot.
    pub(super) fn menu_key_id(&self, key: MenuKey) -> String {
        mod_api::__private::menu_key_id(key).expect("invalid menu key slot")
    }
    /// The canonical enchantment id retained by the load-owned slot.
    pub(super) fn enchantment_key_id(&self, key: EnchantmentKey) -> String {
        mod_api::__private::enchantment_key_id(key).expect("invalid enchantment key slot")
    }

    /// The state `name` names, read as `/setblock` reads a block
    /// (`BlockStateParser.parseForBlock`): `minecraft:oak_stairs[facing=east]`,
    /// `stone` for `minecraft:stone`, the properties left out at their
    /// default. Its id is vanilla's `Block.getId`. None for an unknown block,
    /// a property it doesn't have or a value it can't take, or block NBT
    /// (`{...}`); and, as `/setblock`'s argument, for anything after the
    /// block.
    pub(super) fn block_state_id(&self, name: &str) -> Option<BlockStateId> {
        parse(&self.pack.block_registries, self.pack.assets.registry, name)
    }
    /// The state `state` as `/setblock` takes it back
    /// (`BlockStateParser.serialize`): its block, then each property in
    /// name order, `minecraft:oak_stairs[facing=east,half=bottom,...]`, or
    /// the block alone for one with no properties. None for an id no state
    /// has (`Block.stateById` gives air for one).
    pub(super) fn block_state_string(&self, state: BlockStateId) -> Option<String> {
        self.pack.assets.registry.state(state).map(serialize)
    }
    /// The default state of `block` (`minecraft:oak_stairs`, or `oak_stairs`):
    /// `Block.defaultBlockState`, the one a block takes when nothing else
    /// says. None for an unknown block.
    pub(super) fn block_state_default(&self, block: BlockKey) -> Option<BlockStateId> {
        super::api_values::default_state(block)
    }
    /// `BlockState.canBeReplaced()`: whether placing a block where this state stands
    /// replaces it (air, grass, water, snow layers). None for an unknown state.
    pub(super) fn block_state_replaceable(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).replaceable)
    }
    /// `BlockBehaviour.Properties.destroyTime`, as `BlockState.getDestroySpeed` reads it:
    /// stone 1.5, obsidian 50, bedrock -1 (it can't be broken). None for an unknown state.
    pub(super) fn block_state_destroy_speed(&self, state: BlockStateId) -> Option<f32> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).hardness)
    }
    /// The explosion resistance the block of this state declares (`Block.getExplosionResistance`):
    /// stone 6, obsidian 1200, bedrock 3,600,000. None for an unknown state.
    pub(super) fn block_state_explosion_resistance(&self, state: BlockStateId) -> Option<f32> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).explosion_resistance)
    }
    /// `BlockState.getSoundType()`: its volume, pitch and the sound events for breaking,
    /// stepping on, placing, hitting and falling on it. None for an unknown state.
    pub(super) fn block_state_sound(&self, state: BlockStateId) -> Option<BlockSoundType> {
        self.pack.assets.registry.state(state)?;
        Some(horizon_client::block_sounds::own(
            &self.res.facts().get(state).sounds,
        ))
    }
    /// `BlockState.hasBlockEntity()`. None for an unknown state.
    pub(super) fn block_state_has_block_entity(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).block_entity)
    }
    /// `BlockState.getMapColor`: the state's `MapColor.id`, 0 for none. None for an unknown state.
    pub(super) fn block_state_map_color(&self, state: BlockStateId) -> Option<u8> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).map_color)
    }
    /// The immutable local ordinals of `block`, independent of current world ID order.
    pub(super) fn block_states_of(&self, block: BlockKey) -> Option<BlockStateRange> {
        mod_api::__private::block_state_range(block).ok()
    }

    /// The properties of a resolved block, copied for the mod as plain data.
    pub(super) fn block_state_definition(&self, block: BlockKey) -> Option<Vec<StateProperty>> {
        let default = super::api_values::default_state(block)?;
        let state = self.pack.assets.registry.state(default)?;
        let properties = self.pack.block_registries.blocks.get(&state.name)?;
        Some(
            properties
                .iter()
                .map(|(name, values)| StateProperty::new(name.clone(), values.clone()))
                .collect(),
        )
    }

    /// Match against the existing command registry definitions without allocating on the call.
    pub(super) fn block_state_matches(
        &self,
        state: BlockStateId,
        block: BlockKey,
        constraints: &[StateConstraint],
    ) -> Option<Result<bool, StateMatchError>> {
        let default = super::api_values::default_state(block)?;
        let registry = self.pack.assets.registry;
        let target = registry.state(default)?;
        let state = registry.state(state)?;
        let definition = self.pack.block_registries.blocks.get(&target.name)?;
        Some(match_state_properties(
            definition,
            state.name == target.name,
            |name, value| {
                state
                    .properties
                    .get(name)
                    .is_some_and(|actual| actual == value)
            },
            constraints,
        ))
    }

    /// `Item.getCraftingRemainder`: Horizon currently stores the remainder's
    /// item id, so the result is a single plain stack.
    pub(super) fn item_crafting_remainder(&self, stack: &ItemStack) -> Option<ItemStack> {
        crate::crafting::CraftingRemainder::crafting_remainder(self.res.items(), stack)
    }

    pub(in crate::game) fn crafting_remainder(&self, stack: &ItemStack) -> Option<ItemStack> {
        let mut remainder = self.item_crafting_remainder(stack);
        if !self.options.execution_role.is_client()
            && mod_api::__private::item_listens(mod_api::HookId::CraftingRemainder, &stack.item)
        {
            self.native_crafting_remainder(stack, &mut remainder);
        }
        remainder.filter(|value| value.count > 0)
    }

    pub(in crate::game) fn craft_grid_remainders(&self, recipe: &str) -> Vec<Option<ItemStack>> {
        self.recipes().remainders(
            recipe,
            &self.craft_grid[..self.craft_size * self.craft_size],
            self.craft_size,
            &|s| self.crafting_remainder(s),
        )
    }

    /// `Block.asItem`: the block's item name, including air for a block with
    /// no item; None if the block name is unknown.
    pub(super) fn block_item(&self, block: BlockKey) -> Option<mod_api::ItemKey> {
        let behaviours = self.res.behaviours();
        let id = super::api_values::behavior_block(block, behaviours)?;
        mod_api::__private::item_key(&behaviours.block_data(id).item)
    }

    /// `BlockItem.getBlock`: the block `item` places. The behaviour table
    /// keeps one binding per item that places a block (`wheat_seeds` to
    /// `wheat`), and the state table names it.
    pub(super) fn item_block(&self, item: ItemKey) -> Option<BlockKey> {
        let behaviours = self.res.behaviours();
        let id = mod_api::__private::item_key_id(item)?;
        let block = behaviours.item_binding(id)?.block;
        mod_api::__private::block_key(&behaviours.table.info(block).name)
    }
}

pub(super) fn match_state_properties(
    definition: &[(String, Vec<String>)],
    same_type: bool,
    has_value: impl Fn(&str, &str) -> bool,
    constraints: &[StateConstraint],
) -> Result<bool, StateMatchError> {
    let mut matches = same_type;
    for (index, constraint) in constraints.iter().enumerate() {
        let Some((_, values)) = definition.iter().find(|(name, _)| name == &constraint.name) else {
            return Err(StateMatchError::UnknownProperty(index as u32));
        };
        if !values.contains(&constraint.value) {
            return Err(StateMatchError::InvalidValue(index as u32));
        }
        matches &= has_value(&constraint.name, &constraint.value);
    }
    Ok(matches)
}

/// The command registries' block table of `registry` alone, built once: what
/// the parser checks properties against, whether a world is open or not.
pub(super) fn block_registries(registry: &'static BlockRegistry) -> Arc<Registries> {
    static CACHE: Mutex<Vec<(usize, Arc<Registries>)>> = Mutex::new(Vec::new());
    let key = std::ptr::from_ref(registry) as usize;
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, reg)) = cache.iter().find(|(cached, _)| *cached == key) {
        return reg.clone();
    }
    let reg = Arc::new(Registries {
        blocks: (*super::chat::block_properties(registry)).clone(),
        ..Registries::default()
    });
    cache.push((key, reg.clone()));
    reg
}

/// `BlockStateParser.parseForBlock` of the whole of `name`, NBT included, as
/// `/setblock` reads it; a state with NBT has no id.
pub(super) fn parse(
    reg: &Registries,
    registry: &BlockRegistry,
    name: &str,
) -> Option<BlockStateId> {
    let mut reader = Reader::new(name);
    let input = block::parse(&mut reader, reg, false).ok()?;
    if reader.can_read() || input.nbt.is_some() {
        return None;
    }
    let state = BlockState {
        name: input.id,
        properties: input.properties.into_iter().collect(),
    };
    registry.resolve(&state).ok()
}

/// `BlockStateParser.serialize`: the block, then `[key=value,...]` in the
/// state definition's (name) order.
pub(super) fn serialize(state: &BlockState) -> String {
    if state.properties.is_empty() {
        return state.name.clone();
    }
    let properties: Vec<String> = state
        .properties
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    format!("{}[{}]", state.name, properties.join(","))
}

/// `Util.makeDescriptionId`: `<kind>.<namespace>.<path>`, `/` in the path as `.`.
fn description_id(kind: &str, id: &str) -> Option<String> {
    let (namespace, path) = id.split_once(':')?;
    Some(format!("{kind}.{namespace}.{}", path.replace('/', ".")))
}

impl crate::crafting::CraftingRemainder for Game<'_> {
    fn crafting_remainder(&self, stack: &ItemStack) -> Option<ItemStack> {
        Game::crafting_remainder(self, stack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Air, a stair with two properties (its default `facing=north,half=bottom`)
    /// and wheat with an integer `age`, as `parse` reads them.
    fn blocks() -> (Arc<Registries>, &'static BlockRegistry) {
        let facts = br#"{"defaults":{"minecraft:air":0,"minecraft:oak_stairs":1,"minecraft:wheat":5},"states":[
            ["air",true,false,"","minecraft:air"],
            ["s",false,false,"",{"id":"minecraft:oak_stairs","properties":{"facing":"north","half":"bottom"}}],
            ["s",false,false,"",{"id":"minecraft:oak_stairs","properties":{"facing":"north","half":"top"}}],
            ["s",false,false,"",{"id":"minecraft:oak_stairs","properties":{"facing":"east","half":"bottom"}}],
            ["s",false,false,"",{"id":"minecraft:oak_stairs","properties":{"facing":"east","half":"top"}}],
            ["w",false,false,"",{"id":"minecraft:wheat","properties":{"age":"0"}}],
            ["w",false,false,"",{"id":"minecraft:wheat","properties":{"age":"1"}}]
        ]}"#;
        let registry = Box::leak(Box::new(
            BlockRegistry::from_facts_json(&facts[..]).unwrap(),
        ));
        (block_registries(registry), registry)
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn item_and_block_registry_facts_use_the_loaded_tables() {
        use super::super::api_values::TryIntoApi;
        use mod_api::Host;
        super::super::native_entity_refs::tests::with_game(|game| {
            let host: &dyn Host = game;
            let remainder = host
                .item_crafting_remainder(
                    &ItemStack::new("minecraft:water_bucket", 1)
                        .try_into_api()
                        .unwrap(),
                )
                .unwrap();
            assert_eq!(
                remainder.item(),
                mod_api::__private::item_key("minecraft:bucket").unwrap()
            );
            assert_eq!(remainder.count(), 1);
            assert!(
                host.item_crafting_remainder(
                    &ItemStack::new("minecraft:stone", 1).try_into_api().unwrap()
                )
                .is_none()
            );
            assert_eq!(
                host.block_item(mod_platform::__private::part_tests::block_key(
                    "minecraft:stone"
                ))
                .and_then(mod_api::__private::item_key_id),
                Some("minecraft:stone")
            );
            assert_eq!(
                host.block_item(mod_platform::__private::part_tests::block_key(
                    "minecraft:water"
                ))
                .and_then(mod_api::__private::item_key_id),
                Some("minecraft:air")
            );
            let item = |id| mod_api::__private::item_key(id).unwrap();
            let block = mod_platform::__private::part_tests::block_key;
            assert_eq!(
                host.item_block(item("minecraft:oak_planks")),
                Some(block("minecraft:oak_planks"))
            );
            // `Block.byItem(WHEAT_SEEDS)` is `Blocks.WHEAT`, whose item id is
            // the seeds' even though the names differ.
            assert_eq!(
                host.item_block(item("minecraft:wheat_seeds")),
                Some(block("minecraft:wheat"))
            );
            // A standing/wall item places its standing block; the wall variant
            // is the placement's, not the item's (`Block.byItem(TORCH)`).
            assert_eq!(
                host.item_block(item("minecraft:torch")),
                Some(block("minecraft:torch"))
            );
            assert_eq!(host.item_block(item("minecraft:stick")), None);
        });
    }

    /// A mod item maps back to the content block it places: the item's
    /// binding is the block the content root registered for it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn item_block_maps_a_mod_items_back_to_its_block() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game_at};
        use mod_api::{self, Hooks, Host};
        use std::sync::Arc;

        const CHILD: &str = "HORIZON_ITEM_BLOCK_MOD_CHILD";
        if run_fresh_cases(
            "game::native_block_states::tests::item_block_maps_a_mod_items_back_to_its_block",
            CHILD,
            &["game"],
        ) {
            return;
        }
        const CRATE: &str = "item_test:crate";
        struct TestMod;
        #[mod_api::__subscriptions]
        impl Hooks for TestMod {
            fn blocks(
                &self,
                _context: &mod_api::RegistrationContext,
                event: &mut mod_api::BlocksEvent<'_>,
            ) {
                event
                    .blocks
                    .push(mod_api::Block::new(CRATE, "minecraft:stone"));
            }
        }
        let mods: mod_api::Mods =
            Arc::from([mod_api::ModEntry::new("item_test", Box::new(TestMod))]);
        mod_platform::__private::install(mods.clone());
        crate::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = super::super::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        with_game_at(root, |game| {
            let host: &dyn Host = game;
            assert_eq!(
                host.item_block(mod_api::__private::item_key(CRATE).unwrap()),
                mod_api::__private::block_key(CRATE)
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn host_key_iteration_matches_the_26_3_registries_in_registry_order() {
        use mod_api::Host;
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        // The content root publishes the built-in entity-type and menu orders, as startup does.
        let root = super::super::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        super::super::native_entity_refs::tests::with_game_at(root.clone(), |game| {
            let host: &dyn Host = game;
            let blocks = host.block_keys();
            assert_eq!(blocks.len(), 1286);
            assert_eq!(blocks[0].id(host), "minecraft:air");
            assert_eq!(blocks.last().unwrap().id(host), "minecraft:firefly_bush");

            let items = host.item_keys();
            assert_eq!(items.len(), 1658);
            assert_eq!(items[0].id(host), "minecraft:air");
            assert_eq!(items.last().unwrap().id(host), "minecraft:ominous_bottle");

            let entities = host.entity_type_keys();
            assert_eq!(entities.len(), 161);
            assert_eq!(entities[0].id(host), "minecraft:acacia_boat");
            assert_eq!(
                entities.last().unwrap().id(host),
                "minecraft:fishing_bobber"
            );

            let menus = host.menu_keys();
            assert_eq!(menus.len(), 25);
            assert_eq!(menus[0].id(host), "minecraft:generic_9x1");
            assert_eq!(menus.last().unwrap().id(host), "minecraft:stonecutter");

            // Enchantments load with the item tables, which this harness stops short of;
            // read them the way a world open does.
            let items = horizon_client::items::Items::load(
                &root,
                game.pack.assets.registry,
                game.res.facts(),
                &horizon_world::loot::PackOverlay::NONE,
                game.res.item_defaults(),
            )
            .unwrap();
            mod_api::__private::resolve_enchantments(
                items.enchantments.list.iter().map(|entry| &*entry.id),
            )
            .unwrap();
            let enchantments = host.enchantment_keys();
            assert_eq!(enchantments.len(), 43);
            assert_eq!(enchantments[0].id(host), "minecraft:aqua_affinity");
            assert_eq!(
                enchantments.last().unwrap().id(host),
                "minecraft:wind_burst"
            );

            // A second read is the same table in the same order.
            assert_eq!(host.block_keys(), blocks);
            assert_eq!(host.enchantment_keys(), enchantments);
        });
    }

    #[test]
    #[ignore = "requires prepared resources"]
    fn public_host_resolves_literal_block_and_effect_names() {
        mod_platform::testing::with_mods(std::sync::Arc::from([]), || {
            let root =
                std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
            super::super::native_entity_refs::tests::with_game_at(root, |game| {
                let host: &dyn mod_api::Host = game;
                assert_eq!(
                    host.block_key("minecraft:stone").unwrap().id(host),
                    "minecraft:stone"
                );
                assert_eq!(
                    host.effect_key("minecraft:speed").unwrap().id(host),
                    "minecraft:speed"
                );
                assert_eq!(host.block_key("test:missing_block"), None);
                assert_eq!(host.effect_key("test:missing_effect"), None);
            });
        });
    }

    #[test]
    fn description_ids_write_a_path_separator_as_a_dot() {
        assert_eq!(
            super::description_id("fluid_type", "ruby:molten/iron").as_deref(),
            Some("fluid_type.ruby.molten.iron")
        );
        assert_eq!(super::description_id("block", "no_namespace"), None);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn fluid_type_properties_answer_each_fluid_family() {
        use mod_api::{Host, HostFluidStates};
        super::super::native_entity_refs::tests::with_game(|game| {
            let host: &dyn Host = game;
            let service = host.fluid_states();
            let read = |id| {
                service
                    .type_properties(host.fluid_key(id).unwrap())
                    .unwrap()
            };
            let water = FluidTypeProperties {
                description_id: "block.minecraft.water".into(),
                light_level: 0,
                fall_distance_modifier: 0.0,
                can_push_entity: true,
                can_swim: true,
                can_drown: true,
                can_extinguish: true,
                can_hydrate: true,
                supports_boating: true,
                explosion_resistance: 100.0,
                randomly_ticks: false,
                drip_particle: Some("minecraft:dripping_water".into()),
            };
            let lava = FluidTypeProperties {
                description_id: "block.minecraft.lava".into(),
                light_level: 15,
                fall_distance_modifier: 0.5,
                can_push_entity: true,
                can_swim: false,
                can_drown: false,
                can_extinguish: false,
                can_hydrate: false,
                supports_boating: false,
                explosion_resistance: 100.0,
                randomly_ticks: true,
                drip_particle: Some("minecraft:dripping_lava".into()),
            };
            let inert = |description_id: &str| FluidTypeProperties {
                description_id: description_id.into(),
                light_level: 0,
                fall_distance_modifier: 1.0,
                can_push_entity: false,
                explosion_resistance: 0.0,
                randomly_ticks: false,
                drip_particle: None,
                ..lava.clone()
            };
            assert_eq!(read("minecraft:water"), water);
            assert_eq!(read("minecraft:flowing_water"), water);
            assert_eq!(read("minecraft:lava"), lava);
            assert_eq!(read("minecraft:flowing_lava"), lava);
            assert_eq!(read("minecraft:empty"), inert("block.minecraft.air"));
            assert_eq!(
                read("horizon:definition_test"),
                inert("fluid_type.horizon.definition_test")
            );
            assert_eq!(host.fluid_key("minecraft:no_such_fluid"), None);
        });
    }

    /// `FluidState.createLegacyBlock`: a source is its block at level 0, flowing water of amount
    /// 3 is level 5, falling adds 8, the empty fluid is air; states no fluid can be in have none.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn legacy_blocks_follow_the_liquid_levels() {
        use mod_api::{Host, HostFluidStates};
        super::super::native_entity_refs::tests::with_game(|game| {
            let host: &dyn Host = game;
            let service = host.fluid_states();
            // A mod may edit the copy it is given, so any amount and source flag can reach the
            // service: start from the engine's state and overwrite them.
            let state = |id: &str, amount, falling, source| {
                let kind = FluidKind::from_name(id).unwrap_or(FluidKind::Water);
                let _ = EngineFluidState::new(kind, 1, falling);
                let mut state =
                    mod_api::__private::fluid_state(host.fluid_key(id).unwrap(), 1, falling, false);
                (state.amount, state.source) = (amount, source);
                state
            };
            let block = |state: FluidState| {
                let id = service.legacy_block(state)?;
                game.pack.assets.registry.state(BlockStateId(id.0)).cloned()
            };
            let liquid = |name: &str, level: &str| {
                Some(BlockState {
                    name: name.into(),
                    properties: [("level".to_owned(), level.to_owned())].into(),
                })
            };
            assert_eq!(
                block(state("minecraft:water", 8, false, true)),
                liquid("minecraft:water", "0")
            );
            assert_eq!(
                block(state("minecraft:flowing_water", 3, false, false)),
                liquid("minecraft:water", "5")
            );
            assert_eq!(
                block(state("minecraft:flowing_lava", 3, true, false)),
                liquid("minecraft:lava", "13")
            );
            assert_eq!(
                block(state("minecraft:flowing_water", 1, false, false)),
                liquid("minecraft:water", "7")
            );
            assert_eq!(
                block(state("minecraft:empty", 0, false, false)).map(|s| s.name),
                Some("minecraft:air".into())
            );
            for refused in [
                state("minecraft:flowing_water", 9, false, false),
                state("minecraft:flowing_water", 0, false, false),
                state("minecraft:water", 3, false, true),
                state("minecraft:empty", 8, false, false),
                state("horizon:definition_test", 8, false, true),
                state("minecraft:flowing_water", u8::MAX, true, false),
            ] {
                assert_eq!(service.legacy_block(refused), None, "{refused:?}");
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn fluid_definitions_and_constraints_use_the_loaded_registry() {
        use mod_api::{Host, HostFluidStates};
        super::super::native_entity_refs::tests::with_game(|game| {
            let host: &dyn Host = game;
            let water = host.fluid_key("minecraft:water").unwrap();
            let flowing = host.fluid_key("minecraft:flowing_water").unwrap();
            let service = host.fluid_states();
            for phase in [
                mod_api::__private::ModPhase::Setup,
                mod_api::__private::ModPhase::Reloading,
            ] {
                let _scope = mod_api::__private::PhaseOwner::scope(phase);
                assert!(matches!(
                    service.definition(water),
                    Some(FluidDefinition::World { .. })
                ));
                assert_eq!(service.validate(water, &[]), Ok(()));
            }
            let tank = host.fluid_key("horizon:definition_test").unwrap();
            assert_eq!(service.definition(tank), Some(FluidDefinition::TankOnly));
            assert_eq!(
                service.validate(tank, &[]),
                Err(FluidStateError::NoWorldState)
            );
            for (id, amount, source, property_count) in [
                ("minecraft:empty", 0, false, 0),
                ("minecraft:water", 8, true, 1),
                ("minecraft:flowing_water", 1, false, 2),
                ("minecraft:lava", 8, true, 1),
                ("minecraft:flowing_lava", 1, false, 2),
            ] {
                let key = host.fluid_key(id).unwrap();
                let FluidDefinition::World {
                    default,
                    properties,
                } = service.definition(key).unwrap()
                else {
                    panic!("world definition")
                };
                assert_eq!(
                    (
                        default.fluid,
                        default.amount,
                        default.source,
                        default.falling
                    ),
                    (key, amount, source, amount != 0)
                );
                assert_eq!(properties.len(), property_count);
                assert_eq!(service.validate(key, &[]), Ok(()));
            }
            assert_eq!(
                service.validate(
                    flowing,
                    &[
                        StateConstraint::new("unknown", "true"),
                        StateConstraint::new("level", "1"),
                        StateConstraint::new("level", "1")
                    ]
                ),
                Err(FluidStateError::UnknownProperty(0))
            );
            let FluidDefinition::World {
                default,
                properties,
            } = service.definition(water).unwrap()
            else {
                panic!("world fluid")
            };
            assert!(default.is_source());
            assert!(properties.iter().any(|p| p.name == "falling"));
            assert_eq!(
                service.validate(water, &[StateConstraint::new("level", "8")]),
                Err(FluidStateError::UnknownProperty(0))
            );
            assert_eq!(
                service.validate(flowing, &[StateConstraint::new("level", "9")]),
                Err(FluidStateError::InvalidValue(0))
            );
            assert_eq!(
                service.validate(
                    flowing,
                    &[
                        StateConstraint::new("level", "8"),
                        StateConstraint::new("level", "8")
                    ]
                ),
                Err(FluidStateError::Duplicate(1))
            );
            assert!(default.matches(&[StateConstraint::new("falling", "true")]));
            assert!(!default.matches(&[StateConstraint::new("unknown", "true")]));
            let FluidDefinition::World {
                default: flowing_default,
                ..
            } = service.definition(flowing).unwrap()
            else {
                panic!("flowing fluid")
            };
            assert!(flowing_default.matches(&[StateConstraint::new("level", "1")]));
            assert!(!flowing_default.matches(&[StateConstraint::new("level", "8")]));
        });
    }

    #[test]
    fn block_property_constraints_validate_and_match_partial_states() {
        let (reg, registry) = blocks();
        let target = registry.state(BlockStateId(1)).unwrap();
        let definition = &reg.blocks["minecraft:oak_stairs"];
        let check = |id, constraints: &[StateConstraint]| {
            let state = registry.state(BlockStateId(id)).unwrap();
            match_state_properties(
                definition,
                target.name == state.name,
                |name, value| {
                    state
                        .properties
                        .get(name)
                        .is_some_and(|actual| actual == value)
                },
                constraints,
            )
        };
        let east = StateConstraint::new("facing", "east");
        assert_eq!(check(4, std::slice::from_ref(&east)), Ok(true));
        assert_eq!(check(1, std::slice::from_ref(&east)), Ok(false));
        assert_eq!(check(0, std::slice::from_ref(&east)), Ok(false));
        assert_eq!(
            check(4, &[StateConstraint::new("shape", "straight")]),
            Err(StateMatchError::UnknownProperty(0))
        );
        assert_eq!(
            check(4, &[StateConstraint::new("facing", "up")]),
            Err(StateMatchError::InvalidValue(0))
        );
        assert_eq!(check(4, &[]), Ok(true));
        assert_eq!(definition.len(), 2);
        assert!(
            definition
                .iter()
                .any(|(name, values)| name == "half" && values.contains(&"top".to_owned()))
        );
    }

    #[test]
    fn a_state_name_reads_as_setblock_reads_it_and_round_trips() {
        let (reg, registry) = blocks();
        let id = |name: &str| parse(&reg, registry, name);
        assert_eq!(id("minecraft:air"), Some(BlockStateId(0)));
        assert_eq!(id("oak_stairs"), Some(BlockStateId(1)));
        assert_eq!(
            id("oak_stairs[half=top,facing=east]"),
            Some(BlockStateId(4))
        );
        assert_eq!(
            id("minecraft:oak_stairs[facing=east]"),
            Some(BlockStateId(3))
        );
        // IntegerProperty.getValue: Integer.parseInt.
        assert_eq!(id("wheat[age=01]"), Some(BlockStateId(6)));
        for state in 0..7 {
            let text = serialize(registry.state(BlockStateId(state)).unwrap());
            assert_eq!(id(&text), Some(BlockStateId(state)), "{text}");
        }
        assert_eq!(
            serialize(registry.state(BlockStateId(4)).unwrap()),
            "minecraft:oak_stairs[facing=east,half=top]"
        );
        assert_eq!(
            serialize(registry.state(BlockStateId(0)).unwrap()),
            "minecraft:air"
        );
    }

    #[test]
    fn a_state_name_that_does_not_read_has_no_id() {
        let (reg, registry) = blocks();
        for name in [
            "minecraft:nothing",
            "oak_stairs[shape=straight]",
            "oak_stairs[facing=up]",
            "oak_stairs[facing=east,facing=north]",
            "wheat[age=2]",
            "oak_stairs{}",
            "oak_stairs extra",
            "#minecraft:stairs",
            "",
        ] {
            assert_eq!(parse(&reg, registry, name), None, "{name:?}");
        }
    }

    #[test]
    fn a_block_key_resolves_its_default_state() {
        use mod_platform::__private::part_tests;
        let (_, registry) = blocks();
        mod_platform::__private::install(Default::default());
        crate::game::api_values::register_game_keys([
            "minecraft:oak_stairs".to_owned(),
            "minecraft:wheat".to_owned(),
        ]);
        crate::game::api_values::resolve_block_definitions(registry).unwrap();
        assert_eq!(
            crate::game::api_values::default_state(part_tests::block_key("oak_stairs")).unwrap(),
            BlockStateId(1)
        );
        assert_eq!(
            crate::game::api_values::default_state(part_tests::block_key("minecraft:wheat"))
                .unwrap(),
            BlockStateId(5)
        );
        // A valid key has no mapping on a different owning thread. Raw unknown slots are
        // refused by the generated host boundary, not revalidated by typed operations.
        let key = part_tests::block_key("minecraft:wheat");
        assert!(
            std::thread::spawn(move || crate::game::api_values::default_state(key))
                .join()
                .unwrap()
                .is_none()
        );
    }
}

#[cfg(test)]
mod facts_tests {
    use crate::game::api_values::TryIntoApi;
    use mod_api::{BlockStateRecord, HostBlockStates};

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn block_state_facts_read_vanillas_tables_through_the_host() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let state = |name: &str| {
                game.pack
                    .assets
                    .registry
                    .default_state(name)
                    .unwrap()
                    .try_into_api()
                    .unwrap()
            };
            let (stone, obsidian, bedrock) = (
                state("minecraft:stone"),
                state("minecraft:obsidian"),
                state("minecraft:bedrock"),
            );
            let (air, grass, chest) = (
                state("minecraft:air"),
                state("minecraft:short_grass"),
                state("minecraft:chest"),
            );
            let stairs = state("minecraft:oak_stairs");
            let facts = game.block_states();
            // destroyTime: stone 1.5, obsidian 50, bedrock -1 (unbreakable).
            assert_eq!(facts.destroy_speed(stone), Some(1.5));
            assert_eq!(facts.destroy_speed(obsidian), Some(50.0));
            assert_eq!(facts.destroy_speed(bedrock), Some(-1.0));
            // canBeReplaced: air and grass, not stone.
            assert_eq!(facts.replaceable(air), Some(true));
            assert_eq!(facts.replaceable(grass), Some(true));
            assert_eq!(facts.replaceable(stone), Some(false));
            assert_eq!(facts.has_block_entity(chest), Some(true));
            assert_eq!(facts.has_block_entity(stone), Some(false));
            // MapColor.STONE is id 11 and air has none.
            assert_eq!(facts.map_color(stone), Some(11));
            assert_eq!(facts.map_color(air), Some(0));
            // SoundType.STONE: its break event, at volume and pitch 1.
            let sound = facts.sound(stone).unwrap();
            assert_eq!(&*sound.break_sound, "minecraft:block.stone.break");
            assert_eq!((sound.volume, sound.pitch), (1.0, 1.0));
            // Oak stairs: facing 4 × half 2 × shape 5 × waterlogged 2 = 80 states, its default
            // among them; stone has one.
            let range = facts
                .states_of(mod_platform::__private::part_tests::block_key(
                    "minecraft:oak_stairs",
                ))
                .unwrap();
            assert_eq!(range.len(), 80);
            assert!(range.contains(stairs));
            let key = mod_platform::__private::part_tests::block_key("minecraft:oak_stairs");
            assert!(range.iter().all(|state| state.block() == key));
            assert_eq!(
                range
                    .iter()
                    .map(mod_api::BlockState::ordinal)
                    .collect::<Vec<_>>(),
                (0..80).collect::<Vec<u16>>()
            );
            assert_eq!(
                facts
                    .states_of(mod_platform::__private::part_tests::block_key(
                        "minecraft:stone"
                    ))
                    .map(|r| (r.first(), r.len())),
                Some((stone, 1))
            );
            // A forged local ordinal refuses at the generated Game crossing.
            let invalid = BlockStateRecord::new(stone.block(), u16::MAX).into();
            assert_eq!(facts.destroy_speed(invalid), None);
            assert_eq!(facts.sound(invalid), None);
        });
    }
}

#[cfg(test)]
mod crafting_remainder_tests {
    use super::*;
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    struct Worn;
    #[mod_api::horizon_part]
    impl mod_api::ItemPart for Worn {
        fn crafting_remainder(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::CraftingRemainderEvent<'_>,
        ) {
            CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            *event.remainder = if event.stack.damage() == 0 {
                let mut stack = event.stack.clone();
                stack.set_count(65).unwrap();
                Some(stack)
            } else if event.stack.damage() >= 4 {
                None
            } else {
                let mut stack = event.stack.clone();
                stack.set_count(1).unwrap();
                stack.set_damage(event.stack.damage() + 1);
                Some(stack)
            };
        }
    }
    struct Empty;
    #[mod_api::horizon_part]
    impl mod_api::ItemPart for Empty {
        fn crafting_remainder(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::CraftingRemainderEvent<'_>,
        ) {
            CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            *event.remainder = None;
        }
    }
    struct DefaultPart;
    #[mod_api::horizon_part]
    impl mod_api::ItemPart for DefaultPart {
        fn inventory_tick(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::InventoryTickEvent<'_>,
        ) {
        }
    }
    struct RemainderMod;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for RemainderMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let tool = r.item_key("minecraft:wooden_hoe");
            r.item_part(tool, Worn);
            let milk = r.item_key("minecraft:milk_bucket");
            r.item_part(milk, DefaultPart);
            let water = r.item_key("minecraft:water_bucket");
            r.item_part(water, Empty);
        }
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn crafting_remainder_without_parts_matches_data() {
        let mods: mod_api::Mods = Vec::new().into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                assert!(!mod_api::__private::has_part_hook(
                    mod_api::HookId::CraftingRemainder
                ));
                let grid = [
                    Some(ItemStack::new("minecraft:milk_bucket", 1)),
                    None,
                    Some(ItemStack::new("minecraft:stone", 1)),
                    None,
                ];
                let remaining = game
                    .recipes()
                    .remainders("missing", &grid, 2, &|s| game.crafting_remainder(s));
                assert_eq!(
                    remaining,
                    vec![
                        Some(ItemStack::new("minecraft:bucket", 1)),
                        None,
                        None,
                        None
                    ]
                );
            });
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn crafting_remainder_part_override_and_default() {
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("remainder", Box::new(RemainderMod))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                super::super::native_entity_refs::tests::resolve_parts(game).unwrap();
                let mut tool = ItemStack {
                    damage: 2,
                    ..ItemStack::new("minecraft:wooden_hoe", 1)
                };
                tool.data_mut()
                    .set_other("minecraft:max_damage", Some(horizon_core::nbt::Tag::Int(5)));
                tool = game.crafting_remainder(&tool).unwrap();
                assert_eq!((tool.damage, tool.count), (3, 1));
                tool = game.crafting_remainder(&tool).unwrap();
                assert_eq!((tool.damage, tool.count), (4, 1));
                assert!(game.crafting_remainder(&tool).is_none());
                let calls = CALLS.load(std::sync::atomic::Ordering::Relaxed);
                for id in ["minecraft:milk_bucket", "minecraft:lava_bucket"] {
                    let remainder = game.crafting_remainder(&ItemStack::new(id, 1)).unwrap();
                    assert_eq!((&*remainder.item, remainder.count), ("minecraft:bucket", 1));
                }
                assert_eq!(
                    CALLS.load(std::sync::atomic::Ordering::Relaxed),
                    calls,
                    "items without this subscription call no Part"
                );
                let water = ItemStack::new("minecraft:water_bucket", 1);
                assert!(
                    game.crafting_remainder(&water).is_none(),
                    "None suppresses the data remainder"
                );
                let bucket = game.item_crafting_remainder(&water).unwrap();
                assert_eq!((&*bucket.item, bucket.count), ("minecraft:bucket", 1));
                assert!(game.item_crafting_remainder(&tool).is_none());
                assert!(
                    game.crafting_remainder(&ItemStack::new("minecraft:wooden_hoe", 1))
                        .is_none(),
                    "oversized Part output is refused"
                );
                use crate::furnace::FurnaceCooking;
                let fuel = ItemStack {
                    damage: 2,
                    ..ItemStack::new("minecraft:wooden_hoe", 1)
                };
                let mut furnace = crate::containers::Furnace {
                    kind: crate::containers::FurnaceKind::Furnace,
                    items: [
                        Some(ItemStack::new("minecraft:iron_ore", 1)),
                        Some(fuel.clone()),
                        None,
                    ],
                    lit_remaining: 0,
                    lit_total: 0,
                    cooking_timer: 0,
                    cooking_total: 0,
                    speed: 1.0,
                    recipes_used: Default::default(),
                };
                let mut drops = Vec::new();
                furnace.tick(
                    &game.loop_context,
                    game.recipes(),
                    game.items(),
                    game.res.providers(),
                    &mut drops,
                    &|s| game.crafting_remainder(s),
                );
                let grid = [Some(fuel), None, None, None];
                let crafted = game
                    .recipes()
                    .remainders("missing", &grid, 2, &|s| game.crafting_remainder(s));
                assert_eq!(furnace.items[1], crafted[0]);
                assert_eq!(
                    furnace.items[1].as_ref().map(|s| (s.damage, s.count)),
                    Some((3, 1))
                );
                let role = game.options.execution_role;
                game.options.execution_role =
                    crate::game::execution_role::ExecutionRole::DedicatedServer;
                let remainder = game.crafting_remainder(grid[0].as_ref().unwrap()).unwrap();
                assert_eq!((remainder.damage, remainder.count), (3, 1));
                game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
                let calls = CALLS.load(std::sync::atomic::Ordering::Relaxed);
                assert!(game.crafting_remainder(grid[0].as_ref().unwrap()).is_none());
                assert_eq!(CALLS.load(std::sync::atomic::Ordering::Relaxed), calls);
                game.options.execution_role = role;
            })
        });
    }
}
