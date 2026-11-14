// Types that lived in the API root before the domain split.
use crate::*;

/// Why an entity type's attribute declaration or addition was refused
/// ([`Registrar::declare_entity_attributes`](EntityAttributesRegistrar::declare_entity_attributes),
/// [`Registrar::modify_entity_attributes`](EntityAttributesRegistrar::modify_entity_attributes)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum EntityAttributeError {
    /// The entity type's id is invalid or empty, or, for a declared set, in a
    /// namespace the declaring mod does not own.
    InvalidId,
    /// The set names the same attribute twice.
    DuplicateAttribute,
    /// A base value is not finite.
    NotFinite,
    /// Another declaration already covers the entity type.
    Duplicate,
}

/// Why a mod's attribute type declaration was refused
/// ([`Registrar::declare_attribute`](AttributesRegistrar::declare_attribute)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum AttributeRegistrationError {
    /// Only both-side packages may change the attribute registry.
    WrongSide,
    /// The attribute's id is malformed, empty or in a namespace the
    /// declaring mod does not own (a vanilla attribute's included).
    InvalidId,
    /// Another declaration already covers the attribute.
    Duplicate,
    /// The default or a bound is not finite.
    NotFinite,
    /// `min > max`, or the default is not inside `[min, max]`
    /// (`RangedAttribute`'s own checks).
    InvalidBounds,
}

/// An effect requested once through [`Registrar::effect_key`](EffectsRegistrar::effect_key). The registry is fixed until restart.
// no wire: decoding validates the u32 slot with EffectKey::from_slot on the host; wasm carries the host-issued opaque token without consulting guest tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its field is private, so only registration creates a key
pub struct EffectKey(pub(crate) u32);

/// An entity type (`minecraft:zombie`) in the built-in entity-type registry, in
/// registration order. Returned by `EntityRef::kind`; Copy/Eq comparisons do not
/// allocate. Request `key.id(host)` for its namespaced name (an owned diagnostic
/// string). A key stays valid until the game exits.
// The derived scalar codec validates host slots; wasm carries the host-issued token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
#[wire(via = u32)]
// exhaustive: incoming scalar slots are checked against the host's registry
pub struct EntityTypeKey(pub(crate) u32);

impl From<&EntityTypeKey> for u32 {
    fn from(key: &EntityTypeKey) -> Self {
        key.0
    }
}
impl TryFrom<u32> for EntityTypeKey {
    type Error = &'static str;
    fn try_from(slot: u32) -> Result<Self, Self::Error> {
        // A guest transports the host-issued identity; only the host can validate it.
        #[cfg(target_arch = "wasm32")]
        {
            Ok(Self(slot))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self::from_slot(slot)
        }
    }
}

/// A playing sound's channel, for `play_sound_source` and
/// `play_streaming_source`: an id only, to tell sounds apart across the two
/// hooks. It gives no control over the channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ChannelRef {
    /// The channel's id, unique among the live ones.
    pub id: u64,
}

/// A player on the server, as a server hook sees them.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ServerPlayer {
    /// The player's UUID.
    pub uuid: u128,
    /// The player's name.
    pub name: String,
    /// The operator level, 0 to 4 (`ops.json`; single player's owner with
    /// cheats on is 4).
    pub permission: u8,
    /// `minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`.
    pub dimension: String,
    /// The player's position in blocks.
    pub position: [f64; 3],
    /// 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: i32,
    /// Health points, 20 at full; 0 once dead.
    pub health: f32,
    /// Where they look, in degrees (`yRot`, `xRot`), as their last move told
    /// the server.
    pub yaw: f32,
    /// The player's pitch in degrees.
    pub pitch: f32,
    /// The network entity id the player has this session, the one hooks that only know entity ids
    /// (`living_incoming_damage`, `living_knockback`) give for the player; 0 when not known.
    /// API @API:server_player_entity_id@.
    pub entity_id: i32,
    /// The IP address the player connects from, without the port (behind
    /// a Velocity proxy, the address the proxy forwards; empty when not
    /// known).
    pub address: String,
}

/// A persistent entity UUID in the same bit order as the value from `EntityRef::uuid`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
// exhaustive: a UUID is exactly one public 128-bit value
pub struct EntityUuid(pub u128);

/// An entity of the open world, for the `Host` methods that read or change one
/// (`entity.position(host)`). It holds the world's epoch and the entity's id: ids
/// are shared by every server level and never reused while the server world is
/// open, and an entity that changes dimension gets a new one, as in vanilla.
/// On a client the epoch identifies one client level and changes when that level
/// is replaced. Server reads and writes reject a handle from an earlier epoch,
/// of an entity that is gone, or used on a client. `Host::entity` makes a server
/// handle from an event's entity id; `ClientHost::crosshair_target` supplies a
/// client handle for an entity hit.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct EntityRef {
    pub(crate) epoch: WorldEpoch,
    pub(crate) id: u32,
}

