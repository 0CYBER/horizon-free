//! The natural spawning pass on the parallel tick's helper threads (docs/parallel-natural-spawning.md):
//! the chunks' spawn attempts, which only read, run in lanes against the caps as the pass found
//! them, and the server thread then commits what they proposed in the pass's chunk order.
use super::*;
use crate::game::creatures::prep;
use horizon_client::mobs::{
    finalize::{self, FinalizeLevel},
    spawn_selection::{self, Lane, Pack},
};
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

/// A mob a lane made for an accepted request: the mob, what its finalizeSpawn created beside it
/// (jockeys and mounts), and the chickens already in the level it seats riders on, as
/// `(chicken, rider)`. Its ids are the lane's own provisional ones until the commit takes it.
pub(super) struct Made {
    mob: Mob,
    created: Vec<Mob>,
    rides: Vec<(u32, u32)>,
}

/// A lane's [`FinalizeLevel`]: what finalizeSpawn reads of the level from the lane's own world
/// view and the shared borrows, with no `Game`. A lane runs only with no mod loaded, so the
/// finalizeSpawn event has no listener to ask.
struct LaneFinalize<'a, 'g> {
    l: &'a Level<'g, NoGame>,
    rides: Vec<(u32, u32)>,
}
impl FinalizeLevel for LaneFinalize<'_, '_> {
    fn mob_maps(&self) -> &horizon_client::neoforge_mob_maps::NeoForgeMobMaps {
        self.l.w.mob_maps
    }
    fn biome(&self, pos: [i32; 3]) -> String {
        // A spawn site without its chunk is neutral.
        self.l
            .biome(pos[0], pos[1], pos[2])
            .map_or_else(|| "minecraft:plains".to_owned(), |b| b.name.clone())
    }
    fn biome_tag(&self, tag: &str, pos: [i32; 3]) -> bool {
        let Some(b) = self.l.biome(pos[0], pos[1], pos[2]) else {
            return false;
        };
        self.l.w.data.biome_has(tag, &b.name)
    }
    fn structures_with_piece_at(&self, pos: [i32; 3]) -> Vec<String> {
        structures_reaching_in(
            self.l.w.world,
            &|| self.l.host.locator.clone(),
            pos,
            &|_| true,
        )
        .into_iter()
        .filter(|(_, piece)| *piece)
        .map(|(n, _)| n)
        .collect()
    }
    fn moon_phase(&self, _: [i32; 3]) -> usize {
        (self.l.w.day_time.div_euclid(24000).rem_euclid(8)) as usize
    }
    fn spawning_monsters(&self) -> bool {
        self.l.host.spawning_monsters
    }
    fn no_collision(&self, bounds: Aabb) -> bool {
        // The same answer the pass's `GameLevel` gives: block collisions with the shulkers in the
        // level and the lane's own adds.
        let mut scene = SceneCollisions {
            world: self.l.w.world,
            binding: self.l.w.collision,
            subject: None,
        };
        let grown = bounds.inflate(Vec3::new(1e-7, 1e-7, 1e-7));
        no_collision_empty(&mut scene, bounds)
            && !self
                .l
                .mobs_near(grown)
                .any(|(_, b, _, shulker)| shulker && b.intersects(grown))
            && !self
                .l
                .pending_ids
                .borrow()
                .iter()
                .any(|(id, b)| id == "minecraft:shulker" && b.intersects(grown))
    }
    fn free_chicken(&self, bounds: Aabb) -> Option<u32> {
        self.l
            .shared_chickens?
            .iter()
            .find(|(_, b)| b.intersects(bounds))
            .map(|(id, _)| *id)
    }
    fn ride_chicken(&mut self, chicken: u32, rider: u32) {
        self.rides.push((chicken, rider));
    }
    fn game_time(&self) -> i64 {
        self.l.w.day_time
    }
    fn halloween(&self) -> bool {
        finalize::is_halloween()
    }
    fn create(&mut self, kind: &str, _: Reason) -> Option<Mob> {
        if self.l.w.peaceful() && !finalize::allowed_in_peaceful(kind) {
            return None;
        }
        let t = self.l.host.mob_types.get(kind)?.clone();
        Some(Mob::new(self.l.provisional_id(), t, Vec3::ZERO, 0.0))
    }
    fn spawn_data(&self) -> finalize::SpawnData<'_> {
        finalize::SpawnData {
            registries: self.l.host.registries,
            enchantments: self.l.host.enchantments,
            items: self.l.host.items,
            camel_husk_size: self.l.host.camel_husk_size,
        }
    }
}

