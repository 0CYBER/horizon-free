//! Dropped-item handles for native server mods.
use super::{Game, commands::Target};
use horizon_client::{
    items::ItemStack,
    lan::playerdata::{ints_uuid, uuid_ints},
};
use mod_api::{EntityRef, ItemEntityRef};

impl Game<'_> {
    fn native_item_entity_index(&self, item: ItemEntityRef) -> Option<usize> {
        let Target::Item(id) = self.native_entity_target(item.entity())? else {
            return None;
        };
        self.dropped.iter().position(|entity| entity.id == id)
    }

    /// Narrows an entity to a dropped item, or None for another kind, a
    /// stale handle, a client, or an item temporarily taken out for ticking.
    pub(super) fn item_entity(&self, entity: EntityRef) -> Option<ItemEntityRef> {
        let item = ItemEntityRef::new(entity);
        self.native_item_entity_index(item).map(|_| item)
    }

    /// `ItemEntity.getItem`: the complete stack, or None if unavailable.
    pub(super) fn item_entity_stack(&self, item: ItemEntityRef) -> Option<ItemStack> {
        if self.block_entity_tick_items.is_some() {
            return self.scoped_item_entity_stack(item);
        }
        self.unscoped_item_entity_stack(item)
    }
    /// Writes a nonempty complete stack using `ItemEntity.setItem`. Empty
    /// stacks are refused: vanilla discards the item on its next tick. To
    /// remove an item entity, use the removal handle. False also for an
    /// unavailable item or outside a server.
    pub(super) fn item_entity_set_stack(&mut self, item: ItemEntityRef, stack: ItemStack) -> bool {
        if stack.count == 0
            || &*stack.item == "minecraft:air"
            || self.native_item_entity_index(item).is_none()
        {
            return false;
        }
        self.native_set_item_entity(self.dim.id(), item.entity().id(), stack)
    }

    /// The signed pickup delay; the legacy infinite sentinel reads as vanilla's
    /// 32767. None also covers a stored value outside i32.
    pub(super) fn item_entity_pickup_delay(&self, item: ItemEntityRef) -> Option<i32> {
        let delay = self
            .dropped
            .get(self.native_item_entity_index(item)?)?
            .pickup_delay;
        if delay == i64::from(u32::MAX) {
            Some(i32::from(i16::MAX))
        } else {
            i32::try_from(delay).ok()
        }
    }

    /// `ItemEntity.setPickUpDelay`.
    pub(super) fn item_entity_set_pickup_delay(&mut self, item: ItemEntityRef, ticks: i32) -> bool {
        let Some(index) = self.native_item_entity_index(item) else {
            return false;
        };
        self.dropped[index].pickup_delay = i64::from(ticks);
        true
    }

    /// `ItemEntity.setDefaultPickUpDelay`: ten ticks.
    pub(super) fn item_entity_set_default_pickup_delay(&mut self, item: ItemEntityRef) -> bool {
        self.item_entity_set_pickup_delay(item, 10)
    }

    /// `ItemEntity.setNoPickUpDelay`.
    pub(super) fn item_entity_set_no_pickup_delay(&mut self, item: ItemEntityRef) -> bool {
        self.item_entity_set_pickup_delay(item, 0)
    }

    /// `ItemEntity.setNeverPickUp`: the infinite-delay sentinel 32767.
    pub(super) fn item_entity_set_never_pickup(&mut self, item: ItemEntityRef) -> bool {
        self.item_entity_set_pickup_delay(item, i32::from(i16::MAX))
    }

    /// `ItemEntity.getAge`, including negative lifetime sentinels.
    pub(super) fn item_entity_age(&self, item: ItemEntityRef) -> Option<i32> {
        Some(self.dropped.get(self.native_item_entity_index(item)?)?.age)
    }

    /// The UUID set by `ItemEntity.setTarget`, limiting pickup to that player;
    /// None also means an unavailable item. This is not the thrower.
    pub(super) fn item_entity_target(&self, item: ItemEntityRef) -> Option<u128> {
        self.dropped
            .get(self.native_item_entity_index(item)?)?
            .target
            .map(ints_uuid)
    }

    /// `ItemEntity.setTarget`; None allows any player to pick it up.
    pub(super) fn item_entity_set_target(
        &mut self,
        item: ItemEntityRef,
        target: Option<u128>,
    ) -> bool {
        let Some(index) = self.native_item_entity_index(item) else {
            return false;
        };
        self.dropped[index].target = target.map(uuid_ints);
        true
    }

    /// `ItemEntity.getOwner`: the thrower as a resolved entity, not the pickup
    /// target. None for no thrower, an unloaded or different-level thrower,
    /// an unavailable handle, or one Horizon cannot resolve.
    pub(super) fn item_entity_owner(&self, item: ItemEntityRef) -> Option<EntityRef> {
        let uuid = self
            .dropped
            .get(self.native_item_entity_index(item)?)?
            .thrower?;
        let target = self.target_by_uuid(ints_uuid(uuid))?;
        if self.target_level(target) != self.dim {
            return None;
        }
        let id = u32::try_from(self.network_entity_id(target)?).ok()?;
        self.entity(id)
    }
}

