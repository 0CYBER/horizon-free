// Types that lived in the API root before the domain split.
use crate::*;

/// A saved fluid name retained for the lifetime of the process.
// no wire: decoding validates the u64 slot with UnresolvedFluid::from_slot on the host; wasm carries the host-issued opaque token without consulting guest tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only trusted engine boundaries create handles
pub struct UnresolvedFluid(pub(crate) u64);

/// A dimension (`minecraft:the_nether`), as the game's table numbers it: a `LevelRef`'s identity.
/// A key stays the same dimension until the game exits; `key.id(host)` gives the name.
// no wire: decoding validates the u32 slot with DimensionKey::from_slot on the host; wasm carries the host-issued opaque token without consulting guest tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its field is private, so only the game's table creates a key
pub struct DimensionKey(pub(crate) u32);

/// Why a game rule category declaration was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum GameRuleError {
    /// The identifier is invalid.
    InvalidId,
    /// The namespace does not belong to the declaring mod.
    ForeignNamespace,
    /// The category is already declared.
    Duplicate,
}

/// A boolean game rule declared by a mod.
// no wire: decoding validates the u32 slot with BoolGameRuleKey::from_slot on the host; wasm carries the host-issued opaque token without consulting guest tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the registration table creates keys
pub struct BoolGameRuleKey(pub(crate) u32);

/// A load-owned fluid identity requested through [`Registrar::fluid_key`](FluidsRegistrar::fluid_key) or resolved through
/// [`DataLoadContext::fluid_key`].
// no wire: decoding validates the u32 slot with FluidKey::from_slot on the host; wasm carries the host-issued opaque token without consulting guest tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the registration table creates keys
pub struct FluidKey(pub(crate) u32);

/// Sampling parameters for `LevelRef::find_closest_biome`, in blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeSearch {
    /// Centre of the sampled spiral, in block coordinates.
    pub origin: [i32; 3],
    /// Maximum horizontal search radius in blocks.
    pub radius: i32,
    /// Distance between sampled columns in blocks.
    pub horizontal_step: i32,
    /// Distance between sampled heights in blocks.
    pub vertical_step: i32,
}

impl BiomeSearch {
    /// Maximum potential biome samples in one synchronous search (65,536).
    /// Work is bounded before searching, even when the origin would match immediately.
    pub const MAX_SAMPLES: u64 = 65_536;

    /// The radius must be nonnegative and both sampling steps positive.
    /// A search returns `None` if `(2 * (radius / horizontal_step) + 1)^2`
    /// times `((dimension_height - 1) / vertical_step + 1)` exceeds [`Self::MAX_SAMPLES`].
    pub fn new(origin: [i32; 3], radius: i32, horizontal_step: i32, vertical_step: i32) -> Self {
        Self {
            origin,
            radius,
            horizontal_step,
            vertical_step,
        }
    }
}

/// The first matching sample in vanilla's closest-biome search order.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeSearchResult {
    /// Registry id of the matching biome.
    pub biome: String,
    /// Block coordinates of the matching sample.
    pub position: [i32; 3],
}

impl BiomeSearchResult {
    /// Pair a matching biome with its sampled position.
    pub fn new(biome: String, position: [i32; 3]) -> Self {
        Self { biome, position }
    }
}

/// Build range and sky properties of a loaded level's dimension type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct LevelDimensionShape {
    /// Lowest buildable block Y.
    pub min_y: i32,
    /// Build height in blocks.
    pub height: i32,
    /// Height used by portals and chorus fruit.
    pub logical_height: i32,
    /// Whether the dimension has skylight.
    pub sky_light: bool,
    /// Whether the dimension has a ceiling.
    pub has_ceiling: bool,
}

impl LevelDimensionShape {
    /// Construct a dimension shape from its build and sky properties.
    pub fn new(
        min_y: i32,
        height: i32,
        logical_height: i32,
        sky_light: bool,
        has_ceiling: bool,
    ) -> Self {
        Self {
            min_y,
            height,
            logical_height,
            sky_light,
            has_ceiling,
        }
    }
}

/// A fluid state copied from the engine. For engine-built values, amount is zero exactly for
/// the empty fluid. A mod may change a placement event's copy; that mod-provided data is
/// forwarded to later Parts and is never consumed by the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct FluidState {
    /// Registered fluid identity.
    pub fluid: FluidKey,
    /// Fluid amount; zero only for empty, otherwise 1 through 8.
    pub amount: u8,
    /// Whether the fluid is falling.
    pub falling: bool,
    /// Whether the engine identifies this as a source.
    pub source: bool,
}

impl FluidState {
    /// Whether this is the empty fluid.
    pub fn is_empty(self) -> bool {
        self.amount == 0
    }
    /// Whether the engine identifies this state as a source.
    pub fn is_source(self) -> bool {
        self.source
    }
    /// Match carried vanilla properties locally. Unknown names or noncanonical values
    /// do not match. Validate against `host.fluid_states()` separately for indexed errors.
    pub fn matches(self, constraints: &[StateConstraint]) -> bool {
        constraints.iter().all(|c| match c.name.as_str() {
            "falling" if !self.is_empty() => c.value == if self.falling { "true" } else { "false" },
            "level" if !self.is_empty() && !self.source => {
                (1..=8).contains(&self.amount) && c.value.as_bytes() == [b'0' + self.amount]
            }
            _ => false,
        })
    }
}

/// The empty frame a fire is about to fill with a nether portal, for
/// `portal_spawn` (`BlockEvent.PortalSpawnEvent.getPortalSize`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct PortalShape {
    /// The portal runs along X (else along Z).
    pub x_axis: bool,
    /// Its bottom-left inside block.
    pub bottom_left: [i32; 3],
    /// The portal frame's inner width in blocks.
    pub width: i32,
    /// The portal frame's inner height in blocks.
    pub height: i32,
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// A server explosion about to happen, for `explosion_start`
/// (`ServerLevel.explode`'s `ServerExplosion`).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ExplosionStart<'a> {
    /// The level it happens in (`minecraft:overworld`).
    pub dimension: &'a str,
    /// The explosion's center in blocks.
    pub center: [f64; 3],
    /// The explosion's radius in blocks.
    pub radius: f32,
    /// It sets fires (`createFire`).
    pub fire: bool,
    /// What the explosion does to blocks.
    pub interaction: BlockInteraction,
    /// The damage type of its damage source (`minecraft:explosion`,
    /// `minecraft:player_explosion`, `minecraft:bad_respawn_point`).
    pub damage_type: &'a str,
    /// The entity that explodes (`getDirectSourceEntity`): a creeper, a wind
    /// charge, the bearer of Wind Charged, when Horizon tracks it.
    pub source: Option<u32>,
    /// `getIndirectSourceEntity`: a living source itself, a primed TNT's
    /// owner, or a projectile's living owner (a ghast for its fireball, a
    /// wither for its skull, the thrower of a wind charge).
    pub owner: Option<u32>,
}