impl Level<'_, NoGame> {
    /// Creates a mob the spawner accepted, finalized the game's way (`finalizeSpawn` through
    /// [`finalize::Ctx::finalize`] over the lane's view of the level, its draws on the chunk's
    /// random between the position draws, as the serial pass's [`GameLevel::finalize_spawned`]
    /// does with no mod listening). The commit adds it with what it created.
    pub(super) fn finalized<R: RandomSource>(
        &self,
        r: &SpawnRequest,
        random: &mut R,
    ) -> Option<(Spawned, Made)> {
        let kind = self.host.mob_types.get(r.entity)?.clone();
        let [x, y, z] = r.position;
        let mut mob = Mob::new(self.provisional_id(), kind, Vec3::new(x, y, z), r.yaw);
        // NaturalSpawner.spawnCategoryForPosition passes each member the group data the previous
        // one returned, starting from null.
        let group = if r.group_index == 0 {
            None
        } else {
            self.group.borrow().clone()
        };
        let difficulty = local_difficulty_in(self.w.world, r.position);
        let mut level = LaneFinalize {
            l: self,
            rides: Vec::new(),
        };
        let mut cx = finalize::Ctx {
            level: &mut level,
            random,
            difficulty,
            created: Vec::new(),
            jockey_rider: None,
        };
        let group = cx.finalize(&mut mob, r.reason, group);
        let created = std::mem::take(&mut cx.created);
        let rides = level.rides;
        *self.group.borrow_mut() = group;
        for m in &created {
            let bounds = m.bounds();
            self.pending.borrow_mut().push(bounds);
            self.pending_ids
                .borrow_mut()
                .push((m.kind.id.clone(), bounds));
        }
        let bounds = mob.bounds();
        Some((
            Spawned {
                bounds,
                school: true,
                cluster_size: None,
            },
            Made { mob, created, rides },
        ))
    }
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
) -> Vec<Pack<Made>> {
    let mobs = LevelHost::mobs(&level.game);
    let boxes = MobBoxes::new(mobs);
    let chickens: Vec<(u32, Aabb)> = mobs
        .iter()
        .filter(|c| {
            c.species() == Species::Chicken
                && c.alive()
                && c.extra.rider.is_none()
                && c.extra.mount.is_none()
        })
        .map(|c| (c.id, c.bounds()))
        .collect();
    let squares =
        (level.nearest.len() > PLAYERS_SCANNED).then(|| PlayerSquares::new(&level.nearest));
    let rebuild = prep::hookless(&level.w);
    let game_time = level.w.game_time;
    let (host, players, nearest, costed) =
        (&level.host, &level.players, &level.nearest, &level.costed);
    let attempts = AtomicU64::new(0);
    let part = |part: &[(i32, i32)], out: &mut Vec<Pack<Made>>| {
        let lane_level = Level {
            game: NoGame,
            host: host.clone(),
            shared_boxes: Some(&boxes),
            shared_squares: squares.as_ref(),
            shared_chickens: Some(&chickens),
            w: rebuild(),
            players: players.clone(),
            nearest: nearest.clone(),
            player_squares: Default::default(),
            pending: Default::default(),
            pending_ids: Default::default(),
            spawned: Default::default(),
            rides: Default::default(),
            provisional: Default::default(),
            boxes: Default::default(),
            box_queries: Default::default(),
            group: Default::default(),
            costed: costed.clone(),
            spawner: None,
            checked: Default::default(),
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
            lane_level.pending_ids.borrow_mut().clear();
            horizon_threads::work::set_chunk(chunk.0, chunk.1);
            let mut random = chunk_random(chunk, game_time);
            out.extend(lane.chunk(
                &lane_level,
                &mut random,
                categories,
                chunk,
                &mut |l, r, rng| {
                    let (spawned, made) = l.finalized(r, rng)?;
                    l.pending.borrow_mut().push(spawned.bounds);
                    Some((spawned, made))
                },
            ));
        }
        horizon_threads::work::clear_work();
        attempts.fetch_add(spawn_selection::take_attempts(), Ordering::Relaxed);
    };
    let packs = horizon_client::parallel::par_map_on(chunks, 2, lanes, part);
    spawn_selection::add_attempts(attempts.into_inner());
    packs
}

