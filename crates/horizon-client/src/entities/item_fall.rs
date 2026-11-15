//! A dropped item's construction and its fall distance (`Entity.fallDistance`).
use super::{ItemEntity, ItemStep};
use crate::items::ItemStack;
use horizon_core::geometry::Vec3;

impl ItemEntity {
    /// A fresh `ItemEntity` at `position` moving at `velocity`: full health,
    /// no age, delay or owner, not yet ticked.
    pub fn new(id: u32, stack: ItemStack, position: Vec3, velocity: Vec3, bob_offset: f32) -> Self {
        Self {
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            health: 5,
            fire_ticks: 0,
            thrower: None,
            target: None,
            id,
            stack,
            position,
            previous: position,
            velocity,
            on_ground: false,
            vertical_collision: false,
            horizontal_collision: false,
            fall_distance: 0.0,
            age: 0,
            pickup_delay: 0,
            bob_offset,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
            // The caller gives it the own random that drew its UUID and bobOffs.
            random: horizon_core::random::EntityRandom::own(
                horizon_core::random::LegacyRandom::new(0),
            ),
        }
    }
    /// `Entity.move`'s fall bookkeeping after the item's move: `reset_clip`
    /// is the cast of a move of a block or more over fall-resetting blocks
    /// and water (the fall it leaves), then `checkFallDamage` adds the drop,
    /// narrowed to a float, outside water and ends the fall on the ground.
    /// Returns the fall it landed with, if it landed after one: the block's
    /// `fallOn` is the caller's.
    pub fn check_fall(
        &mut self,
        step: &ItemStep,
        reset_clip: impl FnOnce(f64, Vec3, Vec3) -> f64,
    ) -> Option<f64> {
        let (from, moved, applied) = step.fall_move?;
        if applied {
            self.fall_distance = reset_clip(self.fall_distance, from, moved);
        }
        if !self.was_touching_water && moved.y < 0.0 {
            self.fall_distance -= f64::from(moved.y as f32);
        }
        let landed = (self.on_ground && self.fall_distance > 0.0).then_some(self.fall_distance);
        if self.on_ground {
            self.fall_distance = 0.0;
        }
        landed
    }
    /// `Entity.baseTick`: lava halves the fall each tick.
    pub fn lava_fall(&mut self, in_lava: bool) {
        if in_lava {
            self.fall_distance *= 0.5;
        }
    }
}