impl EntityRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(epoch: WorldEpoch, id: u32) -> Self {
        Self { epoch, id }
    }
    /// The entity's id (the network id events carry).
    pub fn id(self) -> u32 {
        self.id
    }
    /// The open of the world the handle belongs to.
    pub fn epoch(self) -> WorldEpoch {
        self.epoch
    }
}

/// The resolved holder of a mob's or a boat's leash, or its absence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum LeashHolder {
    /// The entity is not leashed.
    Unleashed,
    /// A player, mob or boat holds the leash.
    Entity(EntityRef),
    /// A fence knot holds the leash at this block.
    Knot([i32; 3]),
}

/// A living entity of the open world: a mob, a player, an armor stand or the
/// dragon (vanilla's `LivingEntity`). `host.living(entity)` makes one from an
/// [`EntityRef`] of such an entity, and [`LivingRef::entity`] goes back, so the
/// entity's methods work on it too. Stale handles read as `None`, as an
/// `EntityRef`'s do.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct LivingRef(pub(crate) EntityRef);

impl LivingRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
    /// The entity's id (the network id events carry).
    pub fn id(self) -> u32 {
        self.0.id()
    }
}

impl From<LivingRef> for EntityRef {
    fn from(living: LivingRef) -> Self {
        living.0
    }
}

/// How an attribute modifier changes its attribute
/// (`AttributeModifier.Operation`). The value is the base plus every
/// `AddValue`, then grows by that sum times each `AddMultipliedBase`, then is
/// multiplied by one plus each `AddMultipliedTotal`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
pub enum AttributeOperation {
    /// `add_value`: adds the amount.
    AddValue,
    /// `add_multiplied_base`: adds the amount times the base.
    AddMultipliedBase,
    /// `add_multiplied_total`: multiplies by one plus the amount.
    AddMultipliedTotal,
}

/// An attribute modifier (vanilla's `AttributeModifier`): its id, which is
/// unique on one attribute of one entity, the amount and the operation.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
pub struct AttributeModifier {
    /// The modifier's id (`mymod:sprint_boost`).
    pub id: Identifier,
    /// The amount the operation applies.
    pub amount: f64,
    /// How it applies.
    pub operation: AttributeOperation,
}

impl AttributeModifier {
    /// A modifier with id `id`.
    pub fn new(id: Identifier, amount: f64, operation: AttributeOperation) -> Self {
        Self {
            id,
            amount,
            operation,
        }
    }
}

/// Whether an attribute modifier is saved with its entity
/// (`AttributeInstance.addPermanentModifier`) or lasts until the entity
/// unloads (`addTransientModifier`), as the modifiers of worn items and
/// effects do.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
pub enum ModifierLifetime {
    /// Not saved: gone when the entity is saved and loaded again.
    Transient,
    /// Saved with the entity.
    Permanent,
}

/// A living entity's active effect (vanilla's `MobEffectInstance`), as
/// [`LivingRef`] reads its effects and as `add_effect` gives one: the
/// effect's id, an [`Identifier`] namespaced as the registry has it
/// (`minecraft:speed`, a content mod's `mod:custom`),
/// the ticks left (-1 is infinite), the amplifier from 0, and the flags a
/// beacon-style effect carries: `ambient` (an ambient one gives no
/// particles), `visible` (whether its particles show) and `show_icon`
/// (whether the HUD and inventory list it).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ActiveEffect {
    /// The effect's load-owned registry key; `id.id(host)` reads its canonical name.
    pub id: EffectKey,
    /// The ticks left; -1 is infinite.
    pub duration: i32,
    /// The amplifier, from 0.
    pub amplifier: i32,
    /// `MobEffectInstance.isAmbient`: an ambient effect shows no particles.
    pub ambient: bool,
    /// `MobEffectInstance.isVisible`: whether its particles show.
    pub visible: bool,
    /// `MobEffectInstance.showIcon`: whether the HUD and the inventory list
    /// it. Vanilla keeps it apart from `visible`.
    pub show_icon: bool,
}

impl ActiveEffect {
    /// Construct an effect with its duration, amplifier and visual flags, as
    /// vanilla's six-argument `MobEffectInstance` constructor takes them.
    pub fn new(
        id: EffectKey,
        duration: i32,
        amplifier: i32,
        ambient: bool,
        visible: bool,
        show_icon: bool,
    ) -> Self {
        Self {
            id,
            duration,
            amplifier,
            ambient,
            visible,
            show_icon,
        }
    }
}

/// The answer to `mob_effect_applicable`: whether an effect can land on an
/// entity. An unanswered hook defers to the vanilla tag check; `Allow` and
/// `Deny` override it, as NeoForge's `MobEffectEvent.Applicable.Result`
/// does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum EffectApplicable {
    /// The effect lands, whatever the tags say.
    Allow,
    /// The effect is refused, whatever the tags say.
    Deny,
}

/// A vanilla `Mob`, narrowed from an entity with `host.mob(entity)`.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct MobRef(pub(crate) EntityRef);

impl MobRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an entity handle.
    pub fn entity(self) -> EntityRef {
        self.0
    }
    /// The same entity as a living handle.
    pub fn living(self) -> LivingRef {
        LivingRef::new(self.0)
    }
}

