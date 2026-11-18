//! Owned loot values at the guarded game boundary. No inventory count rules apply.
use super::{TryIntoApi, TryIntoEngine};
use horizon_world::loot as engine;
use mod_api::world::snapshots as api;

#[cfg(any(test, feature = "test-support"))]
thread_local! { pub static CONVERSIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LootValueError {
    UnknownItem,
    InvalidName,
    InvalidPatch,
}

impl TryIntoApi for engine::LootStack {
    type Api = api::LootStack;
    type Error = LootValueError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        #[cfg(any(test, feature = "test-support"))]
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
        #[cfg(any(test, feature = "test-support"))]
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

pub fn queried_stack(stack: &engine::LootStack) -> Result<mod_api::ItemStack, LootValueError> {
    if stack.count <= 0 || stack.damage < 0 {
        return Err(LootValueError::InvalidPatch);
    }
    horizon_items::items::ItemStack::from_loot(stack)
        .try_into_api()
        .map_err(|_| LootValueError::InvalidPatch)
}
