//! The client door of the move gate (#2414): the one function that arms a
//! physical entity move, and the only code allowed to reach the collide
//! entries. A family that resolves its own movement goes through a [`MoveGate`]
//! here; `clippy.toml` bans calling the `resolve*` entries anywhere else, so a
//! forgotten family cannot compile around the gate.
//!
//! [`arm`] is also where the `entity_move` hook fires (#2414, design #2308):
//! decide, fire, apply — in that one function, on the authoritative side, only
//! when a listener subscribes (one branch otherwise: no snapshot, no
//! allocation, no mod scan).
use crate::collisions::{EntityCollisions, SceneCollisions};
use horizon_core::geometry::{Aabb, Vec3, border::BorderGeometry};
use horizon_world::block_collision_scan::BlockCollisionWorld;
use horizon_world::collision_context::CollisionContext;
use horizon_world::entity_move::MoverType;
use horizon_world::move_gate::MoveGate;
use horizon_world::native_load_context::{LoopContext, LoopSide};
use horizon_world::world_movement::EntityMovementWorld;
use mod_api::MoveCause;
use std::borrow::Cow;
use std::io;

/// The `entity_move` hook's target (#2414): the generated dispatch the
/// catalog's generator requires of a target, with the loop's own reader as
/// the context. Its declaration carries `isolate`, so the same generated
/// walk is the one the move gate dispatches through — no alias, no second
/// route.
pub fn native_entity_move(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    cause: MoveCause,
    delta: &mut [f64; 3],
    canceled: &mut bool,
) {
    let reader = horizon_client_render::api_context::LoopReader(context);
    mod_api::dispatch_entity_move(&reader.context(), entity, kind, cause, delta, canceled);
}

/// The cause the mod API names for a `MoverType` (`entity_move` of #2414):
/// vanilla's five mover names, one for one.
fn cause_of(cause: MoverType) -> MoveCause {
    match cause {
        MoverType::SelfMovement => MoveCause::SelfMovement,
        MoverType::Player => MoveCause::Player,
        MoverType::Piston => MoveCause::Piston,
        MoverType::ShulkerBox => MoveCause::ShulkerBox,
        MoverType::Shulker => MoveCause::Shulker,
    }
}

/// Arm one physical `Entity.move` invocation (#2414): the entity `entity` is
/// about to move by `delta` for `cause`'s sake. The `entity_move` hook fires
/// here — in this one function — on the authoritative side, and only when a
/// listener subscribes; with none, or on a client's own prediction, the
/// movement is recorded unchanged and the tick cost is the subscriber branch.
/// A listener that cancels skips the invocation; a replacement outside ±64
/// blocks per axis, or not a finite number, is refused whole — the movement
/// stands as the requested delta would. A listener that unwinds keeps the
/// **requested** delta — the generated isolated dispatch rolls its writes
/// back, skips it from this hook's next dispatch for the session and leaves
/// its other hooks and Parts alone — and the move carries on.
///
/// `kind` names the entity's type (`minecraft:zombie`) only when called: a
/// listener needs it, a move nobody listens to must not pay for it. It runs
/// after the subscriber check, so with no listener a move costs one branch
/// and builds no string.
#[inline(never)]
pub fn arm_with<'k>(
    gate: &mut MoveGate,
    context: &LoopContext,
    entity: u32,
    kind: impl FnOnce() -> Cow<'k, str>,
    cause: MoverType,
    delta: Vec3,
) {
    // The requested delta is the floor every listener result falls back to:
    // armed first, the decision replaced only by a result that passes
    // validation — the request stays what `overridden` measures against.
    gate.arm(cause, delta);
    if context.side() != LoopSide::Server {
        return;
    }
    if !mod_api::__private::listened(mod_api::HookId::EntityMove) {
        return;
    }
    let kind = kind();
    let mut replaced = [delta.x, delta.y, delta.z];
    let mut canceled = false;
    let reader = horizon_client_render::api_context::LoopReader(context);
    mod_api::dispatch_entity_move(
        &reader.context(),
        entity,
        &kind,
        cause_of(cause),
        &mut replaced,
        &mut canceled,
    );
    if canceled {
        gate.skip();
        return;
    }
    // Design #2308: replacement components lie in ±64 blocks and are finite;
    // a malformed result is rejected whole — like the requested delta would
    // be — never clamped.
    if replaced.iter().all(|v| v.is_finite() && v.abs() <= 64.0) {
        gate.decide(Vec3::new(replaced[0], replaced[1], replaced[2]));
    }
}