/// A mob the game has made but not yet added to a level (`finalize_spawn`'s and
/// `mob_spawn_position_check`'s mob), lent to one hook call. It is not an [`EntityRef`]: the
/// mob is in no level yet, so no entity lookup finds it. Reads take `&dyn Host`
/// (`mob.kind(host)`); where a hook hands out an [`UnspawnedMobEdits`] with it, that makes the
/// edits. Every read and edit after the hook returns, or with another lend's handle,
/// is refused with [`UnspawnedMobError::Expired`], so a copy kept past the call changes nothing.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct UnspawnedMobRef {
    pub(crate) epoch: WorldEpoch,
    pub(crate) lend: u64,
}

impl UnspawnedMobRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(epoch: WorldEpoch, lend: u64) -> Self {
        Self { epoch, lend }
    }
    /// The open of the world the handle belongs to.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn epoch(self) -> WorldEpoch {
        self.epoch
    }
    /// Which of that world's lends it names.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn lend(self) -> u64 {
        self.lend
    }
}

/// The player, or mobs of the given species: the class filter of a
/// [`GoalSpec::NearestTarget`] and of a [`GoalSpec::AvoidEntity`]. The
/// species ids are namespaced (`minecraft:zombie`); a bare id reads as
/// `minecraft:`'s, like the effects' ids.
#[derive(Clone, Debug)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum MobFilter {
    /// The nearest player.
    Player,
    /// Mobs of these species (`Vec` of namespaced ids).
    Species(Vec<String>),
}

/// A vanilla AI goal, as a mob's `add_goal` and `add_target_goal` give one:
/// only the kinds Horizon's own species code runs, with the vanilla
/// constructor's parameters (`MeleeAttackGoal(speed, followUnseen)`). Goals
/// with behavior of their own (a `MoveToBlockGoal` subclass's block test)
/// stay out: they take code, not parameters.
#[derive(Clone, Debug)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum GoalSpec {
    /// `FloatGoal`: swim up in water or lava. Vanilla parameters: none.
    Float,
    /// `ClimbOnTopOfPowderSnowGoal`: jump onto powder snow. Parameters: none.
    ClimbOnTopOfPowderSnow,
    /// `PanicGoal(speed)`, or `TurtlePanicGoal(speed)` with `water_range`
    /// set: always head for water this far away, not only when on fire.
    Panic {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Search distance for water in blocks, as `TurtlePanicGoal`'s.
        water_range: Option<i32>,
    },
    /// `RandomStrollGoal(speed, interval)`; `water_avoiding` set to its
    /// probability makes `WaterAvoidingRandomStrollGoal`, `swim`
    /// `RandomSwimmingGoal`, `flying` `WaterAvoidingRandomFlyingGoal`.
    Stroll {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Ticks between tries; the goal starts with chance 1 in `interval` each tick.
        interval: i32,
        /// Probability, 0 to 1, of preferring a spot away from water.
        water_avoiding: Option<f32>,
        /// Swim around in water instead of walking.
        swim: bool,
        /// Pick flying destinations.
        flying: bool,
    },
    /// `LookAtPlayerGoal(range, probability, onlyHorizontal)`: the nearest
    /// player, the nearest mob with `mobs`, or only mobs of `only`'s
    /// species.
    LookAtPlayer {
        /// Distance in blocks the goal looks for something to watch.
        range: f32,
        /// Chance, 0 to 1, each tick that the goal starts.
        probability: f32,
        /// Turn the head only sideways, never up or down.
        only_horizontal: bool,
        /// Also watch the nearest mob, not only the nearest player.
        mobs: bool,
        /// A mob type id (`minecraft:cow`) that restricts the mobs watched to that species.
        only: Option<String>,
    },
    /// `RandomLookAroundGoal`. Vanilla parameters: none.
    RandomLookAround,
    /// `MeleeAttackGoal(speed, followUnseen)`.
    MeleeAttack {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Keep chasing a target the mob can no longer see.
        follow_unseen: bool,
    },
    /// `LeapAtTargetGoal(leapMotion)`.
    LeapAtTarget {
        /// The upward speed of the leap, in blocks per tick.
        yd: f32,
    },
    /// `AvoidEntityGoal(what, maxDist, walk, sprint)`: sprint away from
    /// `what` once it is closer than `max_dist`.
    AvoidEntity {
        /// Which entities the mob avoids.
        what: MobFilter,
        /// Distance in blocks within which the mob flees.
        max_dist: f32,
        /// Speed multiplier while fleeing at a distance.
        walk: f64,
        /// Speed multiplier while fleeing close by.
        sprint: f64,
    },
    /// `TemptGoal(speed, items, canScare)`: `items` are item ids
    /// (`minecraft:wheat`; a bare id reads as `minecraft:`'s).
    Tempt {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Item ids that tempt the mob.
        items: Vec<String>,
        /// Whether a moving player holding the item scares the mob off.
        can_scare: bool,
    },
    /// `BreedGoal(speed)`. An animal keeps it; other mobs refuse it.
    Breed {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
    },
    /// `FollowParentGoal(speed)`. An animal keeps it; other mobs refuse it.
    FollowParent {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
    },
    /// `FollowMobGoal(speed, stopDistance, areaSize)`.
    FollowMob {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Distance in blocks at which the mob stops following.
        stop: f32,
        /// Radius in blocks searched for a mob to follow.
        area: f32,
    },
    /// `RestrictSunGoal`. Vanilla parameters: none.
    RestrictSun,
    /// `FleeSunGoal(speed)`.
    FleeSun {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
    },
    /// `EatBlockGoal`. Vanilla parameters: none.
    EatBlock,
    /// `BreathAirGoal`. Vanilla parameters: none.
    BreathAir,
    /// `TryFindWaterGoal`. Vanilla parameters: none.
    TryFindWater,
    /// `NearestAttackableTargetGoal` (the target selector): hunt `prey`,
    /// checking every `interval` ticks, `must_see` (`mustSee`) and
    /// `must_reach` (`mustReach`).
    NearestTarget {
        /// Which entities the mob hunts.
        prey: MobFilter,
        /// Only pick a target the mob can see.
        must_see: bool,
        /// Only pick a target the mob can path to.
        must_reach: bool,
        /// Ticks between tries; the goal starts with chance 1 in `interval` each tick.
        interval: i32,
    },
    /// `HurtByTargetGoal`, `alert` as its `setAlert()`.
    HurtByTarget {
        /// Also alert nearby mobs of the same kind, as `setAlert()`.
        alert: bool,
    },
}

