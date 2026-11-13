//! The natural spawning pass on the parallel tick's helper threads (docs/parallel-natural-spawning.md):
//! the chunks' spawn attempts, which only read, run in lanes against the caps as the pass found
//! them, and the server thread then commits what they proposed in the pass's chunk order.
use super::*;
use crate::game::creatures::prep;
use horizon_client::mobs::spawn_selection::{self, Lane, Pack};
use std::sync::atomic::{AtomicU64, Ordering};

/// Fewer candidate chunks than this stay on the server thread. A pass costs about a microsecond
/// an attempt (the official SMP's 14,000 attempts take 15 ms) and a player brings 289 chunks and
/// a few hundred attempts, so a pass over two players' worth of chunks is some half a
/// millisecond: about what handing out the work and building a level for every part costs.
const MIN_CHUNKS: usize = 512;

#[cfg(test)]
thread_local! {
    /// A test's lanes, in place of the parallel tick's.
    pub(super) static LANES: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// The lanes the pass runs in, or none for the serial pass: the parallel tick is on with two
/// lanes or more, the chunks are many, nothing keeps the draws on one thread (a replayed seed
/// sequence) and no mod hears the spawns (its hooks run on the server thread).
pub(super) fn lanes_for<H: LevelHost>(
    level: &Level<'_, H>,
    chunks: usize,
    spawning: bool,
) -> Option<usize> {
    #[cfg(test)]
    let tested = LANES.with(std::cell::Cell::get);
    #[cfg(not(test))]
    let tested = None;
    let lanes = match tested {
        Some(lanes) => lanes,
        None if horizon_client::parallel::parallel_tick() > 0 && chunks >= MIN_CHUNKS => {
            horizon_client::parallel::lanes()
        }
        None => return None,
    };
    let serial = !spawning
        || (tested.is_none() && lanes < 2)
        || level.spawn_hooks()
        || horizon_client::random_source::repeating_per_thread();
    (!serial).then_some(lanes)
}

/// A chunk's own random: the same whichever lane takes the chunk, and a new one every tick.
fn chunk_random(chunk: (i32, i32), game_time: i64) -> LegacyRandom {
    let mut z = (u64::from(chunk.0 as u32) << 32 | u64::from(chunk.1 as u32))
        ^ (game_time as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    // splitmix64.
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    LegacyRandom::new((z ^ (z >> 31)) as i64)
}

/// What every chunk of `chunks` proposes, against `state` as it stands, in the chunks' order.
/// Each part of the chunks gets a level of its own over the same shared reads: the pending
/// boxes and the lazy grids are not shared.
pub(super) fn propose(
    level: &GameLevel<'_, '_>,
    state: &SpawnState,
    categories: &[Category],
    chunks: &[(i32, i32)],
    lanes: usize,
) -> Vec<Pack<Mob>> {
    let boxes = MobBoxes::new(level.game.mobs());
    let squares =
        (level.nearest.len() > PLAYERS_SCANNED).then(|| PlayerSquares::new(&level.nearest));
    let rebuild = prep::hookless(&level.w);
    let game_time = level.w.game_time;
    let (host, players, nearest, costed) =
        (&level.host, &level.players, &level.nearest, &level.costed);
    let attempts = AtomicU64::new(0);
    let part = |part: &[(i32, i32)], out: &mut Vec<Pack<Mob>>| {
        let lane_level = Level {
            game: NoGame,
            host: host.clone(),
            shared_boxes: Some(&boxes),
            shared_squares: squares.as_ref(),
            w: rebuild(),
            players: players.clone(),
            nearest: nearest.clone(),
            player_squares: Default::default(),
            pending: Default::default(),
            pending_ids: Default::default(),
            boxes: Default::default(),
            box_queries: Default::default(),
            costed: costed.clone(),
        };
        let mut lane = Lane::new(state);
        // Whatever an earlier part left on this thread is not this part's.
        spawn_selection::take_attempts();
        for &chunk in part {
            if !lane_level.can_spawn_in_chunk(chunk.0, chunk.1) {
                continue;
            }
            // A chunk sees the mobs of the pass's start only: what the lane proposed before it
            // is for the commit to weigh.
            lane_level.pending.borrow_mut().clear();
            horizon_threads::work::set_chunk(chunk.0, chunk.1);
            let mut random = chunk_random(chunk, game_time);
            out.extend(
                lane.chunk(&lane_level, &mut random, categories, chunk, &mut |l, r| {
                    let mob = l.finalized(r)?;
                    let bounds = mob.bounds();
                    l.pending.borrow_mut().push(bounds);
                    Some((
                        Spawned {
                            bounds,
                            school: true,
                        },
                        mob,
                    ))
                }),
            );
        }
        horizon_threads::work::clear_work();
        attempts.fetch_add(spawn_selection::take_attempts(), Ordering::Relaxed);
    };
    let packs = horizon_client::parallel::par_map_on(chunks, 2, lanes, part);
    spawn_selection::add_attempts(attempts.into_inner());
    packs
}

/// The proposals taken in the pass's chunk order: the spawns the caps and the mobs before them
/// still allow join `pending`, as the serial pass's do.
pub(super) fn commit(
    level: &GameLevel<'_, '_>,
    state: &mut SpawnState,
    packs: Vec<Pack<Mob>>,
    pending: &mut Vec<Mob>,
) {
    let mut accepted = Vec::new();
    spawn_selection::commit(state, level, packs, &mut accepted, |mob| pending.push(mob));
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::{chunk::Chunk, mobs::spawn_selection::Member};

    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            LANES.with(|l| l.set(None));
        }
    }

    /// A lit grass plain: every chunk within 9 of the origin, the player's square among them.
    fn plain() -> Game<'static> {
        let mut game = crate::game::native_machines::tests::server_game();
        game.build_registries();
        game.set_allow_commands(true);
        for x in -9..=9 {
            for z in -9..=9 {
                let mut chunk = Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), Arc::new(chunk));
            }
        }
        let grass = game
            .pack
            .assets
            .registry
            .default_state("minecraft:grass_block");
        for x in -144..144 {
            for z in -144..144 {
                game.world.set(x, 63, z, grass.unwrap()).unwrap();
            }
        }
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5))
                .unwrap(),
        );
        game.update_chunk_tickets();
        game
    }

    /// The mobs a pass adds are the same whether one lane or three propose them: every chunk's
    /// proposals come from its own random against the caps the pass started with.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_spawns_are_the_same_in_any_number_of_lanes() {
        let _reset = Reset;
        let mut game = plain();
        let (mobs, random) = (game.mobs.clone(), game.world.random.clone());
        let next = game.next_entity;
        let mut runs = Vec::new();
        for lanes in [1, 3] {
            (game.mobs, game.world.random, game.next_entity) = (mobs.clone(), random.clone(), next);
            LANES.with(|l| l.set(Some(lanes)));
            for tick in 1..=6 {
                // Every 400th tick the creatures spawn too.
                game.world.game_time = 400 * tick;
                game.spawn_natural();
            }
            runs.push(
                game.mobs
                    .iter()
                    .map(|m| (m.kind.id.clone(), m.position, m.yaw))
                    .collect::<Vec<_>>(),
            );
        }
        assert!(runs[0].len() > 3, "the plain spawns: {}", runs[0].len());
        assert_eq!(runs[0], runs[1]);
    }

    /// Two chunks near one player each propose a monster, together over the player's cap: the
    /// commit takes the first in the pass's order and drops the second.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_second_of_two_proposals_over_a_players_cap_is_dropped() {
        let game = plain();
        let level = game.natural_level();
        let nearby = Counted {
            entity: "minecraft:zombie",
            category: Category::Monster,
            block: [8, 64, 8],
            is_mob: true,
            persistent: false,
        };
        // One under the cap of 70 monsters near the player.
        let counted = vec![nearby; 69];
        let spawn_players = vec![SpawnPlayer {
            position: [8.5, 64.0, 8.5],
            spectator: false,
        }];
        let mut state = SpawnState::new(&level, 289, spawn_players, counted, |_, _| true);
        let pack = |chunk: (i32, i32), id: u8| Pack {
            category: Category::Monster,
            chunk,
            members: vec![Member {
                entity: "minecraft:zombie".to_owned(),
                species: Species::Zombie,
                category: Category::Monster,
                block: [chunk.0 * 16 + 3, 64, chunk.1 * 16 + 3],
                bounds: Aabb::new(Vec3::ZERO, Vec3::ZERO),
                made: id,
            }],
        };
        let mut taken = Vec::new();
        let mut accepted = Vec::new();
        spawn_selection::commit(
            &mut state,
            &level,
            vec![pack((1, 0), 1), pack((0, 1), 2)],
            &mut accepted,
            |id| taken.push(id),
        );
        assert_eq!(taken, [1]);
    }
}
