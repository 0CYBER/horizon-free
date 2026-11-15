// Types that lived in the API root before the domain split.
use crate::*;

/// A block requested once through [`Registrar::block_key`](BlocksRegistrar::block_key). Its load-owned slot survives reload.
// no wire: decoding validates the u32 slot with BlockKey::from_slot on the host; wasm carries the host-issued opaque token without consulting guest tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its field is private, so only registration creates a key
pub struct BlockKey(pub(crate) u32);

/// A block state as the game's state table numbers it (`Block.BLOCK_STATE_REGISTRY`'s id): what
/// worldgen providers and fluid queries answer. Valid for the running game only; never save it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
// exhaustive: a state id is its number
#[derive(horizon_mod_api_macros::Wire)]
pub struct BlockStateId(pub u32);

/// A block entity type requested at registration. Its slot remains stable for the process.
// no wire: decoding validates the u32 slot with BlockEntityTypeKey::from_slot on the host; wasm carries the host-issued opaque token without consulting guest tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the load-owned table creates keys
pub struct BlockEntityTypeKey(pub(crate) u32);

/// The existing block entity tick's write-back choice on native and wasm.
// exhaustive: the wire protocol has exactly two snapshot write-back choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum BlockEntityTickReply {
    /// Validate and apply the full payload, as ticks do by default.
    #[default]
    Changed,
    /// Discard this Part's payload edits without encoding, decoding or applying
    /// its reply snapshot. Earlier Parts' accepted `Changed` edits survive.
    /// When every Part answers `Unchanged`, the original entity and revision
    /// survive only while its original placeholder remains, with no tick-only
    /// dirty, container or comparator effects. Independent Host writes retain
    /// their own effects.
    Unchanged,
}

// no wire: borrowed host or registrar backend; access stays scoped to the owning call.
/// A hook's mutable block entity payload, with a read-only type identity.
/// The private handle cannot be constructed or taken out by a mod.
///
/// ```compile_fail
/// use mod_api::{BlockEntityTickEvent, ModBlockEntity};
/// fn replace(event: &mut BlockEntityTickEvent<'_>) {
///     *event.entity = ModBlockEntity::default();
/// }
/// ```
///
/// ```compile_fail
/// use mod_api::BlockEntityTickEvent;
/// fn take(event: &mut BlockEntityTickEvent<'_>) {
///     let _ = std::mem::take(&mut event.entity);
/// }
/// ```
///
/// ```compile_fail
/// use mod_api::{BlockEntityTickEvent, ModBlockEntity};
/// fn access(event: &mut BlockEntityTickEvent<'_>) {
///     let _: &mut ModBlockEntity = event.entity.entity;
/// }
/// ```
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ModBlockEntityView<'a> {
    pub(crate) entity: &'a mut ModBlockEntity,
    tick_listener: bool,
    tick_original: Option<(
        crate::item::containers::ModBlockEntityFields,
        crate::item::containers::ModRevision,
        Option<Compound>,
    )>,
}

impl<'a> ModBlockEntityView<'a> {
    // hidden: a game-side helper lending the checked view to a hook
    #[doc(hidden)]
    pub fn new(entity: &'a mut ModBlockEntity) -> Self {
        Self {
            entity,
            tick_listener: false,
            tick_original: None,
        }
    }
    /// Id.
    pub fn id(&self) -> &str {
        self.entity.id()
    }

    /// Begin one tick listener's lazy payload checkpoint.
    // hidden: the generated listener dispatch brackets each tick listener with this; mods never call it
    #[doc(hidden)]
    pub fn __begin_tick_listener(&mut self) {
        self.tick_listener = true;
    }

    /// Accept this listener's edits, or restore the preceding accepted payload.
    // hidden: the generated listener dispatch brackets each tick listener with this; mods never call it
    #[doc(hidden)]
    pub fn __finish_tick_listener(&mut self, reply: BlockEntityTickReply) {
        self.tick_listener = false;
        if let Some((fields, revision, client_update_fields)) = self.tick_original.take()
            && reply == BlockEntityTickReply::Unchanged
        {
            self.entity.fields = fields;
            self.entity.revision = revision;
            self.entity.client_update_fields = client_update_fields;
        }
    }

    fn checkpoint_tick_listener(&mut self) {
        if self.tick_listener && self.tick_original.is_none() {
            self.tick_original = Some((
                self.entity.fields.clone(),
                self.entity.revision,
                self.entity.client_update_fields.clone(),
            ));
        }
    }
    /// Replace the one-shot fields the block's next change sends clients; an empty compound
    /// clears them. They go over what `block_entity_update_tag` gives (or alone, without
    /// one) in the single packet every current viewer gets, and are gone. Call
    /// `block_entity_sync` to send them: until a change reaches a viewer they wait, however
    /// many chunks are sent, and they end with the block entity. They are for a one-shot
    /// effect, not state: nothing saves, clones or compares them, and the revision stays.
    /// A client merges them into its copy's `nbt` as any update field, where they stay until
    /// sent again: a changing value, such as a counter, tells one effect from the next.
    /// Fields that would take the packet past what a client reads (2 MiB of network NBT) are
    /// dropped with a warning, and the change goes without them. A transaction that rolls the
    /// block entity back also drops a batch queued before it.
    pub fn set_client_update_fields(&mut self, fields: Compound) {
        self.checkpoint_tick_listener();
        self.entity.client_update_fields = Some(fields);
    }
}