impl GoalSpec {
    /// The goal's kind id: the same snake_case spelling `goals` and
    /// `target_goals` read and `remove_goal` and `remove_target_goal` take
    /// (`melee_attack`). A species variant of a vanilla goal (a bee's
    /// `BeeHurtByOtherGoal`) keeps its own vanilla name instead.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::ClimbOnTopOfPowderSnow => "climb_on_top_of_powder_snow",
            Self::Panic { .. } => "panic",
            Self::Stroll { .. } => "stroll",
            Self::LookAtPlayer { .. } => "look_at_player",
            Self::RandomLookAround => "random_look_around",
            Self::MeleeAttack { .. } => "melee_attack",
            Self::LeapAtTarget { .. } => "leap_at_target",
            Self::AvoidEntity { .. } => "avoid_entity",
            Self::Tempt { .. } => "tempt",
            Self::Breed { .. } => "breed",
            Self::FollowParent { .. } => "follow_parent",
            Self::FollowMob { .. } => "follow_mob",
            Self::RestrictSun => "restrict_sun",
            Self::FleeSun { .. } => "flee_sun",
            Self::EatBlock => "eat_block",
            Self::BreathAir => "breath_air",
            Self::TryFindWater => "try_find_water",
            Self::NearestTarget { .. } => "nearest_target",
            Self::HurtByTarget { .. } => "hurt_by_target",
        }
    }
}

/// One goal of a mob's selector, as `goals` and `target_goals` read it.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct MobGoalEntry {
    /// The goal's priority: a lower number preempts a higher one.
    pub priority: i32,
    /// The goal's kind id (see [`GoalSpec::id`]).
    pub id: &'static str,
    /// Whether the goal is running now.
    pub running: bool,
}

/// The ender dragon of the open world, narrowed with `host.dragon(entity)`.
/// It is also a [`MobRef`] and a [`LivingRef`].
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct DragonRef(pub(crate) EntityRef);

impl DragonRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
    /// The same entity as a mob handle.
    pub fn mob(self) -> MobRef {
        MobRef::new(self.0)
    }
}

/// An ender dragon phase (`EnderDragonPhase`), in vanilla's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DragonPhase {
    /// Circling the end island (`DragonPhaseHoldingPattern`).
    HoldingPattern,
    /// Flying at a player and shooting fireballs.
    StrafePlayer,
    /// Flying toward the fountain to land.
    LandingApproach,
    /// Descending onto the fountain.
    Landing,
    /// Lifting off the fountain.
    Takeoff,
    /// Perched, breathing fire.
    SittingFlaming,
    /// Perched, looking for a target.
    SittingScanning,
    /// Perched, about to breathe.
    SittingAttacking,
    /// Charging a player in melee.
    ChargingPlayer,
    /// Dying, flying to the fountain.
    Dying,
    /// Hovering in place.
    Hover,
}

impl DragonPhase {
    const ALL: [Self; 11] = [
        Self::HoldingPattern,
        Self::StrafePlayer,
        Self::LandingApproach,
        Self::Landing,
        Self::Takeoff,
        Self::SittingFlaming,
        Self::SittingScanning,
        Self::SittingAttacking,
        Self::ChargingPlayer,
        Self::Dying,
        Self::Hover,
    ];
    /// The phase's vanilla id, as the `DragonPhase` tag and the synced data
    /// carry it.
    pub fn id(self) -> i32 {
        self as i32
    }
    /// The phase with a vanilla id (`EnderDragonPhase.getById`).
    pub fn from_id(id: i32) -> Option<Self> {
        usize::try_from(id)
            .ok()
            .and_then(|i| Self::ALL.get(i))
            .copied()
    }
}