/// One server world open or client level; it rejects a previous world's handles.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct WorldEpoch(pub(crate) u64);

impl WorldEpoch {
    /// Create an epoch for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(n: u64) -> Self {
        Self(n)
    }
    /// Read the epoch number.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// A block in a dimension, vanilla's `GlobalPos`: the dimension's key
/// (`minecraft:the_nether`) and the block. The dimension is data, so it need
/// not be open; `host.level(&pos.dimension().to_string())` gives its level when it is.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct GlobalPos {
    pub(crate) dimension: Identifier,
    pub(crate) pos: BlockPos,
}

impl GlobalPos {
    /// A block in the dimension with key `dimension`.
    pub fn new(dimension: Identifier, pos: BlockPos) -> Self {
        Self { dimension, pos }
    }
    /// The dimension's key (`minecraft:overworld`).
    pub fn dimension(&self) -> &Identifier {
        &self.dimension
    }
    /// The block.
    pub fn pos(&self) -> BlockPos {
        self.pos
    }
}

impl std::hash::Hash for GlobalPos {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(
            &(self.dimension.namespace(), self.dimension.path(), self.pos),
            state,
        );
    }
}

/// A player's respawn point (`ServerPlayer.RespawnConfig`): the block they come back at, the
/// way they face there and whether it is forced (`/spawnpoint`) rather than a bed or anchor that
/// must still be there.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a mod builds it with a literal; the game reads and writes the same three values
#[derive(horizon_mod_api_macros::Wire)]
pub struct PlayerRespawnConfig {
    /// The dimension and block.
    pub position: GlobalPos,
    /// The yaw in degrees (`Mth.wrapDegrees`, -180 to 180 when read).
    pub yaw: f32,
    /// Whether the point holds without its block (`/spawnpoint`).
    pub forced: bool,
}

/// A level of the open world, by its dimension (`minecraft:the_nether`).
/// `Host::level` makes one, and `entity.level(host)` gives an entity's.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct LevelRef {
    pub(crate) epoch: WorldEpoch,
    pub(crate) dimension: DimensionKey,
    /// Which open of a runtime level this names: 0 for a level every world has, the number the
    /// host gave one open of a level a mod created at run time.
    pub(crate) generation: u32,
}

impl LevelRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(epoch: WorldEpoch, dimension: DimensionKey) -> Self {
        Self {
            epoch,
            dimension,
            generation: 0,
        }
    }
    /// Create a handle for one open of a runtime level.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn with_generation(epoch: WorldEpoch, dimension: DimensionKey, generation: u32) -> Self {
        Self {
            epoch,
            dimension,
            generation,
        }
    }
    /// The open of a runtime level the handle names; 0 for a level every world has.
    // hidden: a game-side helper; a handle of an earlier open reads as stale
    #[doc(hidden)]
    pub fn generation(self) -> u32 {
        self.generation
    }
    /// The level's dimension. `level.dimension().id(host)` gives its name.
    pub fn dimension(self) -> DimensionKey {
        self.dimension
    }
    /// The name of the level's dimension, for the game's own tables.
    // hidden: a game-side helper; mods ask the host
    #[doc(hidden)]
    pub fn dimension_id(self) -> &'static str {
        crate::content::registry_keys::dimension_id(self.dimension)
            .expect("the game's table made the key")
    }
    /// Draw from the level's own random stream (vanilla's `Level.random`), in vanilla's order:
    /// `draw` gets the stream and the level keeps it after. None where `random` is None.
    pub fn with_random<T>(
        self,
        host: &mut dyn ServerHost,
        draw: impl FnOnce(&mut LegacyRandom) -> T,
    ) -> Option<T> {
        let mut random = self.random(host)?;
        let drawn = draw(&mut random);
        self.set_random(host, random);
        Some(drawn)
    }
    /// The open of the world the handle belongs to.
    pub fn epoch(self) -> WorldEpoch {
        self.epoch
    }
}

/// A mod-owned watered area in one server level. Created by [`LevelRef::add_water_ticket`].
/// Explicit removal, owner disable and level unload invalidate it. Never saved.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct WaterTicket {
    pub(crate) level: LevelRef,
    pub(crate) id: u64,
}

impl WaterTicket {
    /// Construct a handle at the game boundary.
    // hidden: a game-side constructor; mods receive tickets from `add_water_ticket`
    #[doc(hidden)]
    pub fn new(level: LevelRef, id: u64) -> Self {
        Self { level, id }
    }
    /// The level this transient ticket belongs to.
    pub fn level(self) -> LevelRef {
        self.level
    }
    /// The session-only identity used by the game.
    // hidden: a game-side helper; mods use the handle's methods
    #[doc(hidden)]
    pub fn id(self) -> u64 {
        self.id
    }
}

/// A raid of a level (`Raid`), by the id `Raids` keeps it under, which the
/// raids file saves. `level.raid_at(host, pos)`, `level.raids(host)`,
/// `level.raid(host, id)` and `mob.raid(host)` make one. A handle resolves
/// again on each call: it reads as `None` or `false` once the raid is gone,
/// for a handle of an earlier open, for a closed level, and on a client.
/// Reads and writes reach parked server levels without moving them into place.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct RaidRef {
    pub(crate) level: LevelRef,
    pub(crate) id: i32,
}

impl RaidRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(level: LevelRef, id: i32) -> Self {
        Self { level, id }
    }
    /// The level the raid belongs to.
    pub fn level(self) -> LevelRef {
        self.level
    }
    /// The raid's id in its level (the `raid` argument of the raid hooks).
    pub fn id(self) -> i32 {
        self.id
    }
}

/// Where a raid stands (`Raid.RaidStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum RaidStatus {
    /// Waiting or fighting (`ONGOING`).
    Ongoing,
    /// Every wave was beaten (`VICTORY`).
    Victory,
    /// No villager is left or the time ran out (`LOSS`).
    Loss,
    /// Ended by `stop`, the `/raid stop` command or the `raids` rule (`STOPPED`).
    Stopped,
}

/// A chunk position in an open level. A handle may be kept after unloading;
/// reads resolve it again, returning `false` or `None` when it is no longer loaded.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ChunkRef {
    pub(crate) level: LevelRef,
    pub(crate) pos: [i32; 2],
}

