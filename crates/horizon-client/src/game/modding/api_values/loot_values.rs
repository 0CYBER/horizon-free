//! Owned loot values at the guarded game boundary. No inventory count rules apply.
use super::{TryIntoApi, TryIntoEngine};
use horizon_world::loot as engine;
use mod_api::world::snapshots as api;

pub(crate) fn dispatch_loot_function(
    kind: &str,
    data: &serde_json::Value,
    ctx: &engine::LootContext<'_>,
    stack: &mut engine::LootStack,
    tags: &dyn engine::LootTags,
) {
    if !mod_api::__private::listened(mod_api::HookId::LootFunction) {
        return;
    }
    let (Ok(ctx), Ok(mut value)) = (ctx.try_into_api(), stack.try_into_api()) else {
        return;
    };
    let tags = crate::game::loot_tags::LootTags(tags);
    mod_api::loot_callbacks::native_loot_function(kind, data, &ctx, &mut value, &tags);
    // Stage the complete candidate before making the one engine write.
    if let Ok(value) = value.try_into_engine(&()) {
        *stack = value;
    }
}
pub(crate) fn dispatch_loot_condition(
    kind: &str,
    data: &serde_json::Value,
    ctx: &engine::LootContext<'_>,
    holds: &mut bool,
    tags: &dyn engine::LootTags,
) {
    if !mod_api::__private::listened(mod_api::HookId::LootCondition) {
        return;
    }
    let Ok(ctx) = ctx.try_into_api() else {
        return;
    };
    let tags = crate::game::loot_tags::LootTags(tags);
    mod_api::loot_callbacks::native_loot_condition(kind, data, &ctx, holds, &tags);
}

#[cfg(test)]
thread_local! { static CONVERSIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LootValueError {
    UnknownItem,
    InvalidName,
    InvalidPatch,
}

impl TryIntoApi for engine::LootStack {
    type Api = api::LootStack;
    type Error = LootValueError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        #[cfg(test)]
        CONVERSIONS.with(|count| count.set(count.get() + 1));
        let item = mod_api::__private::item_key(&self.item).ok_or(LootValueError::UnknownItem)?;
        let mut value = api::LootStack::new(item, self.count);
        value.damage = self.damage;
        value.enchantments = self.enchantments.clone();
        value.potion = self.potion.clone();
        value.map = self.map;
        value.exploration = self.exploration.as_ref().map(|map| {
            Box::new(api::ExplorationMap::new(
                map.destination.clone(),
                map.decoration.clone(),
                map.zoom,
                map.search_radius,
                map.skip_existing_chunks,
            ))
        });
        value.ominous_amplifier = self.ominous_amplifier;
        value.stew_effects = self.stew_effects.clone();
        value.names = self
            .names
            .iter()
            .map(|(id, name)| {
                horizon_client_lib::api_text::captured_text(name)
                    .map(|name| (id.clone(), name))
                    .map_err(|_| LootValueError::InvalidName)
            })
            .collect::<Result<_, _>>()?;
        value.components = self.components.clone();
        Ok(value)
    }
}
impl<C: ?Sized> TryIntoEngine<C> for api::LootStack {
    type Engine = engine::LootStack;
    type Error = LootValueError;
    fn try_into_engine(&self, _: &C) -> Result<Self::Engine, Self::Error> {
        let id = mod_api::__private::item_key_id(self.item).ok_or(LootValueError::UnknownItem)?;
        // NBT's vanilla container-depth limit applies before copying mod-owned trees.
        let mut pending: Vec<_> = self
            .components
            .iter()
            .filter_map(|(_, tag)| tag.as_ref().map(|tag| (tag, 0)))
            .collect();
        while let Some((tag, depth)) = pending.pop() {
            use horizon_core::nbt::Tag;
            if matches!(tag, Tag::List(_) | Tag::Compound(_)) && depth >= 512 {
                return Err(LootValueError::InvalidPatch);
            }
            match tag {
                Tag::List(values) => pending.extend(values.iter().map(|value| (value, depth + 1))),
                Tag::Compound(values) => {
                    pending.extend(values.values().map(|value| (value, depth + 1)))
                }
                _ => {}
            }
        }
        let names = self
            .names
            .iter()
            .map(|(id, name)| {
                let name: horizon_client::text::Text = name
                    .try_into_engine(&())
                    .map_err(|_| LootValueError::InvalidName)?;
                Ok((id.clone(), name.component()))
            })
            .collect::<Result<_, _>>()?;
        Ok(engine::LootStack {
            item: id.to_owned(),
            count: self.count,
            damage: self.damage,
            enchantments: self.enchantments.clone(),
            potion: self.potion.clone(),
            map: self.map,
            exploration: self.exploration.as_ref().map(|map| {
                Box::new(engine::ExplorationMap {
                    destination: map.destination.clone(),
                    decoration: map.decoration.clone(),
                    zoom: map.zoom,
                    search_radius: map.search_radius,
                    skip_existing_chunks: map.skip_existing_chunks,
                })
            }),
            ominous_amplifier: self.ominous_amplifier,
            stew_effects: self.stew_effects.clone(),
            names,
            components: self.components.clone(),
        })
    }
}