/// A dropped item entity of the open world, narrowed with `host.item_entity`.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ItemEntityRef(pub(crate) EntityRef);

impl ItemEntityRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
}

/// A display entity of the open world (`block_display`, `item_display` or
/// `text_display`), narrowed with `host.display`.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct DisplayRef(pub(crate) EntityRef);

impl DisplayRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
}

/// A projectile of the open world (vanilla's `Projectile`), narrowed with
/// `host.projectile`: arrows, tridents (a player's too), firework rockets,
/// thrown items, fireballs, wind charges, skulls, shulker bullets and mods'
/// thrown types.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ProjectileRef(pub(crate) EntityRef);

impl ProjectileRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
}

/// A cast fishing hook of the open world (vanilla's `FishingHook`), narrowed
/// from an entity with `host.fishing_hook(entity)`. It does not narrow to a
/// [`ProjectileRef`]: a hook's owner is always the player who cast it.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct FishingHookRef(pub(crate) EntityRef);

impl FishingHookRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
}

/// A villager or wandering trader (vanilla's `AbstractVillager`, a `Merchant`),
/// narrowed from an entity with `host.merchant(entity)`. If a villager's saved
/// profession is unresolved, offers read empty and merchant mutations are
/// refused until its profession is available again; saved offers are retained.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct MerchantRef(pub(crate) EntityRef);

impl MerchantRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
}

/// Which `Display` subclass an entity is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DisplayKind {
    /// Block.
    Block,
    /// Item.
    Item,
    /// Text.
    Text,
}

impl DisplayKind {
    /// The entity type id: `minecraft:block_display`, `minecraft:item_display` or `minecraft:text_display`.
    pub fn entity_type(self) -> &'static str {
        match self {
            Self::Block => "minecraft:block_display",
            Self::Item => "minecraft:item_display",
            Self::Text => "minecraft:text_display",
        }
    }
    /// The kind of an entity type id, or None for another entity type.
    pub fn from_entity_type(id: &str) -> Option<Self> {
        [Self::Block, Self::Item, Self::Text]
            .into_iter()
            .find(|kind| kind.entity_type() == id)
    }
}

/// `Display.BillboardConstraints`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum Billboard {
    /// Fixed.
    Fixed,
    /// Vertical.
    Vertical,
    /// Horizontal.
    Horizontal,
    /// Center.
    Center,
}

impl Billboard {
    /// Every constraint, in ordinal order.
    pub const ALL: [Self; 4] = [Self::Fixed, Self::Vertical, Self::Horizontal, Self::Center];
    /// `getSerializedName`: `fixed`, `vertical`, `horizontal` or `center`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
            Self::Center => "center",
        }
    }
    /// The constraint with that name, or None for one vanilla doesn't have.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.name() == name)
    }
}

/// `Display.TextDisplay.Align`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum TextAlignment {
    /// Center.
    Center,
    /// Left.
    Left,
    /// Right.
    Right,
}

impl TextAlignment {
    /// `getSerializedName`: `center`, `left` or `right`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// A display's `Transformation`: a translation, a left rotation, a scale and a
/// right rotation. Quaternions are `[x, y, z, w]`.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct Transformation {
    /// The offset in blocks.
    pub translation: [f32; 3],
    /// The rotation applied after the scale, as a quaternion (x, y, z, w).
    pub left_rotation: [f32; 4],
    /// The scale on each axis.
    pub scale: [f32; 3],
    /// The rotation applied before the scale, as a quaternion (x, y, z, w).
    pub right_rotation: [f32; 4],
}

impl Transformation {
    /// `Transformation.IDENTITY`.
    pub const IDENTITY: Self = Self {
        translation: [0.0; 3],
        left_rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
        right_rotation: [0.0, 0.0, 0.0, 1.0],
    };
}

/// `ProjectileDeflection`: how `ProjectileRef::deflect` turns a projectile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum Deflection {
    /// `REVERSE`: backwards at `-power / 2` of the speed, turned about 180
    /// degrees by a random amount.
    Reverse,
    /// `AIM_DEFLECT`: along the deflecting entity's look.
    AimDeflect,
    /// `MOMENTUM_DEFLECT`: along the deflecting entity's movement.
    MomentumDeflect,
}

/// Why a projectile's pierce level is unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PierceLevelError {
    /// The host is not a logical server.
    WrongSide,
    /// The projectile handle is stale or unavailable.
    Stale,
    /// The projectile has no arrow state.
    NotArrow,
}

impl NoBackend for PierceLevelError {
    fn no_backend() -> Self {
        Self::Stale
    }
}

/// Why a player's collision exclusions were not set (`EntityRef::set_collision_exclusions`,
/// `ClientHost::set_collision_exclusions`). The boxes the player had stay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum CollisionExclusionsError {
    /// The handle is stale, or a client has no player of its own.
    Stale,
    /// The entity is not a player: only players keep collision exclusions.
    NotAPlayer,
    /// More than 64 boxes.
    TooMany,
    /// A coordinate that is NaN or infinite.
    NotFinite,
    /// A box whose minimum is above its maximum on an axis.
    Inverted,
}