impl std::ops::Deref for ModBlockEntityView<'_> {
    type Target = crate::item::containers::ModBlockEntityFields;
    fn deref(&self) -> &Self::Target {
        self.entity
    }
}

impl std::ops::DerefMut for ModBlockEntityView<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.checkpoint_tick_listener();
        self.entity
    }
}

/// What an explosion does to blocks (`Explosion.BlockInteraction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BlockInteraction {
    /// Blocks are left alone.
    Keep,
    /// Blocks are destroyed and drop everything.
    Destroy,
    /// Blocks are destroyed and their drops may decay.
    DestroyWithDecay,
    /// A wind burst: it pushes, and triggers doors, buttons, bells and the like.
    TriggerBlock,
}

/// A depth-tested outline of the client's block selection shape.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct BlockRim {
    /// The block whose outline is drawn.
    pub pos: [i32; 3],
    /// The outline colour as `0xAARRGGBB`.
    pub argb: u32,
    /// The line width in pixels.
    pub width: f32,
}

impl BlockRim {
    /// An outline of the block at `pos` in `argb`, `width` pixels wide.
    pub const fn new(pos: [i32; 3], argb: u32, width: f32) -> Self {
        Self { pos, argb, width }
    }
}

/// Supported vanilla `Block.UPDATE_*` flags for a level block write.
// no wire: encodes the unsigned u32 bits() projection of the internal i32 flag word, preserving unknown bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BlockUpdateFlags(pub(crate) i32);

impl BlockUpdateFlags {
    /// No update flags.
    pub const NONE: Self = Self(0);
    /// Notify neighboring blocks.
    pub const NEIGHBORS: Self = Self(1);
    /// Send the block update to clients.
    pub const CLIENTS: Self = Self(2);
    /// Suppress the client-side block update notification.
    pub const INVISIBLE: Self = Self(4);
    /// Skip neighbor shape updates.
    pub const KNOWN_SHAPE: Self = Self(16);
    /// Suppress drops caused by neighbor shape updates.
    pub const SUPPRESS_DROPS: Self = Self(32);
    /// The replacement is caused by a piston.
    pub const MOVE_BY_PISTON: Self = Self(64);
    /// Skip wire shape updates.
    pub const SKIP_SHAPE_UPDATE_ON_WIRE: Self = Self(128);
    /// Skip the removed block entity's side effects, including container spills.
    pub const SKIP_BLOCK_ENTITY_SIDE_EFFECTS: Self = Self(256);
    /// Skip the new block's on-place behavior.
    pub const SKIP_ON_PLACE: Self = Self(512);
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn bits(self) -> i32 {
        self.0
    }
}

impl std::ops::BitOr for BlockUpdateFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// The empty-context block shape requested from a level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BlockShapeKind {
    /// Selection outline (`BlockState.getShape`).
    Outline,
    /// Collision shape (`BlockState.getCollisionShape`).
    Collision,
}

/// A block face for a level support query.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum BlockFace {
    /// Facing down (-y).
    Down,
    /// Facing up (+y).
    Up,
    /// Facing north (-z).
    North,
    /// Facing south (+z).
    South,
    /// Facing west (-x).
    West,
    /// Facing east (+x).
    East,
}

/// The support area tested on a block face.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum SupportType {
    /// Full.
    Full,
    /// Center.
    Center,
    /// Rigid.
    Rigid,
}

/// One block entity in an open level, identified by its lifecycle generation.
/// Removal, replacement or chunk reload invalidates the handle, even at the same
/// position. In-place data edits and temporary tick take/put preserve identity.
/// Stale handles read as `None`; Result-returning methods report `Stale`.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BlockEntityRef {
    pub(crate) level: LevelRef,
    pub(crate) position: [i32; 3],
    pub(crate) generation: u64,
}

/// Why a block-entity operation could not resolve its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BlockEntityError {
    /// The entity was removed, replaced, unloaded or belongs to another level epoch.
    Stale,
}

impl NoBackend for BlockEntityError {
    fn no_backend() -> Self {
        Self::Stale
    }
}

/// An effect a fake player's action is stopped from starting. Kinds may be added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum FakeEffect {
    /// A menu or screen: a container, a station, a sign's editor.
    Menu,
    /// Sleeping in a bed.
    Sleep,
    /// Riding an entity or a vehicle.
    Ride,
    /// Going to another dimension.
    ChangeDimension,
}

