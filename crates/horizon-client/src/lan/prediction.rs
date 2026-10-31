//! The movement prediction's replay (docs/anticheat.md): each client tick a player's record holds
//! ([`horizon_anticheat::prediction::Record`]) is played again with the client's own movement code,
//! from where the last one ended, and compared with where the client said it went.
//!
//! The inputs are known exactly: a vanilla client sends ServerboundPlayerInputPacket after its tick's
//! aiStep (auto-jump included), its sprinting just before the move, and its item use from the key
//! handling before the tick. So nothing about them is guessed: a candidate that sprinted or skipped
//! the item-use slowdown where the client said otherwise would let speed and no-slowdown cheats
//! through. What the server can't know is tried, every combination of it, the tick as heard first,
//! keeping the closest outcome: whether the client stood on something on the first tick after a
//! start (a teleport), and on the tick where the server's item use changed, the use as it was before
//! (the server ends a use on its own tick, the client on its own).
//!
//! The replay then goes on from where the client said it is, so a small error never builds up, and a
//! client that lies a little each tick is measured against its own last position.

use std::collections::VecDeque;

use horizon_anticheat::prediction::{ClientTick, KEPT};
use horizon_core::geometry::Vec3;

use crate::motion::PlayerMotion;
use crate::sprint::Keys;

/// A move shorter than this isn't sent (LocalPlayer.sendPosition: 2.0E-4).
const UNSENT: f64 = 2.0e-4;

/// Below this the replay and the client agree (float noise).
pub const EXACT: f64 = 1e-6;

/// One client tick played again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Played {
    /// How far the closest outcome ended from where the client said.
    pub offset: f64,
    /// Which of the unknowns it took: 1, the ground the other way (the first tick after a start); 2,
    /// the item use before the server's change. 0 is the tick as the server heard it.
    pub candidate: u8,
    /// The replay and the client agree about the ground (the move's flag 1).
    pub ground: bool,
}

/// A player as its client moves it.
#[derive(Clone)]
pub struct Replay {
    /// The player at the end of the last tick played.
    body: Box<PlayerMotion>,
    /// Where the client last said it was.
    said: Vec3,
    /// The record's ticks played so far.
    played: u64,
    /// No tick played since the start: whether the client stood on something is unknown.
    fresh: bool,
    /// The server's item-use slowdown at the last tick played.
    using: Option<f32>,
    /// The last ticks played, oldest first, at most [`KEPT`].
    pub ticks: VecDeque<Played>,
}

impl Replay {
    /// A replay that starts at `at`, standing still (a teleport the client accepted), after the
    /// record's first `played` ticks. `server` is the server's own copy of the player.
    pub fn new(server: &PlayerMotion, at: [f64; 3], played: u64) -> Self {
        let mut body = Box::new(server.clone());
        let at = Vec3::new(at[0], at[1], at[2]);
        (body.position, body.old_position, body.velocity) = (at, at, Vec3::ZERO);
        body.fall_distance = 0.0;
        body.events.clear();
        Self {
            body,
            said: at,
            played,
            fresh: true,
            using: server.item_use_speed,
            ticks: VecDeque::new(),
        }
    }

    /// The record's ticks played so far.
    pub fn played(&self) -> u64 {
        self.played
    }

    /// Ticks that aren't played (riding, sleeping...): the replay goes on after them.
    pub fn skip_to(&mut self, ended: u64) {
        self.played = ended;
    }

