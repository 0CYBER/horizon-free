//! Stored movement state for the resident scene. This binds the validated air,
//! jump, collision, support, restitution and stored health bindings. Entity lifecycle,
//! block-inside effects and audiovisual event consumers remain separate work.
use crate::{Result, assets::Assets, collisions::SceneCollisions, world::World};
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape, traverse_blocks},
    position::BlockPos,
};
use horizon_world::{
    attribute_map::ValueKind,
    attributes::{Modifier, Operation, entity::Catalog, live::LiveAttributes},
    block_collision::BlockCollisionRegistry,
    block_landing::BlockLandingRegistry,
    collision_context::{CollisionContext, CollisionEntity, CollisionItem},
    edge_backoff::{self, EdgeBackoffInput, EdgeBackoffWorld},
    entity_move::{self, CollisionFlags, EntityMoveWorld, MovementHistory, MoverType},
    entity_obstruction::EntityIdentity,
    ground_support::{GroundState, find_supporting_block},
    living_air::{self, LivingAirWorld},
    living_fall::ImpulseContext,
    living_motion::{self, LivingMotionWorld},
    movement_emission::{
        self as emission, MovementDistances, MovementEmission, MovementEmissionWorld, MovementEvent,
    },
    movement_properties::{
        BoundMovementProperties, MovementBlockProvider, MovementProperties, MovementTags,
        RegistryMovementWorld,
    },
    movement_restitution::{self, RestitutionInput, RestitutionWorld},
    movement_statistics::{self, MovementStat, MovementStatisticsWorld},
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

pub(crate) mod environment;

pub struct MotionResources {
    pub facts: crate::blocks::BlockFacts,
    pub attributes: Arc<Catalog>,
    pub blocks: BoundMovementProperties,
    pub climbable: BTreeSet<String>,
    pub fall_reset: BTreeSet<String>,
    pub landing: BlockLandingRegistry,
    pub health: crate::health::HealthResources,
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
        let properties = MovementProperties::from_nbt(&data, assets.registry)?;
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
            facts: crate::blocks::BlockFacts::load(root, assets.registry)?,
            landing: crate::landing::load(root, &properties)?,
            health: crate::health::HealthResources::load(root)?,
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

/// Ordered requests and phase records for world/audio/network consumers. Landing
/// hurt dispatch updates stored health synchronously before these are drained.
#[derive(Clone, Copy, Debug)]
pub enum MotionEvent {
    Waypoint {
        receiver: bool,
        enabled: bool,
    },
    Health(crate::health::HealthEvent),
    Fall(crate::landing::FallEvent),
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
    /// `Entity.walkingStepSound` on this block.
    Emission {
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
    pub fluids: horizon_world::entity_fluid::EntityFluidInteraction,
    pub in_water: bool,
    pub eye_in_water: bool,
    pub in_lava: bool,
    pub swimming: bool,
    pub fire_ticks: i32,
    pub air: i32,
    pub respiration: i32,

    pub position: Vec3,
    pub velocity: Vec3,
    pub ground: GroundState,
    pub collision: CollisionFlags,
    pub history: MovementHistory,
    pub fall_distance: f64,
    pub stuck_multiplier: Vec3,
    pub yaw: f32,
    pub head_yaw: f32,
    pub jumping: bool,
    pub no_jump_delay: i32,
    pub jump_trigger_time: i32,
    /// Abilities.mayfly / flying (creative flight).
    pub may_fly: bool,
    /// Abilities.invulnerable (creative): only damage bypassing invulnerability hurts.
    pub invulnerable: bool,
    /// Spectator mode: no physics, always flying (`Player.tick`).
    pub spectator: bool,
    pub flying: bool,
    /// The jump key last tick (LocalPlayer.aiStep's wasJumping).
    pub was_jumping: bool,
    /// `LocalPlayer.autoJumpEnabled` (the Auto-Jump option) and
    /// `autoJumpTime`; `was_auto_jump` is this tick's `wasAutoJump`.
    pub auto_jump_enabled: bool,
    pub auto_jump_time: i32,
    pub was_auto_jump: bool,
    /// `input.getMoveVector()` this tick (before `modifyInput`).
    pub move_vector: [f32; 2],
    /// A usable glider (elytra) is worn; the game sets it each tick.
    pub glider: bool,
    /// The view pitch (xRot), for gliding.
    pub pitch: f32,
    /// Gliding (shared flag 7) and LivingEntity.fallFlyTicks.
    pub fall_flying: bool,
    pub fall_fly_ticks: i32,
    /// Glider durability to take this tick (updateFallFlying).
    pub glider_wear: u32,
    pub sprinting: bool,
    pub sprint_input: crate::sprint::SprintState,
    pub shift_key_down: bool,
    pub local_crouching: bool,
    pub pose: crate::pose::Pose,
    pub dimensions: crate::pose::Dimensions,
    pub speed: f32,
    /// `LocalPlayer.itemUseSpeedMultiplier` while using an item (not riding).
    pub item_use_speed: Option<f32>,
    pub last_climbable: Option<BlockPos>,
    /// `Entity.moveDist`, `flyDist` and `nextStep`: footsteps come once per
    /// metre walked, not once per move.
    pub steps: MovementDistances,
    pub needs_sync: bool,
    pub sync_position: bool,
    pub events: Vec<MotionEvent>,
    pub attributes: LiveAttributes<Box<dyn FnMut() -> i32>>,
    pub impulse: ImpulseContext,
    pub health: crate::health::PlayerHealth,
    /// Status effects and armor protection seen by movement and damage.
    pub status: crate::effects::StatusFlags,
}
fn attribute_error(error: impl std::fmt::Debug) -> io::Error {
    io::Error::other(format!("movement attribute: {error:?}"))
}
impl PlayerMotion {
    pub fn load_environment_once(
        &mut self,
        pending: &mut bool,
        extra: Option<&horizon_world::nbt::Tag>,
    ) {
        if std::mem::take(pending) {
            self.fire_ticks = extra
                .and_then(|t| crate::save::number(t.get("Fire")))
                .unwrap_or(-20) as i32;
            self.air = extra
                .and_then(|t| crate::save::number(t.get("Air")))
                .unwrap_or(300) as i32;
        }
    }
    pub fn ignite_for_ticks(&mut self, ticks: i32) -> Result<()> {
        let scaled = (f64::from(ticks) * self.attribute("minecraft:burning_time")?).ceil() as i32;
        self.fire_ticks = self.fire_ticks.max(scaled);
        Ok(())
    }

    pub fn new(resources: &MotionResources, position: Vec3) -> Result<Self> {
        // Process-owned identities determine native collection callback order.
        // Differential fixtures can replay a reference process's identities.
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
        Self::with_attribute_identities(resources, position, &hashes, next)
    }
    /// Explicit identity allocation for deterministic reference replay.
    pub fn with_attribute_identities(
        resources: &MotionResources,
        position: Vec3,
        hashes: &[i32],
        next: Box<dyn FnMut() -> i32>,
    ) -> Result<Self> {
        let mut attributes = LiveAttributes::new(
            resources.attributes.clone(),
            "minecraft:player",
            hashes,
            next,
        )
        .map_err(attribute_error)?;
        Ok(Self {
            health: crate::health::PlayerHealth::new(&resources.health, &mut attributes)?,
            status: crate::effects::StatusFlags::default(),
            impulse: ImpulseContext::default(),
            position,
            velocity: Vec3::ZERO,
            ground: GroundState::default(),
            collision: CollisionFlags::default(),
            history: MovementHistory::default(),
            fall_distance: 0.0,
            stuck_multiplier: Vec3::ZERO,
            yaw: 0.0,
            head_yaw: 0.0,
            jumping: false,
            no_jump_delay: 0,
            jump_trigger_time: 0,
            may_fly: false,
            invulnerable: false,
            spectator: false,
            flying: false,
            was_jumping: false,
            auto_jump_enabled: false,
            auto_jump_time: 0,
            was_auto_jump: false,
            move_vector: [0.0, 0.0],
            glider: false,
            pitch: 0.0,
            fall_flying: false,
            fall_fly_ticks: 0,
            glider_wear: 0,
            sprinting: false,
            sprint_input: crate::sprint::SprintState::default(),
            shift_key_down: false,
            local_crouching: false,
            pose: crate::pose::Pose::Standing,
            dimensions: crate::pose::Pose::Standing.dimensions(1.0),
            fluids: horizon_world::entity_fluid::EntityFluidInteraction::new([
                environment::WATER,
                environment::LAVA,
            ]),
            in_water: false,
            eye_in_water: false,
            in_lava: false,
            swimming: false,
            fire_ticks: -20,
            air: 300,
            respiration: 0,
            speed: 0.0,
            item_use_speed: None,
            last_climbable: None,
            steps: MovementDistances {
                next_step: 1.0,
                ..MovementDistances::default()
            },
            needs_sync: false,
            sync_position: false,
            events: Vec::new(),
            attributes,
        })
    }
    pub fn bounds(&self) -> Aabb {
        self.dimensions.bounds(self.position)
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
    /// Add (or remove) a transient attribute modifier, e.g. from equipment
    /// (`LivingEntity.collectEquipmentChanges` applying item modifiers).
    pub fn set_modifier(
        &mut self,
        attribute: &str,
        id: &str,
        amount: f64,
        operation: &str,
        present: bool,
    ) -> Result<()> {
        let Some(key) = self.attributes.key(attribute) else {
            return Ok(());
        };
        let Some(handle) = self
            .attributes
            .get_instance(Some(key))
            .map_err(attribute_error)?
        else {
            return Ok(());
        };
        let operation = match operation {
            "add_multiplied_base" => Operation::AddMultipliedBase,
            "add_multiplied_total" => Operation::AddMultipliedTotal,
            _ => Operation::AddValue,
        };
        let modifier = Modifier::new(id, amount, operation)?;
        self.attributes
            .edit(&handle, |value, observer| {
                value.remove(modifier.id(), observer)?;
                if present {
                    value.add_transient(modifier, observer)?;
                }
                Ok(())
            })
            .map_err(attribute_error)?;
        Ok(())
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
    /// Local input sprint phase for the resident dry, standing, empty-equipment
    /// player. Fluid, pose, vehicle, ability-flight and effect bindings remain
    /// separate; hunger and collision conditions use this actor's actual state.
    pub fn sprint_input_tick(&mut self, keys: crate::sprint::Keys) -> Result<()> {
        let conditions = crate::sprint::Conditions {
            sprinting: self.sprinting,
            enough_food: self.health.food.has_enough_food(),
            on_ground: self.ground.on_ground,
            horizontal_collision: self.collision.horizontal,
            minor_horizontal_collision: self.collision.minor,
            moving_slowly: self.moving_slowly(),
            in_water: self.in_water,
            under_water: self.eye_in_water,
            swimming: self.swimming,
            ..Default::default()
        };
        // The sprint setter edits attributes and flags, independent of local
        // keyboard history. Retain the input prefix even if that setter fails.
        let mut state = self.sprint_input;
        let result = state.tick(keys, conditions, 7, |value| self.set_sprinting(value));
        self.sprint_input = state;
        result
    }
    fn remove_frost(&mut self) -> Result<()> {
        let key = self
            .attributes
            .key("minecraft:movement_speed")
            .ok_or("missing movement speed")?;
        // Native removeFrost obtains the instance even when no modifier exists.
        // Pure value lookup would miss lazy creation and its pending updates.
        if let Some(handle) = self
            .attributes
            .get_instance(Some(key))
            .map_err(attribute_error)?
        {
            self.attributes
                .edit(&handle, |value, observer| {
                    value.remove("minecraft:powder_snow", observer)
                })
                .map_err(attribute_error)?;
        }
        Ok(())
    }
}

pub struct MotionWorld<'p, 'w, 'r> {
    pub player: &'p mut PlayerMotion,
    pub scene: &'w mut World,
    pub assets: &'r Assets,
    pub collision_binding: &'w BlockCollisionRegistry<'r>,
    pub resources: &'w MotionResources,
}
enum TickInput {
    Living([f32; 3]),
    PreparedLocal { direction: [f32; 2], jumping: bool },
}
impl MotionWorld<'_, '_, '_> {
    fn context(&self) -> CollisionContext<'static> {
        CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(0),
                y: self.player.position.y,
                descending: self.player.shift_key_down,
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
    pub(crate) fn no_player_collision(&mut self, bounds: Aabb) -> Result<bool> {
        let context = self.context();
        Ok(world_movement::no_world_collision(
            &mut self.collisions(),
            world_movement::CollisionCheckRequest {
                entity: EntityIdentity(0),
                context,
                bounds,
                candidate_budget: 1_000_000,
            },
        )?)
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
    /// Dry, standing movement phases of LivingEntity.aiStep, including server
    /// frost-modifier cleanup with zero frozen ticks. Equipment, interpolation,
    /// effects and world/entity effects remain unbound. Inputs are the native
    /// float xxa/yya/zza fields, before LivingEntity.applyInput damping.
    pub fn movement_tick(&mut self, input: [f32; 3]) -> Result<()> {
        self.dry_tick(TickInput::Living(input))
    }
    /// Dry Player.aiStep movement with a prepared local input vector. The
    /// caller applies LocalPlayer input preparation once. Empty inventory,
    /// equipment and nearby-entity sets are the current scene domain.
    pub fn player_movement_tick(&mut self, direction: [f32; 2], jumping: bool) -> Result<()> {
        if self.player.jump_trigger_time > 0 {
            self.player.jump_trigger_time -= 1;
        }
        self.player.peaceful_regeneration(self.scene.health_rules)?;
        // LocalPlayer.aiStep: double-tap jump toggles creative flight.
        let was_jumping = std::mem::replace(&mut self.player.was_jumping, jumping);
        let flying_before = self.player.flying;
        if self.player.spectator {
            self.player.flying = true;
            self.player.ground.on_ground = false;
        } else if !self.player.may_fly {
            self.player.flying = false;
        } else if !was_jumping && jumping && !self.player.was_auto_jump {
            if self.player.jump_trigger_time == 0 {
                self.player.jump_trigger_time = 7;
            } else {
                self.player.flying = !self.player.flying;
                if self.player.flying && self.player.ground.on_ground {
                    self.jump_from_ground()?;
                }
                self.player.jump_trigger_time = 0;
            }
        }
        let toggled = flying_before != self.player.flying;
        // Player.tryToStartFallFlying (canGlide: airborne, not flying, a glider).
        let can_glide = self.player.glider
            && !self.player.ground.on_ground
            && !self.player.flying
            && !self.player.in_water
            && !self.player.in_lava;
        if jumping && !toggled && !was_jumping && !self.player.fall_flying && can_glide {
            self.player.fall_flying = true;
        }
        // LivingEntity.tick fallFlyTicks, then updateFallFlying.
        if self.player.fall_flying {
            self.player.fall_fly_ticks += 1;
            let still = self.player.glider
                && !self.player.ground.on_ground
                && !self.player.flying
                && !self.player.in_water
                && !self.player.in_lava;
            if !still {
                self.player.fall_flying = false;
            } else if (self.player.fall_fly_ticks + 1) % 10 == 0
                && ((self.player.fall_fly_ticks + 1) / 10) % 2 == 0
            {
                self.player.glider_wear += 1;
            }
        } else {
            self.player.fall_fly_ticks = 0;
        }
        if self.player.flying {
            // Vertical flight: jump up, shift down, three times the flying speed.
            let ya = i32::from(jumping) - i32::from(self.player.shift_key_down);
            if ya != 0 {
                self.player.velocity.y += f64::from(ya as f32 * 0.05 * 3.0);
            }
        }
        self.dry_tick(TickInput::PreparedLocal {
            direction,
            jumping: jumping && !self.player.flying,
        })?;
        if self.player.spectator {
            self.player.ground.on_ground = false;
        }
        if self.player.flying {
            self.player.fall_distance = 0.0;
            if self.player.ground.on_ground && !self.player.spectator {
                self.player.flying = false;
            }
        }
        self.player.head_yaw = self.player.yaw;
        // Player.aiStep refreshes speed after movement, not before travel.
        self.player.speed = self.player.attribute("minecraft:movement_speed")? as f32;
        Ok(())
    }
    /// Resident simulation phases in native order. Enclosing entity/world tick
    /// services are still incomplete; this composes clocks, movement and hunger.
    pub fn simulation_tick(&mut self, direction: [f32; 2], jumping: bool) -> Result<()> {
        self.update_fluid_environment()?;
        self.simulation_tick_with_fluids(direction, jumping)
    }
    fn simulation_tick_with_fluids(&mut self, direction: [f32; 2], jumping: bool) -> Result<()> {
        self.player.health.advance_clock();
        self.tick_environment_damage()?;
        let previous = self.player.position;
        self.player.swimming = self.player.sprinting
            && self.player.in_water
            && (self.player.swimming || self.player.eye_in_water)
            && !self.player.flying
            && !self.player.spectator;
        self.player_movement_tick(direction, jumping)?;
        self.apply_fire_contact()?;
        self.player.refresh_attributes()?;
        self.update_player_pose()?;
        // The resident scene accepts the resolved local movement directly.
        // Vanilla dispatches this separately from travel, on packet acceptance.
        self.check_movement_statistics(previous.vector_to(self.player.position))?;
        self.player
            .food_tick(&self.resources.health, self.scene.health_rules)
    }
    pub fn controlled_simulation_tick(&mut self, keys: crate::sprint::Keys) -> Result<()> {
        self.update_local_crouching()?;
        let mut keys = keys;
        // LocalPlayer.aiStep: a pending auto-jump presses jump for this tick
        // (input.makeJump) and keeps the jump from toggling flight.
        self.player.was_auto_jump = self.player.auto_jump_time > 0;
        if self.player.was_auto_jump {
            self.player.auto_jump_time -= 1;
            keys.jump = true;
        }
        self.update_fluid_environment()?;
        self.player.sprint_input_tick(keys)?;
        self.player.shift_key_down = keys.shift;
        let direction = keys.direction();
        self.player.move_vector = direction;
        let sneak = if direction[0] * direction[0] + direction[1] * direction[1] != 0.0
            && self.player.moving_slowly()
        {
            Some(self.player.attribute("minecraft:sneaking_speed")? as f32)
        } else {
            None
        };
        self.simulation_tick_with_fluids(
            crate::input::modify_input(direction, self.player.item_use_speed, sneak),
            keys.jump,
        )
    }
    pub fn check_movement_statistics(&mut self, delta: Vec3) -> Result<()> {
        movement_statistics::check(self, delta)
    }
    fn dry_tick(&mut self, input: TickInput) -> Result<()> {
        if self.player.no_jump_delay > 0 {
            self.player.no_jump_delay -= 1;
        }
        let mut velocity = self.player.velocity;
        if velocity.x * velocity.x + velocity.z * velocity.z < 9.0e-6 {
            velocity.x = 0.0;
            velocity.z = 0.0;
        }
        if velocity.y.abs() < 0.003 {
            velocity.y = 0.0;
        }
        self.player.velocity = velocity;
        let mut input = match input {
            TickInput::Living(v) => Vec3::new(
                f64::from(v[0] * 0.98),
                f64::from(v[1]),
                f64::from(v[2] * 0.98),
            ),
            TickInput::PreparedLocal { direction, jumping } => {
                self.player.jumping = jumping;
                Vec3::new(f64::from(direction[0]), 0.0, f64::from(direction[1]))
            }
        };
        if self.player.health.dead_or_dying() {
            self.player.jumping = false;
            input.x = 0.0;
            input.z = 0.0;
        }
        if self.player.shift_key_down && !self.player.flying && self.player.in_water {
            self.player.velocity.y -= f64::from(0.04f32);
        }
        if self.player.jumping {
            let height = self.player.fluids.fluid_height(
                self,
                if self.player.in_water {
                    environment::WATER
                } else {
                    environment::LAVA
                },
            );
            if !self.player.flying
                && (self.player.in_water || self.player.in_lava)
                && (!self.player.ground.on_ground || height > 0.4)
            {
                self.player.velocity.y += f64::from(0.04f32);
            } else if self.player.ground.on_ground && self.player.no_jump_delay == 0 {
                self.jump_from_ground()?;
                self.player.no_jump_delay = 10;
            }
        } else {
            self.player.no_jump_delay = 0;
        }
        if self.player.swimming {
            let look_y = f64::from(-self.player.pitch.to_radians().sin());
            let near_surface = BlockPos::containing(Vec3::new(
                self.player.position.x,
                self.player.position.y + 0.9,
                self.player.position.z,
            ));
            if look_y <= 0.0
                || self.player.jumping
                || self.fluid_at(near_surface)?.kind() != horizon_world::fluid::FluidKind::Empty
            {
                let blend = if look_y < -0.2 { 0.085 } else { 0.06 };
                self.player.velocity.y += (look_y - self.player.velocity.y) * blend;
            }
        }
        if self.player.fall_flying {
            self.travel_fall_flying()?;
        } else if self.player.flying {
            // Player.travel while flying: keep 0.6 of the vertical speed.
            let y = self.player.velocity.y;
            self.travel_in_air(input)?;
            self.player.velocity.y = y * 0.6;
        } else if !self.player.flying && (self.player.in_water || self.player.in_lava) {
            horizon_world::living_fluid::travel_in_fluid(self, input)?;
        } else {
            self.travel_in_air(input)?;
        }
        self.player.remove_frost()
    }
    pub fn jump_from_ground(&mut self) -> Result<()> {
        living_motion::jump_from_ground(self)?;
        self.player
            .events
            .push(MotionEvent::Health(crate::health::HealthEvent::Statistic {
                name: "jump",
                amount: 1,
            }));
        self.player
            .health
            .food
            .add_exhaustion(if self.player.sprinting { 0.2 } else { 0.05 });
        Ok(())
    }
    /// LivingEntity.travelFallFlying with updateFallFlyingMovement.
    fn travel_fall_flying(&mut self) -> Result<()> {
        let v = self.player.velocity;
        let (yaw, pitch) = (self.player.yaw.to_radians(), self.player.pitch.to_radians());
        let look = Vec3::new(
            f64::from(-yaw.sin() * pitch.cos()),
            f64::from(-pitch.sin()),
            f64::from(yaw.cos() * pitch.cos()),
        );
        let lean = pitch;
        let look_h = (look.x * look.x + look.z * look.z).sqrt();
        let move_h = (v.x * v.x + v.z * v.z).sqrt();
        let gravity = 0.08;
        let lift = f64::from(lean.cos()).powi(2);
        let mut m = Vec3::new(v.x, v.y + gravity * (-1.0 + lift * 0.75), v.z);
        if m.y < 0.0 && look_h > 0.0 {
            let convert = m.y * -0.1 * lift;
            m = Vec3::new(
                m.x + look.x * convert / look_h,
                m.y + convert,
                m.z + look.z * convert / look_h,
            );
        }
        if lean < 0.0 && look_h > 0.0 {
            let convert = move_h * f64::from(-lean.sin()) * 0.04;
            m = Vec3::new(
                m.x - look.x * convert / look_h,
                m.y + convert * 3.2,
                m.z - look.z * convert / look_h,
            );
        }
        if look_h > 0.0 {
            m = Vec3::new(
                m.x + (look.x / look_h * move_h - m.x) * 0.1,
                m.y,
                m.z + (look.z / look_h * move_h - m.z) * 0.1,
            );
        }
        self.player.velocity = Vec3::new(m.x * 0.99, m.y * 0.98, m.z * 0.99);
        let movement = self.player.velocity;
        self.move_self(movement)
    }
    /// `LocalPlayer.move`: the move, then `updateAutoJump` with the float
    /// horizontal displacement.
    pub fn move_self(&mut self, movement: Vec3) -> Result<()> {
        let before = self.player.position;
        entity_move::move_entity(self, MoverType::SelfMovement, movement)?;
        let dx = (self.player.position.x - before.x) as f32;
        let dz = (self.player.position.z - before.z) as f32;
        self.update_auto_jump(dx, dz)
    }
    /// `LocalPlayer.updateAutoJump` behind `canAutoJump`.
    pub fn update_auto_jump(&mut self, xa: f32, za: f32) -> Result<()> {
        let p = &*self.player;
        // canAutoJump; MotionWorld drives an unmounted player (not a passenger).
        let ready = crate::auto_jump::can_auto_jump(
            p.auto_jump_enabled,
            p.auto_jump_time,
            p.ground.on_ground,
            p.shift_key_down,
            false,
            p.move_vector,
            1.0,
        );
        if !ready || f64::from(self.block_jump_factor()?) < 1.0 {
            return Ok(());
        }
        // Player.getSpeed: the movement speed attribute as it is now.
        let speed = self.player.attribute("minecraft:movement_speed")? as f32;
        let p = &*self.player;
        let jumper = crate::auto_jump::AutoJumper {
            position: p.position,
            max_y: p.bounds().max_position().y,
            x_rot: p.pitch,
            y_rot: p.yaw,
            speed,
            move_vector: p.move_vector,
            width: p.dimensions.width,
            height: p.dimensions.height,
            jump_boost: p.status.jump_boost,
        };
        if crate::auto_jump::update_auto_jump(self, &jumper, xa, za)? {
            self.player.auto_jump_time = 1;
        }
        Ok(())
    }
}
impl crate::auto_jump::AutoJumpWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn collision_shape(&mut self, p: BlockPos) -> Result<horizon_core::geometry::VoxelShape> {
        let context = self.context();
        let state = self.block(p);
        let binding = self.collision_binding;
        Ok(binding.collision_shape_with_context(&mut self.collisions(), state, p, &context)?)
    }
    fn collision_boxes(&mut self, bounds: Aabb) -> Result<Vec<Aabb>> {
        let context = self.context();
        let mut scene = self.collisions();
        let mut boxes = Vec::new();
        for hit in horizon_world::block_collision_scan::BlockCollisionScan::new(
            &mut scene, context, bounds, false, 1_000_000,
        )? {
            boxes.extend_from_slice(hit?.shape.boxes());
        }
        Ok(boxes)
    }
}
// Current resident movement domain: dry, standing, no vehicles or gliding.
// The shared kernel also handles swimming/crouching/flight when those states
// are supplied by their eventual runtime bindings.
impl MovementStatisticsWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn passenger(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn swimming(&mut self) -> Result<bool> {
        Ok(self.player.swimming)
    }
    fn eyes_in_water(&mut self) -> Result<bool> {
        Ok(self.player.eye_in_water)
    }
    fn in_water(&mut self) -> Result<bool> {
        Ok(self.player.in_water)
    }
    fn on_climbable(&mut self) -> Result<bool> {
        self.climbable()
    }
    fn on_ground(&mut self) -> Result<bool> {
        Ok(self.player.ground.on_ground)
    }
    fn sprinting(&mut self) -> Result<bool> {
        Ok(self.player.sprinting)
    }
    fn crouching(&mut self) -> Result<bool> {
        Ok(self.player.pose == crate::pose::Pose::Crouching)
    }
    fn fall_flying(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn award(&mut self, stat: MovementStat, amount: i32) -> Result<()> {
        self.player
            .events
            .push(MotionEvent::Health(crate::health::HealthEvent::Statistic {
                name: stat.name(),
                amount,
            }));
        Ok(())
    }
    fn exhaust(&mut self, amount: f32) -> Result<()> {
        self.player.health.food.add_exhaustion(amount);
        Ok(())
    }
}
impl MovementBlockProvider for MotionWorld<'_, '_, '_> {
    type Error = io::Error;
    fn movement_block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        Ok(self.block(p))
    }
}
impl EdgeBackoffWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn max_up_step(&mut self) -> Result<f32> {
        Ok(self.player.attribute("minecraft:step_height")? as f32)
    }
    fn staying_on_ground_surface(&mut self) -> Result<bool> {
        Ok(self.player.shift_key_down)
    }
    fn on_ground(&mut self) -> Result<bool> {
        Ok(self.player.ground.on_ground)
    }
    fn no_collision(&mut self, bounds: Aabb) -> Result<bool> {
        self.no_player_collision(bounds)
    }
}
impl EntityMoveWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn no_physics(&self) -> bool {
        self.player.spectator
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
    fn back_off_edge(&mut self, movement: Vec3, mover: MoverType) -> Result<Vec3> {
        let input = EdgeBackoffInput {
            movement,
            bounds: self.player.bounds(),
            flying: false,
            self_or_player: matches!(mover, MoverType::SelfMovement | MoverType::Player),
            fall_distance: self.player.fall_distance,
            query_budget: 1_000_000,
        };
        edge_backoff::back_off_from_edge(self, input)
            .map_err(|e| io::Error::other(e.to_string()).into())
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
        // Fluid immersion is handled before travel. Tagged reset blocks use
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
        horizon_world::living_landing::check_living_fall_damage(self, dy, grounded, state, position)
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
        <Self as MovementEmissionWorld>::movement_emission(self)
    }
    fn passenger(&mut self) -> Result<bool> {
        Ok(false)
    }
    /// `Entity.applyMovementEmissionAndPlaySound`: a footstep (`Emission`)
    /// only once the walked distance passes `nextStep`. Emitting on every
    /// move played a step sound each tick, even standing still on the
    /// ground (gravity moves the player every tick), as an endless loop.
    fn apply_movement_emission(
        &mut self,
        kind: MovementEmission,
        movement: Vec3,
        position: BlockPos,
        state: BlockStateId,
    ) -> Result<()> {
        emission::apply_movement_emission(self, kind, movement, position, state)
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
// The player's footsteps. Swimming, flapping and the step game event are
// not bound here (the sculk listeners track the player's steps themselves).
impl MovementEmissionWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    type State = BlockStateId;
    fn distances(&self) -> MovementDistances {
        self.player.steps
    }
    fn set_movement_distance(&mut self, value: f32) {
        self.player.steps.movement = value;
    }
    fn set_flight_distance(&mut self, value: f32) {
        self.player.steps.flight = value;
    }
    fn set_next_step(&mut self, value: f32) {
        self.player.steps.next_step = value;
    }
    fn on_position(&mut self) -> Result<BlockPos> {
        self.support(movement_support::ON_SUPPORT_OFFSET)
    }
    fn block_state(&mut self, p: BlockPos) -> Result<BlockStateId> {
        Ok(self.block(p))
    }
    fn is_air(&mut self, s: BlockStateId) -> Result<bool> {
        let name = &self
            .assets
            .registry
            .state(s)
            .ok_or("missing scene state")?
            .name;
        Ok(matches!(
            name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        ))
    }
    fn climbable_tag(&mut self, s: BlockStateId) -> Result<bool> {
        let name = &self
            .assets
            .registry
            .state(s)
            .ok_or("missing scene state")?
            .name;
        Ok(self.resources.climbable.contains(name))
    }
    fn is_powder_snow(&mut self, s: BlockStateId) -> Result<bool> {
        let name = &self
            .assets
            .registry
            .state(s)
            .ok_or("missing scene state")?
            .name;
        Ok(name == "minecraft:powder_snow")
    }
    fn on_ground(&mut self) -> Result<bool> {
        Ok(self.player.ground.on_ground)
    }
    fn crouching(&mut self) -> Result<bool> {
        Ok(self.player.pose == crate::pose::Pose::Crouching)
    }
    fn on_rails(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn swimming(&mut self) -> Result<bool> {
        Ok(self.player.swimming)
    }
    fn in_water(&mut self) -> Result<bool> {
        Ok(self.player.in_water)
    }
    fn walking_step_sound(&mut self, position: BlockPos, state: BlockStateId) -> Result<()> {
        self.player
            .events
            .push(MotionEvent::Emission { position, state });
        Ok(())
    }
    fn step_event(&mut self, _: BlockStateId) -> Result<()> {
        Ok(())
    }
    fn next_step(&mut self) -> Result<f32> {
        Ok(self.player.steps.next_step())
    }
    fn water_swim_sound(&mut self) -> Result<()> {
        Ok(())
    }
    fn entity_event(&mut self, _: MovementEvent) -> Result<()> {
        Ok(())
    }
    fn flapping(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn on_flap(&mut self) -> Result<()> {
        Ok(())
    }
    /// `Player.getMovementEmission`: nothing while flying or sneaking on
    /// the ground.
    fn movement_emission(&mut self) -> Result<MovementEmission> {
        let discreet = self.player.ground.on_ground && self.player.shift_key_down;
        Ok(if self.player.flying || discreet {
            MovementEmission::None
        } else {
            MovementEmission::All
        })
    }
}
impl RestitutionWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn suppressing_bounce(&mut self) -> Result<bool> {
        Ok(self.player.shift_key_down)
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
        self.resources
            .landing
            .bounce_on(
                self,
                state,
                position,
                horizon_world::block_landing::LandingEntityKind::Player,
            )
            .map_err(|e| e.to_string().into())
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
        Ok(self.player.attribute("minecraft:movement_speed")? as f32)
    }
    fn flying_speed(&mut self) -> Result<f32> {
        if self.player.flying {
            // Abilities.flyingSpeed 0.05, doubled when sprinting.
            return Ok(if self.player.sprinting { 0.1 } else { 0.05 });
        }
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
        Ok(self.player.shift_key_down)
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
        Ok(self.player.status.levitation)
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
        Ok(self.player.status.jump_boost.is_some())
    }
    fn jump_boost_amplifier(&mut self) -> Result<i32> {
        self.player
            .status
            .jump_boost
            .ok_or_else(|| "absent jump boost".into())
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
        Ok(self.player.status.slow_falling)
    }
    fn controlling_passenger_is_player(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn speed(&mut self) -> Result<f32> {
        Ok(self.player.attribute("minecraft:movement_speed")? as f32)
    }
    fn omnidirectional_air_mover(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn air_drag_modifier(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:air_drag_modifier")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Footsteps on the prepared vanilla resources, when they are present:
    /// none while standing (gravity still moves the player every tick), one
    /// per `nextStep` metre while walking.
    #[test]
    fn resident_water_and_lava_are_not_treated_as_air() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, 3).unwrap();
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        for x in 19..=21 {
            for y in 3..=7 {
                for z in 19..=21 {
                    scene.set(x, y, z, water).unwrap();
                }
            }
        }
        let mut w = MotionWorld {
            player: &mut player,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
        };
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert!(
            MovementStatisticsWorld::in_water(&mut w).unwrap(),
            "resident water must reach the player movement adapter"
        );
        assert!(MovementStatisticsWorld::eyes_in_water(&mut w).unwrap());
        assert!(
            (w.player.velocity.y + 0.005).abs() < 1e-5,
            "water drag/gravity must replace air travel: {:?}",
            w.player.velocity
        );
        let lava = assets.registry.default_state("minecraft:lava").unwrap();
        for x in 19..=21 {
            for y in 3..=7 {
                for z in 19..=21 {
                    w.scene.set(x, y, z, lava).unwrap();
                }
            }
        }
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert!(
            w.player.health.health() < 20.0,
            "lava must damage a survival player"
        );
        // Water extinguishes, swimming is a real pose, and deep water consumes air.
        for x in 19..=21 {
            for y in 3..=7 {
                for z in 19..=21 {
                    w.scene.set(x, y, z, water).unwrap();
                }
            }
        }
        w.player.fire_ticks = 160;
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert!(w.player.fire_ticks <= 0);
        w.player.air = -19;
        w.player.health.hurt.cooldown = 0;
        let hp = w.player.health.health();
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert_eq!(w.player.air, 0);
        assert!(w.player.health.health() < hp, "drowning must hurt");
        w.player.invulnerable = true;
        let hp = w.player.health.health();
        w.player.air = -19;
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert_eq!(w.player.health.health(), hp);
        w.player.invulnerable = false;
        w.player.air = 300;
        w.controlled_simulation_tick(crate::sprint::Keys {
            forward: true,
            sprint: true,
            ..Default::default()
        })
        .unwrap();
        assert!(w.player.swimming);
        assert_eq!(w.player.pose, crate::pose::Pose::Swimming);
        // Direct fire contact ignites; resistance prevents damage but does not hide flames.
        let fire = assets.registry.default_state("minecraft:fire").unwrap();
        for x in 19..=21 {
            for y in 3..=7 {
                for z in 19..=21 {
                    w.scene.set(x, y, z, fire).unwrap();
                }
            }
        }
        w.player.position = Vec3::new(20.5, 4.0, 20.5);
        w.player.velocity = Vec3::ZERO;
        w.player.fire_ticks = -20;
        w.player.health.hurt.cooldown = 0;
        let hp = w.player.health.health();
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert!(
            w.player.fire_ticks < 0,
            "direct fire hurts during the immunity delay"
        );
        assert!(w.player.health.health() < hp);
        w.player.status.fire_resistance = true;
        w.player.health.hurt.cooldown = 0;
        let hp = w.player.health.health();
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert_eq!(w.player.health.health(), hp);
        w.player.invulnerable = true;
        w.player.fire_ticks = -20;
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert_eq!(
            w.player.fire_ticks, 1,
            "creative fire contact must remain visible"
        );
        w.player.invulnerable = false;
        // Sprint swimming must keep full input, not the dry crawl/sneak multiplier.
        for x in 19..=21 {
            for y in 2..=7 {
                for z in 19..=30 {
                    w.scene.set(x, y, z, water).unwrap();
                }
            }
        }
        w.player.position = Vec3::new(20.5, 4.0, 20.5);
        w.player.velocity = Vec3::ZERO;
        w.player.local_crouching = false;
        w.player.swimming = true;
        w.player.sprinting = true;
        w.player.set_pose(crate::pose::Pose::Swimming).unwrap();
        for _ in 0..30 {
            w.controlled_simulation_tick(crate::sprint::Keys {
                forward: true,
                sprint: true,
                ..Default::default()
            })
            .unwrap();
        }
        assert!(
            w.player.velocity.z > 0.165,
            "native sprint swim speed must not receive crawl slowdown: {:?}",
            w.player.velocity
        );
        assert!(!w.player.moving_slowly());
        w.player.position = Vec3::new(20.5, 4.0, 20.5);
        w.player.velocity = Vec3::ZERO;
        w.player.pitch = 45.0;
        w.controlled_simulation_tick(crate::sprint::Keys {
            forward: true,
            sprint: true,
            ..Default::default()
        })
        .unwrap();
        assert!(
            w.player.velocity.y < -0.04,
            "look down must steer a swimming player downward"
        );
        w.player.position = Vec3::new(20.5, 4.0, 20.5);
        w.player.velocity = Vec3::ZERO;
        w.player.pitch = -45.0;
        w.controlled_simulation_tick(crate::sprint::Keys {
            forward: true,
            sprint: true,
            ..Default::default()
        })
        .unwrap();
        assert!(
            w.player.velocity.y > 0.02,
            "look up must steer a submerged swimmer upward"
        );
        let extra = horizon_world::nbt::Tag::compound([
            ("Fire", horizon_world::nbt::Tag::Short(160)),
            ("Air", horizon_world::nbt::Tag::Short(8)),
        ]);
        let mut pending = true;
        w.player.load_environment_once(&mut pending, Some(&extra));
        assert_eq!((w.player.fire_ticks, w.player.air), (160, 8));
        let mut respawn = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        respawn.load_environment_once(&mut pending, Some(&extra));
        assert_eq!((respawn.fire_ticks, respawn.air), (-20, 300));
        w.player.in_water = false;
        assert!(
            w.player.moving_slowly(),
            "dry crawling must still slow input"
        );
    }

    #[test]
    fn footsteps_follow_the_walked_distance_not_the_tick() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let mut scene = World::test_scene(&assets, 3).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        let mut steps = |player: &mut PlayerMotion, keys: crate::sprint::Keys, ticks: usize| {
            let mut count = 0;
            for _ in 0..ticks {
                MotionWorld {
                    player: &mut *player,
                    scene: &mut scene,
                    assets: &assets,
                    collision_binding: &binding,
                    resources: &resources,
                }
                .controlled_simulation_tick(keys)
                .unwrap();
                count += std::mem::take(&mut player.events)
                    .iter()
                    .filter(|e| matches!(e, MotionEvent::Emission { .. }))
                    .count();
            }
            count
        };
        // Settle onto the floor, then stand still.
        steps(&mut player, Default::default(), 20);
        assert!(player.ground.on_ground);
        assert_eq!(steps(&mut player, Default::default(), 100), 0);
        let start = player.position;
        let forward = crate::sprint::Keys {
            forward: true,
            ..Default::default()
        };
        let walked = steps(&mut player, forward, 100);
        let metres = player.position.subtract(start).length() * 0.6;
        assert!(metres > 5.0, "walked {metres}");
        assert!(
            (walked as f64 - metres).abs() <= 1.5,
            "{walked} steps over {metres} step-metres"
        );
    }
}
