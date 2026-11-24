//! Stored movement state for the resident scene. This binds the validated air,
//! jump, collision, support, restitution and stored health bindings, and
//! powder snow's block-inside effects and freezing. Entity lifecycle, other
//! block-inside effects and audiovisual event consumers remain separate work.
mod player_init;
use crate::{
    Result, assets::Assets, collisions::SceneCollisions, living_attributes::attribute_error,
    world::World,
};
use horizon_core::{
    geometry::{Aabb, Direction, Vec3, VoxelShape, traverse_blocks, trig},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::native_load_context::LoopContext;
use horizon_world::{
    attribute_map::ValueKind,
    attributes::{Modifier, Operation, entity::Catalog, live::LiveAttributes},
    block_collision::BlockCollisionRegistry,
    block_landing::BlockLandingRegistry,
    climbing::{self, ClimbingRegistry, ClimbingWorld},
    collision_context::{CollisionContext, CollisionEntity, CollisionItem},
    collision_exclusions,
    edge_backoff::{self, EdgeBackoffInput, EdgeBackoffWorld},
    entity_move::{self, CollisionFlags, EntityMoveWorld, MovementHistory, MoverType},
    entity_obstruction::EntityIdentity,
    ground_support::{GroundState, find_supporting_block},
    living_air::{self, LivingAirWorld},
    living_fall::ImpulseContext,
    living_motion::{self, LivingMotionWorld},
    move_gate::MoveGate,
    movement_emission::{
        self as emission, MovementDistances, MovementEmission, MovementEmissionWorld, MovementEvent,
    },
    movement_properties::{
        BoundMovementProperties, MovementBlockError, MovementBlockProvider, MovementProperties,
        MovementTags, NoFactorEdits, RegistryMovementWorld,
    },
    movement_restitution::{self, RestitutionInput, RestitutionWorld},
    movement_statistics::{self, MovementStat, MovementStatisticsWorld},
    movement_support::{self, MovementLocation},
    registry::{BlockStateId, BlockTableFile},
    world_movement::MovementRequest,
    worldgen::tags::TagDefinitions,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
    sync::Arc,
};
mod exclusions;
mod restitution;

/// A living entity jumped (`LivingEvent.LivingJumpEvent`).
#[inline(never)]
pub fn native_living_jump(context: &LoopContext, entity: u32, kind: &str) {
    crate::native_client_hooks::living_jump(context, entity, kind);
}

/// A living entity's jump power is about to be used (beyond NeoForge: there is no counterpart).
/// Mods may change it. Fires on replayed ticks too, so a listener must be a pure function of its inputs.
#[inline(never)]
pub fn native_living_jump_power(context: &LoopContext, entity: u32, kind: &str, power: &mut f32) {
    crate::native_client_hooks::living_jump_power(context, entity, kind, power);
}

/// A living entity's movement speed is about to drive its travel (beyond NeoForge: there is no
/// counterpart). Mods may change it. Fires on replayed ticks too, so a listener must be a pure
/// function of its inputs.
#[inline(never)]
pub fn native_living_speed(context: &LoopContext, entity: u32, kind: &str, speed: &mut f32) {
    crate::native_client_hooks::living_speed(context, entity, kind, speed);
}

/// A living entity's airborne acceleration is about to be used (beyond NeoForge: there is no
/// counterpart). Mods may change it. Fires on replayed ticks too, so a listener must be a pure
/// function of its inputs.
#[inline(never)]
pub fn native_living_flying_speed(context: &LoopContext, entity: u32, kind: &str, speed: &mut f32) {
    crate::native_client_hooks::living_flying_speed(context, entity, kind, speed);
}

/// The player's swim admission for a fluid being tested by the movement loop.
#[inline(never)]
pub fn native_can_swim_in_fluid(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    fluid: &str,
    fire_resistance: bool,
    can_swim: &mut bool,
) {
    if let Some(hook) = crate::native_client_hooks::can_swim_in_fluid() {
        hook(context, entity, kind, fluid, fire_resistance, can_swim);
    }
}

/// A fluid travel invocation: the game context and the generated borrowed event arguments.
#[doc(hidden)]
pub struct LivingFluidTravelCall<'a> {
    pub context: &'a LoopContext,
    pub values: mod_api::__private::LivingFluidTravelCall<'a>,
}

/// The player's existing water or lava travel is about to consume these values.
#[inline(never)]
pub fn native_living_fluid_travel(call: LivingFluidTravelCall<'_>) {
    if let Some(hook) = crate::native_client_hooks::living_fluid_travel() {
        hook(call);
    }
}

/// The player's crouch edge backoff asks whether it is staying on the ground surface
/// (`Player.isStayingOnGroundSurface`; beyond NeoForge: there is no counterpart). Mods may change it.
/// Fires on replayed ticks too, so a listener must be a pure function of its inputs.
#[inline(never)]
pub fn native_player_stays_on_ground_surface(
    context: &LoopContext,
    entity: u32,
    staying: &mut bool,
) {
    crate::native_client_hooks::player_stays_on_ground_surface(context, entity, staying);
}

/// The current a fluid block pushes with (`FlowingFluid.getFlow`) is about to push an entity
/// (beyond NeoForge: there is no counterpart). Mods may change it. Fires on replayed ticks too, so a
/// listener must be a pure function of its inputs.
#[inline(never)]
pub fn native_fluid_flow(context: &LoopContext, fluid: &str, pos: BlockPos, flow: &mut [f64; 3]) {
    crate::native_client_hooks::fluid_flow(context, fluid, pos, flow);
}

/// `FlowingFluid.getFlow` as it pushes an entity: the vanilla vector, then `fluid_flow`.
pub(crate) fn pushing_flow<W: horizon_world::fluid_flow::FluidFlowWorld>(
    context: &LoopContext,
    world: &mut W,
    state: horizon_world::fluid::FluidState,
    pos: BlockPos,
) -> std::result::Result<Vec3, W::Error> {
    let flow = horizon_world::fluid_flow::flow_at(world, state, pos)?;
    let mut edited = [flow.x, flow.y, flow.z];
    native_fluid_flow(context, state.kind().name(), pos, &mut edited);
    Ok(Vec3::new(edited[0], edited[1], edited[2]))
}

mod auto_jump;
pub(crate) mod environment;
use environment::fly_into_wall_damage;
mod fluids;
pub mod ladder;
pub mod surface;
pub use fluids::{
    BubbleColumn, FluidResources, FluidTags, LAVA, PlayerFluids, RiddenBoat, WATER,
    bubble_column_at, bubble_column_push, bubble_column_push_over, bubble_column_surfaces,
};
pub(crate) use surface::ModBlockFactors;
pub use surface::{
    block_friction, native_block_jump_factor, native_block_speed_factor, native_friction,
    native_is_ladder,
};

pub struct MotionResources {
    pub attributes: Arc<Catalog>,
    pub blocks: BoundMovementProperties,
    pub climbable: BTreeSet<String>,
    /// The bound BlockTags.CAUSES_SUFFOCATION predicate input.
    pub causes_suffocation: BTreeSet<String>,
    /// `BlockTags.SOUL_SPEED_BLOCKS`: what Soul Speed's effects look for.
    pub soul_speed_blocks: BTreeSet<String>,
    /// `LivingEntity.onClimbable` tags plus open trapdoors above a ladder.
    pub climbing: ClimbingRegistry<'static>,
    pub fall_reset: BTreeSet<String>,
    /// `BlockTags.INSIDE_STEP_SOUND_BLOCKS`.
    pub inside_step_sound: BTreeSet<String>,
    /// `BlockTags.COMBINATION_STEP_SOUND_BLOCKS`.
    pub combination_step_sound: BTreeSet<String>,
    /// `BlockTags.CRYSTAL_SOUND_BLOCKS`: blocks that chime under footsteps.
    pub crystal_sound: BTreeSet<String>,
    pub landing: BlockLandingRegistry,
    pub health: crate::health::HealthResources,
    pub fluids: FluidResources,
}
impl MotionResources {
    /// [`Self::load_with_facts`] with the block facts decoded here.
    pub fn load(root: &Path, assets: &Assets) -> Result<Self> {
        Self::load_with_facts(
            root,
            assets,
            &crate::blocks::BlockFacts::load(root, assets.registry)?,
        )
    }
    /// The motion tables, with the block facts the game already decoded
    /// (decoding `gameplay-facts.json` again costs a second or more and, with
    /// a big modpack, hundreds of megabytes while it runs).
    pub fn load_with_facts(
        root: &Path,
        assets: &Assets,
        facts: &crate::blocks::BlockFacts,
    ) -> Result<Self> {
        Self::load_with_conditions(
            root,
            assets,
            facts,
            &horizon_world::conditions::LoadConditions::vanilla(),
        )
    }
    /// Motion's damage definitions use the owning startup load's inventory.
    pub fn load_with_conditions(
        root: &Path,
        assets: &Assets,
        facts: &crate::blocks::BlockFacts,
        conditions: &horizon_world::conditions::LoadConditions,
    ) -> Result<Self> {
        let attributes = Arc::new(Catalog::from_json(&horizon_world::json_file::read(
            &root.join("attributes.json"),
        )?)?);
        let file = BlockTableFile::open(&root.join("movement-properties.nbt"), assets.registry)?;
        let properties = MovementProperties::from_table(file.table("movement")?)?;
        let bundle: BTreeMap<String, String> =
            horizon_world::json_file::read(&root.join("block-tags.json"))?;
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
        let climbing = ClimbingRegistry::new(
            assets.registry,
            &tag("minecraft:trapdoors")?,
            &tag("minecraft:climbable")?,
            &tag("minecraft:can_glide_through")?,
        )?;
        Ok(Self {
            landing: crate::landing::load(root, &properties)?,
            health: crate::health::HealthResources::load_with_conditions(root, conditions)?,
            fluids: FluidResources::new(assets, facts)?,
            attributes,
            blocks: properties.bind(&MovementTags {
                fences: tag("minecraft:fences")?,
                walls: tag("minecraft:walls")?,
                suppresses_bounce: tag("minecraft:suppresses_bounce")?,
            })?,
            climbable: tag("minecraft:climbable")?,
            causes_suffocation: tag("minecraft:causes_suffocation")?,
            soul_speed_blocks: tag("minecraft:soul_speed_blocks")?,
            climbing,
            fall_reset: tag("minecraft:fall_damage_resetting")?,
            inside_step_sound: tag("minecraft:inside_step_sound_blocks")?,
            combination_step_sound: tag("minecraft:combination_step_sound_blocks")?,
            crystal_sound: tag("minecraft:crystal_sound_blocks")?,
        })
    }
}

/// What the `location_changed` enchantment effects (Soul Speed) need to know
/// between ticks.
#[derive(Clone, Debug, Default)]
pub struct LocationEffects {
    /// `MovementAffectedByPredicate`: the block under the player
    /// (`getBlockPosBelowThatAffectsMyMovement`) is in `#minecraft:soul_speed_blocks`.
    pub on_soul_speed_block: bool,
    /// `LivingEntity.lastPos`: the block the player was in last tick.
    pub block: Option<BlockPos>,
    /// A landing ran `onChangedBlock` (`LivingEntity.checkFallDamage`).
    pub landed: bool,
    /// The Soul Speed level of the boots seen last tick (0: none).
    pub boots_level: u32,
    /// The Soul Speed level whose attribute modifiers are on the player (0: none).
    pub applied_level: u32,
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
    /// `LivingEntity.handleDamageEvent`: the hurt sound the player hears.
    HurtSound(&'static str),
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
    /// `Entity.walkingStepSound` on this block: `Player.playStepSound`
    /// mutes it (`playMuffledStepSound`) under a swim sound in water or
    /// under a combination block's own step.
    Emission {
        position: BlockPos,
        state: BlockStateId,
        muffled: bool,
    },
    /// `Entity.playAmethystStepSound`: `block.amethyst_block.chime`.
    Chime {
        volume: f32,
        pitch: f32,
    },
    /// `Entity.waterSwimSound`: `entity.player.swim`.
    Swim {
        volume: f32,
        pitch: f32,
    },
    Bounce {
        position: BlockPos,
        state: BlockStateId,
        fall_distance: f64,
    },
    /// `PowderSnowBlock.entityInside`'s server particle: one snowflake at
    /// `position` with `velocity` (`RandomizationType.ALTERNATIVE`).
    Snowflake {
        position: Vec3,
        velocity: Vec3,
    },
    /// `ServerPlayer.onInsideBlock`: the enter_block criterion trigger.
    EnterBlock {
        state: BlockStateId,
    },
    /// `NetherPortalBlock.entityInside`: the player's box swept through a
    /// nether portal block.
    InsidePortal {
        pos: BlockPos,
    },
    /// `LayeredCauldronBlock.handleEntityOnFireInside`: a burning player in
    /// a water or powder snow cauldron; the server lowers its fill level.
    CauldronExtinguish {
        pos: BlockPos,
    },
    /// `PowderSnowBlock.entityInside` (runBefore EXTINGUISH): a burning
    /// player in powder snow destroys the block, without drops.
    PowderSnowMelt {
        pos: BlockPos,
    },
    /// `WitherRoseBlock.entityInside`: the player's box swept through a
    /// wither rose; the server adds its Wither unless peaceful.
    InsideWitherRose,
    /// An `entity_inside` part's `EntityWrite::AddEffect`: the server adds the
    /// effect, unless peaceful.
    InsideEffect {
        id: &'static str,
        duration: i32,
        amplifier: u8,
    },
    /// `ServerPlayer.checkFallDamage` after a mace smash: block particles
    /// from the landing block, more the further the fall.
    SmashLanding {
        state: BlockStateId,
        position: BlockPos,
        fall_distance: f64,
    },
    /// `Entity.doWaterSplashEffect`: the player's splash (or high-speed
    /// splash) sound at its position...
    Splash {
        high_speed: bool,
        volume: f32,
        pitch: f32,
    },
    /// ...its BUBBLE (else SPLASH) particles...
    SplashParticle {
        bubble: bool,
        position: Vec3,
        velocity: Vec3,
    },
    /// ...and the SPLASH game event.
    SplashEvent,
    /// `Entity.sendBubbleColumnParticles`: a SPLASH (or BUBBLE) particle
    /// over the top of a bubble column the player rides.
    BubbleColumnParticle {
        bubble: bool,
        position: Vec3,
    },
    /// `HoneyBlock.maybeDoSlideEffects` for a player sliding down the side of
    /// the honey block at `pos`: its slide sound, and the server's entity
    /// event 53 (the five block particles at the player).
    HoneySlide {
        pos: BlockPos,
        sound: bool,
        particles: bool,
    },
    /// Entity event 67 (`LivingEntity.makeDrownParticles`): the bubbles
    /// around a drowning player (8, or the count a mod set).
    Drown(i32),
}
#[derive(Clone)]
pub struct PlayerMotion {
    /// Entity-owned random stream, independent of the level. Like mobs, a new
    /// body seeds it from its initial identity/position without drawing from the
    /// level. It lives with this body (including guest turns), not player NBT.
    pub random: horizon_core::random::EntityRandom,
    pub swimming: bool,
    /// `Entity.isPassenger()`: this body rides a vehicle. `Entity.updateSwimming`
    /// needs it false, so the server's copy of a guest who boards a boat while
    /// swimming is not left in the swimming box in the seat (#4427).
    pub is_passenger: bool,
    pub fire_ticks: i32,
    pub air: i32,
    pub respiration: i32,

