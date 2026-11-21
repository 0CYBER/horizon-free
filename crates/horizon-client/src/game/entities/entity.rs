//! `Entity`: what every kind in the game's entity lists answers to, so a system
//! can treat them alike and only write down where vanilla's kinds differ.
//!
//! Default methods follow vanilla's `Entity`; a kind overrides one only where
//! its vanilla class does. [`EntityHandle`] names one entity by list and index,
//! so a system can walk handles and reach each entity after other borrows of
//! `Game` end. The walk allocates nothing and dispatch is static: hot paths pay
//! no vtable. A new kind is one line in `entity_lists!` and one `impl Entity`.

use super::commands::Target;
use super::persist::ChunkKey;
use super::{FallingBlock, Game, general::GeneralRow};
use super::{
    boats::Boat, fireworks::Rocket, minecarts::Minecart, tnt::PrimedTnt, trident::ThrownTrident,
};
use horizon_client::entities::{
    ArmorStand, Cushion, ExperienceOrb, ItemEntity, ItemFrame, Painting,
};
use horizon_client::entity_rules::EntityRules;
use horizon_client::join_seq::JoinSeq;
use horizon_client::join_seq::Joined as _;
use horizon_client::mobs::{Mob, Species};
use horizon_client::projectiles::{Projectile, ProjectileKind};
use horizon_core::geometry::{Aabb, Direction, Vec3};
use horizon_core::nbt::Tag;
use horizon_core::position::BlockPos;
use horizon_world::{level::data::PushReaction, move_gate::MoveGate, piston_push};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

/// Assign a stored owner's delayed id at its existing allocation point.
/// Kinds without a lifetime slot pass None; a bound owner must keep its id.
pub(super) fn assign_stored_id(
    next: &mut u32,
    id: &mut u32,
    lifetime: Option<&horizon_mod_runtime::attachments::AttachmentLifetime>,
) -> u32 {
    debug_assert!(*id != 0 || lifetime.is_none_or(|slot| !slot.is_bound()));
    super::lan_entities::assign_entity_id(next, id)
}

/// What `Entity.thunderHit` does to a kind: it sets the entity on fire and
/// calls `hurtServer(lightning_bolt, 5)`, so a kind whose `hurtServer` does
/// nothing (projectiles, rockets, TNT, falling blocks, armor stands here) stays
/// untouched. A strike runs the stages in this order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ThunderHit {
    Untouched,
    /// Burns, takes 5 damage, and the kinds with their own `thunderHit` change.
    Mob,
    /// `VehicleEntity.hurtServer`: 5 x 10 is over the 40 that destroys it.
    Vehicle,
    /// `ExperienceOrb.hurtServer`: 5 damage off 5 health.
    Orb,
    /// `ItemEntity.hurtServer`: burns up unless fire resistant.
    Item,
}

/// What a blast knows about itself, for the entities in its reach to answer.
#[derive(Clone, Copy)]
pub(super) struct Blast {
    /// `VehicleEntity.ignoreExplosion`: a mob's blast, with mob griefing off.
    pub(super) vehicles_ignore: bool,
    /// `ItemEntity.ignoreExplosion` (`shouldAffectBlocklikeEntities`).
    pub(super) items_ignore: bool,
    /// The exploding mob, which `getEntities(source, box)` leaves out.
    pub(super) spared: Option<u32>,
}

pub(super) trait Entity: EntityRules {
    /// The kind's serializer overlays its object-owned attachments itself (for
    /// recursive object trees); the common save walk must not encode them twice.
    const SERIALIZES_ATTACHMENTS: bool = false;

    /// `Entity.getId`.
    fn id(&self) -> u32;
    fn position(&self) -> Vec3;
    fn position_mut(&mut self) -> &mut Vec3;
    /// Where it stood when the last tick began (`Entity.lastKnownPosition`'s
    /// reading): `position - previous_position` is `computeSpeed`.
    fn previous_position(&self) -> Vec3;
    /// `Entity.getDeltaMovement`.
    fn velocity(&self) -> Vec3;
    fn velocity_mut(&mut self) -> &mut Vec3;
    /// `Entity.resetFallDistance`: how far it has fallen is forgotten, so its next landing
    /// hurts it as a step does. Required, so a new kind states whether it keeps a fall distance.
    fn reset_fall_distance(&mut self);
    /// `Entity.getYRot` and `getXRot`: yaw, pitch.
    fn rotation(&self) -> [f32; 2];
    /// The rotation part of `Entity.teleportSetPosition`: `setYRot`, `setXRot`
    /// (clamped to -90..90), then `setOldRot`.
    fn set_rotation(&mut self, rotation: [f32; 2]);
    fn bounds(&self) -> Aabb;
    /// What a selector finds this entity as (`@e`), or `None` for one it cannot
    /// name yet. Required, so a new kind states which.
    fn selector_target(&self) -> Option<Target>;
    /// The entity type id a native mod's join hook names (`minecraft:oak_boat`).
    fn native_type(&self) -> Cow<'static, str>;
    /// Saves entity `index` of this kind into its chunk's entity list: the
    /// chunk and the NBT when it lies in `keys` and is saved with its chunk.
    /// Required, so a new kind states how it saves.
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)>;
    /// Loads a saved entity whose type id is `kind` as this kind and joins it to
    /// the world, or hands `tag` back when `kind` is not one this kind loads.
    /// Required, so a new kind states how it loads: it cannot be saved and then
    /// dropped silently on the way back in.
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag>;

    fn on_ground(&self) -> bool {
        false
    }
    /// `Entity.maxUpStep`.
    fn get_step_height(&self) -> f32 {
        0.0
    }
    fn is_alive(&self) -> bool {
        true
    }
    /// `Entity.isRemoved`.
    fn is_removed(&self) -> bool {
        false
    }
    /// `instanceof LivingEntity`.
    fn is_living(&self) -> bool {
        false
    }
    /// `Entity.isIgnoringBlockTriggers`.
    fn is_ignoring_block_triggers(&self) -> bool {
        false
    }
    /// Whether it presses plates and tripwires: `getEntitiesOfClass` skips
    /// removed entities, and the block's predicate skips the ones above.
    fn triggers_blocks(&self) -> bool {
        !self.is_removed() && !self.is_ignoring_block_triggers()
    }
    /// Whether `tick_blocks` sends the blocks in its box to `entityInside`
    /// (`applyEffectsFromBlocks`). A kind that function sweeps another way, or
    /// whose tick skips that step, says no.
    fn sweeps_blocks_inside(&self) -> bool {
        true
    }
    /// `Entity.thunderHit` for this kind.
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Untouched
    }
    /// The box a lightning strike tests against.
    fn thunder_bounds(&self) -> Aabb {
        self.bounds()
    }
    /// `Entity.ignoreExplosion`: the blast neither hurts nor pushes it. Also
    /// answers for what `getEntities(source, box)` never lists: the exploding
    /// mob, and the dead.
    fn ignores_explosion(&self, _blast: &Blast) -> bool {
        false
    }
    /// Whether a burst of wind moves it: what `Entity.push` moves, unless the
    /// kind says otherwise.
    fn pushed_by_wind(&self) -> bool {
        self.pushed_by_explosion()
    }
    /// Whether `Entity.push` moves it. An arrow in the ground, a wind charge and
    /// an armour stand (`explosion_breaks_decorations` takes those) stay put.
    fn pushed_by_explosion(&self) -> bool {
        true
    }
    /// The box the blast casts its rays at: `getSeenPercent`.
    fn explosion_box(&self) -> Aabb {
        self.bounds()
    }
    /// How far above its feet `pushFromExplosion` aims: `getEyeHeight`, which
    /// defaults to 0.85 of the box.
    fn explosion_eye_height(&self) -> f64 {
        let b = self.explosion_box();
        f64::from((b.max_position().y - b.min_position().y) as f32 * 0.85f32)
    }
    /// Whether this kind can still be waiting for its tick when a projectile
    /// ticks, though it joined before it (orbs, falling blocks, primed TNT: they
    /// tick between the batches, `game/world/ordered_tick.rs`).
    fn ticks_after_projectiles(&self) -> bool {
        false
    }
    /// `Entity.push` with the explosion's knockback.
    fn push_from_explosion(&mut self, push: Vec3) {
        let v = self.velocity_mut();
        *v = Vec3::new(v.x + push.x, v.y + push.y, v.z + push.z);
    }
    /// `Entity.isSupportedBy`.
    fn is_supported_by(&self, _pos: BlockPos) -> bool {
        false
    }
    /// `Entity.getEyeY`, where `updateFluidHeightAndDoFluidPushing` reads the
    /// eye's fluid; the top of the box unless a kind sets it.
    fn fluid_eye_y(&self) -> f64 {
        self.bounds().max_position().y
    }
    /// `Entity.updateFluidInteraction`: its water and lava heights
    /// (`(0, 0)` for a kind that is not pushed by fluid), and the current of
    /// each fluid added to its velocity. One body for every kind.
    fn push_by_fluid(
        &mut self,
        world: &horizon_client::world::World,
        fluids: &horizon_client::motion::FluidResources,
        context: &horizon_world::native_load_context::LoopContext,
    ) -> (f64, f64) {
        if !self.is_pushed_by_fluid() {
            return (0.0, 0.0);
        }
        let (bounds, at, eye) = (self.bounds(), self.position(), self.fluid_eye_y());
        let (water, lava, _) = horizon_client::entities::update_entity_fluids(
            bounds,
            at,
            eye,
            self.velocity_mut(),
            world,
            fluids,
            context,
        );
        (water, lava)
    }
    /// `BubbleColumnBlock.entityInside` through `applyEffectsFromBlocks`: a
    /// column lifts or drags every kind alike.
    fn push_by_bubble_column(
        &mut self,
        world: &horizon_client::world::World,
        registry: &horizon_world::registry::BlockRegistry,
        facts: &horizon_client::blocks::BlockFacts,
        collision: &horizon_world::block_collision::BlockCollisionRegistry<'_>,
    ) -> bool {
        self.push_by_bubble_column_over(world, registry, facts, collision, |_| {})
    }
    /// [`Entity::push_by_bubble_column`], telling `surface` the position of
    /// each column block it lies in (for the bubbles at the top).
    fn push_by_bubble_column_over(
        &mut self,
        world: &horizon_client::world::World,
        registry: &horizon_world::registry::BlockRegistry,
        facts: &horizon_client::blocks::BlockFacts,
        collision: &horizon_world::block_collision::BlockCollisionRegistry<'_>,
        surface: impl FnMut(BlockPos),
    ) -> bool {
        let bounds = self.bounds();
        horizon_client::motion::bubble_column_push_over(
            world,
            registry,
            facts,
            collision,
            bounds,
            self.velocity_mut(),
            surface,
        )
    }
    /// `Entity.getPistonPushReaction`.
    fn get_piston_push_reaction(&self) -> PushReaction {
        PushReaction::PushPull
    }
    /// Width and height of the box a portal sweep follows, or None for a kind
    /// the game does not carry through portals (so far falling blocks; fishing
    /// hooks are off the walk, and `FishingHook.canUsePortal` is false in
    /// vanilla).
    fn portal_size(&self) -> Option<(f32, f32)> {
        None
    }
    /// The box a portal sweep follows at `position` (`Entity.getBoundingBox`).
    /// The default is the box [`Self::portal_size`] describes; a kind whose
    /// real box differs from its dimensions (an open shulker) overrides it.
    fn portal_bounds(&self, position: Vec3) -> Option<Aabb> {
        let (width, height) = self.portal_size()?;
        let half = f64::from(width) / 2.0;
        Some(Aabb::new(
            Vec3::new(position.x - half, position.y, position.z - half),
            Vec3::new(
                position.x + half,
                position.y + f64::from(height),
                position.z + half,
            ),
        ))
    }
    /// Whether [`Self::portal_size`] is Some, without working the size out (a
    /// mob's depends on its age, scale and pose).
    fn sweeps_portals(&self) -> bool {
        self.portal_size().is_some()
    }
    /// `Entity.canUsePortal(false)` for a kind in `portal_size`. A kind that
    /// cannot stays tracked, so its cooldown and process run out.
    fn can_use_portal(&self, _game: &Game<'_>) -> bool {
        true
    }
    /// A boat or minecart: the kinds whose portal pass opens the tick, before
    /// they move (`AbstractMinecart.tick` handles its portal first).
    fn is_vehicle(&self) -> bool {
        false
    }
    /// Whether movement applies portal block effects before Entity's base tick.
    fn portal_after_move(&self) -> bool {
        false
    }
    /// Where it stood at the start of its tick, for the kinds that store it
    /// (None: `portals.before` has it).
    fn portal_from(&self) -> Option<Vec3> {
        None
    }
    /// Whether this tick's movement can have entered a portal block: a stuck
    /// arrow or a trident in the ground that did not move enters none.
    fn portal_sweeps(&self) -> bool {
        true
    }
    /// `Entity.noPhysics` between ticks: its moves pass through the blocks,
    /// and a shulker's lid leaves it be. `stored` is the value a mod wrote for
    /// a kind that keeps no field of its own (`EntityMemory.no_physics`); None
    /// when none was written, so the kind answers its own default.
    fn no_physics(&self, stored: Option<bool>) -> bool {
        stored.unwrap_or(false)
    }
    /// Apply this kind's resolved movement after the invocation's gate is armed.
    /// Every entity kind states how it moves; family ticks resolve through
    /// the same gate in their own movement path.
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3);
    /// Bounds for `AbstractArrow.shouldFall` after a non-SELF piston move.
    fn piston_release_bounds(&self) -> Option<Aabb> {
        None
    }
    fn release_after_piston(&mut self, _no_block_collision: bool) {}
    /// `setOnGroundWithMovement` at the end of a piston's or shulker lid's
    /// `Entity.move`, for the kinds that keep the flag: on a server it runs
    /// for every push, sideways ones and those the collision turned away included.
    fn set_piston_on_ground(&mut self, _on_ground: bool) {}
    /// `Entity.setOnGround` from outside the entity's own move (`/tp`'s
    /// `TeleportCommand.performTeleport`); kinds that keep no ground flag ignore it.
    fn set_on_ground(&mut self, _on_ground: bool) {}
    /// `PistonMovingBlockEntity.causeBounce`: a slime block sets the velocity
    /// along the movement axis.
    fn piston_slime_bounce(&mut self, movement: Direction) {
        let velocity = self.velocity_mut();
        *velocity = piston_push::bounce_velocity(movement, *velocity);
    }
}

