//! Stored movement state for the resident scene. This binds the validated air,
//! jump, collision, support and restitution kernels. Entity lifecycle, damage,
//! block-inside effects and audiovisual event consumers remain separate work.
use crate::{Result, assets::Assets, collisions::SceneCollisions, world::TestWorld};
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape, traverse_blocks},
    position::BlockPos,
};
use horizon_world::{
    attribute_map::ValueKind,
    attributes::{Modifier, Operation, entity::Catalog, live::LiveAttributes},
    block_collision::BlockCollisionRegistry,
    collision_context::{CollisionContext, CollisionEntity, CollisionItem},
    entity_move::{self, CollisionFlags, EntityMoveWorld, MovementHistory, MoverType},
    entity_obstruction::EntityIdentity,
    ground_support::{GroundState, find_supporting_block},
    living_air::{self, LivingAirWorld},
    living_motion::{self, LivingMotionWorld},
    movement_emission::MovementEmission,
    movement_properties::{
        BoundMovementProperties, MovementBlockProvider, MovementProperties, MovementTags,
        RegistryMovementWorld,
    },
    movement_restitution::{self, RestitutionInput, RestitutionWorld},
    movement_support::{self, MovementLocation},
    nbt::{self, Limits},
    registry::BlockStateId,
    world_movement::{self, MovementRequest},
    worldgen::tags::TagDefinitions,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
    sync::Arc,
};

pub struct MotionResources {
    pub attributes: Arc<Catalog>,
    pub blocks: BoundMovementProperties,
    pub climbable: BTreeSet<String>,
    pub fall_reset: BTreeSet<String>,
}
impl MotionResources {
    pub fn load(root: &Path, assets: &Assets) -> Result<Self> {
        let attributes = Arc::new(Catalog::from_json(&serde_json::from_slice(&fs::read(
            root.join("attributes.json"),
        )?)?)?);
        let data = nbt::read_named(
            &fs::read(root.join("movement-properties.nbt"))?,
            Limits::default(),
        )?
        .0
        .tag;
        let properties = MovementProperties::from_nbt(&data, &assets.registry)?;
        let bundle: BTreeMap<String, String> =
            serde_json::from_slice(&fs::read(root.join("block-tags.json"))?)?;
        let mut definitions = TagDefinitions::default();
        for (name, text) in bundle {
            definitions.apply_json(&name, "vanilla", &text)?;
        }
        let built = definitions
            .build(|name, _| assets.registry.default_state(name).map(|_| name.to_owned()))?;
        if !built.missing.is_empty() {
            return Err("unresolved motion block tags".into());
        }
        let tag = |name: &str| -> Result<BTreeSet<String>> {
            Ok(built
                .tags
                .get(name)
                .ok_or_else(|| format!("missing motion tag {name}"))?
                .iter()
                .cloned()
                .collect())
        };
        Ok(Self {
            attributes,
            blocks: properties.bind(&MovementTags {
                fences: tag("minecraft:fences")?,
                walls: tag("minecraft:walls")?,
                suppresses_bounce: tag("minecraft:suppresses_bounce")?,
            })?,
            climbable: tag("minecraft:climbable")?,
            fall_reset: tag("minecraft:fall_damage_resetting")?,
        })
    }
}