/// Gives a lane's mob, and what it created, ids from the pass's own provisional run (every lane
/// counts from its own start), and rewrites the mount and rider links and the chicken rides that
/// name them, as [`Game::resolve_spawn_ids`] expects them.
fn renumber(level: &GameLevel<'_, '_>, made: &mut Made) {
    let mut links: HashMap<u32, u32> = HashMap::new();
    for m in std::iter::once(&mut made.mob).chain(made.created.iter_mut()) {
        let id = level.provisional_id();
        links.insert(m.id, id);
        m.id = id;
    }
    for m in std::iter::once(&mut made.mob).chain(made.created.iter_mut()) {
        if let Some(l) = m.extra.mount.as_mut()
            && let Some(&id) = links.get(l)
        {
            *l = id;
        }
        if let Some(l) = m.extra.rider.as_mut()
            && let Some(&id) = links.get(l)
        {
            *l = id;
        }
    }
    for (_, rider) in made.rides.iter_mut() {
        if let Some(&id) = links.get(rider) {
            *rider = id;
        }
    }
}

/// The proposals taken in the pass's chunk order: the spawns the caps and the mobs before them
/// still allow join `pending`, as the serial pass's do, their jockeys and mounts and chicken
/// rides the level's `spawned` and `rides` for [`Game::add_spawned`].
pub(super) fn commit(
    level: &GameLevel<'_, '_>,
    state: &mut SpawnState,
    packs: Vec<Pack<Made>>,
    pending: &mut Vec<Mob>,
) {
    let mut accepted = Vec::new();
    spawn_selection::commit(state, level, packs, &mut accepted, |mut made| {
        renumber(level, &mut made);
        level.spawned.borrow_mut().extend(made.created);
        level.rides.borrow_mut().extend(made.rides);
        pending.push(made.mob);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_client::{chunk::Chunk, mobs::spawn_selection::Member};

    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            LANES.with(|l| l.set(None));
        }
    }

    /// A lit grass plain: every chunk within 9 of the origin, the player's square among them.
    fn plain(game: &mut Game<'_>) {
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
    }

    /// The mobs a pass adds are the same whether one lane or three propose them: every chunk's
    /// proposals come from its own random against the caps the pass started with.
    #[test]
    #[ignore = "needs the upstream-parallel-mobs feature and prepared resources"]
    fn the_spawns_are_the_same_in_any_number_of_lanes() {
        with_game(|game| {
            let _reset = Reset;
            plain(game);
            let (mobs, random) = (game.mobs.clone(), game.world.random.clone());
            let next = game.next_entity;
            let mut runs = Vec::new();
            for lanes in [1, 3] {
                (game.mobs, game.world.random, game.next_entity) =
                    (mobs.clone(), random.clone(), next);
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
        });
    }

    /// Two chunks near one player each propose a monster, together over the player's cap: the
    /// commit takes the first in the pass's order and drops the second.
    #[test]
    #[ignore = "needs the upstream-parallel-mobs feature and prepared resources"]
    fn the_second_of_two_proposals_over_a_players_cap_is_dropped() {
        with_game(|game| {
            plain(game);
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
            let mut state = SpawnState::new(&level, 289, spawn_players, &counted, |_, _| true);
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
        });
    }
}