/// Bounds and identity needed by the render occlusion culler. Moving renderers
/// use interpolated entity bounds; attached entities use their own world bounds.
pub(super) trait RenderEntityBounds {
    fn render_id(&self) -> u32;
    fn render_bounds(&self, alpha: f64) -> Aabb;
    fn render_glowing(&self, game: &Game<'_>) -> bool {
        game.tracked_entity_glowing(self.render_id())
    }
}

impl RenderEntityBounds for Mob {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, alpha: f64) -> Aabb {
        // Keep models with large independent appendages on the old envelope:
        // their tentacles and wings can extend several blocks past the entity
        // dimensions. Equipment and custom rendering bypass this pre-check at
        // the call site.
        let (width, height, _) = self.awake_dims();
        let at = self.previous.lerp(self.position, f64::from(alpha as f32));
        if matches!(
            self.species(),
            Species::Guardian | Species::ElderGuardian | Species::Phantom
        ) {
            // The model's root scales its appendages too (the elder guardian
            // uses 2.35), so the margin must grow along with the model.
            let root_scale = self.kind.model.pose[6..]
                .iter()
                .copied()
                .map(f32::abs)
                .fold(1.0, f32::max);
            let radius = width.max(height) * 2.5 + 3.0 * f64::from(root_scale * self.model_scale());
            return Aabb::of_size(at, Vec3::new(radius * 2.0, radius * 2.0, radius * 2.0));
        }

        // Vanilla's ordinary models can rotate their head, limbs and death
        // pose beyond the collision box. This envelope covers those poses while
        // remaining close to the rendered mesh for the common upright mobs.
        // A few quadruped death and riding poses reach several blocks past
        // their standing width. Keep a bounded fallback radius for ordinary
        // models while still shrinking the common pig/zombie envelope below
        // the old five-block radius.
        let horizontal = (width * 2.0).max(height * 1.5).max(3.5);
        let vertical = height * 3.0 + 0.5;
        Aabb::new(
            at.subtract(Vec3::new(horizontal, vertical, horizontal)),
            at.add_vector(Vec3::new(horizontal, vertical, horizontal)),
        )
    }

    fn render_glowing(&self, _game: &Game<'_>) -> bool {
        self.is_currently_glowing()
    }
}

impl RenderEntityBounds for ArmorStand {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, alpha: f64) -> Aabb {
        let at = self.previous.lerp(self.position, alpha);
        // ArmorStandRenderer poses the limbs, then attaches hand items, head
        // items, armour and folded elytra. Those can extend well beyond the
        // 0.5 x 1.975 collision box (especially raised arms and head banners).
        // This axis-aligned envelope contains every vanilla pose while staying
        // independent of the stand's yaw and partial-tick rotation.
        let scale = if self.small { 0.5 } else { 1.0 };
        let equipped = self.equipment.iter().any(Option::is_some);
        let horizontal = if equipped { 2.0 } else { 1.5 * scale };
        let lower = if equipped { -1.0 } else { -0.5 * scale };
        let upper = 3.5 * scale
            + if self.equipment[5].is_some() {
                1.5
            } else {
                0.0
            };
        Aabb::new(
            at.subtract(Vec3::new(horizontal, -lower, horizontal)),
            at.add_vector(Vec3::new(horizontal, upper, horizontal)),
        )
    }
}

impl RenderEntityBounds for ItemEntity {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, alpha: f64) -> Aabb {
        let at = self.previous.lerp(self.position, alpha);
        // DroppedItemRenderer bobs, spins, and fans out up to five item copies.
        // The draw path uses the resolved model's exact posed extents; this
        // fallback envelope also covers the largest vanilla ground item.
        Aabb::new(
            at.subtract(Vec3::new(0.75, 0.1, 0.75)),
            at.add_vector(Vec3::new(0.75, 1.0, 0.75)),
        )
    }
}

impl RenderEntityBounds for Boat {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, alpha: f64) -> Aabb {
        let at = self.previous.lerp(self.position, alpha);
        // The boat model turns with yaw and its animated paddles sweep outside
        // the sized collision box. Hurt and bubble-column rotations are covered
        // by this orientation-independent model envelope.
        Aabb::new(
            at.subtract(Vec3::new(1.75, 0.75, 1.75)),
            at.add_vector(Vec3::new(1.75, 1.5, 1.75)),
        )
    }
}

impl RenderEntityBounds for Minecart {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, alpha: f64) -> Aabb {
        let at = self.previous.lerp(self.position, alpha);
        let offset = self.custom_offset.map_or_else(
            || {
                if self.kind == super::minecarts::CartKind::Hopper {
                    1.0
                } else {
                    6.0
                }
            },
            f64::from,
        );
        let display_shift = 0.75 * (offset - 8.0) / 16.0;
        let display_low = at.y + 0.375 - 0.4 + display_shift;
        let display_high = at.y + 0.375 + 0.4 + display_shift + 0.9;
        Aabb::new(
            Vec3::new(at.x - 1.25, (at.y - 0.5).min(display_low), at.z - 1.25),
            Vec3::new(at.x + 1.25, (at.y + 1.0).max(display_high), at.z + 1.25),
        )
    }
}

fn aabb_from_extents(bounds: ([f64; 3], [f64; 3])) -> Aabb {
    Aabb::new(
        Vec3::new(bounds.0[0], bounds.0[1], bounds.0[2]),
        Vec3::new(bounds.1[0], bounds.1[1], bounds.1[2]),
    )
}

impl RenderEntityBounds for Painting {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, _alpha: f64) -> Aabb {
        aabb_from_extents(self.bounds())
    }
}

impl RenderEntityBounds for ItemFrame {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, _alpha: f64) -> Aabb {
        let (lo, hi) = self.bounds();
        let step = self.step().map(f64::from);
        let center: [f64; 3] = std::array::from_fn(|axis| (lo[axis] + hi[axis]) * 0.5);
        // Maps fill a full block, while the held item projects toward the
        // camera from the frame plane. Include both in the occlusion envelope.
        let half: [f64; 3] = std::array::from_fn(|axis| if step[axis] != 0.0 { 0.75 } else { 0.5 });
        aabb_from_extents((
            std::array::from_fn(|axis| center[axis] - half[axis]),
            std::array::from_fn(|axis| center[axis] + half[axis]),
        ))
    }
}

impl RenderEntityBounds for Cushion {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, _alpha: f64) -> Aabb {
        aabb_from_extents(self.bounds())
    }
}

/// Whether this projectile still lives its list's tick: an arrow's state was
/// not removed. A thrown projectile leaves its list through the batch's own
/// keep flags, not a field of its own.
fn shot_alive(shot: &Projectile) -> bool {
    shot.arrow.as_ref().is_none_or(|a| !a.removed)
}

/// The item's view of the shared inside walk: its box, its life, the batch's
/// blocks, and the part's contact, fired mid-walk over a lent level. An item
/// has no per-block vanilla arms of its own in the walk: its fire, lava and
/// cactus contacts are its own tick's (`ItemEntity::environment_tick`), kept
/// where they were.
pub(in crate::game) struct ItemInside<'a, 'g> {
    pub(in crate::game) game: &'a mut Game<'g>,
    pub(in crate::game) item: &'a mut ItemEntity,
}

