//! Non-player entities simulated on the client world. Dropped items follow
//! `ItemEntity.tick`: gravity 0.04, 0.98 drag (block friction on the ground),
//! a ground bounce of -0.5, water buoyancy, pickup delay and 6000-tick despawn.
use crate::{
    blocks::{BlockFacts, Fluid},
    collisions::SceneCollisions,
    items::ItemStack,
    world::World,
};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::block_collision::BlockCollisionRegistry;

#[derive(Clone, Debug)]
pub struct ItemEntity {
    pub id: u32,
    pub stack: ItemStack,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    pub age: u32,
    pub pickup_delay: u32,
    pub bob_offset: f32,
}
impl ItemEntity {
    pub const WIDTH: f64 = 0.25;
    pub fn bounds(&self) -> Aabb {
        let h = Self::WIDTH / 2.0;
        Aabb::new(
            Vec3::new(self.position.x - h, self.position.y, self.position.z - h),
            Vec3::new(self.position.x + h, self.position.y + Self::WIDTH, self.position.z + h),
        )
    }
    /// Returns false when the item should be removed.
    pub fn tick(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        collision: &BlockCollisionRegistry<'_>,
    ) -> bool {
        self.previous = self.position;
        if self.pickup_delay > 0 && self.pickup_delay != u32::MAX {
            self.pickup_delay -= 1;
        }
        let eye = world.block(
            self.position.x.floor() as i32,
            (self.position.y + 0.1).floor() as i32,
            self.position.z.floor() as i32,
        );
        let in_water = facts.get(eye).fluid == Fluid::Water;
        if in_water {
            // ItemEntity.setUnderwaterMovement
            self.velocity = Vec3::new(
                self.velocity.x * 0.99,
                self.velocity.y + if self.velocity.y < 0.06 { 5.0e-4 } else { 0.0 },
                self.velocity.z * 0.99,
            );
        } else {
            self.velocity.y -= 0.04;
        }
        let horizontal = self.velocity.x * self.velocity.x + self.velocity.z * self.velocity.z;
        if !self.on_ground || horizontal > 1.0e-5 || (self.age + self.id) % 4 == 0 {
            let mut scene = SceneCollisions { world, binding: collision };
            let wanted = self.velocity;
            let moved = scene
                .resolve(self.bounds(), wanted, 0.0, self.on_ground)
                .unwrap_or(Vec3::ZERO);
            self.position = Vec3::new(
                self.position.x + moved.x,
                self.position.y + moved.y,
                self.position.z + moved.z,
            );
            self.on_ground = wanted.y < 0.0 && moved.y != wanted.y;
            if moved.x != wanted.x {
                self.velocity.x = 0.0;
            }
            if moved.z != wanted.z {
                self.velocity.z = 0.0;
            }
            if moved.y != wanted.y {
                self.velocity.y = 0.0;
            }
            let mut friction = 0.98;
            if self.on_ground {
                let below = world.block(
                    self.position.x.floor() as i32,
                    (self.position.y - 0.999_999).floor() as i32,
                    self.position.z.floor() as i32,
                );
                friction = f64::from(facts.get(below).friction) * 0.98;
            }
            self.velocity = Vec3::new(self.velocity.x * friction, self.velocity.y * 0.98, self.velocity.z * friction);
            if self.on_ground && self.velocity.y < 0.0 {
                self.velocity.y *= -0.5;
            }
        }
        self.age += 1;
        self.age < 6000 && self.position.y > -128.0
    }
}
