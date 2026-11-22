//! Primed TNT (`PrimedTnt`): 80-tick fuse, gravity and drag, and the move and
//! effect rules of one tick. The `Game` side (priming, explosion, rendering)
//! stays in `horizon-client`.
use horizon_client::entity_rules::EntityRules;
use horizon_client::{Result, collisions::SceneCollisions, mobs::Who};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
};

#[derive(Clone, Debug)]
pub struct PrimedTnt {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: horizon_mod_runtime::attachments::AttachmentLifetime,
    /// `Entity.yRot` and `xRot` (`/tp`, `/data get ... Rotation`).
    pub rotation: [f32; 2],
    /// When it joined its list (`horizon_client::join_seq`); set by the list.
    pub join: horizon_client::join_seq::JoinSeq,
    /// The entity id a server tracks it by (0 until it is tracked).
    pub id: u32,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub fuse: i32,
    pub on_ground: bool,
    /// `Entity.fallDistance` (`fall_distance` in its save).
    pub fall_distance: f64,
    /// `Entity.stuckSpeedMultiplier`: set by powder snow or a cobweb it is
    /// in, it scales the next move, which then starts from rest.
    pub stuck: Vec3,
    /// `Entity.wasTouchingWater`, from its last immersion update.
    pub in_water: bool,
    pub owner: Option<Who>,
    pub guest: Option<u32>,
    /// `Entity.noGravity` (`NoGravity` in its save).
    pub no_gravity: bool,
    /// `PrimedTnt.explosionPower`: 4 by default, saved as `explosion_power`
    /// and clamped to 0..128 when read back.
    pub power: f32,
    /// `PrimedTnt.teleport`: set on the recreated entity, never saved to NBT.
    pub used_portal: bool,
    /// PrimedTnt.DATA_BLOCK_STATE_ID; None is the default TNT state.
    pub block_state: Option<horizon_world::registry::BlockStateId>,
}
impl PrimedTnt {
    /// The live state encoded for each existing save view.
    pub fn block_state_nbt(
        &self,
        registry: &horizon_world::registry::BlockRegistry,
    ) -> horizon_core::nbt::Tag {
        self.block_state.map_or_else(
            || horizon_core::nbt::Tag::String("minecraft:tnt".into()),
            |state| horizon_client::entity_nbt::state_to_nbt(registry, state),
        )
    }