impl horizon_client::block_contact::InsideWalk for ItemInside<'_, '_> {
    type Error = ();
    fn bounds_at(&self, at: Vec3) -> Aabb {
        let b = self.item.bounds();
        let d = at.subtract(self.item.position);
        Aabb::new(
            b.min_position().add_vector(d),
            b.max_position().add_vector(d),
        )
    }
    fn is_alive(&self) -> bool {
        self.item.health > 0
    }
    fn block(&self, at: horizon_core::position::BlockPos) -> horizon_world::registry::BlockStateId {
        self.game.world.block(at.x, at.y, at.z)
    }
    fn is_air(&self, state: horizon_world::registry::BlockStateId) -> bool {
        self.game.pack.assets.registry.state(state).is_none_or(|d| {
            matches!(
                d.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        })
    }
    fn inside_shape(
        &mut self,
        _state: horizon_world::registry::BlockStateId,
        _at: horizon_core::position::BlockPos,
    ) -> horizon_client::block_contact::InsideShape {
        // `getEntityInsideCollisionShape` is the whole block for every block
        // an item meets, as the item's own sweep always treated it.
        horizon_client::block_contact::InsideShape::Full
    }
    fn entity_inside(
        &mut self,
        at: horizon_core::position::BlockPos,
        state: horizon_world::registry::BlockStateId,
        _step: i32,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) -> std::result::Result<(), ()> {
        let id = self.item.id;
        self.game
            .fire_batch_inside(id as i32, "minecraft:item", state, at, precise, effects);
        Ok(())
    }
}

/// The projectile's view of the shared inside walk, the item's shape of it.
pub(in crate::game) struct ProjectileInside<'a, 'g> {
    pub(in crate::game) game: &'a mut Game<'g>,
    pub(in crate::game) shot: &'a mut Projectile,
}

impl horizon_client::block_contact::InsideWalk for ProjectileInside<'_, '_> {
    type Error = ();
    fn bounds_at(&self, at: Vec3) -> Aabb {
        let b = self.shot.bounds();
        let d = at.subtract(self.shot.position);
        Aabb::new(
            b.min_position().add_vector(d),
            b.max_position().add_vector(d),
        )
    }
    fn is_alive(&self) -> bool {
        shot_alive(self.shot)
    }
    fn block(&self, at: horizon_core::position::BlockPos) -> horizon_world::registry::BlockStateId {
        self.game.world.block(at.x, at.y, at.z)
    }
    fn is_air(&self, state: horizon_world::registry::BlockStateId) -> bool {
        self.game.pack.assets.registry.state(state).is_none_or(|d| {
            matches!(
                d.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        })
    }
    fn inside_shape(
        &mut self,
        _state: horizon_world::registry::BlockStateId,
        _at: horizon_core::position::BlockPos,
    ) -> horizon_client::block_contact::InsideShape {
        horizon_client::block_contact::InsideShape::Full
    }
    fn entity_inside(
        &mut self,
        at: horizon_core::position::BlockPos,
        state: horizon_world::registry::BlockStateId,
        _step: i32,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) -> std::result::Result<(), ()> {
        let (id, kind) = (self.shot.id, self.shot.entity_type());
        self.game
            .fire_batch_inside(id as i32, kind, state, at, precise, effects);
        Ok(())
    }
}
#[cfg(test)]
mod render_bounds_tests {
    use super::*;

    #[test]
    fn armor_stand_bounds_include_raised_equipment_space() {
        let mut stand = ArmorStand::new(1, Vec3::new(4.0, 64.0, -2.0), 90.0);
        stand.show_arms = true;
        stand.pose[2] = [180.0, 0.0, 45.0];
        stand.equipment[0] = Some(horizon_client::items::ItemStack {
            item: "minecraft:iron_sword".into(),
            count: 1,
            damage: 0,
            extra: None,
        });
        stand.equipment[5] = Some(horizon_client::items::ItemStack {
            item: "minecraft:banner".into(),
            count: 1,
            damage: 0,
            extra: None,
        });
        let bounds = stand.render_bounds(0.5);
        assert!(bounds.min_position().x <= 2.0);
        assert!(bounds.max_position().x >= 6.0);
        assert!(bounds.min_position().y <= 63.0);
        assert!(bounds.max_position().y >= 69.0);
        assert!(bounds.min_position().z <= -4.0);
        assert!(bounds.max_position().z >= 0.0);
    }

    #[test]
    fn boat_bounds_cover_the_rotated_paddle_sweep() {
        let boat = Boat::new(2, "oak".into(), Vec3::new(0.0, 10.0, 0.0), 90.0);
        let bounds = boat.render_bounds(0.5);
        assert!(bounds.min_position().x <= -1.75);
        assert!(bounds.max_position().x >= 1.75);
        assert!(bounds.min_position().y <= 9.25);
        assert!(bounds.max_position().y >= 11.5);
        assert!(bounds.min_position().z <= -1.75);
        assert!(bounds.max_position().z >= 1.75);
    }

    #[test]
    fn item_frame_bounds_include_the_full_map_and_held_item_depth() {
        let frame = ItemFrame {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 3,
            pos: [0, 0, 0],
            facing: 5,
            glow: false,
            item: None,
            rotation: 0,
            drop_chance: 1.0,
            fixed: false,
            invisible: false,
            check_interval: 0,
            custom_name: None,
            random: horizon_core::random::EntityRandom::own(
                horizon_core::random::LegacyRandom::new(0),
            ),
        };
        let bounds = frame.render_bounds(1.0);
        assert!(bounds.min_position().x <= -0.71875);
        assert!(bounds.max_position().x >= 0.78125);
        assert!(bounds.min_position().y <= 0.0);
        assert!(bounds.max_position().y >= 1.0);
        assert!(bounds.min_position().z <= 0.0);
        assert!(bounds.max_position().z >= 1.0);
    }

    #[test]
    fn minecart_bounds_follow_custom_display_offset() {
        let mut cart = Minecart::new(
            4,
            super::super::minecarts::CartKind::Chest,
            Vec3::new(0.0, 10.0, 0.0),
        );
        cart.custom_offset = Some(32);
        let bounds = cart.render_bounds(1.0);
        assert!(bounds.min_position().y <= 9.5);
        assert!(bounds.max_position().y >= 12.8);
    }
}

/// The fields every kind stores under the same names.
macro_rules! stored_motion {
    () => {
        fn id(&self) -> u32 {
            self.id
        }
        fn position(&self) -> Vec3 {
            self.position
        }
        fn position_mut(&mut self) -> &mut Vec3 {
            &mut self.position
        }
        fn previous_position(&self) -> Vec3 {
            self.previous
        }
        fn velocity(&self) -> Vec3 {
            self.velocity
        }
        fn velocity_mut(&mut self) -> &mut Vec3 {
            &mut self.velocity
        }
    };
}

/// One `ItemEntity.tick`, with the fluid push and the bubble column decided by
/// the `Entity` defaults every kind shares: fluids, move, column, settle,
/// fluids again. Returns false when the item should be removed. A server's tick
/// runs the halves itself, with the landing's `Block.fallOn` between them.
impl Game<'_> {
    /// Tick an item owned by the caller. The results are ordinary survival,
    /// explicit consumption by a block-contact handler, and the fall it landed
    /// with on a server (its block's `fallOn` is the caller's). `escape_speed`:
    /// the item is inside a block and moves without physics this tick, pushed
    /// out at this speed.
    /// The item's `Entity.checkInsideBlocks`, run at its batch with the part's
    /// contact fired mid-walk: the shared sweep over the item's move, the staged
    /// fire over a lent level (as `item_fall_on` lends one), and the flush of
    /// the queued writes into this item, its own tick. Nothing runs without a
    /// listening part: one `any_inside` test is the whole cost.
    pub(in crate::game) fn sweep_item_inside(&mut self, item: &mut ItemEntity) {
        if !horizon_client::block_contact::HOOKS
            .get()
            .is_some_and(|hooks| (hooks.any_inside)())
        {
            return;
        }
        let mut effects = mod_api::InsideEffects::default();
        let mut inside = ItemInside { game: self, item };
        let movement = horizon_world::entity_move::MovementRecord {
            from: inside.item.previous,
            to: inside.item.position,
            original: None,
        };
        let mut visited = crate::fast_hash::FxHashSet::default();
        let _ = horizon_client::block_contact::sweep_movements(
            &mut inside,
            &[movement],
            &mut visited,
            &mut effects,
        );
        let ItemInside { game, item } = inside;
        // `applyAndClear`: the writes apply in the collector's order, in this
        // item's tick, until it is destroyed.
        for planned in effects.take_planned() {
            if item.health <= 0 {
                break;
            }
            if let mod_api::PlannedInside::Write(write) = planned {
                game.apply_item_write(item, write);
            }
        }
    }

    /// A typed entity write an `entity_inside` part queued for an item, applied
    /// in its own tick. An item takes no effects and keeps no stuck multiplier
    /// (its tick's friction reads none): `StuckIn` scales the movement once,
    /// `AddEffect` is skipped, as vanilla's items keep no effect list.
    fn apply_item_write(&mut self, item: &mut ItemEntity, write: mod_api::EntityWrite) {
        let triple = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
        match write {
            mod_api::EntityWrite::Push(v) => {
                item.velocity = item.velocity.add_vector(triple(v));
            }
            mod_api::EntityWrite::SetMovement(v) => item.velocity = triple(v),
            mod_api::EntityWrite::StuckIn(v) => {
                item.velocity = Vec3::new(
                    item.velocity.x * v[0],
                    item.velocity.y * v[1],
                    item.velocity.z * v[2],
                );
            }
            mod_api::EntityWrite::Hurt { amount, .. } => {
                let _ = item.hurt(amount as i32);
            }
            mod_api::EntityWrite::ResetFallDistance => item.fall_distance = 0.0,
            mod_api::EntityWrite::SetFireTicks(ticks) => item.fire_ticks = ticks,
            mod_api::EntityWrite::AddEffect { .. } => {}
        }
    }

    /// The projectile's `Entity.checkInsideBlocks`, the same batch sweep the
    /// item's is: the shared walk over its move, the staged fire over a lent
    /// level, and the flush into this projectile's own tick.
    pub(in crate::game) fn sweep_projectile_inside(&mut self, shot: &mut Projectile) {
        if !horizon_client::block_contact::HOOKS
            .get()
            .is_some_and(|hooks| (hooks.any_inside)())
        {
            return;
        }
        let mut effects = mod_api::InsideEffects::default();
        let mut inside = ProjectileInside { game: self, shot };
        let movement = horizon_world::entity_move::MovementRecord {
            from: inside.shot.previous,
            to: inside.shot.position,
            original: None,
        };
        let mut visited = crate::fast_hash::FxHashSet::default();
        let _ = horizon_client::block_contact::sweep_movements(
            &mut inside,
            &[movement],
            &mut visited,
            &mut effects,
        );
        let ProjectileInside { game, shot } = inside;
        for planned in effects.take_planned() {
            if !shot_alive(shot) {
                break;
            }
            if let mod_api::PlannedInside::Write(write) = planned {
                game.apply_projectile_write(shot, write);
            }
        }
    }

    /// A typed entity write for a projectile, applied in its own tick. A
    /// projectile takes no effects, no damage table and keeps no stuck
    /// multiplier: `Hurt` and `AddEffect` are skipped, `StuckIn` scales the
    /// movement once, as vanilla's projectiles keep none of the three.
    fn apply_projectile_write(&mut self, shot: &mut Projectile, write: mod_api::EntityWrite) {
        let triple = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
        match write {
            mod_api::EntityWrite::Push(v) => {
                shot.velocity = shot.velocity.add_vector(triple(v));
            }
            mod_api::EntityWrite::SetMovement(v) => shot.velocity = triple(v),
            mod_api::EntityWrite::StuckIn(v) => {
                shot.velocity = Vec3::new(
                    shot.velocity.x * v[0],
                    shot.velocity.y * v[1],
                    shot.velocity.z * v[2],
                );
            }
            mod_api::EntityWrite::ResetFallDistance => shot.fall_distance = 0.0,
            mod_api::EntityWrite::SetFireTicks(ticks) => shot.fire_ticks = ticks,
            mod_api::EntityWrite::Hurt { .. } | mod_api::EntityWrite::AddEffect { .. } => {}
        }
    }