impl ChunkRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(level: LevelRef, pos: [i32; 2]) -> Self {
        Self { level, pos }
    }
    /// The chunk coordinates, `[x, z]`.
    pub fn pos(self) -> [i32; 2] {
        self.pos
    }
    /// The level containing the chunk.
    pub fn level(self) -> LevelRef {
        self.level
    }
}

/// The status of a loaded chunk that Horizon can expose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ChunkStatus {
    /// A full chunk in a level's live world.
    Full,
}

/// A chunk's ticket level (`ChunkLevel`), as the server holds it: how much of
/// the level the chunk is kept alive for. Lower is more active. Horizon keeps
/// no level for a chunk partway through generating, so past `FULL` a chunk is
/// `Unloaded`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ChunkTicketLevel {
    /// A level `ChunkLevel` knows, `FULL_CHUNK_LEVEL` (33) or lower: 31 and
    /// lower is entity ticking (`ENTITY_TICKING_LEVEL`), 32 is block ticking
    /// (`BLOCK_TICKING_LEVEL`) and 33 is loaded without ticking.
    Loaded(u8),
    /// No ticket keeps the chunk loaded.
    Unloaded,
}

impl ChunkTicketLevel {
    /// The `ChunkLevel` number, or `None` for an unloaded chunk.
    pub fn level(self) -> Option<u8> {
        match self {
            Self::Loaded(level) => Some(level),
            Self::Unloaded => None,
        }
    }
}

/// A kind of light (`LightLayer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum LightLayer {
    /// Light from emitting blocks (`BLOCK`).
    Block,
    /// Light from the sky (`SKY`).
    Sky,
}

/// The feature flags a world enables (`FeatureFlagSet`, `LevelReader.enabledFeatures`), each
/// as its id (`minecraft:vanilla`, `minecraft:redstone_experiments`), shared with the world.
/// A world that names none enables `minecraft:vanilla` alone, as vanilla's default does.
/// Getting and reading it allocates nothing.
// no wire: encodes the effective iter() list, which synthesizes minecraft:vanilla for empty backing storage; a structural derive would encode zero flags.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct FeatureFlags(pub(crate) Arc<[String]>);

impl FeatureFlags {
    /// The flags of a world's `enabled_features` list; an empty list is vanilla's default.
    pub fn new(ids: Arc<[String]>) -> Self {
        Self(ids)
    }
    /// `FeatureFlagSet.contains`: whether the flag `flag` (`minecraft:redstone_experiments`, or
    /// `redstone_experiments`) is enabled.
    pub fn contains(&self, flag: &str) -> bool {
        let bare = !flag.contains(':');
        self.iter()
            .any(|id| id == flag || (bare && id.strip_prefix("minecraft:") == Some(flag)))
    }
    /// The enabled flag ids.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        let ids = &self.0;
        ids.iter()
            .map(String::as_str)
            .chain(ids.is_empty().then_some("minecraft:vanilla"))
    }
}

/// The flags as their ids, the form they cross into a wasm mod as.
impl From<&FeatureFlags> for Vec<String> {
    fn from(flags: &FeatureFlags) -> Self {
        flags.0.to_vec()
    }
}

impl From<Vec<String>> for FeatureFlags {
    fn from(ids: Vec<String>) -> Self {
        Self(ids.into())
    }
}

/// What an explosion a mod sets off does to blocks
/// (`Level.ExplosionInteraction`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ExplosionInteraction {
    /// Blocks stay; entities are hurt and pushed (`NONE`).
    None,
    /// Blocks break and drop by the `block_explosion_drop_decay` rule (`BLOCK`).
    #[default]
    Block,
    /// Blocks break and drop by the `mob_explosion_drop_decay` rule when the
    /// source may grief (its `entity_mob_griefing` answer; with no source, the
    /// `mob_griefing` rule), and stay when it may not (`MOB`).
    Mob,
    /// Blocks break and drop by the `tnt_explosion_drop_decay` rule (`TNT`).
    Tnt,
    /// No block breaks; buttons, levers, doors, trapdoors, fence gates, bells
    /// and candles react as to a wind charge's burst (`TRIGGER`).
    Trigger,
}

/// The resistance a blast meets in one cell (`ExplosionDamageCalculator.getBlockExplosionResistance`),
/// as [`LevelRef::explosion_resistance`](crate::LevelRef::explosion_resistance) reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ExplosionResistance {
    /// No answer: the chunk isn't loaded, the level or the source is stale, or the cell holds a
    /// state the game doesn't know.
    Unavailable,
    /// No resistance at all (air with no fluid, or a cell an immune-blocks calculator does not
    /// name): a ray loses only its fixed step there. Not the same as a resistance of 0.
    NoResistance,
    /// This resistance: a ray's step through the cell takes `(r + 0.3) * 0.3` of its power.
    Resists(f32),
}

/// The cells a batched explosion resistance read covers
/// ([`LevelRef::explosion_resistances`](crate::LevelRef::explosion_resistances)), in the order
/// the answers come.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ResistanceCells {
    /// Every cell from `min` to `max`, both included: x fastest, then z, then y
    /// (index `((y - min.y) * dz + (z - min.z)) * dx + (x - min.x)`).
    Box {
        /// The low corner.
        min: [i32; 3],
        /// The high corner, included.
        max: [i32; 3],
    },
    /// Each column `(x, z)` in the order given, from `min_y` up to `max_y`, both included.
    Columns {
        /// The columns, a repeated one read again.
        columns: Vec<(i32, i32)>,
        /// The lowest cell of each column.
        min_y: i32,
        /// The highest cell of each column, included.
        max_y: i32,
    },
}

impl ResistanceCells {
    /// The most cells one batched read covers; a larger one is refused.
    pub const MAX: u64 = 262_144;
}

/// Why a batched explosion resistance read was refused, with nothing read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ResistanceCellsError {
    /// A box whose `max` is below its `min` on some axis, or columns whose `max_y` is below `min_y`.
    Inverted,
    /// More cells than [`ResistanceCells::MAX`]: this many.
    TooMany(u64),
    /// A host without a game.
    NoGame,
}

impl crate::NoBackend for ResistanceCellsError {
    fn no_backend() -> Self {
        Self::NoGame
    }
}

/// Why [`LevelRef::explosion_destroy_blocks`](crate::LevelRef::explosion_destroy_blocks)
/// destroyed nothing, with nothing changed and no hook told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ExplosionDestroyError {
    /// More positions than [`ExplosionDestroyError::MAX_POSITIONS`]: this many.
    TooMany(u64),
    /// A stale level, or one not in place; a stale source or one in another level; a client.
    Unavailable,
    /// A centre or radius that isn't finite, or a negative radius.
    InvalidBlast,
    /// A host without a game.
    NoGame,
}