include!("native_item_entity_refs/tick_item_read.rs");

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use crate::game::api_values::test_snapshot;
    use horizon_client::{entities::ArmorStand, mobs::Mob};
    use horizon_core::geometry::Vec3;
    use mod_api::WorldEpoch;

    #[test]
    #[ignore = "daily"]
    fn item_handles_preserve_stacks_and_distinguish_target_from_thrower() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let mut stack = ItemStack {
                item: "minecraft:diamond_pickaxe".into(),
                count: 1,
                damage: 17,
                extra: None,
            };
            stack.data_mut().custom_name =
                Some(horizon_core::nbt::Tag::String("Named pick".into()));
            game.spawn_item(stack.clone(), [0.0; 3], [0.0; 3], 0);
            let entity = game.entity(game.dropped[0].id).unwrap();
            let item = game.item_entity(entity).unwrap();
            assert_eq!(item.entity(), entity);
            let read = item.stack(game).unwrap();
            assert_eq!(
                (read.item().id(game).as_str(), read.count(), read.damage()),
                ("minecraft:diamond_pickaxe", 1, 17)
            );
            let mut changed = stack;
            changed.damage = 21;
            let changed = test_snapshot(&changed);
            assert_eq!(item.set_stack(game, changed.clone()), Ok(true));
            assert_eq!(item.stack(game), Some(changed));
            assert!(item.set_pickup_delay(game, -7));
            assert_eq!(item.pickup_delay(game), Some(-7));
            assert!(item.set_default_pickup_delay(game));
            assert_eq!(item.pickup_delay(game), Some(10));
            game.dropped[0].pickup_delay = i64::from(u32::MAX);
            assert_eq!(item.pickup_delay(game), Some(32767));
            assert!(item.set_never_pickup(game));
            assert_eq!(game.dropped[0].pickup_delay, 32767);
            assert_eq!(item.pickup_delay(game), Some(32767));
            assert!(item.set_no_pickup_delay(game));
            assert_eq!(item.pickup_delay(game), Some(0));
            game.dropped[0].age = -32768;
            assert_eq!(item.age(game), Some(-32768));
            let target = 0x12345678_9abcdef0_10203040_50607080_u128;
            assert!(item.set_target(game, Some(target)));
            assert_eq!(item.target(game), Some(target));
            assert_eq!(item.owner(game), None);
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(99, kind, Vec3::ZERO, 0.0));
            let thrower = game.entity(99).unwrap();
            game.dropped[0].thrower = Some(uuid_ints(game.entity_uuid(thrower).unwrap()));
            assert_eq!(item.owner(game), Some(thrower));
            assert_eq!(item.target(game), Some(target));
            assert!(item.set_target(game, None));
            assert_eq!(item.target(game), None);
            game.armor_stands
                .push(ArmorStand::new(100, Vec3::ZERO, 0.0));
            assert!(game.item_entity(game.entity(100).unwrap()).is_none());
            let stale = ItemEntityRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                entity.id(),
            ));
            assert_eq!(stale.age(game), None);
            assert!(!stale.set_target(game, Some(target)));
            let mut empty = item.stack(game).unwrap();
            assert_eq!(empty.set_count(0), Err(mod_api::StackError::Empty));
            let air = test_snapshot(&ItemStack::new("minecraft:air", 1));
            assert_eq!(item.set_stack(game, air), Ok(false));
            assert_eq!(item.stack(game).unwrap().damage(), 21);
            let taken = game.dropped.lift();
            assert!(item.stack(game).is_none());
            assert!(!item.set_no_pickup_delay(game));
            game.dropped = taken;
        });
    }
}