    pub position: Vec3,
    pub velocity: Vec3,
    pub ground: GroundState,
    pub collision: CollisionFlags,
    pub history: MovementHistory,
    pub fall_distance: f64,
    /// The block fallOn currently running, consumed by its damage call.
    pub(crate) block_landing: Option<(BlockStateId, BlockPos, f64)>,
    pub stuck_multiplier: Vec3,
    /// `Entity.pistonDeltas` and `pistonDeltasGameTime`: how far a moving
    /// block has pushed this entity this tick (0.51 at most on any axis).
    pub piston: horizon_world::piston_movement::PistonMovement,
    pub yaw: f32,
    pub head_yaw: f32,
    pub jumping: bool,
    pub no_jump_delay: i32,
    pub jump_trigger_time: i32,
    /// Abilities.mayfly / flying (creative flight).
    pub may_fly: bool,
    /// `Player.isMobilityRestricted`: blindness, which keeps a sprint from
    /// starting and stops one (`LocalPlayer.isSprintingPossible`).
    pub mobility_restricted: bool,
    /// Abilities.invulnerable (creative): only damage bypassing invulnerability hurts.
    pub invulnerable: bool,
    /// Spectator mode: no physics, always flying (`Player.tick`).
    pub spectator: bool,
    pub flying: bool,
    /// Client-side `Abilities.flyingSpeed`, adjusted by the spectator scroll wheel.
    pub flying_speed: f32,
    /// The jump key last tick (LocalPlayer.aiStep's wasJumping).
    pub was_jumping: bool,
    /// `LocalPlayer.autoJumpEnabled` (the Auto-Jump option) and
    /// `autoJumpTime`; `was_auto_jump` is this tick's `wasAutoJump`.
    pub auto_jump_enabled: bool,
    pub auto_jump_time: i32,
    pub was_auto_jump: bool,
    /// `input.getMoveVector()` this tick (before `modifyInput`).
    pub move_vector: [f32; 2],
    /// `LocalPlayer.xxa` / `zza`: the move vector after `modifyInput`.
    pub applied_input: [f32; 2],
    /// A usable glider (elytra) is worn; the game sets it each tick.
    pub glider: bool,
    /// The view pitch (xRot), for gliding.
    pub pitch: f32,
    /// Gliding (shared flag 7) and LivingEntity.fallFlyTicks.
    pub fall_flying: bool,
    /// Asleep in a bed (`LivingEntity.isSleeping`): the sleeping pose.
    pub sleeping: bool,
    pub fall_fly_ticks: i32,
    /// The server saw the glide end last tick (`updateFallFlying`'s `canGlide`
    /// failed): the cleared flag reaches the client at its next tick.
    pub glide_ending: bool,
    /// The server owns the glide flag (`Player.setSharedFlag(7, ...)` comes
    /// only from its `DATA_SHARED_FLAGS_ID`): the client of a server never
    /// ends a glide itself, as `LocalPlayer` does not —
    /// `LivingEntity.updateFallFlying` clears the flag on the server only, and
    /// the client applies the synced data when `SetEntityData` is handled.
    pub server_owns_glide: bool,
    /// Glider durability to take this tick (updateFallFlying).
    pub glider_wear: u32,
    /// Soul Speed's `location_changed` bookkeeping.
    pub location_effects: LocationEffects,
    /// `LivingEntity.autoSpinAttackTicks` and `autoSpinAttackDmg`: Riptide's
    /// spin. The damage stays until the game has dealt the hit.
    pub auto_spin_attack_ticks: i32,
    pub auto_spin_attack_dmg: f32,
    /// The stack supplied to Player.startAutoSpinAttack, including its components.
    pub auto_spin_attack_item: Option<crate::items::ItemStack>,
    /// The living entities the game put near a spinning player (id, box).
    pub spin_targets: Vec<(u32, Aabb)>,
    /// The entity the spin touched this tick (`doAutoAttackOnTouch`).
    pub spin_hit: Option<u32>,
    pub sprinting: bool,
    pub sprint_input: crate::sprint::SprintState,
    pub shift_key_down: bool,
    pub local_crouching: bool,
    pub pose: crate::pose::Pose,
    pub dimensions: crate::pose::Dimensions,
    /// What `entity_size` answered for this body's pose and vanilla size (see
    /// `game/modding/entity_sizes.rs`); `refresh_dimensions` keeps it while it holds.
    pub size_answer: Option<crate::pose::SizeAnswer>,
    /// The `minecraft:scale` the dimensions were last built with
    /// (`Entity.refreshDimensions`), for the poses the body takes before it
    /// next refreshes.
    pub scale: f32,
    pub speed: f32,
    /// `LocalPlayer.itemUseSpeedMultiplier` while using an item (not riding).
    pub item_use_speed: Option<f32>,
    /// `LocalPlayer.isSlowDueToUsingItem`: an item in use whose `use_effects`
    /// do not allow sprinting.
    pub item_use_slows_sprint: bool,
    /// `LocalPlayer.moveTowardsClosestSpace`'s new x and z speeds this tick,
    /// worked out by the caller (which knows the blocks); applied before the
    /// move.
    pub nudge: [Option<f64>; 2],
    pub last_climbable: Option<BlockPos>,
    /// Result of the latest `LivingEntity.onClimbable` query.
    pub on_climbable: bool,
    /// `Entity.moveDist`, `flyDist` and `nextStep`: footsteps come once per
    /// metre walked, not once per move.
    pub steps: MovementDistances,
    /// `Entity.crystalSoundIntensity` and `lastCrystalSoundPlayTick`: the
    /// amethyst chime grows with steps taken close together.
    pub crystal_sound_intensity: f32,
    pub last_crystal_sound_play_tick: i32,
    pub needs_sync: bool,
    pub sync_position: bool,
    /// Moved by a command or an ender pearl (`ServerPlayer.teleport`): a server
    /// sends the position packet even when the player lands where they stood.
    /// The server takes it at the end of a guest's turn.
    pub teleported: bool,
    /// The position packet's `Relative.DELTA_*` flags of the teleport that
    /// set `teleported` (`TeleportCommand.performTeleport`): a relative axis
    /// keeps its motion, an absolute one is a zero change. Read with
    /// `teleported` at the end of a guest's turn.
    pub teleported_delta: [bool; 3],
    pub events: Vec<MotionEvent>,
    pub attributes: LiveAttributes<Box<dyn horizon_world::attributes::live::Identities>>,
    pub impulse: ImpulseContext,
    /// `ServerPlayer.spawnExtraParticlesOnFall`: a mace smash on a grounded
    /// target makes the next landing throw up the block below.
    pub spawn_extra_particles_on_fall: bool,
    pub health: crate::health::PlayerHealth,
    /// Status effects and armor protection seen by movement and damage.
    pub status: crate::effects::StatusFlags,
    /// Water and lava around the player (`Entity.fluidInteraction`).
    pub fluids: PlayerFluids,
    /// `Entity.isInPowderSnow` / `wasInPowderSnow`.
    pub in_powder_snow: bool,
    pub was_in_powder_snow: bool,
    /// `Entity.DATA_TICKS_FROZEN`.
    pub ticks_frozen: i32,
    /// Leather boots on the feet (`PowderSnowBlock.canEntityWalkOnPowderSnow`).
    pub leather_boots: bool,
    /// An armor piece in `#freeze_immune_wearables` (`LivingEntity.canFreeze`).
    pub freeze_immune: bool,
    /// A worn enchantment's `damage_immunity` to `#burn_from_stepping` (frost walker).
    pub burn_from_stepping_immune: bool,
    /// `Entity.oldPosition`: where this tick started.
    pub old_position: Vec3,
    /// The server's copy of a player another client moves (a LAN guest), while
    /// it travels on the server: `isLocalInstanceAuthoritative` is false, so
    /// its moves check no fall damage.
    pub server_copy: bool,
    /// A speculative anticheat replay, whose native callbacks must not escape.
    pub(crate) replaying: bool,
    /// The span of `events` a wall hit on the server's copy pushed (sound and
    /// damage): `server_travel` keeps them while it drops the simulated moves.
    pub wall_hit_events: Option<std::ops::Range<usize>>,
    /// A client's copy of its own player (`Level.isClientSide`): only the
    /// server hurts it (`Entity.hurt` does nothing on a client) and ticks its
    /// food (`ServerPlayer.tick`). Its hurt animation comes from the server.
    pub client_side: bool,
    /// The player's entity id (a guest's, set as its turn starts).
    pub entity_id: i32,
    /// The player's client has loaded the world since its last respawn or
    /// level change (`ServerGamePacketListenerImpl.hasClientLoaded`); a guest's
    /// turn takes it from its connection.
    pub client_loaded: bool,
    /// `Entity.applyEffectsFromBlocks` runs after travel (always in the game;
    /// differential fixtures turn it off where their reference does).
    pub block_effects: bool,
    /// The last hurts taken (`hurt_blocking`), newest last, for tests and
    /// the control socket's `hurtlog`.
    pub hurt_log: Vec<crate::health::HurtRecord>,
    /// The last melee attacks made on mobs, newest last.
    pub attack_log: Vec<crate::health::AttackRecord>,
    /// The level's world border, whose walls stop the player's movement
    /// from inside (the game keeps it current; None: no border).
    pub border: Option<horizon_core::geometry::border::BorderGeometry>,
    /// A mod's boxes in which the level's blocks don't stop this player (arismoko/horizon#3717).
    pub collision_exclusions: horizon_world::collision_exclusions::CollisionExclusions,
}
impl PlayerMotion {
    /// `ServerGamePacketListenerImpl.handleMovePlayer`: the movement the client
    /// sent is the server copy's own delta movement. A glide's wall hit reads it
    /// as the speed the glide had (`LivingEntity.handleFallFlyingCollisions`'s
    /// moveHorLength); a copy whose travels start from its own last velocity
    /// otherwise keeps a few hundredths there, and no hit reaches the damage.
    pub fn carry_client_movement(&mut self, movement: Vec3) {
        if self.fall_flying {
            self.velocity = movement;
        }
    }
    /// `Player.isSweepAttack`, using the resident player's known horizontal velocity.
    pub fn is_sweep_attack(&self, strength: f32, critical: bool, sword: bool) -> bool {
        let movement = self.velocity;
        let limit = f64::from(self.speed) * 2.5;
        strength > 0.9
            && !critical
            && !self.sprinting
            && self.ground.on_ground
            && movement.x * movement.x + movement.z * movement.z < limit * limit
            && sword
    }
    /// Restore login state once; a later respawn starts with fresh health and food.
    pub fn load_saved_state_once(
        &mut self,
        pending: &mut bool,
        level: &crate::save::Level,
    ) -> Result<()> {
        if std::mem::take(pending) {
            let extra = level.player_extra.as_ref();
            self.fire_ticks = extra
                .and_then(|t| crate::save::number(t.get("Fire")))
                .unwrap_or(-20) as i32;
            self.air = extra
                .and_then(|t| crate::save::number(t.get("Air")))
                .unwrap_or(300) as i32;
            // Entity.saveWithoutId's `Invulnerable` flag and its
            // `invulnerable_time` window (arismoko/horizon#916).
            self.health.permanently_invulnerable = extra
                .and_then(|t| crate::save::number(t.get("Invulnerable")))
                .is_some_and(|v| v != 0);
            self.health.invulnerable_time = extra
                .and_then(|t| crate::save::number(t.get("invulnerable_time")))
                .unwrap_or(0) as i32;
            self.fall_distance = level.fall_distance;
            // LivingEntity.readAdditionalSaveData applies the saved attributes
            // before it reads the health (a raised max_health keeps its hearts).
            self.apply_saved_attributes(extra.and_then(|t| t.get("attributes")))?;
            self.set_health(level.health, false)?;
            // LivingEntity.readAdditionalSaveData: internalSetAbsorptionAmount.
            self.internal_set_absorption(level.absorption)?;
            self.health.food.food = level.food;
            self.health.food.saturation = level.saturation;
            self.flying = level.flying;
        }
        Ok(())
    }
    pub fn set_remaining_fire_ticks(&mut self, ticks: i32) {
        self.fire_ticks = if self.invulnerable {
            ticks.min(1)
        } else {
            ticks
        };
    }
    pub fn ignite_for_ticks(&mut self, ticks: i32) -> Result<()> {
        let scaled = (f64::from(ticks) * self.attribute("minecraft:burning_time")?).ceil() as i32;
        self.set_remaining_fire_ticks(self.fire_ticks.max(scaled));
        self.ticks_frozen = 0;
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
        let next: Box<dyn horizon_world::attributes::live::Identities> = Box::new(move || {
            identity = identity.wrapping_add(1);
            identity
        });
        Self::with_attribute_identities(resources, position, &hashes, next)
    }