impl NoBackend for CollisionExclusionsError {
    fn no_backend() -> Self {
        Self::Stale
    }
}

/// Why a client's hotbar selection was not made (`ClientHost::select_hotbar_slot`). The
/// selection the player had stays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: the three refusals are the contract; a new reason is a breaking change
pub enum HotbarSlotError {
    /// The client shows no world, or has no player of its own.
    NoWorld,
    /// A slot above 8: vanilla's range is 0 to 8, and the server ignores others.
    OutOfRange {
        /// The slot the caller asked for.
        slot: u8,
    },
    /// The number keys do nothing for the local player, as for a spectator (its keys open
    /// the spectator menu instead).
    Refused,
}

impl NoBackend for HotbarSlotError {
    fn no_backend() -> Self {
        Self::NoWorld
    }
}

/// A vanilla `AbstractMinecart`, narrowed from an entity with `host.minecart(entity)`.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct MinecartRef(pub(crate) EntityRef);

impl MinecartRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
}

/// Which minecart an entity is: one per vanilla `AbstractMinecart` subclass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum MinecartKind {
    /// `Minecart`.
    Rideable,
    /// `MinecartChest`.
    Chest,
    /// `MinecartFurnace`.
    Furnace,
    /// `MinecartTNT`.
    Tnt,
    /// `MinecartHopper`.
    Hopper,
    /// `MinecartCommandBlock`.
    CommandBlock,
    /// `MinecartSpawner`.
    Spawner,
}

/// `ChatVisiblity`: which chat a client shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ChatVisibility {
    /// All chat.
    Full,
    /// Only system messages and commands.
    System,
    /// No chat.
    Hidden,
}

/// `HumanoidArm`: a player's main hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum MainHand {
    /// The left hand.
    Left,
    /// The right hand.
    Right,
}

/// A player's client options, as `ServerboundClientInformationPacket` carries them
/// (vanilla's `ClientInformation`). Raw ids map as vanilla's `ByIdMap` does: chat
/// visibility and particle status wrap (`floorMod(id, 3)`), the main hand reads an
/// unknown id as left.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientInfo {
    /// The client's language code, such as `en_us`.
    pub language: String,
    /// The client's view distance in chunks.
    pub view_distance: i8,
    /// Which chat the client shows.
    pub chat_visibility: ChatVisibility,
    /// Whether the client shows chat colours.
    pub chat_colors: bool,
    /// Skin layer bits (`PlayerModelPart`).
    pub model_customisation: u8,
    /// The player's main hand.
    pub main_hand: MainHand,
    /// Whether the client asked for filtered text.
    pub text_filtering: bool,
    /// Whether the player shows in the server list sample.
    pub allows_listing: bool,
    /// How many particles the client draws.
    pub particle_status: ParticleStatus,
}

/// Why a mod removes an entity (`Entity.RemovalReason`): the two reasons
/// that destroy it. The unloading and dimension-change reasons belong to the
/// game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum RemovalReason {
    /// `KILLED`: a mob's effects that act on removal run.
    Killed,
    /// `DISCARDED`, as `Entity.discard`.
    Discarded,
}

/// An online player in one open server world, keyed by UUID.
/// A nested guest turn cannot re-enter a player active in an outer turn;
/// operations requiring that player's swapped state report them unavailable.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct PlayerRef {
    pub(crate) entity: EntityRef,
    pub(crate) uuid: u128,
}

impl PlayerRef {
    /// The same player as an entity handle.
    pub fn entity(self) -> EntityRef {
        self.entity
    }
    /// The player's UUID.
    pub fn uuid(self) -> u128 {
        self.uuid
    }
}

impl From<PlayerRef> for EntityRef {
    fn from(player: PlayerRef) -> Self {
        player.entity
    }
}

/// The original player lent to `player_clone`. Every read expires when the hook returns;
/// this is deliberately not a live player or entity handle.
// exhaustive: its field is private, so mods cannot build or destructure it
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OriginalPlayerRef {
    pub(crate) token: u64,
}

impl OriginalPlayerRef {
    /// Read the original's attachment, including nonpersistent and non-copy-on-death data.
    /// A missing value answers its default without changing the replacement player.
    pub fn attachment<T: Clone + Send + Sync + 'static>(
        self,
        host: &mut dyn crate::Host,
        key: &crate::AttachmentType<T>,
    ) -> Result<T, crate::AttachmentError> {
        key.get(host, crate::Holder::OriginalPlayer(self.token))
    }
}

/// A player's food data.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct PlayerFood {
    /// The food level, 0 to 20.
    pub level: i32,
    /// The saturation, up to the food level.
    pub saturation: f32,
    /// The exhaustion; the game takes a saturation or food point at 4.
    pub exhaustion: f32,
}

/// A player's game profile (authlib's `GameProfile`): the UUID, the name and the signed
/// properties the server received at login, such as `textures` (the skin).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct GameProfile {
    /// The player's UUID.
    pub uuid: u128,
    /// The player's name.
    pub name: String,
    /// The profile's properties in login order; empty for an offline-mode player.
    pub properties: Vec<ProfileProperty>,
}