/// Where a block use clicked, as vanilla's `BlockHitResult` says it. Fields
/// may be added: build one with [`BlockHit::new`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct BlockHit {
    /// The point clicked, in world coordinates (`getLocation`).
    pub point: [f64; 3],
    /// The face clicked, `[0, 1, 0]` for the top (`getDirection`).
    pub face: [i32; 3],
    /// The ray started inside the block's shape (`isInside`).
    pub inside: bool,
}

impl BlockHit {
    /// New.
    pub const fn new(point: [f64; 3], face: [i32; 3], inside: bool) -> Self {
        Self {
            point,
            face,
            inside,
        }
    }
}

/// The result of previewing a tool's block transformation. Applying it is a separate action.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct BlockTransformPreview {
    /// The resulting block state.
    pub state: BlockState,
    /// Sound played by the action.
    pub sound: String,
    /// Optional loot table generated by the action.
    pub loot: Option<String>,
    /// Durability cost of the action.
    pub durability: u32,
    /// Particle or level-event identifier.
    pub particle: String,
}

impl BlockTransformPreview {
    /// Construct a transformation result from the engine's decision.
    pub fn new(
        state: BlockState,
        sound: String,
        loot: Option<String>,
        durability: u32,
        particle: String,
    ) -> Self {
        Self {
            state,
            sound,
            loot,
            durability,
            particle,
        }
    }
}

/// What used bone meal on a block (`BonemealSource`), for `bonemeal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BonemealSource {
    /// Bone meal used through `BoneMealItem.growCrop`: by a player, a
    /// dispenser or a villager.
    Interaction,
    /// A mob growing the block itself.
    Mob,
}

/// One block a placement writes (`entity_place`'s `Placement::Multi`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct BlockPlacement {
    /// Where the block goes.
    pub pos: BlockPos,
    /// The state it is placed in.
    pub state: BlockState,
}

impl BlockPlacement {
    /// Create a placement for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(pos: BlockPos, state: BlockState) -> Self {
        Self { pos, state }
    }
}

/// What an `entity_place` writes: one block, or the blocks of a multi-block
/// placement (`EntityMultiPlaceEvent`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum Placement {
    /// The placed block alone.
    Single,
    /// Every block the placement writes, the primary block first: a bed's
    /// foot and head, a door's halves, a tall plant's two halves.
    Multi(Vec<BlockPlacement>),
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// A player's right click on a block, shared by the block's use behaviours
/// and `right_click_block` (`PlayerInteractEvent.RightClickBlock`). The
/// event hears it once per hand on the clicking client and on the server.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct BlockClick<'a> {
    /// The side running the click (`getSide`).
    pub side: LogicalSide,
    /// The clicking player's level (`getLevel`).
    pub dimension: LevelDimension,
    /// The block clicked (`getPos`).
    pub pos: [i32; 3],
    /// Where it was clicked (`getHitVec`); `hit.face` is `getFace`.
    pub hit: BlockHit,
    /// The hand clicking (`getHand`) and what it holds (`getItemStack`).
    pub hand: Hand,
    /// The stack.
    pub stack: Option<&'a ItemStack>,
    /// The clicking player's UUID and entity id (`getEntity`).
    pub player: u128,
    /// The player entity id.
    pub player_entity_id: i32,
}

impl FallOnEvent<'_> {
    /// Ask for a write at the block the entity landed on. It is applied once the entity's
    /// batch has ticked, in the order asked, if the block is still the state it landed on.
    pub fn request(&mut self, write: LandingWrite) {
        self.writes.push(write);
    }
}

/// Which client synchronization route supplied a block entity's data.
// exhaustive: the protocol supplies block entity tags through these two routes
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum BlockEntityDataSource {
    /// Initial synchronization in a chunk packet; empty tags send no notice.
    Chunk,
    /// A later block entity data packet; even an empty compound sends a notice.
    UpdatePacket,
}

impl EntityInsideEvent<'_> {
    /// Queue one of the five inside-block effect types (`InsideBlockEffectApplier.apply`).
    /// It applies to the swept entity inside its own tick, once per step, in the
    /// collector's step and type order.
    pub fn apply(&mut self, effect: InsideEffect) {
        self.effects.apply(effect);
    }
    /// Queue a typed write to the swept entity that applies just before `effect` in the
    /// flush order (`InsideBlockEffectApplier.runBefore`).
    pub fn run_before(&mut self, effect: InsideEffect, write: EntityWrite) {
        self.effects.run_before(effect, write);
    }
    /// Queue a typed write to the swept entity that applies just after `effect` in the
    /// flush order (`InsideBlockEffectApplier.runAfter`).
    pub fn run_after(&mut self, effect: InsideEffect, write: EntityWrite) {
        self.effects.run_after(effect, write);
    }
    /// Ask for a write at the touched block. It is applied once the entity's batch has
    /// ticked, in the order asked, if the block is still the state the entity touched.
    pub fn request(&mut self, write: InsideWrite) {
        self.writes.push(write);
    }
}