    /// An `entity_inside` part's contact for a kind the batch ticks, fired
    /// mid-walk over a lent level, the whole callback staged: only an answering
    /// callback's writes join the walk, and a panic there is contained, leaving
    /// the batch's rows and the level as they were. The level is lent and given
    /// back per contact, as `item_fall_on` lends its own (arismoko/horizon#2194's carrier
    /// discipline).
    pub(in crate::game) fn fire_batch_inside(
        &mut self,
        entity: i32,
        kind: &'static str,
        state: horizon_world::registry::BlockStateId,
        at: horizon_core::position::BlockPos,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) {
        let Some(dimension) = horizon_client::block_contact::current_level() else {
            return;
        };
        if !horizon_client::block_contact::HOOKS
            .get()
            .is_some_and(|hooks| (hooks.inside_listens)(state))
        {
            return;
        }
        let mut staged = mod_api::InsideEffects::default();
        let mut writes = Vec::new();
        let mut lent = horizon_client::health::LentLevel::take(&mut self.world);
        let mut level = horizon_client::health::DamageLevel::lent(
            &self.world,
            self.pack.assets.registry,
            &mut lent,
            Some(world_items!(self)),
            Some(&self.pack.block_registries),
            &self.damage_hooks,
        );
        if let Some(hooks) = horizon_client::block_contact::HOOKS.get() {
            (hooks.inside_fire)(
                &mut level,
                dimension,
                entity,
                kind,
                state,
                at,
                precise,
                &mut staged,
                &mut writes,
            );
        }
        lent.give_back(&mut self.world);
        effects.absorb_step(staged);
        horizon_client::block_contact::inside_written(state, at, writes);
    }

    pub(super) fn tick_item_with_block_contacts(
        &mut self,
        item: &mut ItemEntity,
        escape_speed: Option<f32>,
        entity_boxes: &[Aabb],
        client: bool,
        splash: &mut impl FnMut(horizon_client::entities::ItemSplash),
    ) -> (bool, bool, super::ahead::Landed) {
        let no_physics = escape_speed.is_some();
        let (step, landed) = self
            .item_tables()
            .move_item(item, escape_speed, entity_boxes, splash);
        let landed = landed.filter(|_| !client);
        if !client && let Some((on, distance)) = step.landed() {
            self.item_fall_on(item.id, on, distance);
        }
        if !client {
            // `Entity.applyEffectsFromBlocks`' checkInsideBlocks, at the batch.
            self.sweep_item_inside(item);
        }
        if let Some(pos) = step.vertical_contact {
            let state = self.world.block(pos.x, pos.y, pos.z);
            let position = item.position;
            let mut discarded = false;
            if crate::game::api_values::block_state::block_listens(
                mod_api::HookId::ItemAfterVerticalCollision,
                state,
            ) {
                let mut stack = Some(item.stack.clone());
                self.native_item_after_vertical_collision(
                    crate::game::native_hooks::ItemAfterVerticalCollisionCall {
                        dimension: self.dim.id(),
                        client,
                        entity: item.id,
                        state,
                        block_pos: [pos.x, pos.y, pos.z],
                        item_pos: [position.x, position.y, position.z],
                        stack: &mut stack,
                        discard: &mut discarded,
                    },
                );
                match stack {
                    Some(stack) => item.stack = stack,
                    None => discarded = true,
                }
            }
            if discarded || item.stack.count == 0 {
                return (false, true, landed);
            }
        }
        let alive = self
            .item_tables()
            .settle_item(item, no_physics, step, splash);
        (alive, false, landed)
    }
}

/// The game's lists of entities, in the order systems walk them. Hands them to
/// `$then`, so one list drives the handles and every typed dispatch.
macro_rules! entity_lists {
    ($then:ident, $($args:tt)*) => {
        $crate::game::entity::$then! {
            ($($args)*)
            Mob(Mob) mobs: mobs,
            Item(ItemEntity) dropped: dropped,
            Boat(Boat) boats: boats,
            Cart(Minecart) carts: carts,
            ArmorStand(ArmorStand) armor_stands: armor_stands,
            Tnt(PrimedTnt) tnts: tnts,
            Falling(FallingBlock) falling: falling,
            Orb(ExperienceOrb) orbs: orbs,
            Projectile(Projectile) projectiles: creatures.projectiles,
            Rocket(Rocket) rockets: rockets,
            Trident(ThrownTrident) tridents: tridents,
            General(GeneralRow) general: resident_entities.general.rows,
        }
    };
}
pub(crate) use entity_lists;

macro_rules! direct_entity_edit {
    (Projectile) => {
        false
    };
    (Cart) => {
        false
    };
    (Falling) => {
        false
    };
    (Boat) => {
        false
    };
    ($kind:ident) => {
        true
    };
}
pub(crate) use direct_entity_edit;

macro_rules! define_entity_handles {
    (() $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        /// Every list, borrowed apart from the rest of `Game`, for
        /// [`for_each_entity`] while the world is borrowed mutably.
        #[derive(Clone, Copy)]
        pub(super) struct EntityLists<'w> {
            $(pub(super) $field: &'w [$ty],)*
        }

        /// One entity, by list and index.
        #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
        pub(super) enum EntityHandle {
            $($kind(usize),)*
        }

        /// Allocation-free storage-order walk with stable IDs and verified index hints.
        /// The next ID is captured before the caller can remove the current entity.
        pub(super) struct EntityIdCursor {
            before: JoinSeq,
            $($field: (Option<(u32, JoinSeq)>, usize, usize),)*
        }

        /// Every list moved out of `Game`, for [`for_each_entity_mut`]: the
        /// world stays readable while the entities change.
        pub(super) struct TakenEntities {
            $(pub(super) $field: horizon_client::join_seq::JoinList<$ty, { $crate::game::entity::direct_entity_edit!($kind) }>,)*
        }


        impl EntityLists<'_> {
            /// Visits every entity once, in the order they joined, whatever the
            /// kind: a merge of the lists, each already in join order, by a
            /// cursor per list. Static dispatch and nothing allocated. Checked:
            /// a list out of join order, or an entity that never joined, stops
            /// the walk with the entity at fault.
            #[allow(dead_code, reason = "the tick order work that reads this has not landed yet")]
            pub(super) fn walk_in_join_order(
                self,
                visit: impl FnMut(EntityHandle, JoinSeq),
            ) -> Result<(), JoinOrderError> {
                let lens = [$(self.$field.len()),*];
                merge_in_join_order(
                    lens,
                    |kind, i| {
                        let mut at = 0;
                        $(if kind == at {
                            return self.$field[i].join_seq();
                        } at += 1;)*
                        let _ = at;
                        unreachable!("kind {kind} is not a list")
                    },
                    |kind, i| {
                        let mut at = 0;
                        $(if kind == at {
                            return EntityHandle::$kind(i);
                        } at += 1;)*
                        let _ = at;
                        unreachable!("kind {kind} is not a list")
                    },
                    visit,
                )
            }
        }

        impl Game<'_> {
            /// Moves every list out; give it back with `restore_entities`.
            pub(super) fn take_entities(&mut self) -> TakenEntities {
                TakenEntities {
                    $($field: self$(.$list)+.lift(),)*
                }
            }
            pub(super) fn restore_entities(&mut self, taken: TakenEntities) {
                $({
                    let expected = self$(.$list)+.len() + taken.$field.len();
                    self$(.$list)+.restore(taken.$field, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
                    debug_assert_eq!(self$(.$list)+.len(), expected);
                })*
            }
            /// Every entity's handle: lengths read now, nothing allocated, and
            /// no borrow of `Game` held, so the caller can change it mid-walk.
            pub(super) fn entity_handles(&self) -> impl Iterator<Item = EntityHandle> + use<> {
                std::iter::empty()
                    $(.chain((0..self$(.$list)+.len()).map(EntityHandle::$kind)))*
            }

            pub(super) fn entity_id_cursor(&self) -> EntityIdCursor {
                EntityIdCursor {
                    before: JoinSeq::upcoming(),
                    $($field: (self$(.$list)+.first().map(|e| (e.id(), e.join_seq())), 0, self$(.$list)+.len()),)*
                }
            }

            /// Verify a stable identity before returning any index-bearing handle.
            /// Removed next identities resume at the next surviving original join.
            /// The captured join ceiling excludes entities spawned by callbacks.
            pub(super) fn next_entity_id(&self, cursor: &mut EntityIdCursor) -> Option<(u32, EntityHandle)> {
                $({
                    let (next, hint, remaining) = &mut cursor.$field;
                    if *remaining > 0 {
                        if let Some((id, join)) = *next {
                            let index = if self$(.$list)+.get(*hint).is_some_and(|e| e.id() == id && e.join_seq() == join) {
                                Some(*hint)
                            } else {
                                self$(.$list)+.iter().position(|e| e.id() == id && e.join_seq() == join).or_else(|| {
                                    self$(.$list)+.iter().position(|e| e.join_seq() > join && e.join_seq() < cursor.before)
                                })
                            };
                            if let Some(i) = index {
                                *remaining -= 1;
                                *hint = i + 1;
                                *next = self$(.$list)+.get(i + 1).filter(|e| e.join_seq() < cursor.before).map(|e| (e.id(), e.join_seq()));
                                return Some((self$(.$list)+[i].id(), EntityHandle::$kind(i)));
                            }
                        }
                        *remaining = 0;
                    }
                })*
                None
            }

            /// Resolve one stable ID, checking its cheap index hint before searching.
            pub(super) fn refresh_entity_handle(&self, id: u32, hint: EntityHandle) -> Option<EntityHandle> {
                match hint {
                    $(EntityHandle::$kind(i) => {
                        if self$(.$list)+.get(i).is_some_and(|e| e.id() == id) { Some(hint) }
                        else { self$(.$list)+.iter().position(|e| e.id() == id).map(EntityHandle::$kind) }
                    })*
                }
            }

            /// Takes the entity out of its list (`Entity.discard` before it ever ticked).
            pub(super) fn discard_entity(&mut self, handle: EntityHandle) {
                match handle {
                    $(EntityHandle::$kind(i) => {
                        self$(.$list)+.remove(i);
                    })*
                }
            }

            /// [`Self::entity_handles`], last of each list first, so the
            /// caller can remove the entity it is on.
            pub(super) fn entity_handles_rev(&self) -> impl Iterator<Item = EntityHandle> + use<> {
                std::iter::empty()
                    $(.chain((0..self$(.$list)+.len()).rev().map(EntityHandle::$kind)))*
            }
        }
    };
}
pub(crate) use define_entity_handles;

/// Why [`EntityLists::walk_in_join_order`] stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "the tick order work that reads this has not landed yet"
)]
pub(super) enum JoinOrderError {
    /// Never put in its list through the list's own `push`, so it has no
    /// join number.
    Unjoined(EntityHandle),
    /// Joined before an entity the walk had already visited: its list is out
    /// of join order.
    OutOfOrder(EntityHandle),
}

/// The merge behind the join-order walk, over `N` lists: `lens` are their
/// lengths, `seq(list, i)` the join number of entity `i` of a list, `handle`
/// names it. Each step takes the lowest next number among the `N` cursors
/// (a stack array; no allocation).
#[allow(
    dead_code,
    reason = "the tick order work that reads this has not landed yet"
)]
fn merge_in_join_order<const N: usize>(
    lens: [usize; N],
    seq: impl Fn(usize, usize) -> JoinSeq,
    handle: impl Fn(usize, usize) -> EntityHandle,
    mut visit: impl FnMut(EntityHandle, JoinSeq),
) -> Result<(), JoinOrderError> {
    let mut cursor = [0usize; N];
    let mut last = JoinSeq::UNJOINED;
    loop {
        let mut best: Option<(usize, JoinSeq)> = None;
        for list in 0..N {
            if cursor[list] < lens[list] {
                let s = seq(list, cursor[list]);
                if !s.is_joined() {
                    return Err(JoinOrderError::Unjoined(handle(list, cursor[list])));
                }
                if best.is_none_or(|(_, b)| s < b) {
                    best = Some((list, s));
                }
            }
        }
        let Some((list, s)) = best else {
            return Ok(());
        };
        let h = handle(list, cursor[list]);
        if s <= last {
            return Err(JoinOrderError::OutOfOrder(h));
        }
        last = s;
        cursor[list] += 1;
        visit(h, s);
    }
}

