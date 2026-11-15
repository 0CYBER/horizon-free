//! Server-side player bots for mods (docs/native-mods.md, "Server hooks for plugin ports: player
//! bots"): a mod that drives a server player itself (a fake client sending its own moves) asks
//! where the player's movement code would take them in one tick, and which block shapes are in
//! a box. Nothing runs unless a mod calls it, and neither changes the player or the level.
use super::Game;
use horizon_client::collisions::SceneCollisions;
use horizon_client::motion::{MotionWorld, PlayerMotion};
use horizon_client::sprint::Keys;
use horizon_core::geometry::{Aabb, Vec3, VoxelShape};
use horizon_core::position::{BlockPos, ChunkPos};
use horizon_world::block_collision_scan::{
    BlockCollisionScan, BlockCollisionWorld, CollisionBlock,
};
use horizon_world::collision_context::CollisionContext;
use mod_api::{PlayerInput, SimulatedMove};
use std::io;
use std::ops::ControlFlow;

/// The most block cells `native_collision_boxes` looks at: a box of about 100 blocks a side.
const COLLISION_CELLS: usize = 1_000_000;

/// The keys a bot's input presses: the direction flags by sign (sprint and the server's input
/// packet read them), and the exact move vector when it isn't a key's (a stick's).
fn input_keys(input: &PlayerInput) -> Option<Keys> {
    if !(input.forward.is_finite() && input.strafe.is_finite()) {
        return None;
    }
    let forward = input.forward.clamp(-1.0, 1.0);
    let strafe = input.strafe.clamp(-1.0, 1.0);
    let key = |v: f32| v == 0.0 || v.abs() == 1.0;
    Some(Keys {
        forward: forward > 0.0,
        backward: forward < 0.0,
        left: strafe > 0.0,
        right: strafe < 0.0,
        jump: input.jump,
        shift: input.sneak,
        sprint: input.sprint,
        analog: (!key(forward) || !key(strafe)).then_some([strafe, forward]),
        ..Keys::default()
    })
}

/// `LocalPlayer.aiStep`'s sprint: it needs a forward impulse (`hasForwardImpulse`).
fn sprints(input: &PlayerInput) -> bool {
    input.sprint && input.forward > 1.0e-5
}

/// A box from `[min_x, min_y, min_z, max_x, max_y, max_z]`, when its corners are numbers in order.
fn aabb_of(b: [f64; 6]) -> Option<Aabb> {
    (b.iter().all(|v| v.is_finite()) && b[0] <= b[3] && b[1] <= b[4] && b[2] <= b[5])
        .then(|| Aabb::new(Vec3::new(b[0], b[1], b[2]), Vec3::new(b[3], b[4], b[5])))
}

fn corners(b: Aabb) -> [f64; 6] {
    let (lo, hi) = (b.min_position(), b.max_position());
    [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z]
}