impl GameProfile {
    /// A profile from its UUID, name and properties.
    pub fn new(uuid: u128, name: String, properties: Vec<ProfileProperty>) -> Self {
        Self {
            uuid,
            name,
            properties,
        }
    }
}

/// One property of a [`GameProfile`] (authlib's `Property`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ProfileProperty {
    /// The property's name, such as `textures`.
    pub name: String,
    /// Its value, as the session server sent it (base64 for `textures`).
    pub value: String,
    /// The session server's signature, when the value is signed.
    pub signature: Option<String>,
}

impl ProfileProperty {
    /// A property from its name, value and optional signature.
    pub fn new(name: String, value: String, signature: Option<String>) -> Self {
        Self {
            name,
            value,
            signature,
        }
    }
}

/// Why [`PlayerRef::open_mod_menu`] opened nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum OpenMenuError {
    /// The menu type is not a content mod's (`data/<id>/horizon/menu/`).
    NotModMenu,
    /// The player is stale, not connected (a fake player), or their client lacks Horizon's
    /// menu types (a vanilla client); also a host without a game.
    PlayerUnavailable,
    /// The extra data is past the 32 KiB a menu may carry.
    TooLarge,
    /// The title could not cross to the game.
    Title(crate::text::TextFormError),
}

impl From<crate::text::TextFormError> for OpenMenuError {
    fn from(error: crate::text::TextFormError) -> Self {
        Self::Title(error)
    }
}

impl NoBackend for OpenMenuError {
    fn no_backend() -> Self {
        Self::PlayerUnavailable
    }
}

/// A player's experience (`Player.experienceLevel`, `experienceProgress`,
/// `totalExperience`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct PlayerExperience {
    /// The experience level.
    pub level: i32,
    /// Progress toward the next level, from 0 to 1.
    pub progress: f32,
    /// The total experience points.
    pub total: i32,
}

/// A change requested by an effect tick, applied after its hooks return.
// hidden: a game-side type, not part of the mod surface
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum EffectTickChange {
    Heal(f32),
    Hurt(String, f32),
    AddExhaustion(f32),
    Eat(i32, f32),
}

impl ApplyEffectTickEvent<'_> {
    /// Queue `LivingEntity.heal` on the holder.
    pub fn heal(&mut self, amount: f32) {
        self.changes.push(EffectTickChange::Heal(amount));
    }

    /// Queue damage of a registered damage type on the holder: a vanilla
    /// one (`minecraft:magic`, `minecraft:wither`) or a datapack's
    /// (`demo:heat`). It reaches the holder's hurt hooks with that type; an
    /// unregistered type hurts nothing.
    pub fn hurt(&mut self, damage_type: &str, amount: f32) {
        self.changes
            .push(EffectTickChange::Hurt(damage_type.to_owned(), amount));
    }

    /// Queue food exhaustion; a non-player holder ignores it.
    pub fn add_exhaustion(&mut self, amount: f32) {
        self.changes.push(EffectTickChange::AddExhaustion(amount));
    }

    /// Queue `FoodData.eat`; a non-player holder ignores it.
    pub fn eat(&mut self, nutrition: i32, saturation_modifier: f32) {
        self.changes
            .push(EffectTickChange::Eat(nutrition, saturation_modifier));
    }
}

/// A player's server-side abilities (`Abilities`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct PlayerAbilities {
    /// Takes no damage.
    pub invulnerable: bool,
    /// Is flying now.
    pub flying: bool,
    /// May start flying.
    pub may_fly: bool,
    /// Breaks blocks at once and uses no items (creative).
    pub instabuild: bool,
    /// May break and place blocks.
    pub may_build: bool,
    /// The flying speed.
    pub fly_speed: f32,
    /// Walking speed (`Abilities.walkingSpeed`, vanilla's 0.1).
    pub walk_speed: f32,
}

/// What a fake player does in `PlayerRef::perform_action`: a player's own action, run through
/// the game's path for it, as a client's packet is. Its answer is an [`ActionOutcome`], or an
/// [`ActionRefused`] when none of it ran.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: mods build one; the game runs every kind, so a new kind must be handled there
#[derive(horizon_mod_api_macros::Wire)]
pub enum PlayerAction {
    /// Use the item in `hand` on the block at `pos`, clicked at `hit`
    /// (`ServerPlayerGameMode.useItemOn`): the block's use, then the item's `useOn`, which
    /// places a block item.
    UseOn {
        /// The hand whose item is used.
        hand: Hand,
        /// The block clicked (`BlockHitResult.getBlockPos`).
        pos: [i32; 3],
        /// Where on it.
        hit: BlockHit,
    },
    /// Use the item in `hand` in the air (`ServerPlayerGameMode.useItem`).
    UseItem {
        /// The hand whose item is used.
        hand: Hand,
    },
    /// Interact with `target` using one hand (`Player.interactOn`), without falling back to
    /// using the item in the air when the entity passes the click.
    InteractOn {
        /// The hand whose item is used.
        hand: Hand,
        /// The entity clicked, in the fake's level.
        target: EntityRef,
        /// The hit relative to the entity's origin, as in an entity interaction packet.
        location: [f64; 3],
    },
    /// Attack `target` with the main hand (`ServerGamePacketListenerImpl.handleInteract`'s
    /// ATTACK, then `Player.attack`): a player, a mob or another entity, or a fireball, a wind
    /// charge or a shulker bullet turned back. Mobs don't take the fake as the player to answer.
    Attack {
        /// What is hit, in the fake's level.
        target: EntityRef,
    },
    /// Break the block at `pos` at once (`ServerPlayerGameMode.destroyBlock`), with the game
    /// mode's and the held tool's rules, the tool's wear and the drops, and no digging time:
    /// a mod that digs over time counts the progress itself. Outside creative, a block no
    /// digging breaks (destroy time -1, such as bedrock) stays.
    BreakBlock {
        /// The block broken.
        pos: [i32; 3],
    },
}