impl ExplosionDestroyError {
    /// The most positions one call takes.
    pub const MAX_POSITIONS: u64 = 65_536;
}

impl crate::NoBackend for ExplosionDestroyError {
    fn no_backend() -> Self {
        Self::NoGame
    }
}

/// What made a `tags_updated` notice fire (`TagsUpdatedEvent`'s two subclasses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum TagsUpdateCause {
    /// `TagsUpdatedEvent.ServerDataLoad`: a server built its data tables.
    ServerDataLoad {
        /// False on a world's first open, true after `/reload`.
        reload: bool,
    },
    /// `TagsUpdatedEvent.ClientPacketReceived`: a client applied the tags its server sent.
    ClientPacketReceived {
        /// The server is the single player window's own.
        integrated_server: bool,
    },
}

impl TagsUpdateCause {
    /// `TagsUpdatedEvent.shouldUpdateStaticData`: whether to update static data, which single
    /// player's two threads share, so that there only the server side updates it.
    pub fn should_update_static_data(self) -> bool {
        !matches!(
            self,
            Self::ClientPacketReceived {
                integrated_server: true
            }
        )
    }
}

/// What made a `data_maps_updated` notice fire (`DataMapsUpdatedEvent.UpdateCause`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DataMapsUpdateCause {
    /// `SERVER_RELOAD`: the server merged its data maps, on a world's first open or after
    /// `/reload`.
    ServerReload,
    /// `CLIENT_SYNC`: a client applied the data maps its server sent.
    ClientSync,
}

/// The category of an entity's type (`MobCategory`, `EntityType.getCategory`):
/// what natural spawning counts it as. Every kind that isn't a mob is `Misc`,
/// except the ender dragon, a `Monster`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum MobCategory {
    /// `MONSTER`.
    Monster,
    /// `CREATURE`.
    Creature,
    /// `AMBIENT`.
    Ambient,
    /// `AXOLOTLS`.
    Axolotls,
    /// `UNDERGROUND_WATER_CREATURE`.
    UndergroundWaterCreature,
    /// `WATER_CREATURE`.
    WaterCreature,
    /// `WATER_AMBIENT`.
    WaterAmbient,
    /// `MISC`.
    Misc,
}

/// How many mobs of each spawning category a level's natural spawning allows
/// (`MobCategory.getMaxInstancesPerChunk`), read and set through
/// `LevelRef::spawn_limits` and `set_spawn_limits`. Each limit counts twice, as
/// vanilla's own does: the level spawns a category only while its count of
/// that category's mobs stays under `limit * spawnable chunks / 289` (one
/// player's 17 by 17 chunks hold the vanilla value), and only in a chunk with
/// a player near it who has fewer than `limit` of them around. A limit of 0
/// stops the category's natural spawning. `MobCategory::Misc` never spawns
/// naturally, so it has no limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: one field per vanilla spawning category; mods build it from `VANILLA`
#[derive(horizon_mod_api_macros::Wire)]
pub struct SpawnLimits {
    /// `MONSTER`: 70 in vanilla.
    pub monster: u32,
    /// `CREATURE`: 10 in vanilla.
    pub creature: u32,
    /// `AMBIENT`: 15 in vanilla.
    pub ambient: u32,
    /// `AXOLOTLS`: 5 in vanilla.
    pub axolotls: u32,
    /// `UNDERGROUND_WATER_CREATURE`: 5 in vanilla.
    pub underground_water_creature: u32,
    /// `WATER_CREATURE`: 5 in vanilla.
    pub water_creature: u32,
    /// `WATER_AMBIENT`: 20 in vanilla.
    pub water_ambient: u32,
}

impl SpawnLimits {
    /// Vanilla's limits, which every level starts with.
    pub const VANILLA: Self = Self {
        monster: 70,
        creature: 10,
        ambient: 15,
        axolotls: 5,
        underground_water_creature: 5,
        water_creature: 5,
        water_ambient: 20,
    };
}

impl Default for SpawnLimits {
    fn default() -> Self {
        Self::VANILLA
    }
}

/// Which categories a level's natural spawning (`NaturalSpawner.spawnForChunk`)
/// may spawn at all, read and set through `LevelRef::spawn_flags` and
/// `set_spawn_flags`. A flag only takes away, for its level alone: monsters
/// still need the `spawn_monsters` game rule, and nothing spawns naturally with
/// `spawn_mobs` off. Spawners, patrols, sieges, phantoms, a new chunk's
/// creatures and other custom spawners don't read the flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: one switch for monsters, one for the rest; mods build it with a literal
#[derive(horizon_mod_api_macros::Wire)]
pub struct SpawnFlags {
    /// Whether `MobCategory::Monster` spawns: off acts as the `spawn_monsters`
    /// rule off does (`getFilteredSpawningCategories`' `spawnEnemies`).
    pub monsters: bool,
    /// Whether every other spawning category spawns: creatures, ambient and
    /// water mobs, axolotls. Horizon's own; vanilla has no such switch.
    pub friendlies: bool,
}

impl Default for SpawnFlags {
    /// Both on, as every level starts.
    fn default() -> Self {
        Self {
            monsters: true,
            friendlies: true,
        }
    }
}

/// Why a mob spawns (`EntitySpawnReason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SpawnReason {
    /// `NATURAL`: the level's own spawning each tick.
    Natural,
    /// `CHUNK_GENERATION`: a new chunk's creature packs.
    ChunkGeneration,
    /// `SPAWNER`: a mob spawner block.
    Spawner,
    /// `TRIAL_SPAWNER`: a trial spawner.
    TrialSpawner,
    /// `REINFORCEMENT`: a zombie calling for help.
    Reinforcement,
    /// `PATROL`: a pillager patrol.
    Patrol,
    /// `STRUCTURE`: a structure's own mobs.
    Structure,
    /// `EVENT`: a world event, such as a village siege.
    Event,
    /// `COMMAND`: `/summon`.
    Command,
    /// `SPAWN_ITEM_USE`: a spawn egg.
    SpawnItemUse,
    /// `MOB_SUMMONED`: a mob that summons another (an evoker's vexes).
    MobSummoned,
    /// `CONVERSION`: a mob that became another.
    Conversion,
    /// `BREEDING`: a baby.
    Breeding,
    /// `TRIGGERED`: a block or item set it off (a sculk shrieker's warden).
    Triggered,
    /// `JOCKEY`: a rider spawned on a mount.
    Jockey,
    /// `DISPENSER`: a dispenser fired a spawn egg.
    Dispenser,
    /// `BUCKET`: a bucketed mob released.
    Bucket,
    /// `LOAD`: loaded from saved data.
    Load,
    /// `DIMENSION_TRAVEL`: a mob that crossed a portal.
    DimensionTravel,
}