entity_lists!(define_entity_handles,);
horizon_client::impl_joined!(FallingBlock);

macro_rules! entity_access {
    (shared, $list:expr, $index:expr, $entity:ident, $body:expr) => {{
        let $entity = &$list[$index];
        $body
    }};
    (exclusive, $list:expr, $index:expr, $entity:ident, $body:expr) => {
        $list.update($index, |$entity| $body)
    };
}
pub(crate) use entity_access;

macro_rules! match_entity {
    (($borrow:ident, $entity:ident, $game:expr, $handle:expr, $body:expr)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        match $handle {
            $($crate::game::entity::EntityHandle::$kind(i) => {
                $crate::game::entity::entity_access!($borrow, $game$(.$list)+, i, $entity, $body)
            })*
        }
    };
}
pub(crate) use match_entity;

/// Runs `$body` with `$entity` bound to the handle's entity, as its own type:
/// one match, each arm compiled for its kind, so nothing goes through a vtable.
macro_rules! with_entity {
    ($game:expr, $handle:expr, $entity:ident => $body:expr) => {
        $crate::game::entity::entity_lists!(match_entity, shared, $entity, $game, $handle, $body)
    };
}
pub(super) use with_entity;

#[path = "entity/list_borrows.rs"]
mod list_borrows;
pub(super) use list_borrows::entity_lists_of;
pub(crate) use list_borrows::{borrow_level_list, borrow_lists};
/// The level `$game` ticks, lent to a hit's damage hooks with its block registry and merged
/// tag tables, so a mob or the player beside them can take the hit. With `$attack` (an
/// [`Attack`](horizon_client::health::Attack) the site took first), the level of that attack.
macro_rules! damage_level_of {
    // The level of the hit inside `Game::with_attack`: the attack that scope lends.
    ($game:ident, scoped) => {
        &mut horizon_client::health::DamageLevel::of(
            &mut $game.world,
            $game.pack.assets.registry,
            Some(world_items!($game)),
            Some(&$game.pack.block_registries),
            &$game.damage_hooks,
        )
        .attacked_by($game.combat.attack())
    };
    ($game:ident, $attack:expr) => {
        &mut horizon_client::health::DamageLevel::of(
            &mut $game.world,
            $game.pack.assets.registry,
            Some(world_items!($game)),
            Some(&$game.pack.block_registries),
            &$game.damage_hooks,
        )
        .attacked_by($attack)
    };
    ($game:ident) => {
        &mut horizon_client::health::DamageLevel::of(
            &mut $game.world,
            $game.pack.assets.registry,
            Some(world_items!($game)),
            Some(&$game.pack.block_registries),
            &$game.damage_hooks,
        )
    };
}
pub(super) use damage_level_of;

macro_rules! walk_lists {
    (($lists:expr, $entity:ident, $body:block)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $(for $entity in $lists.$field $body)*
    };
}
pub(crate) use walk_lists;

/// Runs `$body` for every entity, with `$entity` as a reference to its own
/// type: one loop per list, each compiled for its kind, so nothing allocates
/// or goes through a vtable and a check a kind answers with a constant folds
/// away. A kind added to `entity_lists!` joins every walk.
macro_rules! for_each_entity {
    ($lists:expr, $entity:ident => $body:block) => {
        $crate::game::entity::entity_lists!(walk_lists, $lists, $entity, $body)
    };
}
pub(super) use for_each_entity;

macro_rules! walk_lists_mut {
    (($lists:expr, $entity:ident, $body:block)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $($lists.$field.for_each_mut(|$entity| $body);)*
    };
}
pub(crate) use walk_lists_mut;

/// [`for_each_entity`] over [`TakenEntities`], with the entity mutable.
macro_rules! for_each_entity_mut {
    ($lists:expr, $entity:ident => $body:block) => {
        $crate::game::entity::entity_lists!(walk_lists_mut, $lists, $entity, $body)
    };
}
pub(super) use for_each_entity_mut;

/// [`with_entity`], with the entity mutable.
macro_rules! with_entity_mut {
    ($game:expr, $handle:expr, $entity:ident => $body:expr) => {
        $crate::game::entity::entity_lists!(match_entity, exclusive, $entity, $game, $handle, $body)
    };
}
pub(super) use with_entity_mut;

macro_rules! save_lists {
    (($game:expr, $keys:expr, $out:expr)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $(for i in 0..$game$(.$list)+.len() {
            if let Some((key, mut tag)) = <$ty as $crate::game::entity::Entity>::save_nbt($game, i, $keys) {
                if !<$ty as $crate::game::entity::Entity>::SERIALIZES_ATTACHMENTS {
                    $crate::game::persist::write_attachments(
                        &mut tag, $game$(.$list)+[i].id(), &$game.entity_attachments);
                }
                $out.entry(key).or_default().push((
                    $game$(.$list)+[i].id(),
                    $game$(.$list)+[i].join_seq(),
                    tag,
                ));
            }
        })*
    };
}
pub(crate) use save_lists;

/// Saves every entity of every list in `entity_lists!` into `$out` by chunk,
/// each with its id and join number (`$out` maps a chunk to `(id, JoinSeq, Tag)`),
/// each through its own `Entity::save_nbt`: one loop per list, static dispatch,
/// nothing allocated beyond the tags. A kind added to the lists cannot be left
/// out, since `save_nbt` has no default.
macro_rules! save_entities_of_every_kind {
    ($game:expr, $keys:expr, $out:expr) => {
        $crate::game::entity::entity_lists!(save_lists, $game, $keys, $out)
    };
}

/// A matched kind consumes only its own explicit token; failed kind probes never do.
macro_rules! load_with_transfer {
    ($game:expr, $state:expr, $ordinary:ident, $transfer:ident, $($arg:expr),+ $(,)?) => {{
        if let Some(state) = $state.take() {
            $game.$transfer($($arg),+, Some(state));
        } else {
            $game.$ordinary($($arg),+);
        }
    }};
}

macro_rules! load_lists {
    (($game:expr, $kind:expr, $uuid:expr, $player:expr, $tag:expr, $transferred:expr)
     $($kind_name:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {{
        let mut tag = $tag;
        $(match <$ty as $crate::game::entity::Entity>::load_nbt($game, $kind, $uuid, $player, tag, $transferred) {
            Ok(()) => return Ok(()),
            Err(back) => tag = back,
        })*
        Err(tag)
    }};
}
pub(crate) use load_lists;

/// Loads a saved entity as the first kind in `entity_lists!` that claims its
/// type id, through that kind's own `Entity::load_nbt`: one test per kind,
/// static dispatch, nothing allocated (the compound moves through). Evaluates
/// to `Err(tag)` when no kind claims it, and `return`s `Ok` from the caller
/// otherwise. A kind added to the lists cannot be left out, since `load_nbt`
/// has no default.
macro_rules! load_entity_of_any_kind {
    ($game:expr, $kind:expr, $uuid:expr, $player:expr, $tag:expr, $transferred:expr) => {
        $crate::game::entity::entity_lists!(
            load_lists,
            $game,
            $kind,
            $uuid,
            $player,
            $tag,
            $transferred
        )
    };
}

/// The type ids the minecart kind loads.
fn loads_as_cart(kind: &str) -> bool {
    super::minecarts::CartKind::from_id(kind).is_some()
}
/// The type id the primed TNT kind loads.
fn loads_as_tnt(kind: &str) -> bool {
    kind == "minecraft:tnt"
}

impl Game<'_> {
    /// The type ids the mob kind loads.
    fn loads_as_mob(&self, kind: &str) -> bool {
        self.res.mob_types().contains_key(kind)
    }
    /// The type ids the boat kind loads.
    fn loads_as_boat(&self, kind: &str) -> bool {
        super::boats::any_boat_wood(kind)
            .is_some_and(|(wood, chest)| self.boat_models.has(wood, chest))
    }
    /// Loads a saved entity as the first kind in `entity_lists!` that claims
    /// its type id: `Err` hands the compound back when none does.
    pub(super) fn load_listed(
        &mut self,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
    ) -> Result<(), Tag> {
        load_entity_of_any_kind!(self, kind, uuid, player_uuid, tag, &mut None)
    }
    /// Restore a live portal object with its explicitly captured runtime ownership.
    /// This dispatch covers only reconstructible portal kinds; ordinary disk loads
    /// continue through the canonical kind walk without transfer state.
    pub(super) fn load_listed_transferred(
        &mut self,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
        state: super::entity_initialization::TransferredEntityState,
    ) -> Result<(), Tag> {
        let mut state = Some(state);
        load_entity_of_any_kind!(self, kind, uuid, player_uuid, tag, &mut state)
            .or_else(|tag| {
                self.load_unlisted_transferred(
                    Self::chunk_of(horizon_client::entity_nbt::position(&tag).unwrap_or_default()),
                    kind,
                    uuid,
                    tag,
                    &mut state,
                )
            })
            .or_else(|tag| {
                let saved = tag.clone();
                self.load_other_transferred(tag, kind, state.take())
                    .map(|_| ())
                    .ok_or(saved)
            })
    }

    /// The ids of the entities that are not on the walk of `entity_lists!`.
    pub(super) fn entity_ids_off_walk(&self) -> impl Iterator<Item = u32> + '_ {
        self.cushions
            .iter()
            .map(|e| e.id)
            .chain(self.item_frames.iter().map(|e| e.id))
            .chain(self.paintings.iter().map(|e| e.id))
            .chain(self.others.iter().map(|e| e.id))
            .chain(self.breath_clouds.iter().map(|e| e.id))
            .chain(self.fangs.list.iter().map(|e| e.id))
            .chain(self.crystals.iter().map(|e| e.id))
            .chain(self.dragon.iter().map(|e| e.id))
            .chain(
                self.dragon
                    .iter()
                    .filter(|dragon| dragon.id != 0)
                    .flat_map(|dragon| {
                        (0..dragon.parts.len())
                            .filter_map(move |offset| dragon.id.checked_add(offset as u32 + 1))
                    }),
            )
            .chain(self.ominous_spawners.iter().map(|e| e.id))
            .chain(self.resident_entities.knots.iter().map(|e| e.id))
            // Cast hooks live with their players, not in the saveable lists.
            // Keep both their identities and attachment holders until they leave.
            .chain(self.fishing_hooks_here().map(|(hook, _)| hook.id()))
            .chain(self.natural_knot_id_iter())
    }
    /// The ids of every entity in the level that is not removed: one walk.
    pub(super) fn live_entity_ids(&self) -> HashSet<u32> {
        let mut alive: HashSet<u32> = self.entity_ids_off_walk().collect();
        for_each_entity!(entity_lists_of!(self), entity => {
            if !entity.is_removed() {
                alive.insert(entity.id());
            }
        });
        alive
    }
    /// Saves every entity of every kind that lies in `keys` into `out`, by chunk.
    pub(super) fn save_every_entity(
        &mut self,
        keys: &HashSet<ChunkKey>,
        out: &mut HashMap<ChunkKey, Vec<(u32, JoinSeq, Tag)>>,
    ) {
        save_entities_of_every_kind!(self, keys, out);
    }
}

impl Entity for Mob {
    const SERIALIZES_ATTACHMENTS: bool = true;

    fn reset_fall_distance(&mut self) {
        self.fall_distance = 0.0;
    }

