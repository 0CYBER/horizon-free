//! The player's collision exclusions (arismoko/horizon#3717) in its collision checks: `Entity.collide` reads them
//! in `MotionWorld::collide`, `Level.noCollision` here.
use super::*;
use horizon_world::world_movement::CollisionCheckRequest;

impl MotionWorld<'_, '_, '_> {
    /// `CollisionGetter.noCollision(player, bounds)`: blocks, less the player's collision exclusions, and
    /// the host's entity boxes.
    pub fn no_player_collision(&mut self, bounds: Aabb) -> Result<bool> {
        let context = self.context();
        Ok(collision_exclusions::no_world_collision(
            &mut self.movement_collisions(),
            &self.player.collision_exclusions,
            CollisionCheckRequest {
                entity: EntityIdentity(0),
                context,
                bounds,
                candidate_budget: 1_000_000,
            },
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server's player lands after a 10-block fall in one of two cobwebs side by side, at
    /// x 20 and 21, with a mod's box over the first. The web at 21 still catches it
    /// (`makeStuckInBlock` forgets the fall: no damage), the excluded one at 20 doesn't, so
    /// the fall hurts for floor(10 + 1.5 + 1e-6 - 3) = 8.
    #[test]
    #[ignore = "daily"]
    fn an_excluded_block_has_no_inside_effect_and_its_neighbour_still_does() {
        let Some(fixture) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, resources) = (&fixture.assets, &fixture.motion);
        let mut scene = World::test_scene(assets, &fixture.render, 1).unwrap();
        let web = assets.registry.default_state("minecraft:cobweb").unwrap();
        scene.set(20, 4, 20, web).unwrap();
        scene.set(21, 4, 20, web).unwrap();
        let land = |scene: &mut World, x: f64| {
            let at = Vec3::new(x, 4.0, 20.5);
            let mut player = PlayerMotion::new(resources, at).unwrap();
            player
                .collision_exclusions
                .set(&[[20.0, 4.0, 20.0, 21.0, 5.0, 21.0]])
                .unwrap();
            player.fall_distance = 10.0;
            let from = at.add_vector(Vec3::new(0.0, 1.5, 0.0));
            let delta = at.subtract(from);
            player
                .history
                .add(horizon_world::entity_move::MovementRecord {
                    from,
                    to: at,
                    original: Some(delta),
                });
            let mut w = MotionWorld {
                tag_items: None,
                block_registries: None,
                loop_context: crate::test_resources::server_loop_context(),
                damage_hooks: &crate::health::DamageHooks::NONE,
                entities: &[],
                player: &mut player,
                scene,
                assets,
                collision_binding: &fixture.collision,
                resources,
                health: &resources.health,
            };
            w.server_stuck_in_blocks(from).unwrap();
            let pos = BlockPos::new(x.floor() as i32, 3, 20);
            let state = w.block(pos);
            w.check_fall_damage(delta.y, true, state, pos).unwrap();
            player.health.health()
        };
        assert_eq!(land(&mut scene, 21.5), 20.0, "the web outside the box");
        assert_eq!(land(&mut scene, 20.5), 12.0, "the excluded web");
    }
}