// Native mods use these; nothing in Horizon does.
impl Game<'_> {
    /// One tick of the server player `uuid`'s own movement from `input`, as their client would
    /// play it (`LocalPlayer.aiStep` and `Player.travel`): walking and strafing (`forward` and
    /// `strafe`, -1..1), jumping, sneaking (slower, and held at edges), sprinting (with a
    /// forward input only), looking `yaw` and `pitch`; gravity, friction, block speed factors,
    /// stepping up, swimming and climbing as the server's movement prediction replays a client
    /// tick. It starts from where the player is now, with `input.velocity` when given (the
    /// velocity the last call returned: the server doesn't keep a bot's own), else the server's
    /// copy. Nothing is changed: the player stays where they are until the mod moves them (a
    /// move packet from its bot, `native_teleport_player`), and the level is only read.
    ///
    /// None off a server, for a player not here (another level than the one in place, a
    /// guest out of reach inside another player's hook, where the hub is taken out), dead, a
    /// spectator or riding, an input that is not a number, and when the movement code fails.
    /// API @API:player_bots@.
    pub(super) fn player_simulate_move(
        &mut self,
        player: mod_api::PlayerRef,
        input: PlayerInput,
    ) -> Option<SimulatedMove> {
        if !self.native_authoritative() || !(input.yaw.is_finite() && input.pitch.is_finite()) {
            return None;
        }
        if input
            .velocity
            .is_some_and(|v| !v.iter().all(|c| c.is_finite()))
        {
            return None;
        }
        let keys = input_keys(&input)?;
        self.player_name(player)?;
        let mut body = self.native_bot_body(player.uuid())?;
        body.yaw = input.yaw;
        body.pitch = input.pitch.clamp(-90.0, 90.0);
        if let Some([x, y, z]) = input.velocity {
            body.velocity = Vec3::new(x, y, z);
        }
        // Game.sync_player_body's part that depends on the blocks, as the replay does.
        body.nudge = crate::game::closest_space_nudge(&body, &self.world, self.res.facts());
        MotionWorld {
            entities: &[],
            player: &mut body,
            scene: &mut self.world,
            assets: &self.pack.assets,
            collision_binding: self.collision,
            resources: self.motion,
            health: &self.health_resources,
            tag_items: Some(world_items!(self)),
            loop_context: &self.loop_context,
            damage_hooks: &self.damage_hooks,
            block_registries: Some(&self.pack.block_registries),
        }
        .replayed_tick(keys, sprints(&input))
        .ok()?;
        let (p, v) = (body.position, body.velocity);
        let mut result = SimulatedMove::default();
        result.position = [p.x, p.y, p.z];
        result.velocity = [v.x, v.y, v.z];
        result.on_ground = body.ground.on_ground;
        result.horizontal_collision = body.collision.horizontal;
        result.sprinting = body.sprinting;
        Some(result)
    }

    /// A copy of the body of server player `uuid` in the level in place, as it stands now: the
    /// acting player's swapped in, or a guest's as `lan_actions` swaps it in.
    fn native_bot_body(&self, uuid: u128) -> Option<PlayerMotion> {
        if self.lan_acting_uuid == Some(uuid) {
            self.lan_acting?;
            let p = self.player.as_ref()?;
            return (p.health.health() > 0.0 && !p.spectator && self.riding.is_none())
                .then(|| p.clone());
        }
        if self.lan_acting.is_some() {
            return None;
        }
        let g = self.lan.as_ref()?.hub.guests.iter().find(|g| {
            g.placed
                && !g.dead
                && g.uuid == uuid
                && g.dimension == self.dim.id()
                && g.vehicle.is_none()
                && !g.spectator()
        })?;
        let mut body = g.motion.as_deref()?.clone();
        body.position = Vec3::new(g.position[0], g.position[1], g.position[2]);
        body.ground.on_ground = g.on_ground;
        body.yaw = g.yaw;
        body.pitch = g.pitch;
        body.sprinting = g.sprinting;
        body.may_fly = g.creative();
        body.flying = g.abilities.flying;
        body.flying_speed = g.abilities.flying_speed;
        body.fall_flying = g.fall_flying;
        Some(body)
    }

    /// The block collision boxes in `aabb` (`[min_x, min_y, min_z, max_x, max_y, max_z]`) of the
    /// level `dimension`, as `CollisionGetter.getBlockCollisions(null, aabb)` gives them: each block's
    /// collision shape that shares volume with the box, as world-space boxes (a stair gives two,
    /// a fence its post), under the empty collision context (no entity: scaffolding and powder
    /// snow as for nothing standing on them). Entities and the world border are not in it. For a
    /// bot's own collision checks and pathfinding. None as for `scan_block_collisions`: on a
    /// server another level than the one in place, on a client a level it neither shows nor
    /// views, a box that isn't a box (not numbers, or min past max), and one over about a
    /// million blocks. API @API:player_bots@.
    pub(super) fn level_collision_boxes(
        &self,
        level: mod_api::LevelRef,
        aabb: [f64; 6],
    ) -> Option<Vec<[f64; 6]>> {
        let mut boxes = Vec::new();
        self.scan_block_collisions(level, aabb, |found| {
            boxes.extend(found.iter().copied().map(corners));
            ControlFlow::Continue(())
        })?;
        Some(boxes)
    }

    /// Whether any block collision shape is in `aabb`, the block half of
    /// `CollisionGetter.noCollision` negated: `level_collision_boxes`'s walk, stopped at the
    /// first shape. None as for that.
    pub(super) fn level_has_collisions(
        &self,
        level: mod_api::LevelRef,
        aabb: [f64; 6],
    ) -> Option<bool> {
        let mut any = false;
        self.scan_block_collisions(level, aabb, |found| {
            any = !found.is_empty();
            if any {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })?;
        Some(any)
    }

    /// `Level.getBlockCollisions(null, aabb)`'s walk, each block's world-space boxes to `visit`
    /// until it breaks. None for a box that isn't one, one over `COLLISION_CELLS` and a read
    /// error; on a server for another level than the one in place, on a client for a level
    /// `client_view_world` doesn't name and a walk that reaches a column the client doesn't
    /// hold (vanilla's walk passes a missing chunk as empty).
    fn scan_block_collisions(
        &self,
        level: mod_api::LevelRef,
        aabb: [f64; 6],
        mut visit: impl FnMut(&[Aabb]) -> ControlFlow<()>,
    ) -> Option<()> {
        let bounds = aabb_of(aabb)?;
        let (world, refuse_missing) = if self.is_logical_server() {
            (self.level_in_place(level).then_some(&self.world)?, false)
        } else {
            (self.client_view_world(level)?, true)
        };
        let mut scene = ScanColumns {
            scene: SceneCollisions {
                world,
                binding: self.collision,
                subject: None,
            },
            refuse_missing,
        };
        let Ok(scan) = BlockCollisionScan::new(
            &mut scene,
            CollisionContext::empty(false),
            bounds,
            false,
            COLLISION_CELLS,
        ) else {
            return None;
        };
        for hit in scan {
            let Ok(hit) = hit else {
                return None;
            };
            if visit(hit.shape.boxes()).is_break() {
                break;
            }
        }
        Some(())
    }
}