fn entity(value: &engine::LootEntity) -> Result<api::LootEntity, LootValueError> {
    // Vehicles form a singly linked chain. Walk it explicitly rather than recursing.
    let mut chain = Vec::new();
    let mut next = Some(value);
    while let Some(value) = next {
        chain.push(value);
        next = value.vehicle.as_deref();
    }
    let mut vehicle = None;
    for value in chain.into_iter().rev() {
        let mut result = api::LootEntity::default();
        result.kind = value.kind.clone();
        result.living = value.living;
        result.on_fire = value.on_fire;
        result.baby = value.baby;
        result.vehicle = vehicle;
        result.cube_size = value.cube_size;
        result.sheep = value.sheep.clone();
        result.captain = value.captain;
        result.frog_variant = value.frog_variant.clone();
        result.chicken_variant = value.chicken_variant.clone();
        result.powered = value.powered;
        result.equipment = value
            .equipment
            .iter()
            .map(|(slot, stack)| Ok((slot.clone(), stack.try_into_api()?)))
            .collect::<Result<_, _>>()?;
        vehicle = Some(Box::new(result));
    }
    Ok(*vehicle.expect("a chain starts with its supplied entity"))
}
impl TryIntoApi for engine::LootContext<'_> {
    type Api = api::LootContext;
    type Error = LootValueError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        #[cfg(test)]
        CONVERSIONS.with(|count| count.set(count.get() + 1));
        let mut value = api::LootContext::new(self.seed);
        value.entity = self.entity.map(entity).transpose()?;
        value.player = self.player.map(entity).transpose()?;
        value.damage = self
            .damage
            .map(|damage| {
                let mut result = api::LootDamage::default();
                result.kind = damage.kind.clone();
                result.source = damage.source.as_ref().map(entity).transpose()?;
                result.direct = damage.direct.as_ref().map(entity).transpose()?;
                Ok(result)
            })
            .transpose()?;
        value.tool = self
            .tool
            .as_ref()
            .map(|tool| {
                Ok(api::LootTool::new(
                    mod_api::__private::item_key(&tool.item).ok_or(LootValueError::UnknownItem)?,
                    tool.enchantments.clone(),
                ))
            })
            .transpose()?;
        value.queried_table_id = self
            .queried_table_id
            .clone()
            .unwrap_or_else(|| horizon_core::UNKNOWN_LOOT_TABLE.into());
        value.queried_stack = self.queried_stack.as_ref().map(queried_stack).transpose()?;
        value.block = self.block.map(str::to_owned);
        value.luck = self.luck;
        value.biome = self.biome.map(str::to_owned);
        value.place = self.place.as_ref().map(|place| {
            let mut result = api::LootPlace::default();
            result.origin = place.origin;
            result.block_entity = place.block_entity.clone();
            result.level = place.level.as_ref().map(|level| {
                let mut result = api::LootLevel::default();
                result.dimension = level.dimension.clone();
                result.game_time = level.game_time;
                result.day_time = level.day_time;
                result.raining = level.raining;
                result.thundering = level.thundering;
                result
            });
            result
        });
        Ok(value)
    }
}

struct LootAbilityHost;
impl mod_api::Host for LootAbilityHost {}
mod_api::impl_host_areas!(LootAbilityHost);

pub(crate) fn dispatch_loot_ability(tool: &engine::LootStack, ability: &str, holds: &mut bool) {
    if !mod_api::__private::item_listens(mod_api::HookId::CanPerformAction, &tool.item) {
        return;
    }
    let Ok(stack) = queried_stack(tool) else {
        return;
    };
    mod_api::dispatch_can_perform_action(&LootAbilityHost, &stack, ability, holds);
}