/// The local difficulty at a spawn (`DifficultyInstance`): what `finalize_spawn` reads and
/// may change before the mob is finalized.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct Difficulty {
    /// `Difficulty.getId`: 0 peaceful, 1 easy, 2 normal, 3 hard. A larger value is taken as 3.
    pub base: u8,
    /// `DifficultyInstance.getEffectiveDifficulty`: the base scaled by world age, the chunk's
    /// inhabited time and the moon.
    pub effective: f32,
}

impl Difficulty {
    /// A difficulty of `base` and `effective`.
    pub fn new(base: u8, effective: f32) -> Self {
        Self { base, effective }
    }
    /// `DifficultyInstance.getSpecialMultiplier`: 0 below an effective difficulty of 2, 1
    /// above 4, and rising in a straight line between them. Mobs read it for their chance to
    /// pick up loot, a zombie's reinforcements and a skeleton's enchanted bow.
    pub fn special_multiplier(&self) -> f32 {
        if self.effective < 2.0 {
            0.0
        } else if self.effective > 4.0 {
            1.0
        } else {
            (self.effective - 2.0) / 2.0
        }
    }
}

/// What spawned a mob, where `EntitySpawnReason` alone does not say (`FinalizeSpawnEvent.getSpawner`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SpawnSource {
    /// A spawner or trial spawner block at this position.
    Block(BlockPos),
}

/// The spawner block that asked for a spawn check (`MobSpawnEvent.PositionCheck.getSpawner`):
/// what `mob_spawn_placement_check` and `mob_spawn_position_check` get when a block spawns the
/// mob. Natural spawning, chunk generation, patrols, sieges, reinforcements and a spawner
/// minecart have none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum Spawner {
    /// A monster spawner block (`SpawnerBlockEntity`).
    Block {
        /// Where the block is.
        pos: BlockPos,
    },
    /// A trial spawner block (`TrialSpawnerBlockEntity`).
    Trial {
        /// Where the block is.
        pos: BlockPos,
    },
}

/// What a mod answers a spawn check (`MobSpawnEvent.PositionCheck.Result` and
/// `MobSpawnEvent.SpawnPlacementCheck.Result`, one type for both).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SpawnCheck {
    /// `SUCCEED`: the check passes.
    Succeed,
    /// `DEFAULT`: vanilla's own answer stands.
    #[default]
    Default,
    /// `FAIL`: the check fails.
    Fail,
}

/// Where a mod entity type stands to spawn (`SpawnPlacementType`):
/// `SpawnPlacements.isSpawnPositionOk`'s first check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SpawnPlacementType {
    /// `ON_GROUND`: a block that is valid to spawn on below, free room at the feet and the head.
    OnGround,
    /// `IN_WATER`: a water block, with no redstone conductor above.
    InWater,
    /// `IN_LAVA`: a lava block.
    InLava,
    /// `NO_RESTRICTIONS`: anywhere.
    NoRestrictions,
}

/// How bright a spot a mod entity type needs to spawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SpawnLight {
    /// Any light level.
    Any,
    /// Dark enough for a monster (`Monster.isDarkEnoughToSpawn`: sky light, block light
    /// and the dimension's monster light test, which draw from the level's random).
    Dark,
    /// Bright enough for an animal (`Animal.isBrightEnoughToSpawn`: light above 8).
    Bright,
}

/// A mod entity type's spawn rule, declared with
/// [`Registrar::declare_spawn_placement`](SpawnPlacementsRegistrar::declare_spawn_placement):
/// the arguments of NeoForge's `RegisterSpawnPlacementsEvent.register` as data, so one
/// rule serves native and wasm mods. Every field must hold for a spawn to pass; a trial
/// spawner ignores `light`, as for vanilla types.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a mod builds it with a literal; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct SpawnPlacementRule {
    /// Where the type stands (`SpawnPlacementType`).
    pub placement: SpawnPlacementType,
    /// The heightmap natural spawning and chunk generation find the surface with.
    pub heightmap: Heightmap,
    /// A block tag (`minecraft:animals_spawnable_on`, no `#`) the block below must be in.
    pub spawnable_on: Option<String>,
    /// How bright the spot must be.
    pub light: SpawnLight,
    /// A biome tag (no `#`) the spot's biome must be in.
    pub biome_tag: Option<String>,
}

/// Why a spawn placement declaration was refused
/// ([`Registrar::declare_spawn_placement`](SpawnPlacementsRegistrar::declare_spawn_placement)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SpawnPlacementError {
    /// The entity type's id is invalid, empty, or in a namespace the declaring
    /// mod does not own.
    InvalidId,
    /// A tag is not a namespaced tag name (write it without `#`).
    InvalidTag,
    /// Another declaration already covers the entity type.
    Duplicate,
}

/// What a mod answers a despawn check (`MobDespawnEvent.Result`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum Despawn {
    /// `ALLOW`: the mob despawns now.
    Allow,
    /// `DEFAULT`: vanilla's `Mob.checkDespawn` decides.
    #[default]
    Default,
    /// `DENY`: the mob does not despawn.
    Deny,
}

/// Block shape used by an empty-context level ray.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum ClipBlock {
    /// Collider.
    Collider,
    /// Outline.
    Outline,
}

/// Fluid mode for a level ray. Only the listed modes are supported.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum ClipFluid {
    /// None.
    None,
}

/// Result of a block ray, including the vanilla miss result.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClipHit {
    /// Whether the ray hit nothing; the other fields are then the vanilla miss values.
    pub miss: bool,
    /// The hit block's position.
    pub block: [i32; 3],
    /// The face the ray entered.
    pub face: BlockFace,
    /// The exact hit point in blocks.
    pub location: [f64; 3],
    /// Whether the ray started inside the block.
    pub inside: bool,
    /// Whether the hit is the world border.
    pub world_border: bool,
}

/// The world's respawn data (`LevelData.RespawnData`): where a player without a spawn point of
/// their own appears, and facing which way. Horizon keeps it in the Overworld.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct RespawnData {
    /// The spawn block.
    pub pos: [i32; 3],
    /// The yaw in degrees, -180 to 180 (`Mth.wrapDegrees`).
    pub yaw: f32,
    /// The pitch in degrees, -90 to 90.
    pub pitch: f32,
}