/// Explicit phase records until the entity runtime can dispatch damage, events
/// and networking. A queued landing is not a claim that damage was applied.
#[derive(Clone, Copy, Debug)]
pub enum MotionEvent {
    Recorded {
        mover: MoverType,
        movement: Vec3,
    },
    Landing {
        dy: f64,
        grounded: bool,
        state: BlockStateId,
        position: BlockPos,
    },
    Emission {
        movement: Vec3,
        position: BlockPos,
        state: BlockStateId,
    },
    Bounce {
        position: BlockPos,
        state: BlockStateId,
        fall_distance: f64,
    },
}
pub struct PlayerMotion {
    pub position: Vec3,
    pub velocity: Vec3,
    pub ground: GroundState,
    pub collision: CollisionFlags,
    pub history: MovementHistory,
    pub fall_distance: f64,
    pub stuck_multiplier: Vec3,
    pub yaw: f32,
    pub jumping: bool,
    pub sprinting: bool,
    pub speed: f32,
    pub last_climbable: Option<BlockPos>,
    pub needs_sync: bool,
    pub sync_position: bool,
    pub events: Vec<MotionEvent>,
    pub attributes: LiveAttributes<Box<dyn FnMut() -> i32>>,
}
fn attribute_error(error: impl std::fmt::Debug) -> io::Error {
    io::Error::other(format!("movement attribute: {error:?}"))
}
impl PlayerMotion {
    pub fn new(resources: &MotionResources, position: Vec3) -> Result<Self> {
        // Identity hashes affect map order, not queried movement values. These
        // are process-owned identities; no JVM allocation order is assumed.
        let hashes: Vec<_> = resources
            .attributes
            .attributes()
            .enumerate()
            .map(|(i, _)| i as i32)
            .collect();
        let mut identity = 0i32;
        let next: Box<dyn FnMut() -> i32> = Box::new(move || {
            identity = identity.wrapping_add(1);
            identity
        });
        let attributes = LiveAttributes::new(
            resources.attributes.clone(),
            "minecraft:player",
            &hashes,
            next,
        )
        .map_err(attribute_error)?;
        Ok(Self {
            position,
            velocity: Vec3::ZERO,
            ground: GroundState::default(),
            collision: CollisionFlags::default(),
            history: MovementHistory::default(),
            fall_distance: 0.0,
            stuck_multiplier: Vec3::ZERO,
            yaw: 0.0,
            jumping: false,
            sprinting: false,
            speed: 0.0,
            last_climbable: None,
            needs_sync: false,
            sync_position: false,
            events: Vec::new(),
            attributes,
        })
    }
    pub fn bounds(&self) -> Aabb {
        let half = f64::from(0.6_f32 / 2.0);
        Aabb::new(
            Vec3::new(
                self.position.x - half,
                self.position.y,
                self.position.z - half,
            ),
            Vec3::new(
                self.position.x + half,
                self.position.y + f64::from(1.8_f32),
                self.position.z + half,
            ),
        )
    }
    pub fn attribute(&mut self, name: &str) -> Result<f64> {
        let key = self
            .attributes
            .key(name)
            .ok_or_else(|| format!("missing movement attribute {name}"))?;
        Ok(self
            .attributes
            .value(Some(key), ValueKind::Computed)
            .map_err(attribute_error)?)
    }
    pub fn set_sprinting(&mut self, sprinting: bool) -> Result<()> {
        self.sprinting = sprinting;
        let key = self
            .attributes
            .key("minecraft:movement_speed")
            .ok_or("missing movement speed")?;
        let handle = self
            .attributes
            .get_instance(Some(key))
            .map_err(attribute_error)?
            .ok_or("player has no movement speed")?;
        let modifier = Modifier::new(
            "minecraft:sprinting",
            f64::from(0.3_f32),
            Operation::AddMultipliedTotal,
        )?;
        self.attributes
            .edit(&handle, |value, observer| {
                value.remove(modifier.id(), observer)?;
                if sprinting {
                    value.add_transient(modifier, observer)?;
                }
                Ok(())
            })
            .map_err(attribute_error)?;
        Ok(())
    }
}

