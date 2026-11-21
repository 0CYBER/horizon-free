//! The player's collision exclusions (arismoko/horizon#3717) in its collision checks: `Entity.collide` reads them
//! in `MotionWorld::collide`, `Level.noCollision` here.
use super::*;
use horizon_world::world_movement::CollisionCheckRequest;

impl MotionWorld<'_, '_, '_> {
    /// `CollisionGetter.noCollision(player, bounds)`: blocks, less the player's collision exclusions, and
    /// the host's entity boxes.
    pub fn no_player_collision(&mut self, bounds: Aabb) -> Result<bool> {
        if self.entities.is_empty()
            && self.player.collision_exclusions.is_empty()
            && self.player.border.as_ref().is_none_or(|b| {
                !b.is_inside_close_to_border(self.player.position.x, self.player.position.z, bounds)
            })
            && self.collisions().cells_without_collision(bounds)
        {
            return Ok(true);
        }
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

    #[test]
    #[ignore = "daily"]
    fn no_player_collision_fast_path_matches_full() {
        let Some(fixture) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, render, binding, resources) = (
            &fixture.assets,
            &fixture.render,
            &fixture.collision,
            &fixture.motion,
        );
        let mut scene = World::test_scene(assets, render, 2).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        scene.set(22, 64, 20, stone).unwrap();
        let at = Vec3::new(20.5, 64.0, 20.5);
        let mut player = PlayerMotion::new(resources, at).unwrap();
        let bounds = player.bounds();

        let full_check = |p: &mut PlayerMotion, s: &mut World, ents: &[Aabb], b: Aabb| -> bool {
            let w = MotionWorld {
                tag_items: None,
                block_registries: None,
                loop_context: crate::test_resources::server_loop_context(),
                damage_hooks: &crate::health::DamageHooks::NONE,
                entities: ents,
                player: p,
                scene: s,
                assets,
                collision_binding: binding,
                resources,
                health: &resources.health,
            };
            let ctx = w.context();
            collision_exclusions::no_world_collision(
                &mut w.movement_collisions(),
                &w.player.collision_exclusions,
                CollisionCheckRequest {
                    entity: EntityIdentity(0),
                    context: ctx,
                    bounds: b,
                    candidate_budget: 1_000_000,
                },
            )
            .unwrap()
        };
        let fast_check = |p: &mut PlayerMotion, s: &mut World, ents: &[Aabb], b: Aabb| -> bool {
            MotionWorld {
                tag_items: None,
                block_registries: None,
                loop_context: crate::test_resources::server_loop_context(),
                damage_hooks: &crate::health::DamageHooks::NONE,
                entities: ents,
                player: p,
                scene: s,
                assets,
                collision_binding: binding,
                resources,
                health: &resources.health,
            }
            .no_player_collision(b)
            .unwrap()
        };

        // 1. Clear air fast path.
        assert!(fast_check(&mut player, &mut scene, &[], bounds));
        assert!(full_check(&mut player, &mut scene, &[], bounds));

        // 2. Entity box forces fallback and reports collision.
        let blocking_entity = [bounds];
        assert!(!fast_check(
            &mut player,
            &mut scene,
            &blocking_entity,
            bounds
        ));
        assert!(!full_check(
            &mut player,
            &mut scene,
            &blocking_entity,
            bounds
        ));

        // 3. Collision exclusion over stone forces fallback and clears collision.
        let stone_bounds = Aabb::new(Vec3::new(22.2, 64.0, 20.2), Vec3::new(22.8, 65.8, 20.8));
        assert!(!fast_check(&mut player, &mut scene, &[], stone_bounds));
        player
            .collision_exclusions
            .set(&[[22.0, 64.0, 20.0, 23.0, 66.0, 21.0]])
            .unwrap();
        assert!(fast_check(&mut player, &mut scene, &[], stone_bounds));
        assert!(full_check(&mut player, &mut scene, &[], stone_bounds));
        player.collision_exclusions = Default::default();

        // 4. Nearby world border forces fallback and blocks bounds crossing it.
        player.border = Some(
            horizon_core::geometry::border::BorderGeometry::new([0.0, 0.0], 40.0, 100).unwrap(),
        );
        let border_bounds = Aabb::new(Vec3::new(19.8, 64.0, 20.2), Vec3::new(20.4, 65.8, 20.8));
        assert!(!fast_check(&mut player, &mut scene, &[], border_bounds));
        assert!(!full_check(&mut player, &mut scene, &[], border_bounds));
    }
}