    /// LivingEntity.aiStep: speeds too small to matter stop.
    fn drop_tiny_velocity(&mut self) {
        let v = &mut self.velocity;
        if v.x * v.x + v.z * v.z < 9.0e-6 {
            (v.x, v.z) = (0.0, 0.0);
        }
        if v.y.abs() < 0.003 {
            v.y = 0.0;
        }
    }
    /// `Entity.getTicksRequiredToFreeze`.
    pub const TICKS_REQUIRED_TO_FREEZE: i32 = 140;
    /// `Entity.getPercentFrozen`.
    pub fn percent_frozen(&self) -> f32 {
        self.ticks_frozen.min(Self::TICKS_REQUIRED_TO_FREEZE) as f32
            / Self::TICKS_REQUIRED_TO_FREEZE as f32
    }
    /// `Entity.isFullyFrozen`.
    pub fn is_fully_frozen(&self) -> bool {
        self.ticks_frozen >= Self::TICKS_REQUIRED_TO_FREEZE
    }
    /// `LivingEntity.canFreeze`: not a spectator, no freeze-immune armor.
    pub fn can_freeze(&self) -> bool {
        !self.spectator && !self.freeze_immune
    }
    /// What powder snow's contextual collision asks of the player.
    pub fn powder_snow_subject(&self) -> crate::collisions::PowderSnowSubject<'static> {
        crate::collisions::PowderSnowSubject {
            fall_distance: self.fall_distance,
            leather_boots: self.leather_boots,
            ..Default::default()
        }
    }
    /// `LivingEntity.tryAddFrost`: the frozen slowdown, on anything but air.
    fn try_add_frost(&mut self, on_air: bool) -> Result<()> {
        if on_air || self.ticks_frozen <= 0 {
            return Ok(());
        }
        let Some(key) = self.attributes.key("minecraft:movement_speed") else {
            return Ok(());
        };
        let Some(handle) = self
            .attributes
            .get_instance(Some(key))
            .map_err(attribute_error)?
        else {
            return Ok(());
        };
        let slow = -0.05f32 * self.percent_frozen();
        let modifier = Modifier::new(
            "minecraft:powder_snow",
            f64::from(slow),
            Operation::AddValue,
        )?;
        self.attributes
            .edit(&handle, |value, observer| {
                value.add_transient(modifier, observer)
            })
            .map_err(attribute_error)?;
        Ok(())
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
        self.set_modifier_unclamped(attribute, id, amount, operation, present)?;
        if attribute == "minecraft:max_health" {
            self.clamp_health_to_max()?;
        }
        Ok(())
    }
    /// `set_modifier` without the max_health clamp, for a remove-all/re-add
    /// batch that ends with one `clamp_health_to_max` (vanilla only removes
    /// the modifiers that really went away, so the hearts must survive it).
    pub fn set_modifier_unclamped(
        &mut self,
        attribute: &str,
        id: &str,
        amount: f64,
        operation: &str,
        present: bool,
    ) -> Result<()> {
        crate::living_attributes::set_modifier(
            &mut self.attributes,
            attribute,
            id,
            amount,
            operation,
            present,
        )
    }
    /// `LivingEntity.onAttributeUpdated` for `max_health`: a health above the
    /// new maximum drops to it (#8573: health_boost ending at 24/24 leaves 20).
    pub fn clamp_health_to_max(&mut self) -> Result<()> {
        let max = self.attribute("minecraft:max_health")? as f32;
        let health = self.health.health();
        if health > max {
            self.set_health(max, false)?;
        }
        Ok(())
    }
    /// `AttributeMap.pack` in `AttributeInstance.Packed.LIST_CODEC`'s form: the
    /// player's `attributes` list (LivingEntity.addAdditionalSaveData), each
    /// instance's id, base and permanent modifiers.
    pub fn saved_attributes(&self) -> horizon_core::nbt::Tag {
        self.packed_attributes(true)
    }
    /// `ServerPlayer.restoreFrom`: the attributes a player's new body takes
    /// over from the old one. A body that comes back from a death takes the
    /// base values only (`assignBaseValues`); the same player in another
    /// level (`ServerPlayer.teleport`) is the same entity, with its permanent
    /// modifiers too (`restoreAll`, `assignPermanentModifiers`). A scale of
    /// 0.1 is still 0.1 after a respawn or a trip through a portal (#3684).
    pub fn carried_attributes(&self, restore_all: bool) -> horizon_core::nbt::Tag {
        self.packed_attributes(restore_all)
    }
    fn packed_attributes(&self, modifiers: bool) -> horizon_core::nbt::Tag {
        horizon_core::nbt::Tag::List(crate::living_attributes::pack(&self.attributes, modifiers))
    }
    /// `AttributeMap.apply` of a saved `attributes` list
    /// (LivingEntity.readAdditionalSaveData): each known attribute takes its
    /// saved base and permanent modifiers; unknown ones are skipped.
    pub fn apply_saved_attributes(&mut self, saved: Option<&horizon_core::nbt::Tag>) -> Result<()> {
        crate::living_attributes::apply_saved(&mut self.attributes, saved)
    }
    /// `Player.readAdditionalSaveData`: the movement speed's base is the abilities' walking speed.
    pub fn set_walking_speed_base(&mut self, speed: f32) -> Result<()> {
        let key = self
            .attributes
            .key("minecraft:movement_speed")
            .ok_or("missing movement speed")?;
        let handle = self
            .attributes
            .get_instance(Some(key))
            .map_err(attribute_error)?
            .ok_or("player has no movement speed")?;
        self.attributes
            .edit(&handle, |value, observer| {
                value.set_base_value(f64::from(speed), observer)
            })
            .map_err(attribute_error)?;
        Ok(())
    }
    pub fn set_sprinting(&mut self, sprinting: bool) -> Result<()> {
        self.sprinting = sprinting;
        self.set_sprint_modifier(sprinting)
    }
    /// The movement speed's `minecraft:sprinting` modifier alone, the flag left as it is:
    /// `ClientPacketListener.handleUpdateAttributes` takes the server's modifiers as sent, so a
    /// client can sprint without it.
    pub fn set_sprint_modifier(&mut self, sprinting: bool) -> Result<()> {
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
    /// Whether the movement speed has the `minecraft:sprinting` modifier.
    pub fn has_sprint_modifier(&self) -> bool {
        let Some(key) = self.attributes.key("minecraft:movement_speed") else {
            return false;
        };
        self.attributes
            .lookup(Some(&key))
            .and_then(|h| self.attributes.instance(&h).ok())
            .is_some_and(|i| {
                i.modifiers()
                    .iter()
                    .any(|m| m.id() == "minecraft:sprinting")
            })
    }
    /// Local input sprint phase (`LocalPlayer.aiStep`'s sprint decisions).
    /// Vehicle bindings remain separate; hunger, flight, water, collision, the
    /// item in use and blindness use this actor's actual state.
    pub fn sprint_input_tick(&mut self, keys: crate::sprint::Keys) -> Result<()> {
        self.sprint_input_tick_with_window(keys, 7)
    }
    /// The same phase with Java's `Options.sprintWindow` value.
    pub fn sprint_input_tick_with_window(
        &mut self,
        keys: crate::sprint::Keys,
        sprint_window: i32,
    ) -> Result<()> {
        let conditions = self.sprint_conditions();
        self.run_sprint_tick(keys, conditions, sprint_window)
    }
    /// The same phase for a passenger: `LocalPlayer.isSprintingPossible` asks the
    /// vehicle's `canSprint` and this client's authority there, not food.
    pub fn sprint_input_tick_riding(
        &mut self,
        keys: crate::sprint::Keys,
        sprint_window: i32,
        vehicle_can_sprint: bool,
        vehicle_local: bool,
    ) -> Result<()> {
        let conditions = crate::sprint::Conditions {
            passenger: true,
            vehicle_can_sprint,
            vehicle_local,
            ..self.sprint_conditions()
        };
        self.run_sprint_tick(keys, conditions, sprint_window)
    }
    fn sprint_conditions(&self) -> crate::sprint::Conditions {
        self.sprint_conditions_for_swim(self.fluids.was_touching_water, self.is_under_water())
    }
    fn sprint_conditions_for_swim(
        &self,
        in_water: bool,
        under_water: bool,
    ) -> crate::sprint::Conditions {
        crate::sprint::Conditions {
            sprinting: self.sprinting,
            mobility_restricted: self.mobility_restricted,
            enough_food: self.health.food.has_enough_food(),
            may_fly: self.may_fly,
            flying: self.flying,
            // Entity.isInShallowWater.
            shallow_water: in_water && !under_water,
            fall_flying: self.fall_flying,
            on_ground: self.ground.on_ground,
            horizontal_collision: self.collision.horizontal,
            minor_horizontal_collision: self.collision.minor,
            slow_item: self.item_use_slows_sprint,
            moving_slowly: self.moving_slowly_in(in_water),
            in_water,
            under_water,
            swimming: self.swimming,
            ..Default::default()
        }
    }
    fn run_sprint_tick(
        &mut self,
        keys: crate::sprint::Keys,
        conditions: crate::sprint::Conditions,
        sprint_window: i32,
    ) -> Result<()> {
        // The sprint setter edits attributes and flags, independent of local
        // keyboard history. Retain the input prefix even if that setter fails.
        let mut state = self.sprint_input;
        let result = state.tick(keys, conditions, sprint_window.clamp(0, 10), |value| {
            self.set_sprinting(value)
        });
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

/// Which `Entity.applyEffectsFromBlocks` a sweep of the moves stands for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockPass {
    /// The window's own player, which moves itself.
    Client,
    /// A server's player: the damage and the freezing its client's moves cause.
    ServerDamage,
    /// A server's player, before its landing is checked: only the fall reset
    /// of `makeStuckInBlock`.
    ServerStuck,
}

pub struct MotionWorld<'p, 'w, 'r> {
    /// Entity boxes selected by the host for this player's collision queries.
    pub entities: &'w [Aabb],
    pub player: &'p mut PlayerMotion,
    pub scene: &'w mut World,
    pub assets: &'r Assets,
    pub collision_binding: &'w BlockCollisionRegistry<'r>,
    pub resources: &'w MotionResources,
    /// The owning world's frozen damage registry and reloadable tags.
    pub health: &'w crate::health::HealthResources,
    /// Authoritative merged tags, absent on client prediction.
    pub tag_items: Option<&'w crate::items::Items>,
    /// What the motion value hooks are given: the side that owns this player's
    /// tick builds it (server, or the client for its own prediction and replay).
    pub loop_context: &'w LoopContext,
    /// The native damage hooks the hits of this player's tick run (the game's).
    pub damage_hooks: &'w crate::health::DamageHooks,
    /// The cached block parser table, when available to level hooks.
    pub block_registries: Option<&'w crate::command::args::Registries>,
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
                kind: horizon_world::collision_context::EntityContextKind::Player,
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
            subject: Some(self.player.powder_snow_subject()),
        }
    }
    fn movement_collisions(
        &self,
    ) -> crate::collisions::EntityCollisions<'_, SceneCollisions<'_, '_>> {
        let at = [self.player.position.x, self.player.position.z];
        crate::collisions::EntityCollisions {
            blocks: self.collisions(),
            entities: self.entities,
            border: self.player.border.as_ref().map(|b| (b, at)),
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
                edits: &NoFactorEdits,
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
    /// `Player.onClimbable` (never while flying) over `LivingEntity.onClimbable`
    /// at the player's feet: the climbable tag or an open trapdoor above a
    /// matching ladder. It remembers the block.
    pub fn climbable(&mut self) -> Result<bool> {
        if self.player.flying {
            self.player.on_climbable = false;
            return Ok(false);
        }
        let resources = self.resources;
        let found: Result<bool> =
            climbing::on_climbable(self, &resources.climbing).map_err(|e| e.to_string().into());
        let climbable = found?;
        self.player.on_climbable = climbable;
        Ok(climbable)
    }
    pub fn travel_in_air(&mut self, gate: &mut MoveGate, input: Vec3) -> Result<()> {
        living_air::travel_in_air(self, gate, input)
    }
    /// Dry, standing movement phases of LivingEntity.aiStep, including powder
    /// snow's block-inside effects and server freezing (thawing, the frost
    /// modifier, freeze damage). Equipment, interpolation, effects and other
    /// world/entity effects remain unbound. Inputs are the native
    /// float xxa/yya/zza fields, before LivingEntity.applyInput damping.
    pub fn movement_tick(&mut self, input: [f32; 3]) -> Result<()> {
        self.dry_tick(TickInput::Living(input))
    }
    /// Dry Player.aiStep movement with a prepared local input vector. The
    /// caller applies LocalPlayer input preparation once. Empty inventory,
    /// equipment and nearby-entity sets are the current scene domain.
    pub fn player_movement_tick(&mut self, direction: [f32; 2], jumping: bool) -> Result<()> {
        self.fluid_base_tick()?;
        self.player_movement_tick_with_fluids(direction, jumping)
    }
    fn player_movement_tick_with_fluids(
        &mut self,
        direction: [f32; 2],
        jumping: bool,
    ) -> Result<()> {
        // Entity.baseTick
        self.player.was_in_powder_snow = self.player.in_powder_snow;
        self.player.in_powder_snow = false;
        self.player.old_position = self.player.position;
        self.player
            .peaceful_regeneration(self.loop_context, self.scene.health_rules)?;
        // LocalPlayer.aiStep: double-tap jump toggles creative flight.
        let (was_jumping, toggled, jumped) = self.player.creative_flight_toggle(jumping, true);
        if jumped {
            self.jump_from_ground()?;
        }
        // The server clears the glide flag from the position the client sent
        // last tick, and the client applies it a tick later, before aiStep: a
        // landing glide lasts one more tick on the ground, with no ground
        // friction, so a jump then keeps the speed (the elytra hop).
        // A client of a server takes the flag from the server itself: vanilla's
        // LocalPlayer never ends a glide of its own, and keeping the local
        // prediction there would clear the flag before the server's
        // SetEntityData does (#7616).
        let ending = std::mem::take(&mut self.player.glide_ending);
        if ending && !self.player.server_owns_glide {
            self.player.fall_flying = false;
        }
        // Player.tryToStartFallFlying (canGlide: airborne, not flying, a glider;
        // never from inside a liquid).
        let can_glide = self.player.glider
            && !self.player.ground.on_ground
            && !self.player.flying
            && self.player.status.levitation.is_none()
            && !self.player.fluids.was_touching_water
            && !self.player.is_in_lava();
        // LocalPlayer.aiStep: `!onClimbable()` before tryToStartFallFlying.
        if jumping
            && !toggled
            && !was_jumping
            && !self.climbable()?
            && !self.player.fall_flying
            && can_glide
        {
            self.player.fall_flying = true;
        }
        // LivingEntity.updateFallFlying (fallFlyTicks is counted after the move).
        if self.player.fall_flying {
            // Entity.checkFallDistanceAccumulation: a glide that isn't diving
            // (falling slower than 0.5 a tick) keeps the fall distance at 1, so
            // landing from it hurts no more than a one-block drop.
            if self.player.velocity.y > -0.5 && self.player.fall_distance > 1.0 {
                self.player.fall_distance = 1.0;
            }
            // canGlide alone: water and lava don't end a glide, levitation does.
            let still = self.player.glider
                && !self.player.ground.on_ground
                && !self.player.flying
                && self.player.status.levitation.is_none();
            if !still {
                // The server clears the flag; a client of one waits for its
                // synced data (the SetEntityData handler sets fall_flying).
                if !self.player.server_owns_glide {
                    self.player.glide_ending = true;
                }
            } else if (self.player.fall_fly_ticks + 1) % 10 == 0
                && ((self.player.fall_fly_ticks + 1) / 10) % 2 == 0
            {
                self.player.glider_wear += 1;
            }
        }
        // LocalPlayer.goDownInWater, before LivingEntity.aiStep's velocity cleanup.
        if self.player.shift_key_down && !self.player.flying && self.touching_swimmable_fluid() {
            self.player.velocity.y += f64::from(-0.04f32);
        }
        if self.player.flying {
            // Vertical flight: jump up, shift down, three times the flying speed.
            let ya = i32::from(jumping) - i32::from(self.player.shift_key_down);
            if ya != 0 {
                self.player.velocity.y += f64::from(ya as f32 * self.player.flying_speed * 3.0);
            }
        }
        // LocalPlayer.aiStep: a player inside a block is nudged out of it.
        let [x, z] = std::mem::take(&mut self.player.nudge);
        if let Some(x) = x {
            self.player.velocity.x = x;
        }
        if let Some(z) = z {
            self.player.velocity.z = z;
        }
        self.dry_tick(TickInput::PreparedLocal {
            direction,
            jumping: jumping && !self.player.flying,
        })?;
        // LivingEntity.tick counts fallFlyTicks after aiStep.
        if self.player.fall_flying {
            self.player.fall_fly_ticks += 1;
        } else {
            self.player.fall_fly_ticks = 0;
        }
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
        self.player.health.advance_clock();
        self.fluid_base_tick()?;
        self.simulation_tick_with_fluids(direction, jumping)
    }
    fn simulation_tick_with_fluids(&mut self, direction: [f32; 2], jumping: bool) -> Result<()> {
        self.tick_environment_damage()?;
        let previous = self.player.position;
        self.player_movement_tick_with_fluids(direction, jumping)?;
        self.apply_fire_contact()?;
        self.player.refresh_attributes()?;
        self.update_player_pose()?;
        // The resident scene accepts the resolved local movement directly.
        // Vanilla dispatches this separately from travel, on packet acceptance.
        self.check_movement_statistics(previous.vector_to(self.player.position))?;
        let rules = self.scene.health_rules;
        self.player.food_tick(
            self.loop_context,
            &mut crate::health::DamageLevel::of(
                self.scene,
                self.assets.registry,
                self.tag_items,
                self.block_registries,
                self.damage_hooks,
            ),
            self.health,
            rules,
        )
    }
    pub fn controlled_simulation_tick(&mut self, keys: crate::sprint::Keys) -> Result<()> {
        self.controlled_simulation_tick_with_sprint_window(keys, 7)
    }
    /// Local simulation with the configured double-tap sprint window.
    #[horizon_mod_api_macros::replaceable]
    pub fn controlled_simulation_tick_with_sprint_window(
        &mut self,
        keys: crate::sprint::Keys,
        sprint_window: i32,
    ) -> Result<()> {
        self.local_tick(keys, |world, keys, in_water| {
            let conditions = world
                .player
                .sprint_conditions_for_swim(in_water, world.eyes_in_swimmable_fluid());
            world
                .player
                .run_sprint_tick(keys, conditions, sprint_window)
        })
    }
    /// A client's tick played again by the server's movement prediction: the
    /// client's own keys, and the sprinting it said it had (its
    /// ServerboundPlayerCommandPacket comes before the tick's move).
    pub fn replayed_tick(&mut self, keys: crate::sprint::Keys, sprinting: bool) -> Result<()> {
        self.player.replaying = true;
        let result = match self.try_fast_stationary_replayed_tick(keys, sprinting) {
            Ok(true) => Ok(()),
            Ok(false) => self.local_tick(keys, |world, _, _| {
                if world.player.sprinting == sprinting {
                    Ok(())
                } else {
                    world.player.set_sprinting(sprinting)
                }
            }),
            Err(err) => Err(err),
        };
        self.player.replaying = false;
        result
    }
    fn stationary_border_and_hooks_clear(&self, width: f32, pos: Vec3) -> bool {
        if self
            .player
            .border
            .as_ref()
            .is_some_and(|b| b.distance_to_border(pos.x, pos.z) <= f64::from(width) * 0.5 + 1.0)
        {
            return false;
        }
        !(self.loop_context.side() == horizon_world::native_load_context::LoopSide::Server
            && (mod_api::__private::listened(mod_api::HookId::EntityMove)
                || mod_api::__private::listened(mod_api::HookId::BlockSpeedFactor)
                || mod_api::__private::listened(mod_api::HookId::Friction)
                || mod_api::__private::listened(mod_api::HookId::BlockJumpFactor)
                || mod_api::__private::listened(mod_api::HookId::IsLadder)))
    }
    fn stationary_gravity_and_drag(&mut self, max_vy: f64) -> Result<Option<(f64, f32)>> {
        let vy = self.player.velocity.y;
        if vy > max_vy {
            return Ok(None);
        }
        let gravity = living_motion::effective_gravity(self)?;
        if gravity <= 0.0 || gravity.is_nan() || -vy > gravity {
            return Ok(None);
        }
        let drag = living_motion::air_drag(self)?;
        Ok(Some((gravity, drag)))
    }
    fn stationary_floor_and_body_clear(
        &self,
        bounds: Aabb,
        y_floor: i32,
        require_minecraft_air_body: bool,
    ) -> Result<bool> {
        let min_x = (bounds.min_position().x - 1.0e-7).floor() as i32;
        let max_x = (bounds.max_position().x + 1.0e-7).floor() as i32;
        let min_z = (bounds.min_position().z - 1.0e-7).floor() as i32;
        let max_z = (bounds.max_position().z + 1.0e-7).floor() as i32;
        let max_y = (bounds.max_position().y + 1.0e-7).floor() as i32;
        let unit_cube = Aabb::new(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0));
        let cached = self.collision_binding.cached_shapes();
        for x in min_x..=max_x {
            for z in min_z..=max_z {
                let floor_state = self.scene.block(x, y_floor - 1, z);
                if self.collision_binding.has_context_rule(floor_state) {
                    return Ok(false);
                }
                let Ok(Some(shape)) = cached.cached_shape(floor_state) else {
                    return Ok(false);
                };
                if shape.boxes() != std::slice::from_ref(&unit_cube) {
                    return Ok(false);
                }
                if require_minecraft_air_body {
                    let Some(floor_def) = self.assets.registry.state(floor_state) else {
                        return Ok(false);
                    };
                    if !floor_def.name.starts_with("minecraft:")
                        || matches!(
                            floor_def.name.as_str(),
                            "minecraft:magma_block"
                                | "minecraft:slime_block"
                                | "minecraft:turtle_egg"
                        )
                        || self.resources.soul_speed_blocks.contains(&floor_def.name)
                    {
                        return Ok(false);
                    }
                }
                let props = self.resources.blocks.get(floor_state)?;
                if props.coefficients.friction != 0.6
                    || props.coefficients.speed_factor != 1.0
                    || props.coefficients.bounce_restitution != 0.0
                    || crate::block_contact::restitution_listens(floor_state)
                    || props.fence
                    || props.wall
                    || props.coefficients.fence_gate
                    || props.water
                    || props.bubble_column
                {
                    return Ok(false);
                }
                for y in y_floor..=max_y {
                    let body_state = self.scene.block(x, y, z);
                    if self.collision_binding.may_collide(body_state)
                        || self.collision_binding.has_context_rule(body_state)
                    {
                        return Ok(false);
                    }
                    if require_minecraft_air_body {
                        let Some(body_def) = self.assets.registry.state(body_state) else {
                            return Ok(false);
                        };
                        if !matches!(
                            body_def.name.as_str(),
                            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
                        ) {
                            return Ok(false);
                        }
                    } else {
                        let props = self.resources.blocks.get(body_state)?;
                        if props.coefficients.speed_factor != 1.0
                            || props.water
                            || props.bubble_column
                        {
                            return Ok(false);
                        }
                    }
                }
            }
        }
        Ok(true)
    }
    fn try_fast_stationary_replayed_tick(
        &mut self,
        keys: crate::sprint::Keys,
        sprinting: bool,
    ) -> Result<bool> {
        if keys.forward
            || keys.backward
            || keys.left
            || keys.right
            || keys.jump
            || keys.shift
            || keys.sprint
            || keys.analog.is_some_and(|v| v != [0.0, 0.0])
            || sprinting
            || !self.entities.is_empty()
        {
            return Ok(false);
        }
        let p = &*self.player;
        if !p.client_side
            || !p.collision_exclusions.is_empty()
            || p.sprinting
            || p.spectator
            || p.flying
            || p.fall_flying
            || p.swimming
            || p.is_passenger
            || p.sleeping
            || p.auto_jump_time != 0
            || p.was_jumping
            || p.jump_trigger_time != 0
            || p.shift_key_down
            || p.local_crouching
            || p.pose != crate::pose::Pose::Standing
            || p.health.dead_or_dying()
            || p.nudge != [None; 2]
            || p.glide_ending
            || p.fall_fly_ticks != 0
            || p.fall_distance != 0.0
            || p.no_jump_delay != 0
            || p.stuck_multiplier != Vec3::ZERO
            || p.status.slow_falling
            || p.status.levitation.is_some()
            || p.in_powder_snow
            || p.was_in_powder_snow
            || p.ticks_frozen != 0
            || p.auto_spin_attack_ticks != 0
            || p.air != 300
            || p.fire_ticks != -20
            || p.fluids.first_tick
            || p.fluids.was_touching_water
            || p.fluids.was_eye_in_water
            || p.is_in_lava()
            || !p.fluids.interaction.trackers().is_empty()
            || p.fluids.boat.is_some()
            || !p.ground.on_ground
            || p.ground.on_ground_no_blocks
            || p.velocity.x * p.velocity.x + p.velocity.z * p.velocity.z >= 9.0e-6
            || p.steps.movement > p.steps.next_step
            || !p.attributes.updates().is_empty()
            || !p.history.records().is_empty()
            || p.location_effects.on_soul_speed_block
            || !p.position.x.is_finite()
            || !p.position.y.is_finite()
            || !p.position.z.is_finite()
            || p.position.y != p.position.y.floor()
        {
            return Ok(false);
        }
        let speed_key = p.attributes.key("minecraft:movement_speed");
        if p.attributes.lookup(speed_key.as_ref()).is_none()
            || self
                .player
                .attributes
                .has_modifier(speed_key, "minecraft:powder_snow")
                .map_err(attribute_error)?
        {
            return Ok(false);
        }
        if let Some(bounce_key) = self.player.attributes.key("minecraft:bounciness")
            && self
                .player
                .attributes
                .value(Some(bounce_key), ValueKind::Computed)
                .map_err(attribute_error)?
                != 0.0
        {
            return Ok(false);
        }
        let Some((gravity, drag)) = self.stationary_gravity_and_drag(-0.003)? else {
            return Ok(false);
        };
        let p = &*self.player;
        let position = p.position;
        let bounds = p.bounds();
        if !self.stationary_border_and_hooks_clear(p.dimensions.width, position) {
            return Ok(false);
        }
        let Some(supporting) = self.try_flat_unit_cube_support(true, position, bounds) else {
            return Ok(false);
        };
        // Deflate past `cells_without_collision`'s `1e-7` cell expansion so exact
        // integer feet coordinates (`y = 64.0`) cannot round below `y_floor`.
        if !self
            .scene
            .chunks
            .contains_key(&(supporting.x >> 4, supporting.z >> 4))
            || !self
                .collisions()
                .cells_without_collision(bounds.deflate(Vec3::new(1.0e-6, 1.0e-6, 1.0e-6)))
            || !self.stationary_floor_and_body_clear(bounds, position.y.floor() as i32, true)?
        {
            return Ok(false);
        }
        let speed = self.player.attribute("minecraft:movement_speed")? as f32;
        self.player.health.advance_clock();
        self.player
            .peaceful_regeneration(self.loop_context, self.scene.health_rules)?;
        let p = &mut *self.player;
        p.was_auto_jump = false;
        p.was_in_powder_snow = false;
        p.in_powder_snow = false;
        p.was_jumping = false;
        p.jumping = false;
        p.old_position = position;
        p.move_vector = [0.0, 0.0];
        p.applied_input = [0.0, 0.0];
        p.ground.on_ground = true;
        p.ground.horizontal_collision = false;
        p.ground.on_ground_no_blocks = false;
        p.ground.supporting_block = Some(supporting);
        p.collision = horizon_world::entity_move::CollisionFlags {
            horizontal: false,
            vertical: true,
            below: true,
            minor: false,
        };
        p.on_climbable = false;
        p.fire_ticks = -20;
        p.head_yaw = p.yaw;
        p.speed = speed;
        p.velocity = Vec3::new(0.0, (-gravity) * f64::from(drag), 0.0);
        Ok(true)
    }
    fn try_flat_unit_cube_support(
        &self,
        below: bool,
        position: Vec3,
        bounds: Aabb,
    ) -> Option<BlockPos> {
        if !below
            || !position.x.is_finite()
            || !position.y.is_finite()
            || !position.z.is_finite()
            || position.y != position.y.floor()
        {
            return None;
        }
        let y_floor = position.y.floor() as i32;
        let support_y = y_floor - 1;
        let min_x = (bounds.min_position().x - 1.0e-7).floor() as i32;
        let max_x = (bounds.max_position().x + 1.0e-7).floor() as i32;
        let min_z = (bounds.min_position().z - 1.0e-7).floor() as i32;
        let max_z = (bounds.max_position().z + 1.0e-7).floor() as i32;
        let unit_cube = Aabb::new(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0));
        let cached = self.collision_binding.cached_shapes();
        let mut selected = None;
        let mut closest = f64::MAX;
        let dy = f64::from(support_y) + 0.5 - position.y;
        for x in min_x..=max_x {
            for z in min_z..=max_z {
                let state = self.scene.block(x, support_y, z);
                if self.collision_binding.has_context_rule(state) {
                    return None;
                }
                let Ok(Some(shape)) = cached.cached_shape(state) else {
                    return None;
                };
                if shape.boxes() != std::slice::from_ref(&unit_cube) {
                    return None;
                }
                if f64::from(x) < bounds.max_position().x
                    && f64::from(x + 1) > bounds.min_position().x
                    && f64::from(z) < bounds.max_position().z
                    && f64::from(z + 1) > bounds.min_position().z
                {
                    let p = BlockPos::new(x, support_y, z);
                    let dx = f64::from(x) + 0.5 - position.x;
                    let dz = f64::from(z) + 0.5 - position.z;
                    let distance = dx * dx + dy * dy + dz * dz;
                    if distance < closest
                        || distance == closest
                            && selected.is_none_or(|old| {
                                horizon_world::ground_support::compare_positions(old, p) < 0
                            })
                    {
                        selected = Some(p);
                        closest = distance;
                    }
                }
            }
        }
        selected
    }
    /// `LocalPlayer.aiStep` and the travel it ends with; `sprint` is the
    /// sprint phase.
    fn local_tick(
        &mut self,
        keys: crate::sprint::Keys,
        sprint: impl FnOnce(&mut Self, crate::sprint::Keys, bool) -> Result<()>,
    ) -> Result<()> {
        // The base tick (fluids, swimming) precedes LocalPlayer.aiStep.
        self.player.health.advance_clock();
        self.fluid_base_tick()?;
        self.update_local_crouching()?;
        let mut keys = keys;
        // LocalPlayer.aiStep: a pending auto-jump presses jump for this tick
        // (input.makeJump) and keeps the jump from toggling flight.
        self.player.was_auto_jump = self.player.auto_jump_time > 0;
        if self.player.was_auto_jump {
            self.player.auto_jump_time -= 1;
            keys.jump = true;
        }
        let in_water = self.touching_swimmable_fluid();
        sprint(self, keys, in_water)?;
        self.player.shift_key_down = keys.shift;
        let direction = keys.direction();
        self.player.move_vector = direction;
        let sneak = if direction[0] * direction[0] + direction[1] * direction[1] != 0.0
            && self.player.moving_slowly_in(in_water)
        {
            Some(self.player.attribute("minecraft:sneaking_speed")? as f32)
        } else {
            None
        };
        let modified = crate::input::modify_input(direction, self.player.item_use_speed, sneak);
        self.player.applied_input = modified;
        self.simulation_tick_with_fluids(modified, keys.jump)
    }
    pub fn check_movement_statistics(&mut self, delta: Vec3) -> Result<()> {
        movement_statistics::check(self, delta)
    }
    fn dry_tick(&mut self, input: TickInput) -> Result<()> {
        if self.player.no_jump_delay > 0 {
            self.player.no_jump_delay -= 1;
        }
        self.player.drop_tiny_velocity();
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
        self.jump_phase()?;
        let before_travel = self.player.bounds();
        self.player_travel(input)?;
        if self.player.block_effects {
            self.apply_effects_from_blocks()?;
        }
        // LivingEntity.aiStep "freezing" (server).
        if !self.player.in_powder_snow || !self.player.can_freeze() {
            self.player.ticks_frozen = (self.player.ticks_frozen - 2).max(0);
        }
        self.player.remove_frost()?;
        let on = self.support(movement_support::LEGACY_SUPPORT_OFFSET)?;
        let on_air = self.assets.registry.state(self.block(on)).is_none_or(|s| {
            matches!(
                s.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        });
        self.player.try_add_frost(on_air)?;
        self.update_soul_speed_block()?;
        if self.player.health.tick_count % 40 == 0
            && self.player.is_fully_frozen()
            && self.player.can_freeze()
        {
            self.freeze_damage()?;
        }
        if self.player.auto_spin_attack_ticks > 0 {
            self.player.auto_spin_attack_ticks -= 1;
            self.check_auto_spin_attack(before_travel);
        }
        Ok(())
    }
    /// Refresh whether the player stands on a soul speed block, for the
    /// `location_changed` enchantment effects (a server's player too, whose
    /// client does the walking).
    pub fn update_soul_speed_block(&mut self) -> Result<()> {
        self.player.location_effects.on_soul_speed_block = self.on_soul_speed_block()?;
        Ok(())
    }
    /// `MovementAffectedByPredicate` for `#minecraft:soul_speed_blocks`.
    fn on_soul_speed_block(&mut self) -> Result<bool> {
        let on = self.support(movement_support::MOVEMENT_SUPPORT_OFFSET)?;
        Ok(self
            .assets
            .registry
            .state(self.block(on))
            .is_some_and(|s| self.resources.soul_speed_blocks.contains(&s.name)))
    }
    /// Player.travel: swim steering, then LivingEntity.travel (fluids first,
    /// then gliding, then air), keeping 0.6 of the vertical speed while flying.
    fn player_travel(&mut self, input: Vec3) -> Result<()> {
        // The player's one physical move this tick goes through the move gate
        // (arismoko/horizon#2414): whichever travel branch runs, `move_self` arms the gate it
        // receives here.
        let mut gate = MoveGate::new();
        if self.player.swimming {
            let look_y =
                horizon_world::view_direction::view_vector(self.player.pitch, self.player.yaw).y;
            let near_surface = BlockPos::containing(Vec3::new(
                self.player.position.x,
                self.player.position.y + 1.0 - 0.1,
                self.player.position.z,
            ));
            if look_y <= 0.0
                || self.player.jumping
                || self.fluid_at(near_surface).kind() != horizon_world::fluid::FluidKind::Empty
            {
                let blend = if look_y < -0.2 { 0.085 } else { 0.06 };
                self.player.velocity.y += (look_y - self.player.velocity.y) * blend;
            }
        }
        let flying_y = self.player.flying.then_some(self.player.velocity.y);
        if !self.player.flying
            && (self.player.fluids.was_touching_water || self.player.is_in_lava())
        {
            horizon_world::living_fluid::travel_in_fluid(self, &mut gate, input)?;
        } else if self.player.fall_flying {
            // LivingEntity.travelFallFlying: on a climbable the glide ends and
            // the player moves like in the air.
            if self.climbable()? {
                self.travel_in_air(&mut gate, input)?;
                self.player.fall_flying = false;
            } else {
                self.travel_fall_flying(&mut gate)?;
            }
        } else {
            self.travel_in_air(&mut gate, input)?;
        }
        if let Some(y) = flying_y {
            self.player.velocity.y = y * 0.6;
        }
        Ok(())
    }
    /// ServerPlayer's own travel: the server simulates a player too
    /// (`MoveSimulationType.AUTHORITATIVE_SIDE_AND_SERVER`), with no input
    /// (it never sets xxa/zza). Only the delta movement it leaves stays: it
    /// is what a hurt sends the player (`ServerEntity.sendChanges`), and
    /// what the next knockback starts from; the client's next move places
    /// the player.
    pub fn server_travel(&mut self) -> Result<()> {
        self.player.drop_tiny_velocity();
        if let Some((gravity, drag)) = self.can_fast_ground_server_travel()? {
            self.player.velocity = Vec3::new(0.0, (-gravity) * f64::from(drag), 0.0);
            return Ok(());
        }
        let p = &mut *self.player;
        let (position, ground, collision) = (p.position, p.ground, p.collision);
        let (history, fall_distance, steps) =
            (std::mem::take(&mut p.history), p.fall_distance, p.steps);
        let events = p.events.len();
        p.server_copy = true;
        let travelled = self.player_travel(Vec3::ZERO);
        let p = &mut *self.player;
        p.server_copy = false;
        (p.position, p.ground, p.collision) = (position, ground, collision);
        (p.history, p.fall_distance, p.steps) = (history, fall_distance, steps);
        // The wall hit is the server's own: its sound and damage events stay.
        let kept: Vec<_> = p
            .wall_hit_events
            .take()
            .map(|span| p.events.drain(span).collect())
            .unwrap_or_default();
        p.events.truncate(events);
        p.events.extend(kept);
        travelled
    }
    fn can_fast_ground_server_travel(&mut self) -> Result<Option<(f64, f32)>> {
        let p = &*self.player;
        if !self.entities.is_empty()
            || !p.collision_exclusions.is_empty()
            || p.spectator
            || p.flying
            || p.fall_flying
            || p.swimming
            || p.is_passenger
            || p.sleeping
            || p.jumping
            || p.auto_spin_attack_ticks != 0
            || p.fluids.was_touching_water
            || p.is_in_lava()
            || p.in_powder_snow
            || p.was_in_powder_snow
            || !p.ground.on_ground
            || p.ground.on_ground_no_blocks
            || p.fall_distance != 0.0
            || p.velocity.x != 0.0
            || p.velocity.z != 0.0
            || p.stuck_multiplier != Vec3::ZERO
            || p.status.slow_falling
            || p.status.levitation.is_some()
            || p.position.y != p.position.y.floor()
            || !p.position.x.is_finite()
            || !p.position.y.is_finite()
            || !p.position.z.is_finite()
        {
            return Ok(None);
        }
        let Some((gravity, drag)) = self.stationary_gravity_and_drag(0.0)? else {
            return Ok(None);
        };
        let p = &*self.player;
        let bounds = p.bounds();
        if !self.stationary_border_and_hooks_clear(p.dimensions.width, p.position) {
            return Ok(None);
        }
        let y_floor = p.position.y.floor() as i32;
        let min_x = (bounds.min_position().x - 1.0e-7).floor() as i32;
        let max_x = (bounds.max_position().x + 1.0e-7).floor() as i32;
        let min_z = (bounds.min_position().z - 1.0e-7).floor() as i32;
        let max_z = (bounds.max_position().z + 1.0e-7).floor() as i32;
        if p.ground.supporting_block.is_some_and(|sb| {
            sb.y != y_floor - 1 || sb.x < min_x || sb.x > max_x || sb.z < min_z || sb.z > max_z
        }) {
            return Ok(None);
        }
        if !self.stationary_floor_and_body_clear(bounds, y_floor, false)? || self.climbable()? {
            return Ok(None);
        }
        Ok(Some((gravity, drag)))
    }
    /// `LivingEntity.checkAutoSpinAttack`: the first living entity in the
    /// box swept this tick takes the hit and ends the spin, bouncing the
    /// player back at a fifth of its speed; a wall ends it too.
    fn check_auto_spin_attack(&mut self, before: Aabb) {
        let swept = before.union_bounds(self.player.bounds());
        let p = &mut self.player;
        if let Some(&(id, _)) = p.spin_targets.iter().find(|(_, b)| b.intersects(swept)) {
            p.spin_hit = Some(id);
            p.auto_spin_attack_ticks = 0;
            p.velocity = p.velocity.scale(-0.2);
        } else if p.collision.horizontal {
            p.auto_spin_attack_ticks = 0;
        }
    }
    /// `hurtServer(damageSources().freeze(), 1)` for a fully frozen player.
    fn freeze_damage(&mut self) -> Result<()> {
        let id = self
            .resources
            .health
            .damage
            .id("minecraft:freeze")
            .ok_or("missing freeze damage type")?;
        self.player.events.push(MotionEvent::Health(
            crate::health::HealthEvent::FreezeDamage(1.0),
        ));
        let rules = self.scene.health_rules;
        self.player.hurt(
            &mut crate::health::DamageLevel::of(
                self.scene,
                self.assets.registry,
                self.tag_items,
                self.block_registries,
                self.damage_hooks,
            ),
            self.health,
            rules,
            id,
            1.0,
        )?;
        Ok(())
    }
    /// `Entity.applyEffectsFromBlocks` for the blocks Horizon's player reacts
    /// to inside its movement path: powder snow (`PowderSnowBlock.entityInside`
    /// and the FREEZE / EXTINGUISH effects), cobwebs, sweet berry bushes,
    /// cactus and lit campfires, and the magma and slime blocks it stands on
    /// (`stepOn`). Pressure plates, tripwires and rails are the world tick's
    /// scan.
    pub fn apply_effects_from_blocks(&mut self) -> Result<()> {
        self.effects_from_blocks(BlockPass::Client)
    }
    /// The part of `applyEffectsFromBlocks` a server's player must see before
    /// its landing is checked: `ServerGamePacketListenerImpl.handleMovePlayer`
    /// runs `move` and then `doCheckFallDamage`, and `checkInsideBlocks` lets a
    /// cobweb, a sweet berry bush or powder snow (while the block at its feet
    /// is powder snow) call `makeStuckInBlock`, forgetting the fall, before
    /// that check reads it. Only the fall and the impulse are applied; the
    /// slowing is the client's, and `server_block_damage` does the rest. The
    /// moves stay in the history for it.
    pub fn server_stuck_in_blocks(&mut self, from: Vec3) -> Result<()> {
        self.player.old_position = from;
        self.effects_from_blocks(BlockPass::ServerStuck)
    }
    /// What `applyEffectsFromBlocks` does to a server's player along the
    /// moves its client sent since `from` (in its history): the damage of
    /// magma, cactus, lit campfires and grown sweet berry bushes, and powder
    /// snow's FREEZE, then `LivingEntity.aiStep`'s freezing and its damage.
    /// Its client moves it, so the slowing and the rest are the client's.
    pub fn server_block_damage(&mut self, from: Vec3) -> Result<()> {
        self.player.old_position = from;
        // Entity.baseTick
        self.player.was_in_powder_snow = self.player.in_powder_snow;
        self.player.in_powder_snow = false;
        self.effects_from_blocks(BlockPass::ServerDamage)?;
        // LivingEntity.aiStep "freezing" (server).
        if !self.player.in_powder_snow || !self.player.can_freeze() {
            self.player.ticks_frozen = (self.player.ticks_frozen - 2).max(0);
        }
        if self.player.health.tick_count % 40 == 0
            && self.player.is_fully_frozen()
            && self.player.can_freeze()
        {
            self.freeze_damage()?;
        }
        Ok(())
    }
    fn effects_from_blocks(&mut self, pass: BlockPass) -> Result<()> {
        use horizon_world::entity_move::MovementRecord;
        thread_local! {
            static SCRATCH: std::cell::RefCell<(
                Vec<MovementRecord>,
                crate::fast_hash::FxHashSet<i64>,
                mod_api::InsideEffects,
            )> = std::cell::RefCell::new((
                Vec::new(),
                crate::fast_hash::FxHashSet::default(),
                mod_api::InsideEffects::default(),
            ));
        }
        let damage_only = pass != BlockPass::Client;
        let (mut movements, mut visited, mut effects) = SCRATCH.with(|cell| {
            let mut s = cell.borrow_mut();
            (
                std::mem::take(&mut s.0),
                std::mem::take(&mut s.1),
                std::mem::take(&mut s.2),
            )
        });
        struct Restore(
            Vec<MovementRecord>,
            crate::fast_hash::FxHashSet<i64>,
            mod_api::InsideEffects,
        );
        impl Drop for Restore {
            fn drop(&mut self) {
                self.0.clear();
                self.1.clear();
                let _ = self.2.take_planned();
                if self.0.capacity() <= 256 && self.1.capacity() <= 256 {
                    SCRATCH.with(|cell| {
                        let mut s = cell.borrow_mut();
                        *s = (
                            std::mem::take(&mut self.0),
                            std::mem::take(&mut self.1),
                            std::mem::take(&mut self.2),
                        );
                    });
                }
            }
        }
        movements.clear();
        movements.extend(self.player.history.records().iter().copied());
        if pass != BlockPass::ServerStuck {
            self.player.history.clear();
        }
        let position = self.player.position;
        match movements.last() {
            None => movements.push(MovementRecord {
                from: self.player.old_position,
                to: position,
                original: None,
            }),
            Some(last) if last.to.distance_squared(position) > f64::from(9.999_999_4e-11_f32) => {
                movements.push(MovementRecord {
                    from: last.to,
                    to: position,
                    original: None,
                })
            }
            _ => {}
        }
        // isAffectedByBlocks: no physics for spectators.
        if self.player.spectator {
            drop(Restore(movements, visited, effects));
            return Ok(());
        }
        // Block.stepOn for the block under an entity on the ground:
        // MagmaBlock's hot floor hurts unless it steps carefully (sneaks).
        if self.player.ground.on_ground && pass != BlockPass::ServerStuck {
            let on = self.legacy_on_position()?;
            let under = self.assets.registry.state(self.block(on));
            if let Some(block) = under {
                crate::block_contact::step_on(
                    &block.name,
                    crate::block_contact::Contact {
                        entity: self.player.entity_id,
                        kind: "minecraft:player",
                        state: self.block(on),
                        position: on,
                    },
                );
            }
            let magma = under.is_some_and(|s| s.name == "minecraft:magma_block");
            let slime = under.is_some_and(|s| s.name == "minecraft:slime_block");
            if magma && !self.player.shift_key_down {
                self.environment_hurt("minecraft:hot_floor", 1.0)?;
            }
            // TurtleEggBlock.stepOn: a step that is not careful may break an egg.
            if !self.player.shift_key_down {
                let state = self.block(on);
                self.resources
                    .landing
                    .step_on(
                        self,
                        state,
                        on,
                        horizon_world::block_landing::LandingEntityKind::Player,
                    )
                    .map_err(|e| e.to_string())?;
            }
            // SlimeBlock.stepOn: a slow walk on slime drags the horizontal
            // speed (the client moves its own player, so not the server's copy).
            if slime && !damage_only && !self.player.shift_key_down {
                let abs_delta_y = self.player.velocity.y.abs();
                if abs_delta_y < 0.1 {
                    let scale = 0.4 + abs_delta_y * 0.2;
                    self.player.velocity.x *= scale;
                    self.player.velocity.z *= scale;
                }
            }
        }
        visited.clear();
        let _ = effects.take_planned();
        let sweep_result = {
            let mut inside = PlayerInside {
                m: self,
                pass,
                melt_step: None,
            };
            crate::block_contact::sweep_movements(
                &mut inside,
                &movements,
                &mut visited,
                &mut effects,
            )
        };
        let planned_list = effects.take_planned();
        drop(Restore(movements, visited, effects));
        sweep_result?;
        // `applyAndClear`: the plan applies in order, in this tick, until the
        // player dies. The engine's own effects (the powder snow's freeze and
        // extinguish) and the parts' typed writes flush together.
        for planned in planned_list {
            if self.player.health.dead_or_dying() {
                break;
            }
            use mod_api::{InsideEffect, PlannedInside};
            match planned {
                PlannedInside::Effect(InsideEffect::Freeze) => {
                    self.player.in_powder_snow = true;
                    if self.player.can_freeze() {
                        self.player.ticks_frozen = (self.player.ticks_frozen + 1)
                            .min(PlayerMotion::TICKS_REQUIRED_TO_FREEZE);
                    }
                }
                PlannedInside::Effect(InsideEffect::Extinguish) => {
                    self.player.fire_ticks = self.player.fire_ticks.min(0);
                }
                // The player's own ignition paths (the lava cauldron's arm) run in
                // the walk; no engine block queues the other types for it.
                PlannedInside::Effect(_) => {}
                PlannedInside::Write(write) => self.apply_entity_write(write)?,
            }
        }
        Ok(())
    }
    /// The block effects and the frost of a window's own player while a vehicle carries it. Its
    /// client does not travel (`passenger_pose_tick` is all it runs), but `LivingEntity.aiStep`
    /// still applies the effects of the blocks a passenger is in (`applyEffectsFromBlocks`: powder
    /// snow's FREEZE adds a tick of frost) and thaws it when it is out of powder snow. The
    /// frostbite overlay reads these ticks. The server's copy does this in `server_block_damage`.
    pub fn passenger_block_effects_tick(&mut self) -> Result<()> {
        // Entity.baseTick
        self.player.was_in_powder_snow = self.player.in_powder_snow;
        self.player.in_powder_snow = false;
        if self.player.block_effects {
            self.apply_effects_from_blocks()?;
        }
        // LivingEntity.aiStep "freezing" (server), as the client predicts it on foot.
        if !self.player.in_powder_snow || !self.player.can_freeze() {
            self.player.ticks_frozen = (self.player.ticks_frozen - 2).max(0);
        }
        // The vehicle moves the passenger between ticks: the next sweep starts here.
        self.player.old_position = self.player.position;
        Ok(())
    }
    /// The player's view of the shared inside walk: its box, its life, its
    /// blocks and shapes, and its own `entityInside` moments, which keep their
    /// existing code and queue into the shared collector instead of the
    /// hand-written group flush this replaces.
    fn mod_block_inside(
        &mut self,
        state: BlockStateId,
        p: BlockPos,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) {
        let Some(dimension) = crate::block_contact::current_level() else {
            return;
        };
        if !crate::block_contact::HOOKS
            .get()
            .is_some_and(|hooks| (hooks.inside_listens)(state))
        {
            return;
        }
        // The part is told mid-walk, on the level random host, its whole callback
        // staged into a fresh collector and write list by the fire: only an
        // answering callback's writes join the walk, and a panic there is
        // contained, leaving the player's tick as it was.
        let mut staged = mod_api::InsideEffects::default();
        let mut writes = Vec::new();
        let mut level = crate::health::DamageLevel::of(
            &mut *self.scene,
            self.assets.registry,
            self.tag_items,
            self.block_registries,
            self.damage_hooks,
        );
        if let Some(hooks) = crate::block_contact::HOOKS.get() {
            (hooks.inside_fire)(
                &mut level,
                dimension,
                crate::block_contact::Contact {
                    entity: self.player.entity_id,
                    kind: "minecraft:player",
                    state,
                    position: p,
                },
                precise,
                &mod_api::SweptEntity::new(
                    self.player.position,
                    self.player.old_position,
                    self.player.velocity,
                    self.player.pose == crate::pose::Pose::Crouching,
                    self.player.shift_key_down,
                    self.player.fall_distance,
                    true,
                ),
                &mut staged,
                &mut writes,
            );
            effects.absorb_step(staged);
            crate::block_contact::inside_written(state, p, writes);
        }
    }

    /// A typed entity write an `entity_inside` part queued, applied to this
    /// player inside its own tick, in the collector's flush order.
    fn apply_entity_write(&mut self, write: mod_api::EntityWrite) -> Result<()> {
        let triple = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
        match write {
            mod_api::EntityWrite::Push(v) => {
                self.player.velocity = self.player.velocity.add_vector(triple(v));
            }
            mod_api::EntityWrite::SetMovement(v) => self.player.velocity = triple(v),
            mod_api::EntityWrite::StuckIn(v) => {
                self.make_stuck_in_block(triple(v));
            }
            // The damage source's entity row is the game's to resolve when the
            // write applies; the player's own hurt takes the amount.
            mod_api::EntityWrite::Hurt { amount, .. } => {
                self.environment_hurt("minecraft:generic", amount)?;
            }
            mod_api::EntityWrite::ResetFallDistance => self.player.fall_distance = 0.0,
            mod_api::EntityWrite::SetFireTicks(ticks) => self.player.fire_ticks = ticks,
            mod_api::EntityWrite::AddEffect {
                effect,
                duration,
                amplifier,
            } => {
                if let Some(id) = mod_api::__private::effect_key_id(effect) {
                    self.player.events.push(MotionEvent::InsideEffect {
                        id,
                        duration,
                        amplifier,
                    });
                }
            }
        }
        Ok(())
    }
    /// `HoneyBlock.entityInside`: a falling player touching a side of the block
    /// slides down it (`isSlidingDown`, `doSlideMovement`, `maybeDoSlideEffects`).
    fn honey_block_inside(&mut self, pos: BlockPos) -> Result<()> {
        let old_delta_y = |dy: f64| dy / f64::from(0.98_f32) + 0.08;
        let new_delta_y = |dy: f64| (dy - 0.08) * f64::from(0.98_f32);
        let position = self.player.position;
        let velocity = self.player.velocity;
        if self.player.ground.on_ground
            || position.y > f64::from(pos.y) + 0.9375 - 1.0e-7
            || old_delta_y(velocity.y) >= -0.08
        {
            return Ok(());
        }
        let dx = (f64::from(pos.x) + 0.5 - position.x).abs();
        let dz = (f64::from(pos.z) + 0.5 - position.z).abs();
        let overlap = 0.4375 + f64::from(self.player.dimensions.width / 2.0);
        if !(dx + 1.0e-7 > overlap || dz + 1.0e-7 > overlap) {
            return Ok(());
        }
        // doSlideMovement
        let old = old_delta_y(velocity.y);
        if old < -0.13 {
            let factor = -0.05 / old;
            self.player.velocity =
                Vec3::new(velocity.x * factor, new_delta_y(-0.05), velocity.z * factor);
        } else {
            self.player.velocity = Vec3::new(velocity.x, new_delta_y(-0.05), velocity.z);
        }
        self.player.fall_distance = 0.0;
        // maybeDoSlideEffects: the sound, then the server's event 53.
        let random = &mut self.scene.random;
        let sound = random.next_int_bound(5).map_err(|e| format!("{e:?}"))? == 0;
        // Only the server broadcasts event 53 (`!level.isClientSide()`).
        let particles = !self.player.client_side
            && random.next_int_bound(5).map_err(|e| format!("{e:?}"))? == 0;
        // The server also checks the slide criterion every 20 game ticks,
        // including ticks without a sound or a particle event.
        let criterion = !self.player.client_side && self.scene.game_time % 20 == 0;
        if sound || particles || criterion {
            self.player.events.push(MotionEvent::HoneySlide {
                pos,
                sound,
                particles,
            });
        }
        Ok(())
    }
    /// `Entity.getInBlockState`: whether the block at the player's own position
    /// (its feet) is powder snow.
    fn feet_in_powder_snow(&mut self) -> bool {
        let feet = BlockPos::containing(self.player.position);
        let state = self.assets.registry.state(self.block(feet));
        state.is_some_and(|s| s.name == "minecraft:powder_snow")
    }
    /// `Player.makeStuckInBlock` as a server's player sees it: its client
    /// moves it and applies the slowing, but the fall is forgotten and the
    /// impulse context ends here too.
    fn server_stuck_in_block(&mut self) {
        if !self.player.flying {
            self.player.fall_distance = 0.0;
        }
        self.player.impulse.try_reset();
    }
    /// `Player.makeStuckInBlock`: the next move is scaled by `multiplier`
    /// (and the fall forgotten) unless flying; the impulse context resets
    /// either way.
    fn make_stuck_in_block(&mut self, multiplier: Vec3) {
        if !self.player.flying {
            self.player.fall_distance = 0.0;
            self.player.stuck_multiplier = multiplier;
        }
        self.player.impulse.try_reset();
    }
    /// `LavaCauldronBlock.entityInside`: CLEAR_FREEZE, LAVA_IGNITE
    /// (`Entity.lavaIgnite`, 15 seconds) and then `Entity.lavaHurt` (lava,
    /// 4), as ordinary lava does to the player.
    fn lava_cauldron_inside(&mut self) -> Result<()> {
        self.player.ticks_frozen = 0;
        self.player.ignite_for_ticks(300)?;
        self.environment_hurt("minecraft:lava", 4.0)
    }
    /// `LayeredCauldronBlock.entityInside`: on a server, a burning player
    /// asks for the cauldron's `handleEntityOnFireInside` (runBefore
    /// EXTINGUISH, so while still burning), then EXTINGUISH
    /// (`Entity.clearFire`) puts the fire out.
    fn layered_cauldron_inside(&mut self, pos: BlockPos) {
        if self.player.fire_ticks > 0 && !self.player.spectator {
            self.player
                .events
                .push(MotionEvent::CauldronExtinguish { pos });
        }
        self.player.fire_ticks = self.player.fire_ticks.min(0);
    }
    /// `WebBlock.entityInside`: stuck at a quarter of the speed, half with
    /// the Weaving effect.
    fn web_inside(&mut self) {
        let multiplier = if self.player.status.weaving {
            Vec3::new(0.5, 0.25, 0.5)
        } else {
            Vec3::new(0.25, f64::from(0.05_f32), 0.25)
        };
        self.make_stuck_in_block(multiplier);
    }
    /// `SweetBerryBushBlock.entityInside` for the player: slowed, and pricked
    /// (`sweetBerryBush` damage, 1) by a grown bush while its known movement
    /// is at least 0.003 on X or Z.
    fn sweet_berry_bush_inside(&mut self, state: BlockStateId) -> Result<()> {
        self.make_stuck_in_block(Vec3::new(f64::from(0.8_f32), 0.75, f64::from(0.8_f32)));
        self.sweet_berry_bush_damage(state)
    }
    /// The server's half of `SweetBerryBushBlock.entityInside`: the prick.
    fn sweet_berry_bush_damage(&mut self, state: BlockStateId) -> Result<()> {
        let age = self
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("age"))
            .map_or("0", |a| a.as_str());
        if age == "0" {
            return Ok(());
        }
        // Player.isClientAuthoritative: ServerPlayer.getKnownMovement, this
        // tick's movement.
        let known = self.player.position.subtract(self.player.old_position);
        if known.x * known.x + known.z * known.z > 0.0
            && (known.x.abs() >= f64::from(0.003_f32) || known.z.abs() >= f64::from(0.003_f32))
        {
            let id = self
                .resources
                .health
                .damage
                .id("minecraft:sweet_berry_bush")
                .ok_or("missing sweet_berry_bush damage type")?;
            self.player.events.push(MotionEvent::Health(
                crate::health::HealthEvent::SweetBerryBushDamage(1.0),
            ));
            let rules = self.scene.health_rules;
            self.player.hurt(
                &mut crate::health::DamageLevel::of(
                    self.scene,
                    self.assets.registry,
                    self.tag_items,
                    self.block_registries,
                    self.damage_hooks,
                ),
                self.health,
                rules,
                id,
                1.0,
            )?;
        }
        Ok(())
    }
    /// `PowderSnowBlock.entityInside` for the player: stuck while the block
    /// at its feet is powder snow, with a snowflake now and then (server).
    fn powder_snow_inside(&mut self, p: BlockPos) -> Result<()> {
        if !self.feet_in_powder_snow() {
            return Ok(());
        }
        self.make_stuck_in_block(Vec3::new(f64::from(0.9_f32), 1.5, f64::from(0.9_f32)));
        // `ServerLevel.sendParticles` is server-only (`!level.isClientSide()`).
        if !self.player.client_side {
            self.powder_snow_snowflake(p);
        }
        Ok(())
    }
    /// `PowderSnowBlock.entityInside`'s server particle: one snowflake at
    /// the top of `p` when the player moved horizontally this tick.
    fn powder_snow_snowflake(&mut self, p: BlockPos) {
        if !self.feet_in_powder_snow() {
            return;
        }
        // ServerPlayer.getKnownMovement: this tick's movement.
        let known = self.player.position.subtract(self.player.old_position);
        let random = &mut self.scene.random;
        if (known.x != 0.0 || known.z != 0.0) && random.next_boolean() {
            // Mth.randomBetween(random, -1, 1) * (1 / 12)
            let xa = (random.next_float() * 2.0 - 1.0) * 0.083_333_336_f32;
            let za = (random.next_float() * 2.0 - 1.0) * 0.083_333_336_f32;
            self.player.events.push(MotionEvent::Snowflake {
                position: Vec3::new(
                    self.player.position.x,
                    f64::from(p.y + 1),
                    self.player.position.z,
                ),
                velocity: Vec3::new(f64::from(xa), f64::from(0.05_f32), f64::from(za)),
            });
        }
    }
    pub fn jump_from_ground(&mut self) -> Result<()> {
        living_motion::jump_from_ground(self)?;
        if !self.player.replaying {
            native_living_jump(
                self.loop_context,
                self.player.entity_id as u32,
                "minecraft:player",
            );
        }
        self.player
            .events
            .push(MotionEvent::Health(crate::health::HealthEvent::Statistic {
                name: "jump",
                amount: 1,
            }));
        self.player
            .cause_food_exhaustion(if self.player.sprinting { 0.2 } else { 0.05 });
        Ok(())
    }
    /// LivingEntity.travelFallFlying with updateFallFlyingMovement.
    fn travel_fall_flying(&mut self, gate: &mut MoveGate) -> Result<()> {
        let v = self.player.velocity;
        let move_h = (v.x * v.x + v.z * v.z).sqrt();
        let gravity = living_motion::effective_gravity(self)?;
        self.player.velocity = fall_flying_movement(v, self.player.pitch, self.player.yaw, gravity);
        let movement = self.player.velocity;
        self.move_self(gate, movement)?;
        // LivingEntity.handleFallFlyingCollisions: the fall-damage sound plays
        // on both sides (a glider's own travel is client-side, and the sound
        // must reach its own ears, #8906), only the server hurts, and it
        // compares the horizontal speed before `updateFallFlyingMovement`
        // with the delta `Entity.move` leaves (the collided axes zeroed).
        if self.player.collision.horizontal {
            let v = self.player.velocity;
            let after_h = (v.x * v.x + v.z * v.z).sqrt();
            let dmg = fly_into_wall_damage(move_h, after_h);
            if dmg > 0.0 {
                let first = self.player.events.len();
                // getFallDamageSound((int) dmg)
                let sound = if dmg as i32 > 4 {
                    crate::landing::SceneSound::BigFall
                } else {
                    crate::landing::SceneSound::SmallFall
                };
                self.player
                    .events
                    .push(MotionEvent::Fall(crate::landing::FallEvent::Sound {
                        sound,
                        position: None,
                        volume: 1.0,
                        pitch: 1.0,
                    }));
                // Only the server hurts; the sound is shared.
                if !self.player.client_side {
                    let id = self
                        .resources
                        .health
                        .damage
                        .id("minecraft:fly_into_wall")
                        .ok_or("missing fly_into_wall damage type")?;
                    let rules = self.scene.health_rules;
                    self.player.hurt(
                        &mut crate::health::DamageLevel::of(
                            self.scene,
                            self.assets.registry,
                            self.tag_items,
                            self.block_registries,
                            self.damage_hooks,
                        ),
                        self.health,
                        rules,
                        id,
                        dmg,
                    )?;
                    if self.player.server_copy {
                        self.player.wall_hit_events = Some(first..self.player.events.len());
                    }
                }
            }
        }
        Ok(())
    }
    /// `Entity.move(MoverType.SHULKER_BOX or SHULKER, movement)`: a shulker
    /// box's or a shulker's lid pushed the player. An ordinary move — the
    /// collision scan steps and reads the ground flag — whose end writes the
    /// ground state (`Entity.setOnGroundWithMovement`): a lid lifting the body
    /// off the ground leaves it airborne, and the next tick's gravity lands it
    /// on the lid again instead of the client holding it on a stale ground.
    pub fn lid_push(&mut self, mover: MoverType, movement: Vec3) -> Result<()> {
        let gate = crate::move_gate::armed(
            self.loop_context,
            self.player.entity_id as u32,
            "minecraft:player",
            mover,
            movement,
        );
        if gate.skipped() {
            return Ok(());
        }
        entity_move::move_entity(self, mover, gate.delta())
    }
    /// `LocalPlayer.move`: the move, then `updateAutoJump` with the float
    /// horizontal displacement. The gate is this invocation's (arismoko/horizon#2414), armed
    /// here before the move's own handling, no-physics included.
    pub fn move_self(&mut self, gate: &mut MoveGate, movement: Vec3) -> Result<()> {
        crate::move_gate::arm(
            gate,
            self.loop_context,
            self.player.entity_id as u32,
            "minecraft:player",
            MoverType::SelfMovement,
            movement,
        );
        if gate.skipped() {
            return Ok(());
        }
        let before = self.player.position;
        entity_move::move_entity(self, MoverType::SelfMovement, gate.delta())?;
        let dx = (self.player.position.x - before.x) as f32;
        let dz = (self.player.position.z - before.z) as f32;
        self.update_auto_jump(dx, dz)
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
        // EntityGetter.getEntityCollisions queries with a small inflation, and
        // CollisionGetter.getCollisions returns these before block shapes.
        let query = bounds.inflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7));
        let mut boxes: Vec<_> = self
            .entities
            .iter()
            .copied()
            .filter(|b| b.intersects(query))
            .collect();
        let context = self.context();
        let mut scene = self.collisions();
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
        Ok(self.player.is_eye_in_fluid(WATER))
    }
    fn in_water(&mut self) -> Result<bool> {
        Ok(self.player.fluids.was_touching_water)
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
        self.player.cause_food_exhaustion(amount);
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
        let mut staying = self.player.shift_key_down;
        native_player_stays_on_ground_surface(
            self.loop_context,
            self.player.entity_id as u32,
            &mut staying,
        );
        Ok(staying)
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
    /// `Entity.limitPistonMovement`: the per-tick 0.51-block budget shared by
    /// every move on an axis. A failed game-time read leaves the accumulator
    /// as it was, as native does.
    fn limit_piston_movement(&mut self, movement: Vec3) -> Result<Vec3> {
        let time = self.scene.game_time;
        self.player
            .piston
            .limit(movement, || Ok::<i64, Box<dyn std::error::Error>>(time))
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
            // Player.maybeBackOffFromEdge: not while flying.
            flying: self.player.flying,
            self_or_player: matches!(mover, MoverType::SelfMovement | MoverType::Player),
            fall_distance: self.player.fall_distance,
            query_budget: 1_000_000,
        };
        edge_backoff::back_off_from_edge(self, input)
            .map_err(|e| io::Error::other(e.to_string()).into())
    }
    fn collide(&mut self, movement: Vec3) -> Result<Vec3> {
        let max_step = self.player.attribute("minecraft:step_height")? as f32;
        Ok(collision_exclusions::resolve_world_movement(
            &mut self.movement_collisions(),
            &self.player.collision_exclusions,
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
        // ClipContext.Block.FALLDAMAGE_RESETTING: tagged reset blocks are a full
        // block, not their collider; ClipContext.Fluid.WATER: the water's shape.
        // Both shapes are queried for every visited block (the water's first
        // query fixes its cached shape), either one hitting ends the ray.
        let hit = traverse_blocks(from, to, 1_000_000, |p| {
            let water = match self.water_clip(from, to, p) {
                Ok(hit) => hit,
                Err(e) => return Some(Err(e)),
            };
            let name = &self.assets.registry.state(self.block(p)).unwrap().name;
            let reset = self.resources.fall_reset.contains(name)
                && VoxelShape::block().clip(from, to, p).is_some();
            (water || reset).then_some(Ok(()))
        })?;
        hit.transpose().map(|h| h.is_some())
    }
    fn reset_fall_distance(&mut self) -> Result<()> {
        self.player.fall_distance = 0.0;
        Ok(())
    }
    fn movement_history(&mut self) -> &mut MovementHistory {
        &mut self.player.history
    }
    fn locally_authoritative(&mut self) -> Result<bool> {
        Ok(!self.player.server_copy)
    }
    fn set_on_ground_with_movement(
        &mut self,
        below: bool,
        horizontal: bool,
        movement: Vec3,
    ) -> Result<()> {
        let position = self.player.position;
        let bounds = self.player.bounds();
        if let Some(supporting) = self.try_flat_unit_cube_support(below, position, bounds) {
            self.player.ground.on_ground = below;
            self.player.ground.horizontal_collision = horizontal;
            self.player.ground.on_ground_no_blocks = false;
            self.player.ground.supporting_block = Some(supporting);
            return Ok(());
        }
        let context = self.context();
        let mut collisions = SceneCollisions {
            world: self.scene,
            binding: self.collision_binding,
            subject: Some(self.player.powder_snow_subject()),
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
    /// `LocalPlayer.isHorizontalCollisionMinor`: the move that hit a wall was
    /// within 8 degrees of the direction the input asked for.
    fn minor_horizontal_collision(&mut self, movement: Vec3) -> Result<bool> {
        Ok(horizontal_collision_minor(
            self.player.yaw,
            self.player.applied_input,
            movement,
        ))
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
        // LivingEntity.aiStep: Slow Falling and Levitation reset the fall
        // distance every tick, so landing under either does not hurt.
        if self.player.status.slow_falling || self.player.status.levitation.is_some() {
            self.player.fall_distance = 0.0;
        }
        // LivingEntity.checkFallDamage: a landing runs onChangedBlock.
        if grounded && self.player.fall_distance > 0.0 {
            self.player.location_effects.landed = true;
        }
        if self.player.spawn_extra_particles_on_fall && grounded && self.player.fall_distance > 0.0
        {
            self.player.events.push(MotionEvent::SmashLanding {
                state,
                position,
                fall_distance: self.player.fall_distance,
            });
            self.player.spawn_extra_particles_on_fall = false;
        }
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
        self.player_block_speed_factor()
    }
}
// The player's footsteps and swim sounds. Flapping and the step game event
// are not bound here (the sculk listeners track the player's steps themselves).
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
        Ok(self.player.fluids.was_touching_water)
    }
    /// `Player.playStepSound`: in water a swim sound, then the step muffled;
    /// on land the step of an inside-sound block above the floor, or of a
    /// combination block above it with the floor's step muffled.
    fn walking_step_sound(&mut self, position: BlockPos, state: BlockStateId) -> Result<()> {
        let muffled = self.player.fluids.was_touching_water;
        let mut stepped = false;
        if muffled {
            self.water_swim_sound()?;
        } else {
            // Entity.getPrimaryStepSoundBlockPos.
            let above = BlockPos {
                y: position.y + 1,
                ..position
            };
            let above_state = self.block(above);
            let name = &self
                .assets
                .registry
                .state(above_state)
                .ok_or("missing scene state")?
                .name;
            let combination = self.resources.combination_step_sound.contains(name);
            if combination || self.resources.inside_step_sound.contains(name) {
                self.player.events.push(MotionEvent::Emission {
                    position: above,
                    state: above_state,
                    muffled: false,
                });
                if combination {
                    // Entity.playCombinationStepSounds.
                    self.player.events.push(MotionEvent::Emission {
                        position,
                        state,
                        muffled: true,
                    });
                }
                stepped = true;
            }
        }
        if !stepped {
            self.player.events.push(MotionEvent::Emission {
                position,
                state,
                muffled,
            });
        }
        // Entity.walkingStepSound: the amethyst chime after the step, by the
        // floor's state even when the step sound was the block above's.
        let name = &self
            .assets
            .registry
            .state(state)
            .ok_or("missing scene state")?
            .name;
        let tick = self.player.health.tick_count;
        if self.resources.crystal_sound.contains(name)
            && tick >= self.player.last_crystal_sound_play_tick + 20
        {
            // Entity.playAmethystStepSound (the level random stands in for
            // the entity's).
            let elapsed = f64::from(tick - self.player.last_crystal_sound_play_tick);
            let decayed = self.player.crystal_sound_intensity * 0.997_f64.powf(elapsed) as f32;
            let intensity = 1.0_f32.min(decayed + 0.07);
            self.player.crystal_sound_intensity = intensity;
            let pitch = 0.5 + intensity * self.scene.random.next_float() * 1.2;
            let volume = 0.1 + intensity * 1.2;
            self.player
                .events
                .push(MotionEvent::Chime { volume, pitch });
            self.player.last_crystal_sound_play_tick = tick;
        }
        Ok(())
    }
    fn step_event(&mut self, _: BlockStateId) -> Result<()> {
        Ok(())
    }
    fn next_step(&mut self) -> Result<f32> {
        Ok(self.player.steps.next_step())
    }
    /// `Entity.waterSwimSound` and `playSwimSound` (the level random stands
    /// in for the entity's, as for the splash).
    fn water_swim_sound(&mut self) -> Result<()> {
        let v = self.player.velocity;
        let volume = 1.0_f32.min(
            (v.x * v.x * f64::from(0.2_f32) + v.y * v.y + v.z * v.z * f64::from(0.2_f32)).sqrt()
                as f32
                * 0.35,
        );
        let random = &mut self.scene.random;
        let pitch = 1.0 + (random.next_float() - random.next_float()) * 0.4;
        self.player.events.push(MotionEvent::Swim { volume, pitch });
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
        self.part_bounce_restitution(state)
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
        movement: Vec3,
    ) -> Result<Vec3> {
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
            .map_err(|e| e.to_string())?;
        Ok(crate::block_contact::bounce(
            &mut crate::health::DamageLevel::of(
                &mut *self.scene,
                self.assets.registry,
                self.tag_items,
                self.block_registries,
                self.damage_hooks,
            ),
            crate::block_contact::Contact {
                entity: self.player.entity_id,
                kind: "minecraft:player",
                state,
                position,
            },
            fall_distance,
            movement,
        ))
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
impl LivingMotionWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn velocity(&self) -> Vec3 {
        self.player.velocity
    }
    fn set_velocity(&mut self, v: Vec3) -> Result<()> {
        self.player.velocity = v;
        Ok(())
    }
    fn adjust_jump_power(&mut self, mut power: f32) -> f32 {
        native_living_jump_power(
            self.loop_context,
            self.player.entity_id as u32,
            "minecraft:player",
            &mut power,
        );
        power
    }
    fn jump_strength(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:jump_strength")
    }
    fn block_jump_factor(&mut self) -> Result<f32> {
        let location = self.location();
        let context = self.loop_context;
        Ok(movement_support::block_jump_factor(
            &mut RegistryMovementWorld {
                properties: &self.resources.blocks,
                world: self,
                edits: &ModBlockFactors(context),
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

/// `LocalPlayer.isHorizontalCollisionMinor` for the input `[xxa, zza]`.
fn horizontal_collision_minor(yaw: f32, input: [f32; 2], movement: Vec3) -> bool {
    let rad = yaw * (std::f32::consts::PI / 180.0);
    let sin = f64::from(horizon_core::geometry::trig::sin(f64::from(rad)));
    let cos = f64::from(horizon_core::geometry::trig::cos(f64::from(rad)));
    let (xxa, zza) = (f64::from(input[0]), f64::from(input[1]));
    let global_x = xxa * cos - zza * sin;
    let global_z = zza * cos + xxa * sin;
    let a_len = global_x * global_x + global_z * global_z;
    let m_len = movement.x * movement.x + movement.z * movement.z;
    if a_len < f64::from(1.0e-5_f32) || m_len < f64::from(1.0e-5_f32) {
        return false;
    }
    let dot = global_x * movement.x + global_z * movement.z;
    horizon_core::jmath::acos(dot / (a_len * m_len).sqrt()) < f64::from(0.13962634_f32)
}

/// `LivingEntity.updateFallFlyingMovement`: pure glide velocity shared by
/// player travel and authoritative Mob travel. No collision, state or RNG work.
pub(crate) fn fall_flying_movement(v: Vec3, pitch: f32, yaw: f32, gravity: f64) -> Vec3 {
    // getLookAngle and the lean use Mth's sine table and float degrees,
    // the lift Math.cos of the widened lean.
    let look = horizon_world::view_direction::view_vector(pitch, yaw);
    let lean = pitch * trig::DEG_TO_RAD;
    let look_h = (look.x * look.x + look.z * look.z).sqrt();
    let move_h = (v.x * v.x + v.z * v.z).sqrt();
    let lift = horizon_core::jmath::cos(f64::from(lean)).powi(2);
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
        let convert = move_h * f64::from(-trig::sin(f64::from(lean))) * 0.04;
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
    // multiply(0.99F, 0.98F, 0.99F): the float factors widened.
    Vec3::new(
        m.x * f64::from(0.99_f32),
        m.y * f64::from(0.98_f32),
        m.z * f64::from(0.99_f32),
    )
}

/// The block factors an entity reads for a kind that keeps no motion world of its own (mobs,
/// boats, minecarts): the block at `position`, else the one below it, through the same port the
/// player's move uses, with the `block_jump_factor` and `block_speed_factor` parts applied.
struct WorldBlocks<'a>(&'a World);
impl MovementBlockProvider for WorldBlocks<'_> {
    type Error = ();
    fn movement_block_state(&mut self, p: BlockPos) -> std::result::Result<BlockStateId, ()> {
        Ok(self.0.block(p.x, p.y, p.z))
    }
}

fn block_factor_at(
    world: &World,
    properties: &BoundMovementProperties,
    context: &LoopContext,
    position: Vec3,
    supporting_block: Option<BlockPos>,
    read: impl FnOnce(
        &mut RegistryMovementWorld<'_, WorldBlocks<'_>>,
        movement_support::MovementLocation,
    ) -> std::result::Result<f32, MovementBlockError<()>>,
) -> f32 {
    let location = movement_support::MovementLocation {
        position,
        block_position: BlockPos::new(
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        ),
        supporting_block,
    };
    let mut view = WorldBlocks(world);
    read(
        &mut RegistryMovementWorld {
            properties,
            world: &mut view,
            edits: &ModBlockFactors(context),
        },
        location,
    )
    .unwrap_or(1.0)
}

/// `Entity.getBlockSpeedFactor` for a kind that keeps no motion world of its own.
pub fn block_speed_factor_at(
    world: &World,
    properties: &BoundMovementProperties,
    context: &LoopContext,
    position: Vec3,
    supporting_block: Option<BlockPos>,
) -> f32 {
    block_factor_at(
        world,
        properties,
        context,
        position,
        supporting_block,
        |blocks, location| movement_support::block_speed_factor(blocks, location),
    )
}

/// `Entity.getBlockJumpFactor` for a kind that keeps no motion world of its own.
pub fn block_jump_factor_at(
    world: &World,
    properties: &BoundMovementProperties,
    context: &LoopContext,
    position: Vec3,
    supporting_block: Option<BlockPos>,
) -> f32 {
    block_factor_at(
        world,
        properties,
        context,
        position,
        supporting_block,
        |blocks, location| movement_support::block_jump_factor(blocks, location),
    )
}
/// `Entity.getOnPosLegacy` with the world's bound fence and wall tags.
pub(crate) fn legacy_block_support_at(
    world: &World,
    properties: &BoundMovementProperties,
    position: Vec3,
    supporting_block: Option<BlockPos>,
) -> BlockPos {
    movement_support::support_position(
        &mut RegistryMovementWorld {
            properties,
            edits: &NoFactorEdits,
            world: &mut WorldBlocks(world),
        },
        position,
        supporting_block,
        movement_support::LEGACY_SUPPORT_OFFSET,
    )
    .unwrap_or_else(|_| {
        BlockPos::new(
            position.x.floor() as i32,
            (position.y - f64::from(movement_support::LEGACY_SUPPORT_OFFSET)).floor() as i32,
            position.z.floor() as i32,
        )
    })
}

/// The player's `Entity.checkInsideBlocks` moments on the shared walk: the
/// per-block effects keep their existing code, the powder snow queues the
/// collector's FREEZE and EXTINGUISH where the hand-written group flush was, and
/// the part's contact fires mid-walk with its writes staged.
struct PlayerInside<'a, 'p, 'w, 'r> {
    m: &'a mut MotionWorld<'p, 'w, 'r>,
    pass: BlockPass,
    /// The sweep step that first reached powder snow: its blocks are the melt
    /// group, the ones a burning player melts (`PowderSnowBlock.entityInside`
    /// destroys them while the entity is on fire, per contact).
    melt_step: Option<i32>,
}

impl PlayerInside<'_, '_, '_, '_> {
    /// The client's own player moves itself: the passes a server walks for it
    /// apply the damage and the freezing only.
    fn damage_only(&self) -> bool {
        self.pass != BlockPass::Client
    }
}

impl crate::block_contact::InsideWalk for PlayerInside<'_, '_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn bounds_at(&self, at: Vec3) -> Aabb {
        self.m.player.dimensions.bounds(at)
    }
    fn is_alive(&self) -> bool {
        !self.m.player.health.dead_or_dying()
    }
    fn block(&self, at: BlockPos) -> BlockStateId {
        self.m.scene.block(at.x, at.y, at.z)
    }
    fn is_air(&self, state: BlockStateId) -> bool {
        self.m.assets.registry.state(state).is_none_or(|d| {
            matches!(
                d.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        })
    }
    /// `getEntityInsideCollisionShape` for the player: the whole block but for
    /// powder snow (its own collision shape), the filled cauldrons (their content
    /// column), the end portals and tripwires Horizon doesn't follow it into, and
    /// blocks inside the player's collision exclusions.
    fn inside_shape(
        &mut self,
        state: BlockStateId,
        p: BlockPos,
    ) -> crate::block_contact::InsideShape {
        use crate::block_contact::InsideShape;
        // A block a mod's collision exclusions reach is none to be inside (arismoko/horizon#3717).
        if self.m.player.collision_exclusions.spares(p) {
            return InsideShape::Empty;
        }
        let Some(definition) = self.m.assets.registry.state(state) else {
            return InsideShape::Full;
        };
        let name = definition.name.as_str();
        if matches!(name, "minecraft:end_portal" | "minecraft:tripwire") {
            return InsideShape::Empty;
        }
        let powder_snow = name == "minecraft:powder_snow";
        // A filled cauldron's content top in blocks: the lava's 15/16
        // (`LavaCauldronBlock.FILLED_SHAPE`), or a layered cauldron's
        // `getPixelContentHeight`, 6 + 3 per level (`FILLED_SHAPES`).
        let content_top = match name {
            "minecraft:lava_cauldron" => Some(15.0 / 16.0),
            "minecraft:water_cauldron" | "minecraft:powder_snow_cauldron" => {
                let level = self
                    .m
                    .assets
                    .registry
                    .state(state)
                    .and_then(|s| s.properties.get("level"))
                    .and_then(|l| l.parse::<u8>().ok())
                    .unwrap_or(1);
                Some((6.0 + 3.0 * f64::from(level)) / 16.0)
            }
            _ => None,
        };
        if !(powder_snow || content_top.is_some()) {
            return InsideShape::Full;
        }
        // The powder snow's own collision shape (the whole block when it is
        // empty), or the cauldron's shape with its content on top.
        let context = self.m.context();
        let binding = self.m.collision_binding;
        let Ok(shape) =
            binding.collision_shape_with_context(&mut self.m.collisions(), state, p, &context)
        else {
            // A shape the query cannot answer is a block the walk cannot
            // test: not inside.
            return InsideShape::Empty;
        };
        if powder_snow && (shape.is_empty() || shape.boxes() == VoxelShape::block().boxes()) {
            return InsideShape::Full;
        }
        let offset = Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z));
        let mut boxes: Vec<_> = shape.moved(offset).boxes().to_vec();
        if let Some(top) = content_top {
            // The cauldron's shape or its content, `Block.column(12, 4, top)`.
            boxes.push(Aabb::new(
                Vec3::new(2.0 / 16.0, 4.0 / 16.0, 2.0 / 16.0).add_vector(offset),
                Vec3::new(14.0 / 16.0, top, 14.0 / 16.0).add_vector(offset),
            ));
        }
        InsideShape::Boxes(boxes)
    }
    fn entity_inside(
        &mut self,
        p: BlockPos,
        state: BlockStateId,
        step: i32,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) -> std::result::Result<(), Self::Error> {
        let Some(definition) = self.m.assets.registry.state(state) else {
            return Ok(());
        };
        let name = definition.name.as_str();
        // The server's early pass: only what `makeStuckInBlock` does there,
        // before the landing is checked. `server_stuck_in_blocks` keeps the
        // history for `server_block_damage`, so the part is NOT told here —
        // its one contact a tick, vanilla's `entityInside` count, comes from
        // the passes that follow.
        if self.pass == BlockPass::ServerStuck {
            let snow = name == "minecraft:powder_snow" && self.m.feet_in_powder_snow();
            if snow || matches!(name, "minecraft:cobweb" | "minecraft:sweet_berry_bush") {
                self.m.server_stuck_in_block();
            }
            return Ok(());
        }
        // The part's contact, mid-walk, where vanilla runs the block's own
        // `entityInside`; its writes join this step only when it answered.
        // Client and ServerDamage only: those passes each consume the moves
        // they walk, so no pass pair tells the part twice.
        self.m.mod_block_inside(state, p, precise, effects);
        // CactusBlock, CampfireBlock and the cauldrons' entityInside
        // (`entity.hurt`, the fire and the fill level), and WitherRoseBlock's
        // server-side effect.
        match name {
            "minecraft:cactus" => self.m.environment_hurt("minecraft:cactus", 1.0)?,
            "minecraft:lava_cauldron" => self.m.lava_cauldron_inside()?,
            "minecraft:water_cauldron" | "minecraft:powder_snow_cauldron" => {
                self.m.layered_cauldron_inside(p);
            }
            "minecraft:wither_rose" => {
                self.m.player.events.push(MotionEvent::InsideWitherRose);
            }
            "minecraft:campfire" | "minecraft:soul_campfire" => {
                let lit = self
                    .m
                    .assets
                    .registry
                    .state(state)
                    .and_then(|s| s.properties.get("lit"))
                    .is_some_and(|v| v == "true");
                let damage = if name == "minecraft:campfire" {
                    1.0
                } else {
                    2.0
                };
                if lit {
                    self.m.environment_hurt("minecraft:campfire", damage)?;
                }
            }
            "minecraft:sweet_berry_bush" if self.damage_only() => {
                self.m.sweet_berry_bush_damage(state)?;
            }
            // HoneyBlock.entityInside: a server's player slides too, so its
            // fall distance resets (its client moves it).
            "minecraft:honey_block" => self.m.honey_block_inside(p)?,
            // PowderSnowBlock.entityInside's FREEZE and its server snowflake
            // (`ServerLevel.sendParticles`); EXTINGUISH joins it, as the block's
            // own burn-out does. The slowing is the client's
            // (`makeStuckInBlock`).
            "minecraft:powder_snow" => {
                if self.damage_only() {
                    self.m.powder_snow_snowflake(p);
                } else {
                    self.m.powder_snow_inside(p)?;
                }
                effects.apply(mod_api::InsideEffect::Freeze);
                effects.apply(mod_api::InsideEffect::Extinguish);
                // The burning player melts the snow it is told about, at the
                // contact, as vanilla destroys it there; the collector's
                // extinguish applies at the flush, after every contact.
                if self.m.player.fire_ticks > 0 && self.melt_step.is_none_or(|s| s == step) {
                    self.melt_step.get_or_insert(step);
                    self.m
                        .player
                        .events
                        .push(MotionEvent::PowderSnowMelt { pos: p });
                }
            }
            _ => {}
        }
        // ServerPlayer.onInsideBlock: the server's pass reports it too, as the
        // enter_block trigger is the server's to fire.
        self.m.player.events.push(MotionEvent::EnterBlock { state });
        if self.damage_only() {
            return Ok(());
        }
        match name {
            "minecraft:cobweb" => self.m.web_inside(),
            "minecraft:sweet_berry_bush" => self.m.sweet_berry_bush_inside(state)?,
            "minecraft:bubble_column" => self.m.bubble_column_inside(state, p, precise)?,
            "minecraft:nether_portal" => self
                .m
                .player
                .events
                .push(MotionEvent::InsidePortal { pos: p }),
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