/// Temporary state for [`PlayerRef::perform_action_scoped`]. The existing fake acts in
/// survival with a fresh standing body, holding only `stack` in its main hand; its persistent
/// body, items, look, secondary-use state and action timers are restored afterwards.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct PlayerActionSetup {
    /// The main-hand stack, or `None` for an empty hand.
    pub stack: Option<ItemStack>,
    /// Eye position in world coordinates, using the standing eye height (1.62).
    pub eye: [f64; 3],
    /// Vanilla yaw in degrees.
    pub yaw: f32,
    /// Vanilla pitch in degrees.
    pub pitch: f32,
    /// Secondary use, as when holding sneak; this does not change the standing eye height.
    pub secondary_use: bool,
}

impl PlayerActionSetup {
    /// A scoped survival setup with the supplied main-hand stack and viewpoint.
    pub fn new(
        stack: Option<ItemStack>,
        eye: [f64; 3],
        yaw: f32,
        pitch: f32,
        secondary_use: bool,
    ) -> Self {
        Self {
            stack,
            eye,
            yaw,
            pitch,
            secondary_use,
        }
    }
}

/// Tooltip color semantics for an attribute change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: vanilla's `Attribute.Sentiment` has these three values, and the game matches them all
#[derive(horizon_mod_api_macros::Wire)]
pub enum Sentiment {
    /// Increasing values are beneficial.
    #[default]
    Positive,
    /// Increasing values are harmful.
    Negative,
    /// Changes are neutral.
    Neutral,
}

/// Why an entity physically moves (`Entity.move`'s `MoverType`, the
/// `entity_move` hook of #2308/#2414): itself, the player behind a position
/// packet, a piston, a shulker box lid, a shulker mob's opening lid. Boats
/// and minecarts pass `SelfMovement`, as vanilla's own kinds do; `Player` is
/// the server packet sites only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum MoveCause {
    /// `MoverType.SELF`: the body's own tick, travel or packet follow-up.
    SelfMovement,
    /// `MoverType.PLAYER`: the server applying a player's position or vehicle
    /// packet.
    Player,
    /// `MoverType.PISTON`: a moving piston block pushes the body.
    Piston,
    /// `MoverType.SHULKER_BOX`: a shulker box's opening lid pushes it away.
    ShulkerBox,
    /// `MoverType.SHULKER`: a shulker mob's opening lid pushes it away.
    Shulker,
}

impl MoveCause {
    /// Every cause, in ordinal order.
    pub const ALL: [Self; 5] = [
        Self::SelfMovement,
        Self::Player,
        Self::Piston,
        Self::ShulkerBox,
        Self::Shulker,
    ];
}

/// How a declared attribute's modifier lines read on item tooltips: NeoForge's
/// `RangedAttribute`, `PercentageAttribute` and `BooleanAttribute`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: NeoForge's three display kinds, and the tooltip matches them all
#[derive(horizon_mod_api_macros::Wire)]
pub enum AttributeDisplay {
    /// Added values as numbers, multiplied ones as percentages: vanilla's.
    #[default]
    Ranged,
    /// Added values as percentages too: `+0.05` reads `+5%`.
    Percentage,
    /// On or off: an added value above zero reads enabled, any other disabled.
    Boolean,
}

/// How a declared attribute's tooltip lines read beyond their colour (NeoForge's
/// `IAttributeExtension`). The default reads as vanilla's attributes do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
// exhaustive: mods build it with `..Default::default()`, and the game reads every field
#[derive(horizon_mod_api_macros::Wire)]
pub struct AttributeTooltip {
    /// How its modifier amounts read.
    pub display: AttributeDisplay,
    /// The namespaced id of the attribute's base modifier (`getBaseId`): a modifier with this
    /// id reads as the green total of the player's base value and its amount, as
    /// `minecraft:base_attack_damage` does for attack damage. Without a player (recipe search)
    /// it reads as a plain line.
    pub base_modifier: Option<String>,
}
#[cfg(test)]
#[path = "entity_type_key_tests.rs"]
mod entity_type_key_tests;