    fn set_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    /// `Entity.noPhysics`: written through the mod API; no mob tick rewrites it
    /// (a vex sets it only for the length of its own, custom travel). Its own
    /// field is the write's store, so `stored` is None for a mob.
    fn no_physics(&self, _stored: Option<bool>) -> bool {
        self.no_physics
    }
    /// `Entity.move(MoverType.PISTON, ...)` ends in `setPos`, which `Shulker`
    /// overrides to snap to the block centre.
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let moved = self.position.add_vector(movement);
        if self.species() == Species::Shulker {
            crate::mobs::shulker::client_set_pos(self, moved.x, moved.y, moved.z);
        } else {
            self.position = moved;
        }
    }
    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.head_pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.body_yaw = yaw;
        self.head_yaw = yaw;
        self.head_pitch = pitch.clamp(-90.0, 90.0);
        self.previous_yaw = self.yaw;
        self.previous_head_pitch = self.head_pitch;
    }
    fn native_type(&self) -> Cow<'static, str> {
        super::native_hooks::mob_kind(self).into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_mob(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        _uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !game.loads_as_mob(kind) {
            return Err(tag);
        }
        load_with_transfer!(game, transferred, load_mob, load_mob_transferred, tag, None);
        Ok(())
    }
    stored_motion!();
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((self.width(), self.height()))
    }
    fn portal_bounds(&self, position: Vec3) -> Option<Aabb> {
        Some(self.bounds_at(position))
    }
    fn sweeps_portals(&self) -> bool {
        true
    }
    fn can_use_portal(&self, game: &Game<'_>) -> bool {
        game.mob_portal_eligible(self)
    }
    /// `PersistentEntitySectionManager.Callback.onRemove`: discard stops
    /// section enumeration immediately, before the entity list is swept; and
    /// `EntitySelectorParser`'s `Entity::isAlive` drops a dying mob at once.
    fn selector_target(&self) -> Option<Target> {
        (!self.removed && self.alive()).then_some(Target::Mob(self.id))
    }
    fn bounds(&self) -> Aabb {
        Mob::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    fn get_step_height(&self) -> f32 {
        self.kind.step_height as f32
    }
    fn is_alive(&self) -> bool {
        !self.removed && self.alive()
    }
    fn is_removed(&self) -> bool {
        self.removed
    }
    fn is_living(&self) -> bool {
        true
    }
    /// `Bat.isIgnoringBlockTriggers`.
    fn is_ignoring_block_triggers(&self) -> bool {
        self.species() == Species::Bat
    }
    /// `Warden.ignoreExplosion`: while it digs or emerges.
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        !self.alive()
            || blast.spared == Some(self.id)
            || (self.species() == Species::Warden
                && horizon_client::mobs::warden::digging_or_emerging(self))
    }
    /// `getEyeHeight` of the mob as it stands: a baby's, a slime's or a
    /// phantom's own scaled eye height, not the adult kind's. The blast's box
    /// is its real `bounds`, so it needs no override.
    fn explosion_eye_height(&self) -> f64 {
        self.eye_height()
    }
    /// `Entity.push` sets `needsSync`.
    fn push_from_explosion(&mut self, push: Vec3) {
        self.velocity = Vec3::new(
            self.velocity.x + push.x,
            self.velocity.y + push.y,
            self.velocity.z + push.z,
        );
        self.needs_sync = true;
    }
    fn is_supported_by(&self, pos: BlockPos) -> bool {
        self.supporting == Some([pos.x, pos.y, pos.z])
    }
    fn thunder_hit(&self) -> ThunderHit {
        if self.alive() {
            ThunderHit::Mob
        } else {
            ThunderHit::Untouched
        }
    }
    /// The kind's own size, as the strike has always read it.
    fn thunder_bounds(&self) -> Aabb {
        let (p, w) = (self.position, self.kind.width / 2.0);
        Aabb::new(
            Vec3::new(p.x - w, p.y, p.z - w),
            Vec3::new(p.x + w, p.y + self.kind.height, p.z + w),
        )
    }
}

impl Entity for ItemEntity {
    fn reset_fall_distance(&mut self) {
        self.fall_distance = 0.0;
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn set_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
    }
    fn fluid_eye_y(&self) -> f64 {
        ItemEntity::fluid_eye_y(self)
    }
    fn native_type(&self) -> Cow<'static, str> {
        "minecraft:item".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_item(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:item") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_item,
            load_item_transferred,
            tag,
            uuid
        );
        Ok(())
    }
    stored_motion!();
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.25, 0.25))
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Item(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Item
    }
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        blast.items_ignore
    }
    fn explosion_eye_height(&self) -> f64 {
        0.2125
    }
    fn bounds(&self) -> Aabb {
        ItemEntity::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    fn set_piston_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        ItemEntity::piston_slime_bounce(self, movement);
    }
}

impl Entity for ExperienceOrb {
    fn reset_fall_distance(&mut self) {
        // An orb keeps no fall distance here.
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn set_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
    }
    fn ticks_after_projectiles(&self) -> bool {
        true
    }
    fn native_type(&self) -> Cow<'static, str> {
        "minecraft:experience_orb".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_orb(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:experience_orb") {
            return Err(tag);
        }
        load_with_transfer!(game, transferred, load_orb, load_orb_transferred, tag, uuid);
        Ok(())
    }
    stored_motion!();
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.5, 0.5))
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Orb(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Orb
    }
    fn explosion_eye_height(&self) -> f64 {
        f64::from(0.5f32 * 0.85f32)
    }
    fn bounds(&self) -> Aabb {
        ExperienceOrb::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    fn set_piston_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        self.velocity = piston_push::bounce_velocity(movement, self.velocity);
        self.on_ground = false;
    }
}

impl Entity for Boat {
    fn reset_fall_distance(&mut self) {
        // A boat keeps no fall distance here.
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn set_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn set_piston_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
    }
    fn native_type(&self) -> Cow<'static, str> {
        self.entity_type().into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_boat(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !game.loads_as_boat(kind) {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_boat,
            load_boat_transferred,
            tag,
            uuid,
            kind
        );
        Ok(())
    }
    stored_motion!();
    fn is_vehicle(&self) -> bool {
        true
    }
    /// `Boat.getEyeY`: the top of the 0.5625 box.
    fn fluid_eye_y(&self) -> f64 {
        self.position.y + super::boats::HEIGHT
    }
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((super::boats::WIDTH as f32, super::boats::HEIGHT as f32))
    }
    fn can_use_portal(&self, game: &Game<'_>) -> bool {
        game.vehicle_can_use_portal(self.id)
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Boat(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Vehicle
    }
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        blast.vehicles_ignore
    }
    /// `EntityType.OAK_BOAT & co`: the full 0.5625.
    fn explosion_eye_height(&self) -> f64 {
        0.5625
    }
    fn bounds(&self) -> Aabb {
        Boat::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
}

impl Entity for Minecart {
    fn reset_fall_distance(&mut self) {
        self.fall_distance = 0.0;
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn set_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn set_piston_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
    }
    fn native_type(&self) -> Cow<'static, str> {
        self.kind.id().into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_cart(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !loads_as_cart(kind) {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_cart,
            load_cart_transferred,
            tag,
            uuid,
            kind
        );
        Ok(())
    }
    stored_motion!();
    fn is_vehicle(&self) -> bool {
        true
    }
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((
            super::minecarts::WIDTH as f32,
            super::minecarts::HEIGHT as f32,
        ))
    }
    fn can_use_portal(&self, game: &Game<'_>) -> bool {
        game.vehicle_can_use_portal(self.id)
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Cart(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Vehicle
    }
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        blast.vehicles_ignore
    }
    fn bounds(&self) -> Aabb {
        Minecart::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    /// `tick_blocks` sweeps minecarts along with the players.
    fn sweeps_blocks_inside(&self) -> bool {
        false
    }
}

/// Width and height `EntityType.ARMOR_STAND` gives a stand.
fn armor_stand_size(stand: &ArmorStand) -> (f64, f64) {
    if stand.marker {
        (0.0, 0.0)
    } else if stand.small {
        (0.25, f64::from(0.9875f32))
    } else {
        (0.5, f64::from(1.975f32))
    }
}

impl Entity for ArmorStand {
    fn reset_fall_distance(&mut self) {
        self.enchantment_fall_distance = 0.0;
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn set_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.previous_yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
    }
    fn fluid_eye_y(&self) -> f64 {
        ArmorStand::fluid_eye_y(self)
    }
    fn native_type(&self) -> Cow<'static, str> {
        super::armor_stands::TYPE_ID.into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_armor_stand(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:armor_stand") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_armor_stand,
            load_armor_stand_transferred,
            tag,
            uuid
        );
        Ok(())
    }
    stored_motion!();
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Stand(self.id))
    }
    /// `ArmorStand.ignoreExplosion`: a visible stand feels only a burst that
    /// affects block-like entities (`items_ignore` is that flag, negated).
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        self.invisible || blast.items_ignore
    }
    /// `EntityType.ARMOR_STAND`'s size, halved when small; a marker has none.
    fn explosion_box(&self) -> Aabb {
        let (w, h) = armor_stand_size(self);
        let half = w / 2.0;
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - half, p.y, p.z - half),
            Vec3::new(p.x + half, p.y + h, p.z + half),
        )
    }
    fn explosion_eye_height(&self) -> f64 {
        if self.marker {
            0.0
        } else if self.small {
            f64::from(0.9875f32)
        } else {
            f64::from(1.7775f32)
        }
    }
    /// A burst of wind pushes a stand that a blast only breaks.
    fn pushed_by_wind(&self) -> bool {
        true
    }
    fn bounds(&self) -> Aabb {
        ArmorStand::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    /// A living entity's own flag too: keeping the old support flag would run
    /// Frost Walker while the stand is moving over open water.
    fn set_piston_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn is_living(&self) -> bool {
        true
    }
    /// `ArmorStand.isIgnoringBlockTriggers`.
    fn is_ignoring_block_triggers(&self) -> bool {
        self.marker
    }
    /// A NoGravity stand is not swept for the blocks it stands in (plates,
    /// fire): measured beside the real server, it never presses a plate nor
    /// catches fire, and does again once `NoGravity` is cleared. A marker is
    /// not either: `ArmorStand.hasPhysics` is false for it too, so
    /// `noPhysics` keeps it out of `applyEffectsFromBlocks`.
    fn sweeps_blocks_inside(&self) -> bool {
        !self.marker && !self.no_gravity
    }
    fn pushed_by_explosion(&self) -> bool {
        false
    }
    /// `Entity.noPhysics`: `Marker` or `NoGravity` unless a mod wrote it,
    /// recomputed from them on load because `readAdditionalSaveData` does.
    /// Its own field is the write's store, so `stored` is None for a stand.
    fn no_physics(&self, _stored: Option<bool>) -> bool {
        self.no_physics.unwrap_or(self.marker || self.no_gravity)
    }
    /// `ArmorStand.getPistonPushReaction`: a marker is ignored.
    fn get_piston_push_reaction(&self) -> PushReaction {
        if self.marker {
            PushReaction::IgnoreEntity
        } else {
            PushReaction::PushPull
        }
    }
    /// `ArmorStand.getDefaultDimensions`: the box a portal sweep follows, and
    /// the size its exit is placed by. `ArmorStand.readAdditionalSaveData`
    /// makes a stand without `hasPhysics` (a marker or a `NoGravity` one)
    /// `noPhysics`, so `Entity.isAffectedByBlocks` never runs its sweep and it
    /// enters no portal; a marker's box is `MARKER_DIMENSIONS`, 0 by 0, too.
    fn portal_size(&self) -> Option<(f32, f32)> {
        if self.marker || self.no_gravity {
            return None;
        }
        let (width, height) = armor_stand_size(self);
        Some((width as f32, height as f32))
    }
}

