//! `ItemEntity`'s `Entity.move`: the tick's move step, its collision through
//! the move gate (arismoko/horizon#2414) and the block effects around it, in its own module so
//! `entities.rs` keeps its size.
use super::*;
impl ItemEntity {
    /// `ItemEntity.tick` up to the blocks' push. The game has just run the
    /// fluid push (`Entity::push_by_fluid`) and passes the heights it found;
    /// it pushes by a bubble column next (`Entity::push_by_bubble_column`) and
    /// then calls [`ItemEntity::tick_settle`]. `no_physics`: the item moves
    /// through blocks this tick ([`ItemEntity::inside_block`]).
    pub fn tick_move(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        collision: &BlockCollisionRegistry<'_>,
        no_physics: bool,
        heights: (f64, f64),
        entity_boxes: &[Aabb],
        border: Option<&BorderGeometry>,
        blocks: Option<&horizon_world::movement_properties::BoundMovementProperties>,
        context: &horizon_world::native_load_context::LoopContext,
    ) -> ItemStep {
        let step = self.tick_move_before_block_effects(
            world,
            facts,
            collision,
            no_physics,
            None,
            heights,
            entity_boxes,
            border,
            blocks,
            context,
        );
        self.finish_move_block_effects(world, facts, no_physics, blocks, step, context);
        step
    }

    /// `Entity.move`: resolve the item's move before the owner handles block
    /// contact and resumes block effects. `escape_speed`: with `no_physics`
    /// on a server, the speed of `Entity.moveTowardsClosestSpace`
    /// (`nextFloat() * 0.2 + 0.1`).
    pub fn tick_move_before_block_effects(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        collision: &BlockCollisionRegistry<'_>,
        no_physics: bool,
        escape_speed: Option<f32>,
        (water_height, lava_height): (f64, f64),
        entity_boxes: &[Aabb],
        border: Option<&BorderGeometry>,
        blocks: Option<&BoundMovementProperties>,
        context: &horizon_world::native_load_context::LoopContext,
    ) -> ItemStep {
        self.ticks += 1;
        self.previous = self.position;
        if self.pickup_delay > 0 && !self.has_infinite_pickup_delay() {
            self.pickup_delay -= 1;
        }
        let prior_fall = self.fall_distance;
        let old_velocity = self.velocity;
        // Entity.updateInWaterStateAndDoFluidPushing: touching water resets the fall.
        if water_height > 0.0 {
            self.fall_distance = 0.0;
        }
        let in_water = water_height > f64::from(0.1f32);
        if in_water {
            // ItemEntity.setUnderwaterMovement: 0.99F, 5.0E-4F, 0.06F.
            self.set_fluid_movement(0.99_f32);
        } else if lava_height > f64::from(0.1f32) {
            // ItemEntity.setUnderLavaMovement: 0.95F, 5.0E-4F, 0.06F.
            self.set_fluid_movement(0.95_f32);
        } else {
            self.velocity.y -= self.get_gravity();
        }
        // ItemEntity.tick: out of a block placed on it after gravity, so the
        // escape's 0.75 scales the fallen vertical speed (#12018).
        if let Some(speed) = escape_speed {
            self.move_towards_closest_space(world, facts, speed);
        }
        let horizontal = self.velocity.x * self.velocity.x + self.velocity.z * self.velocity.z;
        // ItemEntity.tick: skip move 3 of 4 ticks while grounded with no
        // horizontal speed. The idle branch still runs bubble-column push
        // (`applyEffectsFromBlocksForLastMovements`), so lift builds up and
        // the item moves on the next wake-up (arismoko/horizon#4558).
        if !self.on_ground
            || horizontal > 1.0e-5
            || self.ticks.wrapping_add(self.id).is_multiple_of(4)
        {
            let scene = SceneCollisions {
                world,
                binding: collision,
                subject: None,
            };
            let from = self.position;
            let wanted = self.velocity;
            // The item's Entity.move goes through the move gate (arismoko/horizon#2414): armed
            // before the move's own handling, no-physics and the stuck
            // multiplier included. PR 1a records the movement unchanged.
            let mut gate = crate::move_gate::armed(
                context,
                self.id,
                "minecraft:item",
                horizon_world::entity_move::MoverType::SelfMovement,
                wanted,
            );
            let mut wanted = gate.delta();
            // Entity.move with noPhysics: straight through, the ground as it
            // was — and vanilla returns before the stuck multiplier, which
            // stays pending for the next move with physics.
            let moved = if no_physics {
                wanted
            } else {
                // Entity.move: a block the item sits in (cobweb, powder snow)
                // scales this move once and stops it.
                if self.stuck.length_squared() > 1.0e-7 {
                    wanted = wanted.multiply(self.stuck);
                    self.stuck = Vec3::ZERO;
                    self.velocity = Vec3::ZERO;
                }
                // `Entity.collide`: the entities that `canBeCollidedWith` the item stop it
                // too (a boat, a shulker, a happy ghast held still; not a minecart), and
                // so does the world border (`Entity.collectAllColliders`).
                let mut collisions = crate::collisions::EntityCollisions {
                    blocks: scene,
                    entities: entity_boxes,
                    border: border.map(|b| (b, [self.position.x, self.position.z])),
                };
                crate::move_gate::entities_resolve_with_context(
                    &mut gate,
                    &mut collisions,
                    self.bounds(),
                    wanted,
                    0.0,
                    self.on_ground,
                    self.collision_context(),
                )
                .unwrap_or(Vec3::ZERO)
            };
            let applied = move_applies(wanted, moved);
            if applied {
                self.position = Vec3::new(
                    self.position.x + moved.x,
                    self.position.y + moved.y,
                    self.position.z + moved.z,
                );
            }
            let mut landed = None;
            if !no_physics {
                self.on_ground = wanted.y < 0.0 && moved.y != wanted.y;
                // Entity.checkFallDamage: the move's drop counts out of water, and a landing
                // hands the whole fall to the block below (`getOnPosLegacy`).
                if water_height <= 0.0 && moved.y < 0.0 {
                    self.fall_distance -= f64::from(moved.y as f32);
                }
                if self.on_ground {
                    if self.fall_distance > 0.0 {
                        let below = Vec3::new(0.0, -f64::from(0.2_f32), 0.0);
                        let on = BlockPos::containing(self.position.add_vector(below));
                        landed = Some((on, self.fall_distance));
                    }
                    self.fall_distance = 0.0;
                }
            }
            // Entity.move: what the move was stopped on. They outlive the
            // tick, as vanilla's fields do, for the tracker that sends the
            // position at the type's update interval.
            self.vertical_collision = moved.y != wanted.y;
            // `Mth.equal` on x and z (1e-5 as a float), exact on y.
            let differs = |a: f64, b: f64| (b - a).abs() >= 9.999_999_747_378_752e-6;
            self.horizontal_collision = differs(moved.x, wanted.x) || differs(moved.z, wanted.z);
            if moved.x != wanted.x {
                self.velocity.x = 0.0;
            }
            if moved.z != wanted.z {
                self.velocity.z = 0.0;
            }
            let vertical_contact = if self.vertical_collision && !no_physics {
                let contact = self.vertical_contact(world, collision, blocks, moved);
                let slime =
                    self.on_ground && contact.is_some_and(|pos| slime_at(world, facts, pos));
                self.velocity.y = vertical_collision_velocity(
                    wanted.y,
                    moved.y,
                    ITEM_GRAVITY,
                    self.on_ground,
                    slime,
                );
                contact
            } else {
                None
            };
            return ItemStep {
                old_velocity,
                vertical_contact,
                movement: Some(moved),
                landed,
                prior_fall,
                fall_move: (!no_physics).then_some((from, moved, applied)),
            };
        }
        // applyEffectsFromBlocksForLastMovements: still lift/drag while idle.
        if !no_physics {
            self.slime_step_on(world, facts);
        }
        ItemStep {
            old_velocity,
            landed: None,
            vertical_contact: None,
            movement: None,
            prior_fall,
            fall_move: None,
        }
    }