    /// Plays `tick` again. `server` is the server's copy of the player, for what the server decides
    /// (abilities, effects, attributes); `run` moves a body one client tick with these keys and
    /// sprinting (`MotionWorld::replayed_tick`). None when no candidate could be played.
    pub fn play(
        &mut self,
        tick: &ClientTick,
        server: &PlayerMotion,
        mut run: impl FnMut(&mut PlayerMotion, Keys, bool) -> crate::Result<()>,
    ) -> Option<Played> {
        self.played += 1;
        sync(&mut self.body, server);
        self.body.yaw = tick.yaw;
        self.body.head_yaw = tick.yaw;
        self.body.pitch = tick.pitch;
        // The first tick after a start guesses the ground from the client's move; the replay finds
        // it after that.
        let fresh = std::mem::take(&mut self.fresh);
        if fresh {
            self.body.ground.on_ground = tick.flags.is_some_and(|f| f & 1 != 0);
        }
        let using = std::mem::replace(&mut self.using, server.item_use_speed);
        let said = tick.position.map(|p| Vec3::new(p[0], p[1], p[2]));
        let keys = keys(tick.input);
        let last = self.said;
        let offset = |body: &PlayerMotion| match said {
            Some(p) => body.position.distance(p),
            // No move: the client is within 2.0E-4 of where it last said.
            None => (body.position.distance(last) - UNSENT).max(0.0),
        };
        let unknown = |candidate: u8| {
            candidate & 1 != 0 && fresh || candidate & 2 != 0 && using != server.item_use_speed
        };
        let (offset, candidate) = if !unknown(1) && !unknown(2) {
            // Nothing to try (most ticks): the body itself plays the tick.
            run(&mut self.body, keys, tick.sprinting).ok()?;
            (offset(&self.body), 0)
        } else {
            let mut best: Option<(f64, u8, Box<PlayerMotion>)> = None;
            for candidate in
                (0..4u8).filter(|&c| c == 0 || (1..=2).all(|b| c & b == 0 || unknown(b)))
            {
                let mut body = self.body.clone();
                if candidate & 1 != 0 {
                    body.ground.on_ground = !body.ground.on_ground;
                }
                if candidate & 2 != 0 {
                    body.item_use_speed = using;
                }
                if run(&mut body, keys, tick.sprinting).is_err() {
                    continue;
                }
                let off = offset(&body);
                if best.as_ref().is_none_or(|b| off < b.0) {
                    best = Some((off, candidate, body));
                }
                if off <= EXACT {
                    break;
                }
            }
            let (off, candidate, body) = best?;
            self.body = body;
            (off, candidate)
        };
        let body = &mut self.body;
        body.events.clear();
        body.item_use_speed = server.item_use_speed;
        match said {
            Some(p) => {
                body.position = p;
                self.said = p;
            }
            None if offset > 0.0 => body.position = self.said,
            None => {}
        }
        let ground = tick
            .flags
            .is_none_or(|f| (f & 1 != 0) == self.body.ground.on_ground);
        let played = Played {
            offset,
            candidate,
            ground,
        };
        if self.ticks.len() == KEPT {
            self.ticks.pop_front();
        }
        self.ticks.push_back(played);
        Some(played)
    }
}

/// ServerboundPlayerInputPacket's flags as keys.
fn keys(input: u8) -> Keys {
    let key = |bit: u8| input & bit != 0;
    Keys {
        forward: key(1),
        backward: key(2),
        left: key(4),
        right: key(8),
        jump: key(16),
        shift: key(32),
        sprint: key(64),
        analog: None,
    }
}

