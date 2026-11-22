//! `new ItemEntity(..)` and `ServerLevel.addFreshEntity` of a dropped item, kept
//! apart so `item_toss` can hear the item between the two.
use super::{Game, spill};
use horizon_client::{entities::ItemEntity, items::ItemStack};
use horizon_core::geometry::Vec3;

impl Game<'_> {
    /// Pushes an `ItemEntity` into `dropped` unless `stack` is empty or air,
    /// returning its allocated entity id and motion.
    pub(in crate::game) fn try_push_item_entity(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: Option<[f64; 3]>,
        pickup_delay: u32,
        random: Option<spill::ItemRandom>,
    ) -> Option<(u32, [f64; 3])> {
        push_item_entity_into(
            &mut self.dropped,
            &mut self.persist.uuids,
            &mut self.next_entity,
            stack,
            position,
            velocity,
            pickup_delay,
            random,
        )
    }

    /// `new ItemEntity(..)`: the item at the end of `dropped`, not yet joined
    /// (`join_item_entity`). Returns its motion. Kept as it was, a path a native
    /// mod can declare; `try_push_item_entity` is the one that refuses an empty
    /// or air stack and returns the entity id. An empty or air stack adds
    /// nothing here and gives back no motion.
    #[allow(dead_code)]
    pub(in crate::game) fn push_item_entity(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: Option<[f64; 3]>,
        pickup_delay: u32,
        random: Option<spill::ItemRandom>,
    ) -> [f64; 3] {
        self.try_push_item_entity(stack, position, velocity, pickup_delay, random)
            .map_or([0.0; 3], |(_, motion)| motion)
    }

    /// `ServerLevel.addFreshEntity` of the item `id` that `push_item_entity`
    /// made, with the stack it holds now.
    pub(in crate::game) fn join_item_entity(&mut self, id: u32, position: [f64; 3]) {
        let Some((item, count)) = self
            .dropped
            .iter()
            .rev()
            .find(|d| d.id == id)
            .map(|d| (d.stack.item.clone(), d.stack.count as i32))
        else {
            return;
        };
        if self.remote.is_none() && !mod_api::__private::mods().is_empty() {
            self.native_lifecycle
                .pending_item_spawns
                .push((id, item.to_string(), count, position));
            self.native_joins_flush(false, id);
        } else if self.native_joins_flush(false, id) {
            self.native_item_spawn(id, &item, count, position);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_item_entity_into(
    dropped: &mut horizon_client::join_seq::JoinList<ItemEntity>,
    uuids: &mut super::persist::EntityUuids,
    next_entity: &mut u32,
    stack: ItemStack,
    position: [f64; 3],
    velocity: Option<[f64; 3]>,
    pickup_delay: u32,
    random: Option<spill::ItemRandom>,
) -> Option<(u32, [f64; 3])> {
    if stack.count == 0 || &*stack.item == "minecraft:air" {
        return None;
    }
    let random = random.unwrap_or_else(|| spill::prepare_item_random(velocity));
    *next_entity += 1;
    let id = *next_entity;
    let p = Vec3::new(position[0], position[1], position[2]);
    uuids.insert(id, random.uuid);
    let (bob, yaw) = (random.bob, random.yaw);
    let velocity = velocity.unwrap_or(random.velocity);
    dropped.push(random.state.apply(ItemEntity {
        attachment_lifetime: Default::default(),
        fall_distance: 0.0,
        rotation: [yaw, 0.0],
        pickup_delay: i64::from(pickup_delay),
        random: horizon_core::random::EntityRandom::own(random.own.into()),
        ..ItemEntity::new(
            id,
            stack,
            p,
            Vec3::new(velocity[0], velocity[1], velocity[2]),
            bob,
        )
    }));
    Some((id, velocity))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_item_entity_rejects_empty_and_air_stacks() {
        for stack in [
            ItemStack::new("minecraft:air", 1),
            ItemStack::new("minecraft:stone", 0),
        ] {
            let mut dropped = horizon_client::join_seq::JoinList::default();
            let mut uuids = super::super::persist::EntityUuids::default();
            let mut next_entity = 10;
            let result = push_item_entity_into(
                &mut dropped,
                &mut uuids,
                &mut next_entity,
                stack,
                [0.5, 64.0, 0.5],
                Some([0.1, 0.2, 0.3]),
                10,
                None,
            );
            assert_eq!(result, None);
            assert!(dropped.is_empty());
            assert!(uuids.is_empty());
            assert_eq!(next_entity, 10);
        }
    }

    #[test]
    fn rejected_spawn_leaves_previous_drop_untouched() {
        let mut dropped = horizon_client::join_seq::JoinList::default();
        let mut uuids = super::super::persist::EntityUuids::default();
        let mut next_entity = 10;
        let (existing_id, _) = push_item_entity_into(
            &mut dropped,
            &mut uuids,
            &mut next_entity,
            ItemStack::new("minecraft:stone", 4),
            [0.5, 64.0, 0.5],
            Some([0.0, 0.2, 0.0]),
            40,
            None,
        )
        .expect("valid stack spawns");
        assert_eq!(existing_id, 11);
        let original_thrower = Some([1, 2, 3, 4]);
        let original_target = Some([5, 6, 7, 8]);
        {
            let existing = dropped
                .iter_mut()
                .rev()
                .find(|d| d.id == existing_id)
                .unwrap();
            existing.thrower = original_thrower;
            existing.target = original_target;
        }

        // Rejected air / empty spawns return None and must not yield the previous id.
        for bad in [
            ItemStack::new("minecraft:air", 1),
            ItemStack::new("minecraft:diamond", 0),
        ] {
            let spawned = push_item_entity_into(
                &mut dropped,
                &mut uuids,
                &mut next_entity,
                bad,
                [1.5, 65.0, 1.5],
                Some([0.1, 0.2, 0.3]),
                0,
                None,
            );
            assert_eq!(spawned, None);
            if let Some((id, _)) = spawned
                && let Some(item) = dropped.iter_mut().rev().find(|d| d.id == id)
            {
                item.thrower = None;
                item.pickup_delay = 0;
                item.target = None;
            }
        }

        assert_eq!(next_entity, 11);
        assert_eq!(dropped.len(), 1);
        let existing = dropped.iter().find(|d| d.id == existing_id).unwrap();
        assert_eq!(&*existing.stack.item, "minecraft:stone");
        assert_eq!(existing.stack.count, 4);
        assert_eq!(existing.pickup_delay, 40);
        assert_eq!(existing.thrower, original_thrower);
        assert_eq!(existing.target, original_target);
    }
}