pub struct MotionWorld<'p, 'w, 'r> {
    pub player: &'p mut PlayerMotion,
    pub scene: &'w TestWorld<'r>,
    pub assets: &'r Assets,
    pub collision_binding: &'w BlockCollisionRegistry<'r>,
    pub resources: &'w MotionResources,
}
impl MotionWorld<'_, '_, '_> {
    fn context(&self) -> CollisionContext<'static> {
        CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(0),
                y: self.player.position.y,
                descending: false,
                main_hand: Some(CollisionItem::EMPTY),
            },
            false,
        )
    }
    fn collisions(&self) -> SceneCollisions<'_, '_> {
        SceneCollisions {
            world: self.scene,
            binding: self.collision_binding,
        }
    }
    fn block(&self, p: BlockPos) -> BlockStateId {
        self.scene.block(p.x, p.y, p.z)
    }
    fn support(&mut self, offset: f32) -> Result<BlockPos> {
        let position = self.player.position;
        let support = self.player.ground.supporting_block;
        Ok(movement_support::support_position(
            &mut RegistryMovementWorld {
                properties: &self.resources.blocks,
                world: self,
            },
            position,
            support,
            offset,
        )?)
    }
    fn location(&self) -> MovementLocation {
        MovementLocation {
            position: self.player.position,
            block_position: BlockPos::containing(self.player.position),
            supporting_block: self.player.ground.supporting_block,
        }
    }
    fn climbable(&mut self) -> Result<bool> {
        let p = BlockPos::containing(self.player.position);
        let name = &self
            .assets
            .registry
            .state(self.block(p))
            .ok_or("missing scene state")?
            .name;
        let climbable = self.resources.climbable.contains(name);
        if climbable {
            self.player.last_climbable = Some(p);
        }
        Ok(climbable)
    }
    pub fn travel_in_air(&mut self, input: Vec3) -> Result<()> {
        living_air::travel_in_air(self, input)
    }
    pub fn jump_from_ground(&mut self) -> Result<()> {
        living_motion::jump_from_ground(self)
    }
    pub fn move_self(&mut self, movement: Vec3) -> Result<()> {
        entity_move::move_entity(self, MoverType::SelfMovement, movement)
    }
}
impl MovementBlockProvider for MotionWorld<'_, '_, '_> {
    type Error = io::Error;
    fn movement_block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        Ok(self.block(p))
    }
}
impl EntityMoveWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn no_physics(&self) -> bool {
        false
    }
    fn position(&self) -> Vec3 {
        self.player.position
    }
    fn set_position(&mut self, p: Vec3) -> Result<()> {
        self.player.position = p;
        Ok(())
    }
    fn record_movement(&mut self, mover: MoverType, movement: Vec3) -> Result<()> {
        self.player
            .events
            .push(MotionEvent::Recorded { mover, movement });
        Ok(())
    }
    fn collision_flags(&self) -> CollisionFlags {
        self.player.collision
    }
    fn set_collision_flags(&mut self, value: CollisionFlags) {
        self.player.collision = value;
    }
    fn limit_piston_movement(&mut self, _: Vec3) -> Result<Vec3> {
        Err("piston movement requires a ticking entity runtime".into())
    }
    fn stuck_multiplier(&self) -> Vec3 {
        self.player.stuck_multiplier
    }
    fn set_stuck_multiplier(&mut self, value: Vec3) {
        self.player.stuck_multiplier = value;
    }
    fn velocity(&mut self) -> Result<Vec3> {
        Ok(self.player.velocity)
    }
    fn set_velocity(&mut self, value: Vec3) -> Result<()> {
        self.player.velocity = value;
        Ok(())
    }
    fn back_off_edge(&mut self, movement: Vec3, _: MoverType) -> Result<Vec3> {
        Ok(movement)
    }
    fn collide(&mut self, movement: Vec3) -> Result<Vec3> {
        let max_step = self.player.attribute("minecraft:step_height")? as f32;
        Ok(world_movement::resolve_world_movement(
            &mut self.collisions(),
            MovementRequest {
                entity: EntityIdentity(0),
                context: self.context(),
                bounds: self.player.bounds(),
                movement,
                max_step,
                on_ground: self.player.ground.on_ground,
                candidate_budget: 1_000_000,
            },
        )?)
    }
    fn fall_distance(&self) -> f64 {
        self.player.fall_distance
    }
    fn fall_reset_clip(&mut self, from: Vec3, to: Vec3) -> Result<bool> {
        // Every fluid in this controlled scene is empty. Tagged reset blocks use
        // a full block in ClipContext.FALLDAMAGE_RESETTING, not their collider.
        let hit = traverse_blocks(from, to, 1_000_000, |p| {
            let name = &self.assets.registry.state(self.block(p)).unwrap().name;
            (self.resources.fall_reset.contains(name)
                && VoxelShape::block().clip(from, to, p).is_some())
            .then_some(())
        })?;
        Ok(hit.is_some())
    }
    fn reset_fall_distance(&mut self) -> Result<()> {
        self.player.fall_distance = 0.0;
        Ok(())
    }
    fn movement_history(&mut self) -> &mut MovementHistory {
        &mut self.player.history
    }
    fn locally_authoritative(&mut self) -> Result<bool> {
        Ok(true)
    }
    fn set_on_ground_with_movement(
        &mut self,
        below: bool,
        horizontal: bool,
        movement: Vec3,
    ) -> Result<()> {
        let position = self.player.position;
        let bounds = self.player.bounds();
        let context = self.context();
        let mut collisions = SceneCollisions {
            world: self.scene,
            binding: self.collision_binding,
        };
        self.player.ground.set_with_movement(
            below,
            horizontal,
            bounds,
            Some(movement),
            |bounds| find_supporting_block(&mut collisions, context, position, bounds, 1_000_000),
        )?;
        Ok(())
    }
    fn minor_horizontal_collision(&mut self, _: Vec3) -> Result<bool> {
        Ok(false)
    }
    fn legacy_on_position(&mut self) -> Result<BlockPos> {
        self.support(movement_support::LEGACY_SUPPORT_OFFSET)
    }
    fn block_state(&mut self, p: BlockPos) -> Result<BlockStateId> {
        Ok(self.block(p))
    }
    fn on_ground(&mut self) -> Result<bool> {
        Ok(self.player.ground.on_ground)
    }
    fn check_fall_damage(
        &mut self,
        dy: f64,
        grounded: bool,
        state: BlockStateId,
        position: BlockPos,
    ) -> Result<()> {
        self.player.events.push(MotionEvent::Landing {
            dy,
            grounded,
            state,
            position,
        });
        // This phase is deliberately left for the owning entity lifecycle. Do
        // not fabricate health/damage or reset a caller's fall-distance field.
        Ok(())
    }
    fn removed(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn can_simulate(&mut self) -> Result<bool> {
        Ok(true)
    }
    fn restitute(&mut self, state: BlockStateId, x: bool, z: bool, resolved: Vec3) -> Result<()> {
        let input = RestitutionInput {
            block: state,
            collided_x: x,
            collided_z: z,
            vertical_collision: self.player.collision.vertical,
            collision_below: self.player.collision.below,
            living: true,
            resolved_movement: resolved,
            fall_distance: self.player.fall_distance,
        };
        movement_restitution::restitute_movement(self, input)
    }
    fn client_side(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn movement_emission(&mut self) -> Result<MovementEmission> {
        Ok(MovementEmission::All)
    }
    fn passenger(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn apply_movement_emission(
        &mut self,
        _: MovementEmission,
        movement: Vec3,
        position: BlockPos,
        state: BlockStateId,
    ) -> Result<()> {
        self.player.events.push(MotionEvent::Emission {
            movement,
            position,
            state,
        });
        Ok(())
    }
    fn block_speed_factor(&mut self) -> Result<f32> {
        let efficiency = self.player.attribute("minecraft:movement_efficiency")? as f32;
        let location = self.location();
        let block = movement_support::block_speed_factor(
            &mut RegistryMovementWorld {
                properties: &self.resources.blocks,
                world: self,
            },
            location,
        )?;
        Ok(block + efficiency * (1.0 - block))
    }
}
impl RestitutionWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn suppressing_bounce(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn entity_bounciness(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:bounciness")
    }
    fn velocity(&mut self) -> Result<Vec3> {
        Ok(self.player.velocity)
    }
    fn effective_gravity(&mut self) -> Result<f64> {
        living_motion::effective_gravity(self)
    }
    fn block_suppresses_bounce(&mut self, state: BlockStateId) -> Result<bool> {
        Ok(self.resources.blocks.get(state)?.suppresses_bounce)
    }
    fn block_bounce_restitution(&mut self, state: BlockStateId) -> Result<f32> {
        Ok(self
            .resources
            .blocks
            .get(state)?
            .coefficients
            .bounce_restitution)
    }
    fn air_drag(&mut self) -> Result<f32> {
        living_motion::air_drag(self)
    }
    fn movement_support_position(&mut self) -> Result<BlockPos> {
        self.support(movement_support::MOVEMENT_SUPPORT_OFFSET)
    }
    fn bounce_on(
        &mut self,
        state: BlockStateId,
        position: BlockPos,
        fall_distance: f64,
    ) -> Result<()> {
        self.player.events.push(MotionEvent::Bounce {
            state,
            position,
            fall_distance,
        });
        Ok(())
    }
    fn bounce_event(&mut self) -> Result<()> {
        Ok(())
    }
    fn mark_position_for_sync(&mut self) {
        self.player.sync_position = true;
    }
    fn set_velocity(&mut self, value: Vec3) -> Result<()> {
        self.player.velocity = value;
        Ok(())
    }
}
impl LivingAirWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn movement_support_position(&mut self) -> Result<BlockPos> {
        self.support(movement_support::MOVEMENT_SUPPORT_OFFSET)
    }
    fn on_ground(&mut self) -> Result<bool> {
        Ok(self.player.ground.on_ground)
    }
    fn block_friction(&mut self, p: BlockPos) -> Result<f32> {
        Ok(self
            .resources
            .blocks
            .get(self.block(p))?
            .coefficients
            .friction)
    }
    fn friction_modifier(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:friction_modifier")
    }
    fn speed(&mut self) -> Result<f32> {
        Ok(self.player.speed)
    }
    fn flying_speed(&mut self) -> Result<f32> {
        Ok(if self.player.sprinting {
            0.025999999
        } else {
            0.02
        })
    }
    fn yaw(&mut self) -> Result<f32> {
        Ok(self.player.yaw)
    }
    fn velocity(&self) -> Vec3 {
        self.player.velocity
    }
    fn set_velocity(&mut self, v: Vec3) -> Result<()> {
        self.player.velocity = v;
        Ok(())
    }
    fn on_climbable(&mut self) -> Result<bool> {
        self.climbable()
    }
    fn reset_fall_distance(&mut self) -> Result<()> {
        self.player.fall_distance = 0.0;
        Ok(())
    }
    fn in_scaffolding(&mut self) -> Result<bool> {
        Ok(self
            .assets
            .registry
            .state(self.block(BlockPos::containing(self.player.position)))
            .unwrap()
            .name
            == "minecraft:scaffolding")
    }
    fn suppressing_ladder_slide(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn is_player_class(&self) -> bool {
        true
    }
    fn move_self(&mut self, v: Vec3) -> Result<()> {
        MotionWorld::move_self(self, v)
    }
    fn horizontal_collision(&self) -> bool {
        self.player.collision.horizontal
    }
    fn jumping(&self) -> bool {
        self.player.jumping
    }
    fn was_in_powder_snow(&self) -> bool {
        false
    }
    fn can_walk_powder_snow(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn levitation_amplifier(&mut self) -> Result<Option<i32>> {
        Ok(None)
    }
    fn is_client_side(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn has_chunk_at(&mut self, p: BlockPos) -> Result<bool> {
        Ok(self.scene.chunks.contains_key(&(p.x >> 4, p.z >> 4)))
    }
    fn effective_gravity(&mut self) -> Result<f64> {
        living_motion::effective_gravity(self)
    }
    fn y(&self) -> f64 {
        self.player.position.y
    }
    fn minimum_y(&mut self) -> Result<i32> {
        Ok(0)
    }
    fn discard_friction(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn air_drag_modifier(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:air_drag_modifier")
    }
    fn omnidirectional_air_mover(&mut self) -> Result<bool> {
        Ok(false)
    }
}
impl LivingMotionWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn velocity(&self) -> Vec3 {
        self.player.velocity
    }
    fn set_velocity(&mut self, v: Vec3) -> Result<()> {
        self.player.velocity = v;
        Ok(())
    }
    fn jump_strength(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:jump_strength")
    }
    fn block_jump_factor(&mut self) -> Result<f32> {
        let location = self.location();
        Ok(movement_support::block_jump_factor(
            &mut RegistryMovementWorld {
                properties: &self.resources.blocks,
                world: self,
            },
            location,
        )?)
    }
    fn has_jump_boost(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn jump_boost_amplifier(&mut self) -> Result<i32> {
        Err("absent jump boost".into())
    }
    fn sprinting(&mut self) -> Result<bool> {
        Ok(self.player.sprinting)
    }
    fn yaw(&mut self) -> Result<f32> {
        Ok(self.player.yaw)
    }
    fn mark_needs_sync(&mut self) {
        self.player.needs_sync = true;
    }
    fn no_gravity(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn gravity_attribute(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:gravity")
    }
    fn has_slow_falling(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn controlling_passenger_is_player(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn speed(&mut self) -> Result<f32> {
        Ok(self.player.speed)
    }
    fn omnidirectional_air_mover(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn air_drag_modifier(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:air_drag_modifier")
    }
}