    /// `Entity.getOnPosLegacy` for a clipped move. Resolve the support under
    /// the full item box: at a honey wall, its center can be outside the slime
    /// block even though its box landed on it.
    fn vertical_contact(
        &self,
        world: &World,
        collision: &BlockCollisionRegistry<'_>,
        blocks: Option<&BoundMovementProperties>,
        movement: Vec3,
    ) -> Option<BlockPos> {
        let mut scene = SceneCollisions {
            world,
            binding: collision,
            subject: None,
        };
        let mut ground = horizon_world::ground_support::GroundState::default();
        ground
            .check_support(self.on_ground, self.bounds(), Some(movement), |bounds| {
                horizon_world::ground_support::find_supporting_block(
                    &mut scene,
                    horizon_world::collision_context::CollisionContext::empty(false),
                    self.position,
                    bounds,
                    1_000_000,
                )
            })
            .ok()?;
        if let Some(blocks) = blocks {
            return Some(crate::motion::legacy_block_support_at(
                world,
                blocks,
                self.position,
                ground.supporting_block,
            ));
        }
        // Standalone tests can omit the movement table and its fence/wall tags.
        Some(ground.supporting_block.unwrap_or_else(|| {
            let offset = f64::from(horizon_world::movement_support::LEGACY_SUPPORT_OFFSET);
            BlockPos::containing(self.position.subtract(Vec3::new(0.0, offset, 0.0)))
        }))
    }

    /// Resume the normal block effects after the collision response.
    pub fn finish_move_block_effects(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        no_physics: bool,
        blocks: Option<&BoundMovementProperties>,
        step: ItemStep,
        context: &horizon_world::native_load_context::LoopContext,
    ) {
        if step.movement.is_some() {
            // Entity.move applies the block's speed factor before block effects.
            if !no_physics && let Some(blocks) = blocks {
                let factor = crate::motion::block_speed_factor_at(
                    world,
                    blocks,
                    context,
                    self.position,
                    None,
                );
                self.velocity =
                    horizon_world::entity_move::scale_by_block_speed(self.velocity, factor);
            }
            self.apply_honey_slide(world, facts);
            self.stuck = match self.stuck_multiplier(world, facts) {
                // Entity.makeStuckInBlock resets the fall.
                Some(stuck) => {
                    self.fall_distance = 0.0;
                    stuck
                }
                None => Vec3::ZERO,
            };
        }
    }
}