/// A map's saved data (`MapItemSavedData`) as it stands: where it is, how far it reaches and
/// its pixels. The markers (players, banners, frames) are not part of it.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct MapState {
    /// The centre block `[x, z]`.
    pub center: [i32; 2],
    /// A map pixel is `1 << scale` blocks, 0 to 4.
    pub scale: u8,
    /// The dimension it shows, `minecraft:overworld`.
    pub dimension: String,
    /// A locked map (a cartography table's glass pane) no longer reveals the world.
    pub locked: bool,
    /// Whether it shows its holders' markers.
    pub tracking: bool,
    /// Whether it shows them however far away they are.
    pub unlimited: bool,
    /// 128 by 128 map colour ids, row by row (`x + z * 128`).
    pub colors: Vec<u8>,
}

/// One entity a level ray passes through (`LevelRef::ray_trace_entities`): what
/// `ProjectileUtil.getEntityHitResult` tests for each candidate.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct EntityRayHit {
    /// The entity hit, a handle of the level the ray ran in.
    pub entity: EntityRef,
    /// Where the ray enters the entity's box, grown by the ray's margin, in blocks.
    pub location: [f64; 3],
    /// How far `location` is from the ray's start, in blocks.
    pub distance: f64,
}

/// Non-paused authoritative server ticks since open (not game or frame time).
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct TickId(pub(crate) u64);

impl TickId {
    /// Create a tick ID for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(n: u64) -> Self {
        Self(n)
    }
    /// Read the tick number.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// The world phases (`world_lifecycle`'s `phase`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The phase names are the hook log's stable vocabulary (the Debug text
/// the mixins write); keep them despite the shared prefix.
#[allow(clippy::enum_variant_names)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum WorldPhase {
    /// The open failed; `error_text` is the engine's own error text.
    WorldOpenFailed,
    /// World close final.
    WorldCloseFinal,
    /// The server loop or final save failed; `error_text` carries the error.
    WorldCloseFailed,
}

/// The tick phases carry whether the world runs, including frozen steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ServerTickPhase {
    /// The server tick is about to begin.
    ServerTickStart {
        /// False for a frozen step, when the world does not advance.
        world_running: bool,
    },
    /// The server tick has finished.
    ServerTickEnd {
        /// False for a frozen step, when the world does not advance.
        world_running: bool,
    },
}

/// Context for every world lifecycle phase.
/// `authority` is the server role. LocalAuthoritative and RemoteClient never
/// fire. `epoch` identifies the open attempt, including failed opens;
/// the same epoch identifies its ready, final-close, or failed-close event.
/// `tick` counts non-paused server ticks since the world opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct LifecycleContext {
    /// The server role that fired the phase.
    pub authority: Authority,
    /// Identifies the open attempt this phase belongs to.
    pub epoch: WorldEpoch,
    /// Non-paused server ticks since the world opened.
    pub tick: TickId,
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// The player responsible for a game rule change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct GameRuleActor<'a> {
    /// Player UUID.
    pub uuid: u128,
    /// Server entity ID.
    pub entity_id: i32,
    /// Player name.
    pub name: &'a str,
}

/// How a criterion's advancement progress changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ProgressType {
    /// Grant.
    Grant,
    /// Revoke.
    Revoke,
}

/// A block state's sounds (`BlockState.getSoundType()`, vanilla's `SoundType`): the sound
/// events, as `LevelRef::play_sound` names them, and the volume and pitch they play at.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BlockSoundType {
    /// `SoundType.getVolume`.
    pub volume: f32,
    /// `SoundType.getPitch`.
    pub pitch: f32,
    /// `getBreakSound`: the block broken.
    pub break_sound: std::sync::Arc<str>,
    /// `getStepSound`: walked on.
    pub step: std::sync::Arc<str>,
    /// `getPlaceSound`: placed.
    pub place: std::sync::Arc<str>,
    /// `getHitSound`: hit while being mined.
    pub hit: std::sync::Arc<str>,
    /// `getFallSound`: landed on.
    pub fall: std::sync::Arc<str>,
}

/// Why a block's sound type is read (`BlockPart::sound_type`): which of its sounds the game is
/// about to play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BlockSoundCause {
    /// An entity steps on it (`Entity.playStepSound`): the step sound at volume × 0.15.
    Step,
    /// A living entity lands on it hard enough to be hurt (`LivingEntity.playBlockFallSound`):
    /// the fall sound at volume × 0.5, pitch × 0.75.
    Fall,
    /// It is placed (`BlockItem.place`): the place sound at `(volume + 1) / 2`, pitch × 0.8.
    Place,
    /// It is broken (level event 2001): the break sound at `(volume + 1) / 2`, pitch × 0.8.
    Break,
    /// A sniffer digs it (`Sniffer.emitDiggingParticles`): the hit sound at a fixed volume 0.5
    /// and pitch 0.5, whatever the answer's volume and pitch.
    Dig,
}

/// A vector of doubles, as vanilla's `Vec3`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
// exhaustive: a vector is its three components, as vanilla's `Vec3`
#[derive(horizon_mod_api_macros::Wire)]
pub struct Vec3 {
    /// East (+) and west (-).
    pub x: f64,
    /// Up (+) and down (-).
    pub y: f64,
    /// South (+) and north (-).
    pub z: f64,
}

impl Vec3 {
    /// The zero vector (`Vec3.ZERO`).
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    /// New.
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
}

impl From<glam::DVec3> for Vec3 {
    fn from(v: glam::DVec3) -> Self {
        Self::new(v.x, v.y, v.z)
    }
}

impl From<Vec3> for glam::DVec3 {
    fn from(v: Vec3) -> Self {
        Self::new(v.x, v.y, v.z)
    }
}

impl LevelSide<'_> {
    pub(crate) fn reborrow(&mut self) -> LevelSide<'_> {
        match self {
            Self::Server(host) => LevelSide::Server(*host),
            Self::Client(host) => LevelSide::Client(*host),
        }
    }
}

/// NeoForge's `TriState`: force a part of an action off or on, or leave it
/// to vanilla.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum TriState {
    /// Deny.
    False,
    #[default]
    /// Leave the game's own answer.
    Default,
    /// Allow.
    True,
}

impl TriState {
    /// Whether the part runs, `vanilla` deciding for `Default`.
    pub fn or(self, vanilla: bool) -> bool {
        match self {
            Self::False => false,
            Self::Default => vanilla,
            Self::True => true,
        }
    }
}