/// The world a collision scan walks: a client's columns refuse the walk where one is missing,
/// since the client cannot know what the server holds there.
struct ScanColumns<'w, 'r> {
    scene: SceneCollisions<'w, 'r>,
    refuse_missing: bool,
}

impl<'w> BlockCollisionWorld for ScanColumns<'w, '_> {
    type Error = io::Error;
    type Chunk = &'w horizon_client::chunk::Chunk;
    fn collision_chunk(&mut self, p: ChunkPos) -> io::Result<Option<Self::Chunk>> {
        match self.scene.collision_chunk(p)? {
            None if self.refuse_missing => Err(io::Error::other(
                "collision scan reached a column the client does not hold",
            )),
            chunk => Ok(chunk),
        }
    }
    fn collision_block(&mut self, chunk: Self::Chunk, p: BlockPos) -> io::Result<CollisionBlock> {
        self.scene.collision_block(chunk, p)
    }
    fn is_suffocating(
        &mut self,
        chunk: Self::Chunk,
        block: CollisionBlock,
        p: BlockPos,
    ) -> io::Result<bool> {
        self.scene.is_suffocating(chunk, block, p)
    }
    fn context_shape(
        &mut self,
        block: CollisionBlock,
        p: BlockPos,
        context: &CollisionContext<'_>,
    ) -> io::Result<VoxelShape> {
        self.scene.context_shape(block, p, context)
    }
    fn may_reach_out(&mut self, chunk: Self::Chunk, section_y: i32) -> io::Result<bool> {
        self.scene.may_reach_out(chunk, section_y)
    }
    fn section_is_empty(&mut self, chunk: Self::Chunk, section_y: i32) -> io::Result<bool> {
        self.scene.section_is_empty(chunk, section_y)
    }
    fn missing_chunks_stay_missing(&self) -> bool {
        self.scene.missing_chunks_stay_missing()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(forward: f32, strafe: f32) -> PlayerInput {
        let mut input = PlayerInput::default();
        input.forward = forward;
        input.strafe = strafe;
        input
    }

    #[test]
    fn keys_follow_the_signs() {
        let k = input_keys(&input(1.0, -1.0)).unwrap();
        assert!(k.forward && !k.backward && k.right && !k.left);
        assert!(
            k.analog.is_none(),
            "full keys use the keyboard's own vector"
        );
        let k = input_keys(&input(-1.0, 0.0)).unwrap();
        assert!(k.backward && !k.forward && !k.left && !k.right);
    }

    #[test]
    fn partial_input_is_a_stick() {
        let k = input_keys(&input(0.5, 0.0)).unwrap();
        assert!(k.forward);
        assert_eq!(k.analog, Some([0.0, 0.5]));
        let k = input_keys(&input(3.0, -7.0)).unwrap();
        assert!(k.analog.is_none(), "clamped to full keys");
    }

    #[test]
    fn non_numbers_are_refused() {
        assert!(input_keys(&input(f32::NAN, 0.0)).is_none());
        assert!(input_keys(&input(0.0, f32::INFINITY)).is_none());
    }

    #[test]
    fn sprint_needs_forward() {
        let mut i = input(0.0, 1.0);
        i.sprint = true;
        assert!(!sprints(&i));
        i.forward = -1.0;
        assert!(!sprints(&i));
        i.forward = 1.0;
        assert!(sprints(&i));
        i.sprint = false;
        assert!(!sprints(&i));
    }

    #[test]
    fn boxes_round_trip_and_bad_boxes_are_refused() {
        let b = [0.0, 1.0, 2.0, 3.0, 4.5, 6.0];
        assert_eq!(corners(aabb_of(b).unwrap()), b);
        assert!(aabb_of([1.0, 0.0, 0.0, 0.0, 1.0, 1.0]).is_none());
        assert!(aabb_of([0.0, f64::NAN, 0.0, 1.0, 1.0, 1.0]).is_none());
    }
}