/// What the server decides about the player, from its own copy: the rest is the client's.
fn sync(body: &mut PlayerMotion, server: &PlayerMotion) {
    // The attributes hold the server's sprint modifier: the body sprints as the server thinks until
    // the replay sets the client's own.
    body.attributes = server.attributes.clone();
    body.sprinting = server.sprinting;
    body.status = server.status.clone();
    (body.may_fly, body.flying, body.flying_speed) =
        (server.may_fly, server.flying, server.flying_speed);
    (body.invulnerable, body.spectator, body.sleeping) =
        (server.invulnerable, server.spectator, server.sleeping);
    body.glider = server.glider;
    (body.item_use_speed, body.item_use_slows_sprint) =
        (server.item_use_speed, server.item_use_slows_sprint);
    (
        body.leather_boots,
        body.freeze_immune,
        body.burn_from_stepping_immune,
    ) = (
        server.leather_boots,
        server.freeze_immune,
        server.burn_from_stepping_immune,
    );
    body.border = server.border.clone();
    // The keys already hold the client's auto-jump.
    (body.auto_jump_enabled, body.auto_jump_time) = (false, 0);
    (body.client_side, body.server_copy) = (true, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        assets::Assets,
        motion::{MotionResources, MotionWorld},
        world::World,
    };
    use horizon_anticheat::prediction::Record;
    use std::path::Path;

    /// Keys for each tick: 20 walking, 12 sprint-jumping, 12 strafing while turning, 6 still.
    fn script() -> Vec<(u8, f32)> {
        let mut ticks = Vec::new();
        ticks.extend((0..20).map(|_| (1, 0.0)));
        ticks.extend((0..12).map(|_| (1 | 16 | 64, 0.0)));
        ticks.extend((0..12).map(|i| (1 | 4, 3.0 * i as f32)));
        ticks.extend((0..6).map(|_| (0, 36.0)));
        ticks
    }

    /// The offsets of a client that walks `script` in the test scene, its moves passed through
    /// `cheat` (the position it sends and stays at, from the one before and the honest one).
    fn offsets(cheat: impl Fn(Vec3, Vec3) -> Vec3) -> Option<Vec<Played>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return None;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 2).unwrap();
        // A clean stone floor with its top at y 5, and a slab on the walk to step up on.
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let air = assets.registry.default_state("minecraft:air").unwrap();
        for x in 10..=30 {
            for z in 10..=40 {
                scene.set(x, 4, z, stone).unwrap();
                for y in 5..=12 {
                    scene.set(x, y, z, air).unwrap();
                }
            }
        }
        let slab = assets
            .registry
            .default_state("minecraft:smooth_stone_slab")
            .unwrap();
        scene.set(20, 5, 23, slab).unwrap();
        let start = Vec3::new(20.5, 5.0, 20.5);
        let mut client = PlayerMotion::new(&resources, start).unwrap();
        client.client_side = true;
        let server = PlayerMotion::new(&resources, start).unwrap();
        let mut world = |player: &mut PlayerMotion, f: &mut dyn FnMut(&mut MotionWorld)| {
            f(&mut MotionWorld {
                entities: &[],
                player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            })
        };
        // Settle on the floor first.
        for _ in 0..10 {
            world(&mut client, &mut |w| {
                w.controlled_simulation_tick(Keys::default()).unwrap()
            });
        }
        // The start is a teleport: the client goes on from there, standing still.
        client.velocity = Vec3::ZERO;
        let at = client.position;
        let mut replay = Replay::new(&server, [at.x, at.y, at.z], 0);
        let mut record = Record::default();
        let mut played = Vec::new();
        for (input, yaw) in script() {
            let before = client.position;
            client.yaw = yaw;
            world(&mut client, &mut |w| {
                w.controlled_simulation_tick(keys(input)).unwrap()
            });
            client.position = cheat(before, client.position);
            let p = client.position;
            let ground = u8::from(client.ground.on_ground);
            record.moved(Some([p.x, p.y, p.z]), Some((yaw, 0.0)), ground);
            record.tick_end(input, client.sprinting);
            let ticks: Vec<_> = record.since(replay.played()).unwrap().copied().collect();
            for tick in ticks {
                let run = |body: &mut PlayerMotion, keys, sprinting| {
                    let mut result = Ok(());
                    world(body, &mut |w| result = w.replayed_tick(keys, sprinting));
                    result
                };
                played.push(replay.play(&tick, &server, run).unwrap());
            }
        }
        Some(played)
    }

    #[test]
    fn an_honest_client_is_played_again_exactly() {
        let Some(played) = offsets(|_, honest| honest) else {
            return;
        };
        assert_eq!(played.len(), script().len());
        for (i, p) in played.iter().enumerate() {
            assert!(p.offset <= EXACT, "tick {i}: {p:?}");
            assert!(p.ground, "tick {i}: {p:?}");
            assert!(p.candidate == 0 || i == 0, "tick {i}: {p:?}");
        }
    }

    #[test]
    fn a_faster_or_flying_client_is_not() {
        let Some(speed) = offsets(|before, honest| {
            let d = honest.subtract(before);
            before.add_vector(Vec3::new(d.x * 1.3, d.y, d.z * 1.3))
        }) else {
            return;
        };
        let far = speed.iter().filter(|p| p.offset > 0.01).count();
        assert!(far > 30, "{far} of {} ticks off: {speed:?}", speed.len());
        let fly = offsets(|_, honest| honest.add_vector(Vec3::new(0.0, 0.1, 0.0))).unwrap();
        assert!(fly.iter().all(|p| p.offset > 0.05), "{fly:?}");
    }
}