/// Read-only level services, like vanilla's `LevelReader`.
///
/// A level host cannot be used as a game host:
/// ```compile_fail
/// use mod_api::{Host, LevelHost};
/// fn game(_: &dyn Host) {}
/// fn level(host: &dyn LevelHost) { game(host); }
/// ```
/// ```compile_fail
/// use mod_api::{ClientHost, LevelHost, ServerHost};
/// fn server(_: &mut dyn ServerHost) {}
/// fn client(_: &dyn ClientHost) {}
/// fn level(host: &mut dyn LevelHost) { server(host); client(host); }
/// ```
/// A client-side level cannot write:
/// ```compile_fail
/// use mod_api::LevelSide;
/// fn write(side: LevelSide<'_>) {
///     if let LevelSide::Client(level) = side { level.set_block_property([0, 0, 0], "a", "b", 0); }
/// }
/// ```
pub trait LevelHost {
    /// Read a block entity's saved data without identity metadata from this level.
    /// Stale generations and handles from another level or world return None.
    fn block_entity_data(&self, _entity: crate::BlockEntityRef) -> Option<crate::Compound> {
        None
    }

    /// Read an owned block state through the level's engine read.
    /// Out-of-height or unloaded engine reads retain vanilla's air as `Some(air)`.
    /// Returns `None` when the owning state index is unavailable or refuses the state;
    /// this never substitutes an air state for a failed conversion.
    fn block_state(&self, _pos: [i32; 3]) -> Option<BlockState> {
        None
    }
    /// The block `state` is a state of (`BlockState.getBlock()`): `mymod:shaft`. None for an
    /// unknown state.
    fn state_block(&self, _state: BlockState) -> Option<&str> {
        unimplemented!("LevelHost::state_block")
    }
    /// Parse a block name and properties as `BlockStateParser.parseForBlock`; None for invalid input or unavailable context.
    fn block_state_id(&self, _name: &str) -> Option<BlockState> {
        unimplemented!("LevelHost::block_state_id")
    }
    /// The block's default state (`Block.defaultBlockState`), or None for an unknown block.
    fn block_state_default(&self, _block: BlockKey) -> Option<BlockState> {
        unimplemented!("LevelHost::block_state_default")
    }
    /// The full block state name and properties, as `BlockStateParser.serialize`; None for an unknown id.
    fn state_string(&self, _state: BlockState) -> Option<String> {
        unimplemented!("LevelHost::state_string")
    }
    /// Lowest build Y (`LevelHeightAccessor.getMinY`).
    fn min_y(&self) -> i32 {
        unimplemented!("LevelHost::min_y")
    }
    /// Build height (`LevelHeightAccessor.getHeight`).
    fn height(&self) -> i32 {
        unimplemented!("LevelHost::height")
    }
    /// `BlockState.is(TagKey)`: numeric membership of the resolved block in the merged block tag.
    /// Unknown tags return false; unavailable authoritative tags or block mappings return None.
    fn block_in_tag(&self, _block: BlockKey, _tag: &str) -> Option<bool> {
        unimplemented!("LevelHost::block_in_tag")
    }
    /// The merged-tag service for this level's owning-thread context.
    /// Not on wasm: a frame resource; its methods are the imports.
    fn tags(&self) -> &dyn Tags {
        unimplemented!("LevelHost::tags")
    }
    /// `BlockGetter.getFluidState`: the fluid at `pos`, the empty fluid where there is none.
    /// None where the position is not loaded or outside the build height; it never loads a chunk.
    fn fluid_state(&self, _pos: [i32; 3]) -> Option<FluidState> {
        unimplemented!("LevelHost::fluid_state")
    }
    /// `BlockState.getFluidState`: the fluid a state holds (water for a waterlogged one). None
    /// for an id no state has.
    fn state_fluid_state(&self, _state: BlockState) -> Option<FluidState> {
        unimplemented!("LevelHost::state_fluid_state")
    }
    /// `FluidState.is(TagKey)`: whether the fluid registry name is in the merged fluid tag, as `block_in_tag` reads.
    fn fluid_in_tag(&self, _fluid: &str, _tag: &str) -> Option<bool> {
        unimplemented!("LevelHost::fluid_in_tag")
    }
    /// `BlockState.setValue`, through `native_state_with_property`; None if invalid.
    fn state_with_property(
        &self,
        _state: BlockState,
        _key: &str,
        _value: &str,
    ) -> Option<BlockState> {
        unimplemented!("LevelHost::state_with_property")
    }
    /// The stored light of one layer at `pos` (`BlockAndLightGetter.getBrightness`), 0 to 15.
    /// It reads the game's own light query: above the build height the sky is 15 and below it 0,
    /// and a chunk that isn't loaded reads as full sky and no block light. It never loads a chunk.
    fn brightness(&self, _layer: LightLayer, _pos: [i32; 3]) -> i32 {
        unimplemented!("LevelHost::brightness")
    }
    /// `BlockAndLightGetter.getRawBrightness(pos, darkening)`: the brighter of the block light
    /// and the sky light less `sky_darken`. `CropBlock.randomTick` asks it with 0.
    fn raw_brightness(&self, _pos: [i32; 3], _sky_darken: i32) -> i32 {
        unimplemented!("LevelHost::raw_brightness")
    }
    /// `Level.isLoaded`: `pos` is inside the build height and its chunk is loaded. Asking never
    /// loads the chunk.
    fn is_loaded(&self, _pos: [i32; 3]) -> bool {
        unimplemented!("LevelHost::is_loaded")
    }
    /// `Level.isRainingAt`: it rains in this level and `pos` is open to the sky (full sky light,
    /// nothing motion-blocking or fluid above it) in a biome whose precipitation there is rain,
    /// not snow or none. False where `pos` is not loaded, the chunk is never loaded for it.
    fn is_raining_at(&self, _pos: [i32; 3]) -> bool {
        unimplemented!("LevelHost::is_raining_at")
    }
    /// `LevelReader.enabledFeatures`: the feature flags the world enables.
    fn enabled_features(&self) -> FeatureFlags {
        unimplemented!("LevelHost::enabled_features")
    }
}

/// Access vanilla's `Level.random`, so draws come in vanilla's order.
pub trait LevelRandomHost: LevelHost {
    /// The holders' server-owned attachments; `TYPE.get(host, holder)` reads one through it.
    fn attachments(&mut self) -> &mut dyn AttachmentStore {
        unimplemented!("LevelRandomHost::attachments")
    }
    /// A copy of the level's random stream through `native_level_random`, or None on a
    /// non-authoritative level. Draw with `with_random`, which stores it back.
    fn random(&mut self) -> Option<LegacyRandom> {
        unimplemented!("LevelRandomHost::random")
    }
    /// Replace the level's random stream with `random`, one `random` gave and the mod drew from.
    /// False on a non-authoritative level.
    fn set_random(&mut self, _random: LegacyRandom) -> bool {
        unimplemented!("LevelRandomHost::set_random")
    }
}