impl Entity for PrimedTnt {
    fn reset_fall_distance(&mut self) {
        // Primed TNT keeps no fall distance here.
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn set_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
    }
    fn ticks_after_projectiles(&self) -> bool {
        true
    }
    fn native_type(&self) -> Cow<'static, str> {
        "minecraft:tnt".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_tnt(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !loads_as_tnt(kind) {
            return Err(tag);
        }
        load_with_transfer!(game, transferred, load_tnt, load_tnt_transferred, tag, uuid);
        Ok(())
    }
    stored_motion!();
    /// `EntityTypes.TNT`'s explicit `eyeHeight(0.15F)`. Nothing vanilla reads
    /// for TNT depends on it (only `isEyeInFluid` does), but it is the type's.
    fn fluid_eye_y(&self) -> f64 {
        self.position.y + f64::from(0.15_f32)
    }
    /// `PrimedTnt` is 0.98 across.
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.98, 0.98))
    }
    fn can_use_portal(&self, game: &Game<'_>) -> bool {
        game.tnt_can_use_portal(self.id)
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Tnt(self.id))
    }
    /// Pushed from its position, not its eyes.
    fn explosion_eye_height(&self) -> f64 {
        0.0
    }
    fn bounds(&self) -> Aabb {
        PrimedTnt::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    fn set_piston_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        self.velocity = piston_push::bounce_velocity(movement, self.velocity);
        // Leave the ground so the next tick does not apply the on-ground
        // Y * -0.5 friction that cancels the launch.
        self.on_ground = false;
    }
}

impl Entity for FallingBlock {
    fn reset_fall_distance(&mut self) {
        self.fall_distance = 0.0;
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }
    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    /// `Entity.move(MoverType.PISTON)` ends in `setOnGroundWithMovement`.
    fn set_piston_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn set_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn ticks_after_projectiles(&self) -> bool {
        true
    }
    fn native_type(&self) -> Cow<'static, str> {
        "minecraft:falling_block".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_falling(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:falling_block") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_falling,
            load_falling_transferred,
            tag,
            uuid
        );
        Ok(())
    }
    stored_motion!();
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Falling(self.id))
    }
    fn bounds(&self) -> Aabb {
        horizon_client::falling_block::bounds(self.position)
    }
}

impl Entity for Projectile {
    fn reset_fall_distance(&mut self) {
        if let Some(arrow) = self.arrow.as_deref_mut() {
            arrow.fall_distance = 0.0;
        }
    }
    fn portal_after_move(&self) -> bool {
        !matches!(
            self.kind,
            horizon_client::projectiles::ProjectileKind::LlamaSpit
                | horizon_client::projectiles::ProjectileKind::ShulkerBullet { .. }
        )
    }
    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
        self.previous_yaw = self.yaw;
        self.previous_pitch = self.pitch;
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    /// `Entity.move(MoverType.PISTON)` ends in `setOnGroundWithMovement`.
    fn set_piston_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn set_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn native_type(&self) -> Cow<'static, str> {
        self.entity_type().into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_projectile(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        match kind {
            "minecraft:arrow" | "minecraft:spectral_arrow" => {
                load_with_transfer!(
                    game,
                    transferred,
                    load_arrow,
                    load_arrow_transferred,
                    tag,
                    uuid,
                    player_uuid
                )
            }
            "minecraft:llama_spit" | "minecraft:shulker_bullet" => {
                load_with_transfer!(
                    game,
                    transferred,
                    load_spit_or_bullet,
                    load_spit_or_bullet_transferred,
                    tag,
                    uuid,
                    player_uuid
                )
            }
            "minecraft:fireball"
            | "minecraft:small_fireball"
            | "minecraft:dragon_fireball"
            | "minecraft:wind_charge"
            | "minecraft:breeze_wind_charge"
            | "minecraft:wither_skull" => load_with_transfer!(
                game,
                transferred,
                load_fireball,
                load_fireball_transferred,
                tag,
                uuid,
                player_uuid
            ),
            id if horizon_client::entity_nbt::throwable_kind(id).is_some() => {
                load_with_transfer!(
                    game,
                    transferred,
                    load_throwable,
                    load_throwable_transferred,
                    tag,
                    uuid,
                    player_uuid
                )
            }
            _ => return Err(tag),
        }
        Ok(())
    }
    stored_motion!();
    /// Arrows and the thrown items and fireballs a chunk saves.
    fn portal_size(&self) -> Option<(f32, f32)> {
        (super::nether_portal::is_arrow(self) || super::nether_portal::is_portal_projectile(self))
            .then(|| super::nether_portal::projectile_box(self))
    }
    fn portal_from(&self) -> Option<Vec3> {
        Some(self.previous)
    }
    fn portal_sweeps(&self) -> bool {
        !super::nether_portal::is_arrow(self) || !self.stuck || self.previous != self.position
    }
    fn selector_target(&self) -> Option<Target> {
        use horizon_client::projectiles::ProjectileKind as K;
        // The kinds /data can write (entity_nbt).
        let selectable = self.is_arrow()
            || horizon_client::entity_nbt::saved_with_chunk(self)
            || matches!(
                self.kind,
                K::LargeFireball { .. }
                    | K::SmallFireball
                    | K::DragonFireball
                    | K::WindCharge
                    | K::BreezeWindCharge
            );
        selectable.then_some(Target::Projectile(self.id))
    }
    /// `AbstractArrow.push` does nothing in the ground; a wind charge's motion
    /// stays 0.
    fn pushed_by_explosion(&self) -> bool {
        !self.stuck && !self.kind.ignores_blast()
    }
    fn explosion_eye_height(&self) -> f64 {
        self.kind.eye_height()
    }
    fn push_from_explosion(&mut self, push: Vec3) {
        self.velocity = Vec3::new(
            self.velocity.x + push.x,
            self.velocity.y + push.y,
            self.velocity.z + push.z,
        );
        self.needs_sync = true;
    }
    fn bounds(&self) -> Aabb {
        Projectile::bounds(self)
    }
    /// A shulker bullet's, set when it is made, unless a mod wrote one: the
    /// write has no field on this struct and lands in `stored`, so a cleared
    /// flag is what the movement code reads; an arrow's `AbstractArrow.noPhysics`.
    fn no_physics(&self, stored: Option<bool>) -> bool {
        stored.unwrap_or_else(|| {
            matches!(self.kind, ProjectileKind::ShulkerBullet { .. })
                || self.arrow.as_ref().is_some_and(|a| a.no_physics)
        })
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        Projectile::move_by_piston(self, movement);
    }
    fn piston_release_bounds(&self) -> Option<Aabb> {
        Projectile::piston_release_bounds(self)
    }
    fn release_after_piston(&mut self, no_block_collision: bool) {
        Projectile::release_after_piston(self, no_block_collision);
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        Projectile::piston_slime_bounce(self, movement);
    }
}

impl Entity for Rocket {
    fn reset_fall_distance(&mut self) {
        // A firework rocket keeps no fall distance here.
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }
    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    /// `Entity.move(MoverType.PISTON)` ends in `setOnGroundWithMovement`.
    fn set_piston_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn set_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn native_type(&self) -> Cow<'static, str> {
        "minecraft:firework_rocket".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_rocket(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:firework_rocket") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_rocket,
            load_rocket_transferred,
            tag,
            uuid,
            player_uuid
        );
        Ok(())
    }
    stored_motion!();
    /// A rocket that carries its owner stays.
    fn portal_size(&self) -> Option<(f32, f32)> {
        (self.attached_to.is_none() && !self.attached).then_some((0.25, 0.25))
    }
    fn portal_from(&self) -> Option<Vec3> {
        Some(self.previous)
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Rocket(self.id))
    }
    /// 0.25 wide, so 0.2125 to the eyes.
    fn explosion_eye_height(&self) -> f64 {
        0.2125
    }
    fn bounds(&self) -> Aabb {
        Rocket::bounds(self)
    }
    /// `FireworkRocketEntity.tick`: boosting a player skips the step.
    fn sweeps_blocks_inside(&self) -> bool {
        !self.attached
    }
}

impl Entity for ThrownTrident {
    fn reset_fall_distance(&mut self) {
        // A thrown trident keeps no fall distance here.
    }
    fn portal_after_move(&self) -> bool {
        true
    }
    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    /// `Entity.move(MoverType.PISTON)` ends in `setOnGroundWithMovement`.
    fn set_piston_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn set_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }
    fn native_type(&self) -> Cow<'static, str> {
        "minecraft:trident".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_trident(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:trident") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_trident,
            load_trident_transferred,
            tag,
            uuid
        );
        Ok(())
    }
    stored_motion!();
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.5, 0.5))
    }
    fn portal_from(&self) -> Option<Vec3> {
        Some(self.previous)
    }
    fn portal_sweeps(&self) -> bool {
        !self.in_ground || self.previous != self.position
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Trident(self.id))
    }
    /// Like any arrow (`AbstractArrow.push`) not pushed while in the ground.
    fn pushed_by_explosion(&self) -> bool {
        !self.in_ground
    }
    /// `EntityType.TRIDENT`: eyes at 0.13.
    fn explosion_eye_height(&self) -> f64 {
        f64::from(0.13_f32)
    }
    fn bounds(&self) -> Aabb {
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - 0.25, p.y, p.z - 0.25),
            Vec3::new(p.x + 0.25, p.y + 0.5, p.z + 0.25),
        )
    }
    /// `AbstractArrow.noPhysics`, set once a loyal trident turns back. Its own
    /// field is the write's store, so `stored` is None for a trident.
    fn no_physics(&self, _stored: Option<bool>) -> bool {
        self.no_physics
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        ThrownTrident::move_by_piston(self, movement);
    }
    fn piston_release_bounds(&self) -> Option<Aabb> {
        ThrownTrident::piston_release_bounds(self)
    }
    fn release_after_piston(&mut self, no_block_collision: bool) {
        ThrownTrident::release_after_piston(self, no_block_collision);
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        ThrownTrident::piston_slime_bounce(self, movement);
    }
    /// `GameLevel::tick_blocks` sweeps tridents itself.
    fn sweeps_blocks_inside(&self) -> bool {
        false
    }
}