fn queried_stack(stack: &engine::LootStack) -> Result<mod_api::ItemStack, LootValueError> {
    if stack.count <= 0 || stack.damage < 0 {
        return Err(LootValueError::InvalidPatch);
    }
    horizon_items::items::ItemStack::from_loot(stack)
        .try_into_api()
        .map_err(|_| LootValueError::InvalidPatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("loot_tags_tests.rs");
    use horizon_core::nbt::Tag;
    use mod_api::{Hooks, LootFunctionEvent, ModEntry};
    use mod_platform::testing::with_mods;
    fn stack() -> engine::LootStack {
        engine::LootStack {
            item: "minecraft:stick".into(),
            count: -7,
            damage: -3,
            enchantments: vec![],
            potion: None,
            map: true,
            exploration: None,
            ominous_amplifier: Some(2),
            stew_effects: vec![],
            names: vec![],
            components: vec![
                ("test:array".into(), Some(Tag::ByteArray(vec![7]))),
                ("test:byte".into(), Some(Tag::Byte(7))),
            ],
        }
    }
    fn context() -> engine::LootContext<'static> {
        engine::LootContext {
            queried_table_id: None,
            queried_stack: None,
            entity: None,
            damage: None,
            player: None,
            tool: None,
            block: None,
            luck: 1.5,
            biome: None,
            seed: 42,
            place: None,
        }
    }
    #[test]
    fn loot_context_owns_entity_tool_damage_and_place_facts() {
        super::super::prepare_stack_boundary_tests();
        let mut victim = engine::LootEntity::new("minecraft:sheep");
        victim.baby = true;
        victim.on_fire = true;
        victim.sheep = Some(("red".into(), true));
        victim.equipment.push(("mainhand".into(), stack()));
        victim.vehicle = Some(Box::new(engine::LootEntity::new("minecraft:boat")));
        let damage = engine::LootDamage {
            kind: "minecraft:arrow".into(),
            source: Some(engine::LootEntity::new("minecraft:skeleton")),
            direct: Some(engine::LootEntity::new("minecraft:arrow")),
        };
        let value = engine::LootContext {
            queried_table_id: None,
            queried_stack: None,
            entity: Some(&victim),
            damage: Some(&damage),
            player: None,
            tool: Some(engine::LootTool {
                item: "minecraft:stick".into(),
                enchantments: vec![("minecraft:fortune".into(), 3)],
            }),
            block: Some("minecraft:stone"),
            biome: Some("minecraft:plains"),
            luck: 1.5,
            seed: -42,
            place: Some(std::sync::Arc::new(engine::LootPlace {
                origin: Some([1.0, 2.0, 3.0]),
                block_entity: Some(Tag::ByteArray(vec![7])),
                level: Some(engine::LootLevel {
                    dimension: "test:level".into(),
                    game_time: 17,
                    day_time: -3,
                    raining: true,
                    thundering: false,
                }),
            })),
        }
        .try_into_api()
        .unwrap();
        let victim = value.entity.unwrap();
        assert!(victim.baby && victim.on_fire);
        assert_eq!(victim.sheep, Some(("red".into(), true)));
        assert_eq!(victim.vehicle.unwrap().kind, "minecraft:boat");
        assert_eq!(victim.equipment[0].1.count, -7);
        let damage = value.damage.unwrap();
        assert_eq!(damage.source.unwrap().kind, "minecraft:skeleton");
        assert_eq!(damage.direct.unwrap().kind, "minecraft:arrow");
        let tool = value.tool.unwrap();
        assert_eq!(
            mod_api::__private::item_key_id(tool.item),
            Some("minecraft:stick")
        );
        assert_eq!(tool.enchantments, vec![("minecraft:fortune".into(), 3)]);
        assert_eq!(
            (
                value.block.as_deref(),
                value.biome.as_deref(),
                value.luck,
                value.seed
            ),
            (Some("minecraft:stone"), Some("minecraft:plains"), 1.5, -42)
        );
        let place = value.place.unwrap();
        assert_eq!(place.origin, Some([1.0, 2.0, 3.0]));
        assert_eq!(place.block_entity, Some(Tag::ByteArray(vec![7])));
        let level = place.level.unwrap();
        assert_eq!(
            (
                level.dimension.as_str(),
                level.game_time,
                level.day_time,
                level.raining,
                level.thundering
            ),
            ("test:level", 17, -3, true, false)
        );
    }

    struct Edit;
    #[mod_api::__subscriptions]
    impl Hooks for Edit {
        fn loot_function(&self, event: &mut LootFunctionEvent<'_>) {
            assert_eq!(event.ctx.seed, 42);
            assert_eq!(event.stack.count, -7);
            assert_eq!(event.stack.components[0].1, Some(Tag::ByteArray(vec![7])));
            assert_eq!(event.stack.components[1].1, Some(Tag::Byte(7)));
            event.stack.count = -12;
            event.stack.components.extend([
                ("test:array".into(), None),
                ("test:array".into(), Some(Tag::List(vec![Tag::Int(7)]))),
                ("test:array".into(), Some(Tag::ByteArray(vec![8]))),
            ]);
        }
    }
    #[test]
    fn loot_hook_preserves_signed_candidates_and_ordered_nbt() {
        super::super::prepare_stack_boundary_tests();
        let mut output = stack();
        with_mods(vec![ModEntry::new("edit", Box::new(Edit))].into(), || {
            dispatch_loot_function(
                "test:edit",
                &serde_json::Value::Null,
                &context(),
                &mut output,
                &TestTags,
            );
        });
        assert_eq!(output.count, -12);
        assert_eq!(output.damage, -3);
        assert!(output.map);
        assert_eq!(
            output.components,
            vec![
                ("test:array".into(), Some(Tag::ByteArray(vec![7]))),
                ("test:byte".into(), Some(Tag::Byte(7))),
                ("test:array".into(), None),
                ("test:array".into(), Some(Tag::List(vec![Tag::Int(7)]))),
                ("test:array".into(), Some(Tag::ByteArray(vec![8]))),
            ]
        );
    }
    #[test]
    fn loot_no_subscriber_does_not_convert() {
        super::super::prepare_stack_boundary_tests();
        with_mods(Vec::new().into(), || {
            CONVERSIONS.with(|count| count.set(0));
            let mut output = stack();
            let before = output.clone();
            let mut holds = false;
            for _ in 0..1000 {
                dispatch_loot_function(
                    "test:edit",
                    &serde_json::Value::Null,
                    &context(),
                    &mut output,
                    &TestTags,
                );
                dispatch_loot_condition(
                    "test:edit",
                    &serde_json::Value::Null,
                    &context(),
                    &mut holds,
                    &TestTags,
                );
            }
            assert_eq!(CONVERSIONS.with(|count| count.get()), 0);
            assert_eq!(output, before);
            assert!(!holds);
        });
    }
    #[test]
    fn loot_patch_depth_matches_the_vanilla_nbt_limit() {
        super::super::prepare_stack_boundary_tests();
        let mut value = stack().try_into_api().unwrap();
        let mut tag = Tag::Byte(7);
        for _ in 0..512 {
            tag = Tag::List(vec![tag]);
        }
        value.components = vec![("test:deep".into(), Some(tag))];
        assert!(value.try_into_engine(&()).is_ok());
        let tag = value.components[0].1.take().unwrap();
        value.components[0].1 = Some(Tag::List(vec![tag]));
        assert_eq!(
            value.try_into_engine(&()),
            Err(LootValueError::InvalidPatch)
        );
    }

    #[test]
    fn loot_writeback_keeps_repeated_patch_order() {
        super::super::prepare_stack_boundary_tests();
        let mut value =
            api::LootStack::new(mod_api::__private::item_key("minecraft:stick").unwrap(), -3);
        value.components = vec![
            ("test:value".into(), Some(Tag::Byte(7))),
            ("test:value".into(), Some(Tag::ByteArray(vec![8]))),
        ];
        let output = value.try_into_engine(&()).unwrap();
        assert_eq!(output.count, -3);
        assert_eq!(
            output.components,
            vec![
                ("test:value".into(), Some(Tag::Byte(7))),
                ("test:value".into(), Some(Tag::ByteArray(vec![8]))),
            ]
        );
    }
    #[test]
    fn loot_writeback_keeps_component_removals() {
        super::super::prepare_stack_boundary_tests();
        let mut value =
            api::LootStack::new(mod_api::__private::item_key("minecraft:stick").unwrap(), 1);
        value.components = vec![("test:removed".into(), None)];
        assert_eq!(
            value.try_into_engine(&()).unwrap().components,
            vec![("test:removed".into(), None)]
        );
    }

    struct Invalid;
    #[mod_api::__subscriptions]
    impl Hooks for Invalid {
        fn loot_function(&self, event: &mut LootFunctionEvent<'_>) {
            event.stack.count = 99;
            event.stack.components.clear();
            let mut text = mod_api::text::Text::literal("end");
            for _ in 0..513 {
                let mut parent = mod_api::text::Text::literal("");
                parent.children.push(text);
                text = parent;
            }
            event
                .stack
                .names
                .push(("minecraft:custom_name".into(), text));
        }
    }
    #[test]
    fn loot_invalid_writeback_is_atomic() {
        super::super::prepare_stack_boundary_tests();
        let mut output = stack();
        let before = output.clone();
        with_mods(
            vec![ModEntry::new("invalid", Box::new(Invalid))].into(),
            || {
                dispatch_loot_function(
                    "test:invalid",
                    &serde_json::Value::Null,
                    &context(),
                    &mut output,
                    &TestTags,
                );
            },
        );
        assert_eq!(output, before);
    }
}

#[cfg(test)]
#[path = "loot_query_tests.rs"]
mod query_tests;