/// [`arm_with`] for a mover whose type name is already a borrowed `&str`.
#[inline]
pub fn arm(
    gate: &mut MoveGate,
    context: &LoopContext,
    entity: u32,
    kind: &str,
    cause: MoverType,
    delta: Vec3,
) {
    arm_with(gate, context, entity, || Cow::Borrowed(kind), cause, delta);
}

/// [`arm_with`] for a fresh gate, in one line at a move site whose type name
/// would cost a string to build (a boat's, a ridden vehicle's).
#[inline]
pub fn armed_with<'k>(
    context: &LoopContext,
    entity: u32,
    kind: impl FnOnce() -> Cow<'k, str>,
    cause: MoverType,
    delta: Vec3,
) -> MoveGate {
    let mut gate = MoveGate::new();
    arm_with(&mut gate, context, entity, kind, cause, delta);
    gate
}

/// [`arm`] for a fresh gate, in one line at a move site.
#[inline]
pub fn armed(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    cause: MoverType,
    delta: Vec3,
) -> MoveGate {
    armed_with(context, entity, || Cow::Borrowed(kind), cause, delta)
}

/// The collide entries: the move gate's own (#2414). The `clippy.toml` ban
/// targets their callers; these wrappers are the one caller-side exception,
/// and everything else resolves through them.
#[allow(clippy::disallowed_methods)]
mod collide {
    use super::*;
    /// `SceneCollisions::resolve` through the gate: the armed move's
    /// collision, stopped by the blocks (and powder snow by the scene's
    /// `subject`).
    pub fn scene(
        gate: &mut MoveGate,
        scene: &mut SceneCollisions<'_, '_>,
        bounds: Aabb,
        movement: Vec3,
        max_step: f32,
        on_ground: bool,
    ) -> crate::Result<Vec3> {
        let _ = gate;
        scene.resolve(bounds, movement, max_step, on_ground)
    }
    /// `SceneCollisions::resolve_with_context` through the gate.
    pub fn scene_with_context(
        gate: &mut MoveGate,
        scene: &mut SceneCollisions<'_, '_>,
        bounds: Aabb,
        movement: Vec3,
        max_step: f32,
        on_ground: bool,
        context: CollisionContext<'_>,
    ) -> crate::Result<Vec3> {
        let _ = gate;
        scene.resolve_with_context(bounds, movement, max_step, on_ground, context)
    }
    /// `SceneCollisions::resolve_in_border_with` through the gate: also
    /// stopped by the world border's walls and the selected `entities` boxes.
    /// The scene is consumed, as the entry it wraps consumes it.
    pub fn scene_in_border_with(
        gate: &mut MoveGate,
        scene: SceneCollisions<'_, '_>,
        bounds: Aabb,
        movement: Vec3,
        max_step: f32,
        on_ground: bool,
        border: Option<(&BorderGeometry, [f64; 2])>,
        entities: &[Aabb],
    ) -> crate::Result<Vec3> {
        let _ = gate;
        scene.resolve_in_border_with(bounds, movement, max_step, on_ground, border, entities)
    }
    /// `EntityCollisions::resolve` through the gate.
    pub fn entities<W>(
        gate: &mut MoveGate,
        collisions: &mut EntityCollisions<'_, W>,
        bounds: Aabb,
        movement: Vec3,
        max_step: f32,
        on_ground: bool,
    ) -> crate::Result<Vec3>
    where
        W: BlockCollisionWorld<Error = io::Error> + EntityMovementWorld<Error = io::Error>,
    {
        let _ = gate;
        collisions.resolve(bounds, movement, max_step, on_ground)
    }
    /// `EntityCollisions::resolve_with_context` through the gate.
    pub fn entities_with_context<W>(
        gate: &mut MoveGate,
        collisions: &mut EntityCollisions<'_, W>,
        bounds: Aabb,
        movement: Vec3,
        max_step: f32,
        on_ground: bool,
        context: CollisionContext<'_>,
    ) -> crate::Result<Vec3>
    where
        W: BlockCollisionWorld<Error = io::Error> + EntityMovementWorld<Error = io::Error>,
    {
        let _ = gate;
        collisions.resolve_with_context(bounds, movement, max_step, on_ground, context)
    }
}
pub use collide::{
    entities as entities_resolve, entities_with_context as entities_resolve_with_context,
    scene as scene_resolve, scene_in_border_with as scene_resolve_in_border_with,
    scene_with_context as scene_resolve_with_context,
};