impl EntityRules for FallingBlock {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `FallingBlockEntity.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.04
    }
}

#[cfg(test)]
mod no_gravity_stand_tests {
    use super::Entity as _;
    use horizon_client::entities::ArmorStand;
    use horizon_core::geometry::Vec3;

    #[test]
    fn a_no_gravity_stand_is_not_swept_for_the_blocks_it_is_in() {
        let mut stand = ArmorStand::new(1, Vec3::new(0.5, 100.0, 0.5), 0.0);
        // A resting stand presses a plate (measured beside the real server)...
        assert!(stand.sweeps_blocks_inside() && stand.triggers_blocks());
        // ...a NoGravity one does not, nor does a marker.
        stand.no_gravity = true;
        assert!(!stand.sweeps_blocks_inside());
        stand.no_gravity = false;
        stand.marker = true;
        assert!(!stand.sweeps_blocks_inside());
        assert!(!stand.triggers_blocks());
    }
}

#[cfg(test)]
mod portal_bounds_tests {
    use super::Entity as _;
    use crate::mobs::{Anim, Brain, Flags, Mob, MobType, Part, Species, shulker::PEEK};
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    /// A shulker's type, without the resource tables a `Game` fixture needs.
    fn shulker_kind() -> Arc<MobType> {
        Arc::new(MobType {
            shadow: None,
            id: "minecraft:shulker".into(),
            species: Species::Shulker,
            width: 1.0,
            height: 1.0,
            eye_height: 0.5,
            category: "creature".into(),
            max_health: 30.0,
            speed: 0.0,
            flying_speed: 0.0,
            attack_damage: 0.0,
            attack_knockback: 0.0,
            follow_range: 16.0,
            step_height: 0.5,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Arc::default(),
            item_stack_limits: Default::default(),
            attribute_bounds: Default::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 180.0,
            ambient_interval: 120,
            flags: Flags::default(),
            model: Arc::new(Part {
                name: "root".into(),
                pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                polygons: Vec::new(),
                children: Vec::new(),
            }),
            layers: Default::default(),
            texture: String::new(),
            brain: Brain::Passive,
            anim: Anim::Rabbit,
            sounds: [""; 3],
            sound_overrides: Default::default(),
            food: Arc::default(),
            tempt: Arc::default(),
            passenger_attachments: Vec::new(),
        })
    }

    /// F3+B and the portal sweep follow `Entity.getBoundingBox`: an open
    /// shulker's lid box, not the fixed 1x1x1 its width and height describe.
    #[test]
    fn an_open_lid_is_in_the_debug_and_portal_box() {
        for face in 0..6u8 {
            let mut m = Mob::new(1, shulker_kind(), Vec3::new(0.5, 4.0, 0.5), 0.0);
            m.extra.variant3 = face;
            m.anim.v[PEEK] = 1.0;
            let bounds = m.bounds();
            let portal = m.portal_bounds(m.position).unwrap();
            assert_eq!(portal.min_position(), bounds.min_position(), "face {face}");
            assert_eq!(portal.max_position(), bounds.max_position(), "face {face}");
        }
    }
}

#[cfg(test)]
mod load_claim_tests {
    /// `Mob` is first in `entity_lists!` and claims every id in `mob_types`;
    /// the old load tried every other kind first. Both agree only while no
    /// mob species shares an id with another kind, which this checks.
    #[test]
    fn no_mob_species_is_an_id_another_kind_loads() {
        let others = [
            "minecraft:item",
            "minecraft:experience_orb",
            "minecraft:arrow",
            "minecraft:spectral_arrow",
            "minecraft:llama_spit",
            "minecraft:shulker_bullet",
            "minecraft:fireball",
            "minecraft:small_fireball",
            "minecraft:wind_charge",
            "minecraft:breeze_wind_charge",
            "minecraft:falling_block",
            "minecraft:trident",
            "minecraft:firework_rocket",
            "minecraft:tnt",
            "minecraft:armor_stand",
            "minecraft:ominous_item_spawner",
            "minecraft:ender_dragon",
            "minecraft:end_crystal",
            "minecraft:item_frame",
            "minecraft:glow_item_frame",
            "minecraft:cushion",
            "minecraft:painting",
        ];
        for def in horizon_client::mobs::species::ALL {
            let id = format!("minecraft:{}", def.name);
            assert!(!others.contains(&id.as_str()), "{id}");
            assert!(
                horizon_client::entity_nbt::throwable_kind(&id).is_none(),
                "{id}"
            );
            assert!(super::super::boats::any_boat_wood(&id).is_none(), "{id}");
            assert!(
                super::super::minecarts::CartKind::from_id(&id).is_none(),
                "{id}"
            );
        }
    }
}

#[cfg(test)]
mod trident_selector_tests {
    use super::Entity as _;
    use crate::game::{commands::Target, trident::ThrownTrident};
    use horizon_core::geometry::Vec3;

    /// `@e`, `/kill` and `/data` reach a thrown trident like any other entity
    /// (`EntityTypeTest.forClass(Entity.class)`: every entity in the level).
    #[test]
    fn a_thrown_trident_is_a_selector_target() {
        let t = ThrownTrident::remote(9, Vec3::new(0.5, 100.0, 0.5), 0.0, 0.0);
        assert_eq!(t.selector_target(), Some(Target::Trident(9)));
    }
}

#[cfg(test)]
#[path = "entity/restore_tests.rs"]
mod restore_tests;

#[cfg(test)]
mod join_order_tests {
    use super::{EntityHandle, EntityLists, JoinOrderError};
    use crate::game::{boats::Boat, fireworks::Rocket, trident::ThrownTrident};
    use horizon_client::entities::ArmorStand;
    use horizon_client::join_seq::JoinList;
    use horizon_core::geometry::Vec3;

    fn at() -> Vec3 {
        Vec3::new(0.5, 100.0, 0.5)
    }

    struct Lists {
        stands: JoinList<ArmorStand>,
        tridents: JoinList<ThrownTrident>,
        rockets: JoinList<Rocket>,
        boats: JoinList<Boat, false>,
    }

    impl Lists {
        fn view(&self) -> EntityLists<'_> {
            EntityLists {
                mobs: &[],
                dropped: &[],
                boats: &self.boats,
                carts: &[],
                armor_stands: &self.stands,
                tnts: &[],
                falling: &[],
                orbs: &[],
                projectiles: &[],
                rockets: &self.rockets,
                tridents: &self.tridents,
                general: &[],
            }
        }
        fn walk(&self) -> Result<Vec<EntityHandle>, JoinOrderError> {
            let mut seen = Vec::new();
            self.view().walk_in_join_order(|h, _| seen.push(h))?;
            Ok(seen)
        }
    }

    /// Joins five kinds' entities interleaved, the way a level gets them.
    fn mixed() -> (Lists, Vec<EntityHandle>) {
        let mut l = Lists {
            stands: JoinList::default(),
            tridents: JoinList::default(),
            rockets: JoinList::default(),
            boats: JoinList::default(),
        };
        l.trident(1);
        l.rocket(2);
        l.stand(3);
        l.boat(4);
        l.trident(5);
        l.stand(6);
        l.rocket(7);
        let joined = vec![
            EntityHandle::Trident(0),
            EntityHandle::Rocket(0),
            EntityHandle::ArmorStand(0),
            EntityHandle::Boat(0),
            EntityHandle::Trident(1),
            EntityHandle::ArmorStand(1),
            EntityHandle::Rocket(1),
        ];
        (l, joined)
    }

    impl Lists {
        fn stand(&mut self, id: u32) {
            self.stands.push(ArmorStand::new(id, at(), 0.0));
        }
        fn trident(&mut self, id: u32) {
            self.tridents
                .push(ThrownTrident::remote(id, at(), 0.0, 0.0));
        }
        fn rocket(&mut self, id: u32) {
            self.rockets.push(Rocket::remote(id, at(), Vec3::ZERO));
        }
        fn boat(&mut self, id: u32) {
            self.boats.push(Boat::new(id, "oak".into(), at(), 0.0));
        }
    }

    /// The walk visits entities in the order they joined, not list by list.
    #[test]
    fn mixed_kinds_are_visited_in_join_order() {
        let (lists, joined) = mixed();
        assert_eq!(lists.walk().unwrap(), joined);
        // Walked list by list they would come in another order.
        assert_ne!(joined[0], EntityHandle::ArmorStand(0));
    }

    /// An entity taken out and put back (a tick takes its list) keeps its place.
    #[test]
    fn an_entity_put_back_keeps_its_join_number() {
        let (mut lists, joined) = mixed();
        let taken = lists.tridents.lift();
        let before: Vec<_> = taken
            .iter()
            .map(horizon_client::join_seq::Joined::join_seq)
            .collect();
        let mut back = JoinList::default();
        for t in taken {
            back.push(t);
        }
        lists.tridents = back;
        let after: Vec<_> = lists
            .tridents
            .iter()
            .map(horizon_client::join_seq::Joined::join_seq)
            .collect();
        assert_eq!(before, after);
        assert_eq!(lists.walk().unwrap(), joined);
    }

    /// The walk is checked: a list out of join order, or an entity that never
    /// went through the list's `push`, is reported and not walked past.
    #[test]
    fn the_walk_reports_a_list_out_of_order_and_an_entity_that_never_joined() {
        let (mut lists, _) = mixed();
        lists.tridents.swap(0, 1);
        assert!(matches!(lists.walk(), Err(JoinOrderError::OutOfOrder(_))));
        let (mut lists, _) = mixed();
        lists.stands.push(ArmorStand::new(9, at(), 0.0));
        lists.stands[2].join = horizon_client::join_seq::JoinSeq::UNJOINED;
        assert_eq!(
            lists.walk(),
            Err(JoinOrderError::Unjoined(EntityHandle::ArmorStand(2)))
        );
    }
}

#[cfg(test)]
mod set_rotation_tests {
    use super::Entity as _;
    use horizon_client::entities::ExperienceOrb;

    /// `Entity.teleportSetPosition`: yaw as given, pitch clamped to -90..90, on
    /// every kind (the ones with no field of their own used to drop it).
    #[test]
    fn set_rotation_keeps_yaw_and_clamps_pitch_on_kinds_that_had_none() {
        let mut orb = ExperienceOrb {
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            join: Default::default(),
            no_gravity: false,
            id: 0,
            position: horizon_core::geometry::Vec3::ZERO,
            previous: horizon_core::geometry::Vec3::ZERO,
            velocity: horizon_core::geometry::Vec3::ZERO,
            value: 1,
            age: 0,
            on_ground: false,
            count: 1,
            ticks: 0,
            health: 5,
            air: 300,
            rotation: [0.0, 0.0],
            following: None,
            fire_ticks: 0,
        };
        orb.set_rotation([90.0, 10.0]);
        assert_eq!(orb.rotation(), [90.0, 10.0]);
        orb.set_rotation([0.0, 200.0]);
        assert_eq!(orb.rotation(), [0.0, 90.0]);
    }
}

#[cfg(test)]
mod scoped_edit_tests {
    use super::*;

    #[test]
    fn scoped_walk_skips_only_one_entity_and_keeps_later_kinds() {
        let mut lists = TakenEntities {
            mobs: Default::default(),
            dropped: Default::default(),
            boats: Default::default(),
            carts: Default::default(),
            armor_stands: Default::default(),
            tnts: Default::default(),
            falling: Default::default(),
            orbs: Default::default(),
            projectiles: Default::default(),
            rockets: Default::default(),
            tridents: Default::default(),
            general: Default::default(),
        };
        lists
            .boats
            .push(Boat::new(1, "oak".into(), Vec3::ZERO, 0.0));
        lists
            .boats
            .push(Boat::new(2, "oak".into(), Vec3::ZERO, 0.0));
        lists.carts.push(Minecart::new(
            3,
            super::super::minecarts::CartKind::Rideable,
            Vec3::ZERO,
        ));
        let mut visited = Vec::new();
        for_each_entity_mut!(lists, entity => {
            if entity.id() == 1 {
                return;
            }
            visited.push(entity.id());
            entity.velocity_mut().x = 4.0;
        });
        assert_eq!(visited, [2, 3]);
        assert_eq!(lists.boats[0].velocity.x, 0.0);
        assert_eq!(lists.boats[1].velocity.x, 4.0);
        assert_eq!(lists.carts[0].velocity.x, 4.0);
        let previous = entity_access!(exclusive, lists.boats, 1, boat, {
            let previous = boat.velocity.x;
            boat.velocity.x = 7.0;
            previous
        });
        assert_eq!(previous, 4.0);
        assert_eq!(lists.boats[1].velocity.x, 7.0);
    }
}

#[cfg(test)]
#[path = "entity/taken_pass_tests.rs"]
pub(in crate::game) mod taken_pass_tests;