/// A `LevelReader` with vanilla's `ScheduledTickAccess`. Shape updates cannot write blocks.
pub trait LevelTicksHost: LevelHost {
    /// Schedule a block tick through `native_schedule_block_tick`.
    fn schedule_block_tick(&mut self, _pos: [i32; 3], _block: BlockKey, _delay: i32) -> bool {
        unimplemented!("LevelTicksHost::schedule_block_tick")
    }
    /// Schedule a fluid tick through `native_schedule_fluid_tick`.
    fn schedule_fluid_tick(&mut self, _pos: [i32; 3], _fluid: &str, _delay: i32) -> bool {
        unimplemented!("LevelTicksHost::schedule_fluid_tick")
    }
    /// How strongly the block at `pos` powers its neighbour on the `direction` side, power it
    /// conducts included (`SignalGetter.getSignal(pos, direction)`), 0 to 15: the game's own
    /// read, mods' `block_signal` and `should_check_weak_power` parts included, as
    /// `LevelRef::signal` reads it. It writes nothing and draws nothing, so it leaves the
    /// write it is read in as it was.
    fn signal(&self, _pos: [i32; 3], _direction: Direction) -> i32 {
        unimplemented!("LevelTicksHost::signal")
    }
    /// The strong power alone that the block at `pos` gives that way
    /// (`SignalGetter.getDirectSignal(pos, direction)`), 0 to 15.
    fn direct_signal(&self, _pos: [i32; 3], _direction: Direction) -> i32 {
        unimplemented!("LevelTicksHost::direct_signal")
    }
    /// The strongest strong power reaching the block at `pos` (`SignalGetter.getDirectSignalTo`).
    fn direct_signal_to(&self, _pos: [i32; 3]) -> i32 {
        unimplemented!("LevelTicksHost::direct_signal_to")
    }
    /// The strongest power reaching the block at `pos` (`SignalGetter.getBestNeighborSignal`).
    fn best_neighbor_signal(&self, _pos: [i32; 3]) -> i32 {
        unimplemented!("LevelTicksHost::best_neighbor_signal")
    }
    /// Whether any neighbour powers the block at `pos` (`SignalGetter.hasNeighborSignal`),
    /// asking no side past the first that does.
    fn has_neighbor_signal(&self, _pos: [i32; 3]) -> bool {
        unimplemented!("LevelTicksHost::has_neighbor_signal")
    }
}

/// Mutable level services, like vanilla's `ServerLevel`.
pub trait LevelWriteHost: LevelTicksHost + LevelRandomHost {
    /// Replace the block at `pos` with `state` through `Level.setBlock(pos, state, flags)`, as
    /// `LevelRef::set_block` does: the old block's block entity goes and the new block's is
    /// created in the write, its neighbour and shape updates run per `flags`, and its level events
    /// queue in order with the caller's. A mod block entity starts with the data its
    /// `block_entity` hook gives, which gets this level. True when the block changed; false off an
    /// authoritative server, for a position that is not loaded or outside the build height, an
    /// invalid state, or the state it already is. It never loads a chunk.
    fn set_block(&mut self, _pos: [i32; 3], _state: BlockState, _flags: BlockUpdateFlags) -> bool {
        unimplemented!("LevelWriteHost::set_block")
    }
    /// Remove a block through `Level.removeBlock`, preserving its fluid. `moved` marks a piston move.
    fn remove_block(&mut self, _pos: [i32; 3], _moved: bool) -> bool {
        unimplemented!("LevelWriteHost::remove_block")
    }
    /// Change a block property through `native_set_block_property`.
    fn set_block_property(
        &mut self,
        _pos: [i32; 3],
        _key: &str,
        _value: &str,
        _flags: i32,
    ) -> bool {
        unimplemented!("LevelWriteHost::set_block_property")
    }
    /// Play a sound for nearby players through `play_sound`.
    fn play_sound(
        &mut self,
        _sound: &str,
        _at: [f64; 3],
        _source: usize,
        _volume: f32,
        _pitch: f32,
    ) -> bool {
        unimplemented!("LevelWriteHost::play_sound")
    }
    /// `ServerLevel.sendParticles` through `native_send_particles`.
    /// Item particles require `options.item_stack`; invalid stacks return false.
    fn send_particles(
        &mut self,
        _options: &ParticleOptions,
        _state: Option<BlockState>,
        _at: [f64; 3],
        _spread: [f64; 3],
        _speed: f64,
        _count: i32,
    ) -> bool {
        unimplemented!("LevelWriteHost::send_particles")
    }
    /// `LevelAccessor.levelEvent(null, id, pos, data)` through `native_level_event`: the event
    /// is queued with the write's own events, in order, and plays when the write is done.
    /// `LevelRef::level_event` takes the same `id` and `data`.
    fn level_event(&mut self, _id: i32, _pos: [i32; 3], _data: i32) -> bool {
        unimplemented!("LevelWriteHost::level_event")
    }
}

/// Search parameters for `LevelRef::find_nearest_structure`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct StructureSearch {
    /// Where the search starts, in block coordinates; the distance is horizontal.
    pub origin: [i32; 3],
    /// How many rings of placement cells around the origin's are searched for
    /// random-spread structure sets (vanilla's `searchRadius`); `/locate` uses 100.
    pub radius: i32,
}

impl StructureSearch {
    /// The largest radius one synchronous search takes: `/locate`'s and an
    /// explorer map's 100 rings.
    pub const MAX_RADIUS: i32 = 100;

    /// The radius must be from 0 to [`Self::MAX_RADIUS`].
    pub fn new(origin: [i32; 3], radius: i32) -> Self {
        Self { origin, radius }
    }
}

/// The nearest structure start a search found.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct StructureSearchResult {
    /// Registry id of the structure, the tag's member that matched for a `#tag`.
    pub structure: String,
    /// Where `/locate` reports the start (`StructurePlacement.getLocatePos`):
    /// its chunk's corner at y 0, moved by the set's `locate_offset`.
    pub position: [i32; 3],
}

impl StructureSearchResult {
    /// Pair a found structure with its start position.
    pub fn new(structure: String, position: [i32; 3]) -> Self {
        Self {
            structure,
            position,
        }
    }
}

/// Why a server-wide game rule read was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum GameRuleReadError {
    /// No rule has this identifier.
    UnknownRule,
}
