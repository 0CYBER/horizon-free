//! The cart's `Entity.move` — `AbstractMinecart.move`'s own tick move through
//! the move gate (arismoko/horizon#2414) — in its own module so `minecarts.rs` keeps its size.
use super::Game;
use super::minecarts::{CartKind, Minecart};
use horizon_client::collisions::SceneCollisions;
use horizon_core::geometry::Vec3;
impl Game<'_> {
    /// Entity.move for a cart: collisions zero the blocked components, the world border's walls
    /// among them (`CollisionGetter.borderCollision`, asked for every entity).
    pub(super) fn move_cart(&self, c: &mut Minecart, delta: Vec3) {
        // The cart's own move is a self move (arismoko/horizon#2414: vanilla's
        // `AbstractMinecart.move` passes SELF for its own tick); PR 1a
        // records the movement unchanged.
        let mut gate = horizon_client::move_gate::armed(
            &self.loop_context,
            c.id,
            c.kind.id(),
            horizon_world::entity_move::MoverType::SelfMovement,
            delta,
        );
        let mut delta = gate.delta();
        // Entity.move: a stuck multiplier scales this move and zeroes the velocity.
        if c.stuck.length_squared() > 1.0e-7 {
            delta = delta.multiply(c.stuck);
            c.stuck = Vec3::ZERO;
            c.velocity = Vec3::ZERO;
        }
        let scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        // A zero move stays zero whatever surrounds it (`Entity.collide`, and the
        // step-up needs a blocked horizontal move): a cart standing on its rails,
        // as most do, gathers no entity boxes.
        let entities = if delta.length_squared() == 0.0 {
            Vec::new()
        } else {
            self.cart_entity_collisions(c)
        };
        let mut scene = horizon_client::collisions::EntityCollisions {
            blocks: scene,
            entities: &entities,
            border: Some((self.border.geometry(), [c.position.x, c.position.z])),
        };
        if let Ok(moved) = horizon_client::move_gate::entities_resolve(
            &mut gate,
            &mut scene,
            c.bounds(),
            delta,
            0.0,
            c.on_ground,
        ) {
            c.fall_distance = self.vehicle_fall_reset(c.fall_distance, c.position, moved);
            c.position = Vec3::new(
                c.position.x + moved.x,
                c.position.y + moved.y,
                c.position.z + moved.z,
            );
            c.on_ground = delta.y < 0.0 && moved.y > delta.y;
            // Entity.restituteMovementAfterCollisions: Mth.equal uses a
            // 1e-5 tolerance on x/z (exact on y), so a tiny clip is not a
            // collision. TNT carts use horizontalCollision to detonate.
            c.horizontal_collision =
                (moved.x - delta.x).abs() >= 1.0e-5 || (moved.z - delta.z).abs() >= 1.0e-5;
            if (moved.x - delta.x).abs() >= 1.0e-5 {
                c.velocity.x = 0.0;
            }
            if (moved.z - delta.z).abs() >= 1.0e-5 {
                c.velocity.z = 0.0;
            }
            if moved.y != delta.y {
                c.velocity.y = 0.0;
            }
            // Entity.checkFallDamage: the move's drop counts (not in water, as of
            // the tick before: `isInWater` is `wasTouchingWater`),
            // and a landing hands the whole fall to causeFallDamage, which
            // sets a TNT minecart off at (fall / 10) squared.
            if moved.y < 0.0 && !c.in_water {
                c.fall_distance -= f64::from(moved.y as f32);
            }
            // A move up or none leaves the fall as it is (a saved one too).
            if c.on_ground {
                if c.kind == CartKind::Tnt && c.fall_distance >= 3.0 {
                    let power = c.fall_distance / 10.0;
                    c.fall_blast = Some(power * power);
                }
                c.fall_distance = 0.0;
            }
        }
        // OldMinecartBehavior applies block effects here and at tick end.
        // NewMinecartBehavior applies them after recording each track step.
        if !self.minecart_improvements() {
            self.cart_block_effects(c);
        }
    }
}