    /// `EntityTypes.TNT`'s `sized(0.98F, 0.98F)`, widened from the float.
    pub const SIZE: f64 = 0.98_f32 as f64;
    pub fn bounds(&self) -> Aabb {
        let h = Self::SIZE / 2.0;
        Aabb::new(
            Vec3::new(self.position.x - h, self.position.y, self.position.z - h),
            Vec3::new(
                self.position.x + h,
                self.position.y + Self::SIZE,
                self.position.z + h,
            ),
        )
    }
    /// `Entity.move`: a pending stuck multiplier is used up by the next move,
    /// which starts from rest. Returns it, for a move that scales its step by it.
    pub fn take_stuck(&mut self) -> Option<Vec3> {
        if self.stuck.length_squared() <= 1.0e-7 {
            return None;
        }
        self.velocity = Vec3::ZERO;
        Some(std::mem::replace(&mut self.stuck, Vec3::ZERO))
    }
    /// PowderSnowBlock.getCollisionShape: a top that holds it once it has
    /// fallen more than 2.5 blocks, read with [`Self::collision_context`].
    pub fn powder_snow_subject(&self) -> horizon_client::collisions::PowderSnowSubject<'static> {
        horizon_client::collisions::PowderSnowSubject {
            fall_distance: self.fall_distance,
            ..Default::default()
        }
    }
    pub fn collision_context(&self) -> horizon_world::collision_context::CollisionContext<'static> {
        horizon_world::collision_context::CollisionContext::for_entity(
            horizon_world::collision_context::CollisionEntity {
                identity: horizon_world::entity_obstruction::EntityIdentity(u64::from(self.id)),
                kind: horizon_world::collision_context::EntityContextKind::Other,
                y: self.position.y,
                descending: false,
                main_hand: None,
            },
            false,
        )
    }
    /// Entity.checkFallDamage after a move that stepped `moved_y`: out of
    /// water, each step down is narrowed to a float before it adds to the
    /// fall; landing forgets it. Returns the fall it landed with, when the
    /// caller runs the block's fallOn and posts HIT_GROUND.
    pub fn check_fall_damage(&mut self, moved_y: f64) -> Option<f64> {
        if !self.in_water && moved_y < 0.0 {
            self.fall_distance -= f64::from(moved_y as f32);
        }
        let landed = (self.on_ground && self.fall_distance > 0.0).then_some(self.fall_distance);
        if self.on_ground {
            self.fall_distance = 0.0;
        }
        landed
    }
    /// Entity.applyEffectsFromBlocks for a move from `from` to here
    /// (`original` the step it asked for, when the move was recorded): powder
    /// snow and cobwebs it went through make it stuck, a bubble column lifts or
    /// drags it, and slime it stands on slows it, first. Returns the columns it
    /// is at the top of, which send their surface particles.
    pub fn apply_effects_from_blocks(
        &mut self,
        world: &horizon_client::world::World,
        assets: &horizon_client::assets::Assets,
        facts: &horizon_client::blocks::BlockFacts,
        collision: &horizon_world::block_collision::BlockCollisionRegistry<'_>,
        from: Vec3,
        original: Option<Vec3>,
    ) -> Vec<BlockPos> {
        if self.on_ground {
            // SlimeBlock.stepOn, before the blocks it is inside.
            let under = world.block(
                self.position.x.floor() as i32,
                (self.position.y - f64::from(0.2_f32)).floor() as i32,
                self.position.z.floor() as i32,
            );
            if &*facts.get(under).name == "minecraft:slime_block"
                && let Some(scale) = horizon_client::entities::slime_step_on_scale(self.velocity.y)
            {
                self.velocity.x *= scale;
                self.velocity.z *= scale;
            }
        }
        // Entity.checkInsideBlocks: a column pushes it from every block the
        // move went through, not only those its box ends in.
        let mut tops = Vec::new();
        let stuck = horizon_client::falling_block::apply_effects_from_blocks(
            from,
            self.position,
            original,
            &mut self.velocity,
            &mut self.fall_distance,
            &mut tops,
            |p| {
                use horizon_client::falling_block::Inside;
                match &*facts.get(world.block(p.x, p.y, p.z)).name {
                    "minecraft:powder_snow" => Inside::PowderSnow,
                    "minecraft:cobweb" => Inside::Cobweb,
                    _ => horizon_client::motion::bubble_column_at(
                        world,
                        assets.registry,
                        facts,
                        collision,
                        p,
                    )
                    .map_or(Inside::Nothing, Inside::BubbleColumn),
                }
            },
        );
        if let Some(multiplier) = stuck {
            self.stuck = multiplier;
        }
        tops
    }
    pub fn tick(
        &mut self,
        world: &horizon_client::world::World,
        collision: &horizon_world::block_collision::BlockCollisionRegistry<'_>,
        assets: &horizon_client::assets::Assets,
        facts: &horizon_client::blocks::BlockFacts,
        fluids: &horizon_client::motion::FluidResources,
        context: &horizon_world::native_load_context::LoopContext,
        border: Option<&horizon_core::geometry::border::BorderGeometry>,
        blocks: Option<&horizon_world::movement_properties::BoundMovementProperties>,
    ) -> Result<Vec<BlockPos>> {
        self.tick_among(
            world,
            collision,
            assets,
            facts,
            fluids,
            context,
            border,
            blocks,
            &[],
        )
        .map(|(tops, _)| tops)
    }
    /// [`Self::tick`], its move also stopped by `entities`: the boxes of the
    /// entities that `canBeCollidedWith` it (`EntityGetter.getEntityCollisions`).
    /// Also gives the fall it landed with, if it landed after one.
    #[allow(clippy::too_many_arguments)]
    pub fn tick_among(
        &mut self,
        world: &horizon_client::world::World,
        collision: &horizon_world::block_collision::BlockCollisionRegistry<'_>,
        assets: &horizon_client::assets::Assets,
        facts: &horizon_client::blocks::BlockFacts,
        fluids: &horizon_client::motion::FluidResources,
        context: &horizon_world::native_load_context::LoopContext,
        border: Option<&horizon_core::geometry::border::BorderGeometry>,
        blocks: Option<&horizon_world::movement_properties::BoundMovementProperties>,
        entities: &[Aabb],
    ) -> Result<(Vec<BlockPos>, Option<f64>)> {
        self.previous = self.position;
        self.velocity.y -= self.get_gravity();
        let wanted = self.velocity;
        // The TNT's Entity.move goes through the move gate (arismoko/horizon#2414), armed
        // before the stuck multiplier of the move itself; PR 1a records the
        // movement unchanged.
        let mut gate = horizon_client::move_gate::armed(
            context,
            self.id,
            "minecraft:tnt",
            horizon_world::entity_move::MoverType::SelfMovement,
            wanted,
        );
        let mut wanted = gate.delta();
        if let Some(multiplier) = self.take_stuck() {
            wanted = wanted.multiply(multiplier);
        }
        let scene = SceneCollisions {
            world,
            binding: collision,
            subject: Some(self.powder_snow_subject()),
        };
        // Entity.collide: the world border's walls and the entities that
        // `canBeCollidedWith` it (a boat, a shulker) stop it too.
        let border = border.map(|b| (b, [self.position.x, self.position.z]));
        let mut collisions = horizon_client::collisions::EntityCollisions {
            blocks: scene,
            entities,
            border,
        };
        let moved = horizon_client::move_gate::entities_resolve_with_context(
            &mut gate,
            &mut collisions,
            self.bounds(),
            wanted,
            0.0,
            self.on_ground,
            self.collision_context(),
        )
        .unwrap_or(Vec3::ZERO);
        let from = self.position;
        let recorded = horizon_client::entities::move_applies(wanted, moved);
        if recorded {
            self.position = Vec3::new(
                self.position.x + moved.x,
                self.position.y + moved.y,
                self.position.z + moved.z,
            );
        }
        self.on_ground = wanted.y < 0.0 && moved.y != wanted.y;
        let landed = self.check_fall_damage(moved.y);
        if moved.x != wanted.x {
            self.velocity.x = 0.0;
        }
        if moved.z != wanted.z {
            self.velocity.z = 0.0;
        }
        if moved.y != wanted.y {
            // Entity.restituteMovementAfterCollisions: slime bounces it.
            let below = world.block(
                self.position.x.floor() as i32,
                (self.position.y - f64::from(1.0e-5_f32)).floor() as i32,
                self.position.z.floor() as i32,
            );
            // Restitution reads the velocity, at rest after a stuck move.
            self.velocity.y = horizon_client::entities::vertical_collision_velocity(
                self.velocity.y,
                moved.y,
                0.04,
                wanted.y < 0.0,
                &*facts.get(below).name == "minecraft:slime_block",
            );
        }
        // Entity.move's last step: soul sand and honey slow the TNT's
        // horizontal speed. `blocks` is None only for the isolated unit tests.
        if let Some(blocks) = blocks {
            let factor = horizon_client::motion::block_speed_factor_at(
                world,
                blocks,
                context,
                self.position,
                None,
            );
            self.velocity = horizon_world::entity_move::scale_by_block_speed(self.velocity, factor);
        }
        let tops = self.apply_effects_from_blocks(
            world,
            assets,
            facts,
            collision,
            from,
            recorded.then_some(wanted),
        );
        // Entity.scale(getAirDrag()): 0.98F, widened before it scales motion.
        self.velocity = horizon_client::entity_rules::scale_air_drag(self.velocity);
        if self.on_ground {
            self.velocity = Vec3::new(
                self.velocity.x * 0.7,
                self.velocity.y * -0.5,
                self.velocity.z * 0.7,
            );
        }
        self.fuse -= 1;
        if self.fuse > 0 {
            // PrimedTnt updates immersion after moving and decrementing its
            // fuse; water forgets the fall.
            // `Entity::push_by_fluid` for primed TNT (pushed by fluid, eye at
            // `eyeHeight(0.15F)`); that trait stays in horizon-client.
            let (bounds, at, eye) = (
                self.bounds(),
                self.position,
                self.position.y + f64::from(0.15_f32),
            );
            let (water, _, _) = horizon_client::entities::update_entity_fluids(
                bounds,
                at,
                eye,
                &mut self.velocity,
                world,
                fluids,
                context,
            );
            self.in_water = water > 0.0;
            if self.in_water {
                self.fall_distance = 0.0;
            }
        }
        Ok((tops, landed))
    }
}
horizon_client::impl_joined!(PrimedTnt);
horizon_mod_runtime::impl_attachment_lifetime!(PrimedTnt);
impl EntityRules for PrimedTnt {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `PrimedTnt.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.04
    }
}
