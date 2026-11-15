//! `new ItemEntity(..)` and `ServerLevel.addFreshEntity` of a dropped item, kept
//! apart so `item_toss` can hear the item between the two.
use super::{Game, spill};
use horizon_client::{entities::ItemEntity, items::ItemStack};
use horizon_core::geometry::Vec3;

impl Game<'_> {
    /// `new ItemEntity(..)`: the item at the end of `dropped`, not yet joined
    /// (`join_item_entity`). Returns its motion.
    pub(in crate::game) fn push_item_entity(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: Option<[f64; 3]>,
        pickup_delay: u32,
        random: Option<spill::ItemRandom>,
    ) -> [f64; 3] {
        let random = random.unwrap_or_else(|| spill::prepare_item_random(velocity));
        self.next_entity += 1;
        let p = Vec3::new(position[0], position[1], position[2]);
        self.persist.uuids.insert(self.next_entity, random.uuid);
        let (bob, yaw) = (random.bob, random.yaw);
        let velocity = velocity.unwrap_or(random.velocity);
        self.dropped.push(random.state.apply(ItemEntity {
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [yaw, 0.0],
            pickup_delay: i64::from(pickup_delay),
            random: random.own.into(),
            ..ItemEntity::new(
                self.next_entity,
                stack,
                p,
                Vec3::new(velocity[0], velocity[1], velocity[2]),
                bob,
            )
        }));
        velocity
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
