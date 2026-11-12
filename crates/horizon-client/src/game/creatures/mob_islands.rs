//! The mob loop of `tick_creature_entities` on several threads, with the result of one thread
//! (#14457): vanilla ticks the level's entities one after another, and here too each mob ticks
//! after every mob before it that it could have seen or touched.
//!
//! During the mob loop the level's blocks are read-only (a mob's block writes are events, applied
//! after the loop in order), players are read-only views, and a mob reaches the other mobs only
//! through the snapshot (`Seen`, read near itself or by id), its pushes, and the brain events that
//! write another mob by id. So the mobs split into islands: a mob's reach for the tick (how far it
//! can move, how far it reads others, who it is linked to by id) meets only mobs of its own island.
//! Each group of islands runs its mobs' turns in list order on its own thread
//! ([`super::mob_group::Group`]); the results merge back in list order.
//!
//! The one stream every mob may draw from is the level random: each group draws from a lane of an
//! [`OrderedDraws`], so a mob's first draw waits until every mob before it in the list has had its
//! turn, and the draws come out in the serial order (horizon-core `random/ordered.rs`). A lookup by
//! id of a mob in another group (a remembered mob far away) reads it as the serial loop would show
//! it: as it was before the tick when it comes later in the list, else once its turn is over.
//!
//! `HORIZON_PARALLEL_TICK=1` turns it on (off by default), `verify` also runs the serial loop on
//! copies and logs a tick whose result differs. The loop stays serial under `HORIZON_TEST_SEEDS`,
//! with few ticking mobs, when one island holds most of them, and while a native mod may hook a
//! mob's turn, as the build finds it (`horizon_mod_api_gen::mob_turn`). The hooks that hand
//! nothing back are kept and told at the join, in list order (`native_client_hooks::Deferred`).
//! Everything else a mob can reach in its turn is in its reach: a pet's teleport to its owner, a
//! teleporting meal, a changed speed, a wither's head targets and a warden's suspects; a raid's
//! raiders share an island. How each bound is derived: `docs/mob-islands.md`.
//!
//! Whether a mob is loaded and ticking is decided at its turn, as the serial loop decides it, so
//! every mob is given a ticking mob's reach: one moved before its turn may tick where it was resting.
use super::mob_group::{Group, Shared, Start, Turns};
use super::*;
use horizon_client::{
    fast_hash::{FxHashMap, FxHashSet},
    mobs::{ForeignSeen, MobSlot, WhoLookup, leash, seen_grid::SeenGrid},
};
use horizon_core::{
    geometry::{Aabb, Axis},
    random::OrderedDraws,
};
use std::{
    cell::Cell,
    sync::{Arc, OnceLock},
};

/// Fewer ticking mobs than this tick serially: the plan would cost more than it saves.
const MIN_TICKING: usize = 256;
/// The plan's cell size, in blocks.
const CELL: f64 = 16.0;
/// Cell visits one plan may spend before the tick runs serially instead.
const PLAN_BUDGET: usize = 2_000_000;
/// The least a mob reads around itself: the widest fixed query box of the mob code (`others_near`
/// with 32 blocks), over any smaller follow range.
const MIN_SENSE: f64 = 32.0;
/// How far a reader of the village data (`village_sections`, `others_tickets`) can depend on a
/// villager's points of interest: `sections_to_village` looks up to seven sections away, plus the
/// candidate positions it tests (`land_random_pos` within 15 blocks).
const VILLAGE_SENSE: f64 = 7.0 * 16.0 + 32.0;
/// How far a villager can claim a new point of interest in one tick (the acquisition range).
const POI_CLAIM: f64 = 48.0;
/// Margin added to every reach.
const SLACK: f64 = 2.0;

#[cfg(test)]
thread_local! {
    /// A test's lanes, over the parallel tick's switch.
    static LANES: Cell<Option<usize>> = const { Cell::new(None) };
    /// The groups the last grouped loop ran in.
    static RAN: Cell<usize> = const { Cell::new(0) };
    static VERIFY: Cell<bool> = const { Cell::new(false) };
    static CHECK: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
    static FAIL_THREADS: std::cell::RefCell<Option<FailThreads>> = const { std::cell::RefCell::new(None) };
    static FAIL_TURN: Cell<Option<usize>> = const { Cell::new(None) };
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum FailAt {
    Spawn,
    Setup,
    Turn,
}

#[cfg(test)]
#[derive(Clone)]
struct FailThreads {
    at: FailAt,
    started: Arc<std::sync::Barrier>,
}

#[cfg(test)]
pub(super) fn inject_turn_failure(at: usize) {
    if FAIL_TURN.with(Cell::get) == Some(at) {
        panic!("injected mob turn failure");
    }
}

fn verifying() -> bool {
    #[cfg(test)]
    if VERIFY.with(Cell::get) {
        return true;
    }
    horizon_client::parallel::parallel_tick() == 2
}

/// Waiting lanes each need a runnable thread, not just a configured worker slot. Failure to
/// discover the hardware leaves the loop serial; the shared preparation pool is unaffected.
fn capped_lanes(requested: usize, hardware: Option<usize>) -> usize {
    requested.clamp(1, 8).min(hardware.unwrap_or(1).max(1))
}

/// The threads the loop may run on, capped at the machine's available parallelism.
fn lanes() -> usize {
    let hardware = std::thread::available_parallelism().ok().map(usize::from);
    #[cfg(test)]
    if let Some(lanes) = LANES.with(Cell::get) {
        return capped_lanes(lanes, hardware);
    }
    capped_lanes(horizon_client::parallel::lanes(), hardware)
}

/// Whether a native mod can run in a mob's turn, as the build found it (`HORIZON_MOB_TURN`, from
/// `horizon_mod_api_gen::mob_turn`): then the loop stays serial. Without that finding, any trait
/// mod counts.
fn mods_hook_turns() -> bool {
    match option_env!("HORIZON_MOB_TURN") {
        Some(found) => found == "hooked",
        None => !horizon_client::mod_api::mods().is_empty(),
    }
}

/// Whether native mods are loaded at all: grouped turns keep the hooks that hand nothing back
/// for the join, and skip the others (no mod hooks them while the islands run).
fn defers() -> bool {
    !horizon_client::mod_api::mods().is_empty()
}

/// The mobs of each group, by their place in the list, ascending.
pub(super) struct Plan {
    groups: Vec<Vec<usize>>,
}

/// What the islands need from the game besides the loop's own state.
pub(super) struct Islands<'t> {
    /// The host's own player's UUID, when one is in the level (`Game::who_by_uuid`).
    pub uuid: Option<[i32; 4]>,
    /// A new entity-ticking test for a group's thread (`Game::entity_ticking_area`).
    pub ticking: &'t dyn Fn() -> Box<dyn Fn(i32, i32) -> bool + Send>,
}

impl Islands<'_> {
    /// The groups this tick's mobs run in; None when the loop runs serially.
    pub(super) fn plan(
        &self,
        mobs: &[Mob],
        snapshot: &[Seen],
        turns: &Turns,
        start: &Start,
    ) -> Option<Plan> {
        let lanes = lanes();
        if lanes < 2
            || mods_hook_turns()
            || mobs.len() < MIN_TICKING
            || horizon_client::random_source::repeating_per_thread()
        {
            return None;
        }
        // A content mod's effect ticks through the native mods (`ModEffectTicks`), which run
        // only on the server thread.
        if defers()
            && mobs.iter().any(|m| {
                m.extra
                    .effects
                    .iter()
                    .any(|e| horizon_client_core::potions::is_mod_effect(e.0))
            })
        {
            return None;
        }
        // Where an owner stands: the players a pet can see this tick.
        let players: Vec<([i32; 4], Vec3)> = start
            .player
            .iter()
            .chain(start.guests.iter().map(|g| &g.1))
            .map(|p| (p.uuid, p.position))
            .collect();
        horizon_client::spans::time("islands_plan", || {
            islands(&reaches(mobs, snapshot, turns, &players)?, lanes)
        })
    }

    /// The loop's turns in the plan's groups, each on its own thread, put back together as the
    /// serial loop's one group would have left them.
    pub(super) fn run(
        &self,
        plan: Plan,
        mut mobs: Vec<Mob>,
        snapshot: Vec<Seen>,
        mut start: Start,
        turns: &mut Turns,
        shared: &Shared,
    ) -> Group<Mob> {
        use horizon_client::random_source;
        // `verify`: allocate seeds only in the serial copy, replay them in the grouped copy.
        // Neither the global counter nor the client threads' clock/seed policy is changed.
        let serial = verifying().then(|| {
            let mut serial = Group::new(mobs.clone(), snapshot.clone(), start.clone());
            let (_, seeds) = random_source::record_seeds(|| serial.run(turns, shared, None));
            turns.cached_difficulty = None;
            (serial, seeds)
        });
        let n = mobs.len();
        let lanes = plan.groups.len();
        let draws = OrderedDraws::new(&start.random, lanes);
        // The whole list's grid, as each group's must answer.
        let whole = SeenGrid::bounds_of(snapshot.iter().map(|s| s.bounds));
        let mut slots: Vec<Option<&mut Mob>> = mobs.iter_mut().map(Some).collect();
        let mut groups: Vec<Group<&mut Mob>> = plan
            .groups
            .into_iter()
            .enumerate()
            .map(|(lane, members)| {
                let group_mobs: Vec<&mut Mob> = members
                    .iter()
                    .map(|&k| slots[k].take().expect("a mob is in one group"))
                    .collect();
                // A group reads and writes only its own mobs' bindings and death marks.
                let bindings = group_mobs
                    .iter()
                    .filter_map(|m| start.bindings.get(&m.id).map(|&conn| (m.id, conn)))
                    .collect();
                let died = group_mobs
                    .iter()
                    .filter(|m| start.died.contains(&m.id))
                    .map(|m| m.id)
                    .collect();
                let group_start = Start {
                    events: Vec::new(),
                    random: draws.lane(lane),
                    guests: start.guests.clone(),
                    player: start.player.clone(),
                    bindings,
                    died,
                    ticked: FxHashSet::default(),
                    targets: members.iter().map(|&k| start.targets[k]).collect(),
                };
                let seen = members.iter().map(|&k| snapshot[k].clone()).collect();
                let mut group = Group::new(group_mobs, seen, group_start).answering_as(whole);
                group.global = members;
                group
            })
            .collect();
        #[cfg(test)]
        RAN.with(|r| r.set(lanes));
        let mut board = Board::new(snapshot, &groups, Arc::clone(&draws));
        self.run_threads(
            &mut groups,
            &board,
            turns,
            shared,
            serial.as_ref().map(|s| &s.1),
        );
        // Each mob's entries, in list order: the turns' ranges in its group.
        let mut place = vec![(0u32, 0u32); n];
        for (gi, g) in groups.iter().enumerate() {
            for (li, &k) in g.global.iter().enumerate() {
                place[k] = (gi as u32, li as u32);
            }
        }
        let mut keep = Vec::with_capacity(n);
        let mut targets = std::mem::take(&mut start.targets);
        targets.clear();
        let (mut dead, mut corpses, mut gone, mut guest_events) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for &(gi, li) in &place {
            let (g, li) = (&mut groups[gi as usize], li as usize);
            keep.push(g.keep[li]);
            targets.push(g.targets[li]);
            let from = if li == 0 { [0; 5] } else { g.ends[li - 1] };
            let to = g.ends[li];
            start.events.extend_from_slice(&g.events[from[0]..to[0]]);
            dead.extend(g.dead[from[1]..to[1]].iter().cloned());
            corpses.extend(g.corpses[from[2]..to[2]].iter().cloned());
            gone.extend(g.gone[from[3]..to[3]].iter().cloned());
            guest_events.extend(g.guest_events[from[4]..to[4]].iter().cloned());
        }
        let mut pushes = horizon_client::mobs::PendingPushes::default();
        let mut deferred = Vec::new();
        for g in &mut groups {
            deferred.append(&mut g.deferred);
            // Each member's binding and death mark as its group left them.
            for m in g.mobs.iter().map(MobSlot::mob) {
                match g.bindings.get(&m.id) {
                    Some(&conn) => start.bindings.insert(m.id, conn),
                    None => start.bindings.remove(&m.id),
                };
                if g.died.contains(&m.id) {
                    start.died.insert(m.id);
                } else {
                    start.died.remove(&m.id);
                }
            }
            start.ticked.extend(g.ticked.iter().copied());
            pushes.append(std::mem::take(&mut g.pushes));
            // A later turn can also update an earlier member's snapshot (brain events), so
            // merge the final group entries rather than only its turn-time publications.
            for (&at, seen) in g.global.iter().zip(&g.snapshot) {
                board.frozen[at] = seen.clone();
            }
        }
        drop(groups);
        // The mods hear the kept hooks as the serial loop told them: by turn, each turn's in order.
        deferred.sort_by_key(|d| d.0);
        for (_, d) in deferred {
            horizon_client::native_client_hooks::replay(d);
        }
        let Board { frozen, .. } = board;
        let mut merged = Group::new(
            mobs,
            Vec::new(),
            Start {
                random: draws.finish(),
                targets,
                ..start
            },
        );
        (merged.keep, merged.pushes, merged.snapshot) = (keep, pushes, frozen);
        (merged.dead, merged.corpses, merged.gone) = (dead, corpses, gone);
        merged.guest_events = guest_events;
        if let Some((serial, seeds)) = serial {
            let diff = difference(&serial, &merged)
                .or_else(|| (!seeds.complete()).then(|| "new RNG allocation counts differ".into()));
            report_check(diff, lanes);
        }
        merged
    }

    /// Run `groups` at once, one thread each (the first on this one), each with this thread's
    /// level, `tnt_explodes` rule and tick profiler sampling. Their world is `turns.base` without
    /// its hooks into the game, which can't leave this thread: native mod effects (no mob carries
    /// a content mod's effect while mods are loaded) and the UUID lookup ([`LoopLookup`], the same
    /// answers). The helpers' sampled costs join this thread's sums.
    fn run_threads(
        &self,
        groups: &mut [Group<&mut Mob>],
        board: &Board,
        turns: &Turns,
        shared: &Shared,
        seeds: Option<&Arc<horizon_client::random_source::SeedReplay>>,
    ) {
        let inherited = horizon_client::random_source::seed_replay();
        let seeds = seeds.or(inherited.as_ref());
        let level = horizon_client::block_contact::current_level();
        let tnt = horizon_client::mobs::sulfur_cube::tnt_explodes();
        let rebuild = super::prep::hookless(turns.base);
        let (nearest, single_player, difficulty) =
            (turns.nearest, turns.single_player, turns.difficulty);
        let lookup = LoopLookup(self.uuid);
        let (defers, canceled) = (defers(), turns.canceled);
        let sampling = super::super::tick_profile::sampling();
        let run = |g: &mut Group<&mut Mob>, ticking: Box<dyn Fn(i32, i32) -> bool + Send>| {
            let foreign = Foreign {
                board,
                reader: Cell::new(0),
                defers,
            };
            let mut base = rebuild();
            base.who_lookup = Some(&lookup);
            // The despawn checks look up no mob, as the serial loop's.
            let despawn = base.despawn_view();
            base.foreign = Some(&foreign);
            let mut turns = Turns {
                base: &base,
                despawn,
                ticking: &*ticking,
                nearest,
                single_player,
                canceled: &|id| canceled(id),
                difficulty,
                cached_difficulty: None,
            };
            if let Some(seeds) = seeds {
                horizon_client::random_source::replay_seeds(Arc::clone(seeds), || {
                    g.run(&mut turns, shared, Some(&foreign));
                });
            } else {
                g.run(&mut turns, shared, Some(&foreign));
            }
        };
        let tickings: Vec<_> = groups.iter().map(|_| (self.ticking)()).collect();
        let mut parts = groups.iter_mut().zip(tickings);
        let (first, first_ticking) = parts.next().expect("a plan has groups");
        #[cfg(test)]
        let failure = FAIL_THREADS.with(|f| f.borrow().clone());
        std::thread::scope(|s| {
            // Guards for every not-yet-started lane live inside the scope's callback. If the
            // OS refuses a spawn, they drop before scope joins any started, waiting threads.
            let first_done = first.random.lane_done();
            let pending: Vec<_> = parts
                .map(|(g, ticking)| {
                    let done = g.random.lane_done();
                    (g, ticking, done)
                })
                .collect();
            let mut handles = Vec::with_capacity(pending.len());
            for (g, ticking, done) in pending {
                let run = &run;
                #[cfg(test)]
                let failure = failure.clone();
                #[cfg(test)]
                if let Some(failure) = &failure
                    && failure.at == FailAt::Spawn
                    && g.global[0] == 2
                {
                    // The first worker has reached its RNG request while the server lane and
                    // this later lane have not started. Fail at the OS-spawn error boundary.
                    failure.started.wait();
                    panic!("injected spawn failure at the OS-spawn error boundary");
                }
                let handle = std::thread::Builder::new()
                    .name("mob-island".into())
                    .spawn_scoped(s, move || {
                        let _done = done;
                        #[cfg(test)]
                        if let Some(failure) = &failure
                            && g.global[0] == 1
                        {
                            match failure.at {
                                FailAt::Setup => panic!("injected worker setup failure"),
                                FailAt::Turn => FAIL_TURN.with(|f| f.set(Some(1))),
                                FailAt::Spawn => {
                                    use horizon_core::random::RandomSource;
                                    g.random.begin_item(1);
                                    failure.started.wait();
                                    g.random.next_long();
                                    g.random.end_item();
                                }
                            }
                        }
                        horizon_client::mobs::sulfur_cube::set_tnt_explodes(tnt);
                        super::super::tick_profile::begin_helper(sampling);
                        horizon_client::block_contact::in_level(level, || run(g, ticking));
                        super::super::tick_profile::end_helper()
                    })
                    .expect("a mob island's thread starts");
                handles.push(handle);
            }
            run(first, first_ticking);
            drop(first_done);
            let mut panic = None;
            for handle in handles {
                match handle.join() {
                    Ok(costs) => super::super::tick_profile::merge(costs),
                    Err(payload) if panic.is_none() => panic = Some(payload),
                    Err(_) => {}
                }
            }
            if let Some(payload) = panic {
                std::panic::resume_unwind(payload);
            }
        });
    }
}

/// What one mob can reach this tick, for the plan.
#[derive(Clone, Debug)]
struct Reach {
    /// Its feet's column.
    at: (f64, f64),
    /// Whether it ticks as the loop starts: its island's weight.
    ticking: bool,
    /// Its body, grown by the most it can move.
    body: Aabb,
    /// What its tick reads around itself.
    sense: Aabb,
    /// The villagers' points of interest its tick reads.
    village_reads: Option<Aabb>,
    /// A villager's claims, and the ones it may make this tick.
    claims: Option<Aabb>,
    id: u32,
    /// The mobs it can reach by id at any distance.
    links: Vec<u32>,
    /// Its raid: a raid's raiders order their hurts across the raid (`raider::next_hurt_seq`).
    raid: Option<i32>,
    /// Where a teleport can take it far from `body` (a pet to its owner), and what it reads there.
    away: Option<Away>,
}

/// A mob's reach at a place it can teleport to, apart from where it stands: planned as one more
/// mob of its island standing there.
#[derive(Clone, Debug)]
struct Away {
    at: (f64, f64),
    body: Aabb,
    sense: Aabb,
    village_reads: Option<Aabb>,
}

/// The farthest a mob of `species` can be moved in one tick besides its motion: a teleport.
fn jump(species: Species) -> f64 {
    match species {
        // Enderman.teleport: up to 32 blocks on each axis.
        Species::Enderman => 33.0,
        // Shulker.teleportSomewhere: within 8.
        Species::Shulker => 9.0,
        _ => 0.0,
    }
}
/// The widest `TeleportRandomly` diameter of the items `m` holds in either hand (a used item is
/// always one of them), whatever its species: a mob that finishes eating one (`Fox.aiStep` today)
/// lands within half of it on each axis, then drops to the ground. None when it holds none; a
/// diameter that isn't finite is returned as it is, and keeps the tick serial.
fn teleport_consumable(m: &Mob, items: &horizon_client::items::Items) -> Option<f64> {
    [&m.extra.main_hand, &m.extra.off_hand]
        .into_iter()
        .flatten()
        .filter_map(|id| items.items.get(&**id)?.consumable.as_ref())
        .flat_map(|c| &c.effects)
        .filter_map(|e| match e {
            horizon_client::items::ConsumeEffect::TeleportRandomly(t) => {
                Some(f64::from(t.diameter).abs())
            }
            _ => None,
        })
        .reduce(|a, b| if a.is_nan() || a > b { a } else { b })
}
/// Mobs whose tick reads every villager's points of interest (`village_sections`,
/// `others_tickets`).
fn reads_villages(species: Species) -> bool {
    matches!(
        species,
        Species::Villager
            | Species::IronGolem
            | Species::Fox
            | Species::Zombie
            | Species::ZombieVillager
            | Species::Husk
            | Species::Drowned
            | Species::ZombifiedPiglin
    )
}
fn grow(b: Aabb, by: f64) -> Aabb {
    b.inflate(Vec3::new(by, by, by))
}
fn point_box(p: [i32; 3]) -> Aabb {
    let v = Vec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
    Aabb::new(v, Vec3::new(v.x + 1.0, v.y + 1.0, v.z + 1.0))
}
fn union_box(a: Aabb, b: Aabb) -> Aabb {
    let axes = [Axis::X, Axis::Y, Axis::Z];
    let [x0, y0, z0] = axes.map(|axis| a.min(axis).min(b.min(axis)));
    let [x1, y1, z1] = axes.map(|axis| a.max(axis).max(b.max(axis)));
    Aabb::new(Vec3::new(x0, y0, z0), Vec3::new(x1, y1, z1))
}

/// The largest `speedModifier` a vanilla goal or behaviour hands the move control: a husk's spear
/// charge on a camel husk (`SpearUseGoal`, 4.0); the others are at most 3.0 (a goat's ram). The
/// audit is in `docs/mob-islands.md`.
const MAX_SPEED_MODIFIER: f64 = 4.0;

/// Whether `m` moves as its type does: its own and its effective movement speed, jump strength and
/// flying speed are its type's, and no effect changes them. The `2 * |v| + SLACK` reach is set for
/// those; [`locomotion`] bounds the others.
fn default_locomotion(m: &Mob) -> bool {
    use horizon_client::mobs::finalize::{Attr, value};
    m.attrs.speed == m.kind.speed
        && m.movement_speed() == m.kind.speed
        && m.attrs.jump_strength == m.kind.jump_strength
        && value(m, Attr::FlyingSpeed) == m.kind.flying_speed
        && !m.extra.effects.iter().any(|(effect, ..)| {
            matches!(
                *effect,
                "minecraft:speed"
                    | "minecraft:slowness"
                    | "minecraft:jump_boost"
                    | "minecraft:dolphins_grace"
            )
        })
}

/// The most `m`'s own locomotion can add to how far it moves in one turn, when it isn't its type's
/// ([`default_locomotion`]: 0). Derived in `docs/mob-islands.md`: every `moveRelative` adds at most
/// its speed argument to the velocity, which is the move control's `speed` (a goal's speed
/// modifier times the movement or flying speed attribute), the type's air speed or a constant of
/// at most 0.1; a jump sets the vertical velocity to the jump power; a ridden mob's jump or dash
/// adds its push. Each speed is taken unslowed and at its highest for the turn, since an effect
/// can end, or a witch drink Swiftness, before the mob moves. None when a value isn't finite.
fn locomotion(m: &Mob) -> Option<f64> {
    use horizon_client::mobs::finalize::{Attr, value};
    if default_locomotion(m) {
        return Some(0.0);
    }
    let effects = || {
        m.extra
            .effects
            .iter()
            .map(|&(effect, _, amp)| (effect, amp))
    };
    // Speed: +20% a level (an amplifier below 0 slows, and is left out).
    let swift = effects()
        .filter(|e| e.0 == "minecraft:speed")
        .map(|e| (1.0 + 0.2 * f64::from(e.1.saturating_add(1))).max(1.0))
        .product::<f64>()
        * if m.species() == Species::Witch {
            1.2
        } else {
            1.0
        };
    // `Mob.movement_speed` without what only slows it (Slowness, a drinking witch, a cold
    // strider), with an enderman's anger and sprinting, which can start in the turn.
    let walk = (m.attrs.speed.abs().max(m.kind.speed.abs())
        + if m.species() == Species::Enderman {
            f64::from(0.15f32)
        } else {
            0.0
        })
        * m.speed_bonus().abs().max(1.0)
        * (1.0 + f64::from(0.3f32))
        * swift;
    let walk = walk.max(value(m, Attr::MovementSpeed).abs());
    let fly = value(m, Attr::FlyingSpeed)
        .abs()
        .max(m.kind.flying_speed.abs());
    let modifier = MAX_SPEED_MODIFIER
        .max(m.ai.mov.speed_modifier.abs())
        .max(m.ai.nav.speed_modifier.abs());
    // The move control's speed this turn, or the one it left last turn.
    let speed = (modifier * walk.max(fly)).max(f64::from(m.ai.speed).abs());
    // One `moveRelative` of travel, and one more of a jump (a rabbit's hop) or a swimmer's climb.
    let push = speed.max(f64::from(m.kind.air_speed).abs()).max(0.1);
    // A ridden camel's dash: 22.2222 times its movement speed along its look, and 0.6 up.
    let dash = if matches!(m.species(), Species::Camel | Species::CamelHusk) {
        22.2222 * walk + 0.6
    } else {
        0.0
    };
    let boost = effects()
        .filter(|e| e.0 == "minecraft:jump_boost")
        .map(|e| 0.1 * f64::from(e.1.saturating_add(1)))
        .fold(0.0, f64::max);
    let jump = value(m, Attr::JumpStrength)
        .abs()
        .max(m.attrs.jump_strength.abs())
        .max(m.kind.jump_strength.abs())
        + boost;
    // A ridden horse's jump pushes 0.4 forward, a sprinting jump 0.2.
    let reach = 2.0 * push + dash + 0.4 + 0.2 + jump;
    reach.is_finite().then_some(reach)
}

/// Where a teleport to its owner can put a pet (`tame::teleport_to_owner`): a block within 3 of the
/// owner's on each horizontal axis and 1 vertically, the pet's feet at its bottom centre. `owners`
/// are the players a pet can see this tick that the owner can be (`tame::owned_by`).
fn owner_destinations(m: &Mob, owners: &[([i32; 4], Vec3)]) -> Option<Aabb> {
    m.extra.owner?;
    let (half, height) = (f64::from(m.width()) / 2.0, f64::from(m.height()));
    owners
        .iter()
        .filter(|p| horizon_client::mobs::tame::owned_by(m.extra.owner, p.0))
        .map(|&(_, at)| {
            let (x, y, z) = (at.x.floor(), at.y.floor(), at.z.floor());
            Aabb::new(
                Vec3::new(x - 3.0 + 0.5 - half, y - 1.0, z - 3.0 + 0.5 - half),
                Vec3::new(x + 3.0 + 0.5 + half, y + 1.0 + height, z + 3.0 + 0.5 + half),
            )
        })
        .reduce(union_box)
}

/// The mob ids a warden's state keeps, whatever holds them: its anger's suspects, its brain's
/// memories (attack and roar targets, the nearest attackable, the mobs it sees, its look target),
/// the attackers its hurt noted and the vibration it waits on. It reads and angers them by id at
/// any distance. Read from the state's `Debug` form so that a field added later is covered too:
/// an id that isn't a mob of the list joins nothing.
fn warden_links(m: &Mob, links: &mut Vec<u32>) {
    let Some(state) = &m.extra.warden else {
        return;
    };
    let text = format!("{state:?}");
    let mut rest = text.as_str();
    while let Some(at) = rest.find("Mob(") {
        rest = &rest[at + 4..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if let Ok(id) = rest[..digits].parse() {
            links.push(id);
        }
    }
}

/// Each mob's reach; None when a value is not finite or the lists don't match. Changes to movement,
/// reads or cross-mob writes require a conservative reach and a behavioral regression; verify is
/// no safety net. `owners`: each player a mob can see this tick, by UUID, where it stands.
fn reaches(
    mobs: &[Mob],
    snapshot: &[Seen],
    turns: &Turns,
    owners: &[([i32; 4], Vec3)],
) -> Option<Vec<Reach>> {
    if mobs.len() != snapshot.len() {
        return None;
    }
    let mut out = Vec::with_capacity(mobs.len());
    for (m, s) in mobs.iter().zip(snapshot) {
        if !(m.position.x.is_finite() && m.position.y.is_finite() && m.position.z.is_finite())
            || !(m.velocity.x.is_finite() && m.velocity.y.is_finite() && m.velocity.z.is_finite())
        {
            return None;
        }
        let (bx, bz) = (m.position.x.floor() as i32, m.position.z.floor() as i32);
        let v = m.velocity;
        // A meal that teleports: half its diameter on each axis (and the block it rounds to).
        let meal = match turns
            .base
            .items
            .and_then(|items| teleport_consumable(m, items))
        {
            Some(d) if !d.is_finite() => return None,
            Some(d) => Some(d / 2.0 + 1.0),
            None => None,
        };
        let motion = 2.0 * (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
            + jump(s.species)
            + meal.unwrap_or(0.0)
            + locomotion(m)?
            + SLACK;
        let mut body = grow(s.bounds, motion);
        // ...then the drop to the ground below the landing, as deep as the world goes.
        if meal.is_some() {
            let floor = f64::from(i32::MIN);
            body = union_box(
                body,
                Aabb::new(
                    Vec3::new(body.min(Axis::X), floor, body.min(Axis::Z)),
                    Vec3::new(body.max(Axis::X), floor + 1.0, body.max(Axis::Z)),
                ),
            );
        }
        let sense_range = m.attrs.follow_range.max(MIN_SENSE) + SLACK;
        // A pet that teleports to its owner moves on from there in the same turn.
        let away = owner_destinations(m, owners).map(|near_owner| {
            let body = grow(near_owner, motion);
            let centre = |axis| (body.min(axis) + body.max(axis)) / 2.0;
            Away {
                at: (centre(Axis::X), centre(Axis::Z)),
                body,
                sense: grow(body, sense_range),
                village_reads: reads_villages(s.species).then(|| grow(body, VILLAGE_SENSE)),
            }
        });
        let claims = (s.species == Species::Villager).then(|| {
            [s.job, s.bed, s.potential_job, s.meeting]
                .into_iter()
                .flatten()
                .fold(grow(body, POI_CLAIM), |b, p| union_box(b, point_box(p)))
        });
        let mut links: Vec<u32> = [
            match s.target {
                Some(Who::Mob(id)) => Some(id),
                _ => None,
            },
            match s.hurt_by {
                Some((Who::Mob(id), _)) => Some(id),
                _ => None,
            },
            match s.leash {
                Some(leash::Holder::Mob(id)) => Some(id),
                _ => None,
            },
            s.vehicle,
            m.extra.mount,
            m.extra.rider,
            (s.caravan > 0).then(|| s.caravan - 1),
            s.school.0,
        ]
        .into_iter()
        .flatten()
        .collect();
        match s.species {
            // A vex's owner, an evoker's or illusioner's vex.
            Species::Vex | Species::Evoker | Species::Illusioner if m.extra.counter2 > 0 => {
                links.push(m.extra.counter2 as u32);
            }
            // The side heads' targets, found by id in the level's mobs (`wither::target_pos`).
            // A new one is picked within 20 blocks, inside its reads.
            Species::Wither => links.extend(
                m.extra.ints[..2]
                    .iter()
                    .filter(|&&id| id > 0)
                    .map(|&id| id as u32),
            ),
            Species::Warden => warden_links(m, &mut links),
            _ => {}
        }
        out.push(Reach {
            at: (m.position.x, m.position.z),
            ticking: turns.base.world.loaded(bx, bz) && (turns.ticking)(bx, bz),
            body,
            sense: grow(body, sense_range),
            village_reads: reads_villages(s.species).then(|| grow(body, VILLAGE_SENSE)),
            claims,
            id: s.id,
            links,
            raid: horizon_client::mobs::raider::state(m).and_then(|r| r.raid_id),
            away,
        });
    }
    Some(out)
}

/// Union-find over the mobs' places.
struct Sets {
    parent: Vec<u32>,
}
impl Sets {
    fn find(&mut self, mut i: u32) -> u32 {
        while self.parent[i as usize] != i {
            let up = self.parent[self.parent[i as usize] as usize];
            self.parent[i as usize] = up;
            i = up;
        }
        i
    }
    fn join(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        // The smaller root wins, so islands keep a stable name.
        let (lo, hi) = (a.min(b), a.max(b));
        self.parent[hi as usize] = lo;
    }
}

/// The cells of `CELL` blocks a box covers, as (x0, z0, x1, z1).
fn cells(b: Aabb) -> Option<(i32, i32, i32, i32)> {
    let (x0, x1, z0, z1) = (
        b.min(Axis::X),
        b.max(Axis::X),
        b.min(Axis::Z),
        b.max(Axis::Z),
    );
    if !(x0.is_finite() && x1.is_finite() && z0.is_finite() && z1.is_finite()) {
        return None;
    }
    let c = |v: f64| (v / CELL).floor() as i32;
    Some((c(x0), c(z0), c(x1), c(z1)))
}

/// Boxes by the cells they cover, each visit paid from a budget.
#[derive(Default)]
struct Cells {
    map: FxHashMap<(i32, i32), Vec<u32>>,
}
impl Cells {
    fn visit(b: Aabb, budget: &mut usize, mut f: impl FnMut((i32, i32))) -> Option<()> {
        let (x0, z0, x1, z1) = cells(b)?;
        let n =
            (i64::from(x1) - i64::from(x0) + 1).checked_mul(i64::from(z1) - i64::from(z0) + 1)?;
        *budget = budget.checked_sub(usize::try_from(n).ok()?)?;
        for x in x0..=x1 {
            for z in z0..=z1 {
                f((x, z));
            }
        }
        Some(())
    }
    fn insert(&mut self, i: u32, b: Aabb, budget: &mut usize) -> Option<()> {
        Self::visit(b, budget, |cell| self.map.entry(cell).or_default().push(i))
    }
    fn query(&self, b: Aabb, budget: &mut usize, mut f: impl FnMut(u32)) -> Option<()> {
        let mut found = 0usize;
        Self::visit(b, budget, |cell| {
            if let Some(list) = self.map.get(&cell) {
                found += list.len();
                list.iter().copied().for_each(&mut f);
            }
        })?;
        *budget = budget.checked_sub(found)?;
        Some(())
    }
}

/// The islands of `reaches`, packed into at most `lanes` groups; None when the loop should run
/// serially: too few ticking mobs, a plan past its budget, one island holding most of the ticking
/// mobs, or only one group.
fn islands(reaches: &[Reach], lanes: usize) -> Option<Plan> {
    let ticking = reaches.iter().filter(|r| r.ticking).count();
    if lanes < 2 || ticking < MIN_TICKING {
        return None;
    }
    let n = reaches.len();
    let mut budget = PLAN_BUDGET;
    let mut sets = Sets {
        parent: (0..n as u32).collect(),
    };
    // Joining two mobs that can't reach each other only costs parallelism, never the result, so
    // the plan works in columns of CELL blocks: every mob of a column is one island, and a column
    // joins every column its mobs' reaches meet. A column holds the box around its mobs' bodies,
    // around their reads, around its villagers' points of interest and around the village reads of
    // the mobs that make them.
    #[derive(Clone, Copy)]
    struct Column {
        first: u32,
        body: Aabb,
        sense: Aabb,
        claims: Option<Aabb>,
        village_reads: Option<Aabb>,
    }
    let widen = |a: Option<Aabb>, b: Option<Aabb>| match (a, b) {
        (Some(a), Some(b)) => Some(union_box(a, b)),
        (a, b) => a.or(b),
    };
    let mut columns: Vec<Column> = Vec::new();
    let mut column_of = FxHashMap::<(i32, i32), u32>::default();
    let mut index = FxHashMap::<u32, u32>::default();
    for (i, r) in reaches.iter().enumerate() {
        index.entry(r.id).or_insert(i as u32);
        // Where it stands, then where a teleport can take it, each in its own column.
        let away = r
            .away
            .iter()
            .map(|a| (a.at, a.body, a.sense, None, a.village_reads));
        let parts = std::iter::once((r.at, r.body, r.sense, r.claims, r.village_reads));
        for (at, body, sense, claims, village_reads) in parts.chain(away) {
            let key = ((at.0 / CELL).floor() as i32, (at.1 / CELL).floor() as i32);
            let next = columns.len() as u32;
            let c = *column_of.entry(key).or_insert(next);
            if c == next {
                columns.push(Column {
                    first: i as u32,
                    body,
                    sense,
                    claims,
                    village_reads,
                });
                continue;
            }
            sets.join(columns[c as usize].first, i as u32);
            let col = &mut columns[c as usize];
            col.body = union_box(col.body, body);
            col.sense = union_box(col.sense, sense);
            col.claims = widen(col.claims, claims);
            col.village_reads = widen(col.village_reads, village_reads);
        }
    }
    // Columns by the cells their boxes cover.
    let (mut bodies, mut claims) = (Cells::default(), Cells::default());
    for (c, col) in columns.iter().enumerate() {
        bodies.insert(c as u32, col.body, &mut budget)?;
        if let Some(b) = col.claims {
            claims.insert(c as u32, b, &mut budget)?;
        }
    }
    for col in &columns {
        bodies.query(col.sense, &mut budget, |j| {
            let other = &columns[j as usize];
            if other.body.intersects(col.sense) {
                sets.join(col.first, other.first);
            }
        })?;
        if let Some(reads) = col.village_reads {
            claims.query(reads, &mut budget, |j| {
                let other = &columns[j as usize];
                if other.claims.is_some_and(|b| b.intersects(reads)) {
                    sets.join(col.first, other.first);
                }
            })?;
        }
    }
    let mut raids = FxHashMap::<i32, u32>::default();
    for (i, r) in reaches.iter().enumerate() {
        for id in &r.links {
            if let Some(&j) = index.get(id) {
                sets.join(i as u32, j);
            }
        }
        if let Some(raid) = r.raid {
            let first = *raids.entry(raid).or_insert(i as u32);
            sets.join(first, i as u32);
        }
    }
    // Islands, in the order of their first mob; their ticking mobs are their weight.
    let mut islands: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut slot = FxHashMap::<u32, usize>::default();
    for (i, r) in reaches.iter().enumerate() {
        let root = sets.find(i as u32);
        let k = *slot.entry(root).or_insert_with(|| {
            islands.push((0, Vec::new()));
            islands.len() - 1
        });
        islands[k].1.push(i);
        islands[k].0 += usize::from(r.ticking);
    }
    let largest = islands.iter().map(|i| i.0).max().unwrap_or(0);
    if largest * 4 > ticking * 3 {
        return None;
    }
    // Heaviest first (stable: ties keep the list's order), each to the lightest group.
    islands.sort_by_key(|a| std::cmp::Reverse(a.0));
    let mut groups: Vec<(usize, Vec<usize>)> = vec![(0, Vec::new()); lanes];
    for (weight, members) in islands {
        let g = groups.iter_mut().min_by_key(|g| g.0)?;
        g.0 += weight;
        g.1.extend(members);
    }
    let mut groups: Vec<Vec<usize>> = groups
        .into_iter()
        .filter(|g| !g.1.is_empty())
        .map(|g| g.1)
        .collect();
    for g in &mut groups {
        g.sort_unstable();
    }
    (groups.len() > 1).then_some(Plan { groups })
}

/// Every mob's snapshot entry across the groups, for a lookup by id of a mob in another group:
/// the serial loop shows a mob after its turn to the mobs after it, and as it was before the tick
/// to the mobs before it.
struct Board {
    /// The snapshot as the tick started.
    frozen: Vec<Seen>,
    /// Each mob's entry once its turn is over.
    published: Vec<OnceLock<Seen>>,
    /// The first slot of each id, as the serial snapshot's index.
    index: FxHashMap<u32, u32>,
    /// Each mob's group.
    lane_of: Vec<u32>,
    lanes: usize,
    draws: Arc<OrderedDraws>,
}
impl Board {
    fn new(frozen: Vec<Seen>, groups: &[Group<&mut Mob>], draws: Arc<OrderedDraws>) -> Self {
        let mut index = FxHashMap::default();
        for (i, s) in frozen.iter().enumerate() {
            index.entry(s.id).or_insert(i as u32);
        }
        let mut lane_of = vec![0; frozen.len()];
        for (lane, g) in groups.iter().enumerate() {
            for &k in &g.global {
                lane_of[k] = lane as u32;
            }
        }
        Self {
            published: (0..frozen.len()).map(|_| OnceLock::new()).collect(),
            frozen,
            index,
            lane_of,
            lanes: groups.len(),
            draws,
        }
    }
}

/// A group's view of the [`Board`] at the turn of mob `reader` (its place in the list).
pub(super) struct Foreign<'b> {
    board: &'b Board,
    pub reader: Cell<usize>,
    /// Whether the turns keep their mod hooks for the join ([`defers`]).
    pub defers: bool,
}
impl Foreign<'_> {
    /// A turn's new RNGs, including projectile RNGs and long-jump angle shuffles, follow list
    /// order too. Waiting here uses the same dependency direction as the level stream.
    pub(super) fn in_seed_order<R>(&self, at: usize, f: impl FnOnce() -> R) -> R {
        horizon_client::random_source::in_seed_order(
            Arc::clone(&self.board.draws),
            self.board.lane_of[at] as usize,
            at,
            self.board.lanes,
            f,
        )
    }

    /// Mob `at`'s entry as its turn left it.
    pub(super) fn publish(&self, at: usize, seen: &Seen) {
        let _ = self.board.published[at].set(seen.clone());
    }
}
impl ForeignSeen for Foreign<'_> {
    fn seen(&self, id: u32) -> Option<&Seen> {
        let board = self.board;
        let j = *board.index.get(&id)? as usize;
        let s = if j > self.reader.get() {
            &board.frozen[j]
        } else {
            board.draws.wait_past(board.lane_of[j] as usize, j);
            board.published[j].get()?
        };
        (s.id == id).then_some(s)
    }
}

/// `Game::who_by_uuid` while the mob loop runs: the game's mob list is taken for the loop, so only
/// the host's own player answers.
struct LoopLookup(Option<[i32; 4]>);
impl WhoLookup for LoopLookup {
    fn who_by_uuid(&self, uuid: [i32; 4]) -> Option<Who> {
        (self.0 == Some(uuid)).then_some(Who::Player)
    }
}

/// `m` without the numbers it takes from a process-wide counter, which the copies and the groups
/// both take from: only their order among the mobs that compare them counts, and those share a
/// group (`section_seq`: the mobs of one section, `aquatic.rs`; a raider's `hurt_seq`: its raid,
/// `raids.rs`).
fn without_counters(m: &Mob) -> Mob {
    let mut m = m.clone();
    m.extra.section_seq = 0;
    if let Some(r) = m.extra.raider.as_deref_mut() {
        r.hurt_seq = 0;
    }
    m
}

/// The first difference between the serial loop's result and the groups', if any.
fn difference(serial: &Group<Mob>, grouped: &Group<Mob>) -> Option<String> {
    if serial.mobs.len() != grouped.mobs.len() {
        return Some(format!(
            "{} mobs serially, {} in groups",
            serial.mobs.len(),
            grouped.mobs.len()
        ));
    }
    let plain = |m: &Mob| format!("{:?}", without_counters(m));
    for (ma, mb) in serial.mobs.iter().zip(&grouped.mobs) {
        let (a, b) = (plain(ma), plain(mb));
        if a != b {
            let at = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
            let (from, to) = (at.saturating_sub(120), |s: &str| (at + 120).min(s.len()));
            return Some(format!(
                "mob {} ({}) at {:?} differs: serial ...{}... grouped ...{}...",
                ma.id,
                ma.kind.id,
                ma.position,
                a.get(from..to(&a)).unwrap_or_default(),
                b.get(from..to(&b)).unwrap_or_default(),
            ));
        }
    }
    let debug = |g: &Group<Mob>| {
        format!(
            "{:?} {:?} {:?} {:?} {:?} {:?}",
            g.events, g.dead, g.corpses, g.gone, g.guest_events, g.targets,
        )
    };
    let pushed = |g: &Group<Mob>| {
        let mut pushes = g.pushes.clone();
        g.mobs
            .iter()
            .zip(&g.keep)
            .filter(|(_, keep)| **keep)
            .map(|(m, _)| {
                let mut m = m.clone();
                horizon_client::mobs::apply_mob_pushes(&mut m, &mut pushes);
                plain(&m)
            })
            .collect::<Vec<_>>()
    };
    let snapshot = |g: &Group<Mob>| {
        g.snapshot
            .iter()
            .map(|seen| {
                let mut seen = seen.clone();
                // The same process-wide section ordering token excluded from Mob above.
                seen.section.1 = 0;
                format!("{seen:?}")
            })
            .collect::<Vec<_>>()
    };
    let stream = |g: &Group<Mob>| {
        use horizon_core::random::RandomSource;
        let mut random = g.random.clone();
        (
            random.state(),
            random.next_gaussian().to_bits(),
            random.next_long(),
        )
    };
    [
        (serial.keep != grouped.keep, "the kept mobs"),
        (
            debug(serial) != debug(grouped),
            "the events, deaths or targets",
        ),
        (stream(serial) != stream(grouped), "the level random"),
        (
            serial.died != grouped.died || serial.bindings != grouped.bindings,
            "the death marks or player bindings",
        ),
        (serial.ticked != grouped.ticked, "the ticked mobs"),
        (
            snapshot(serial) != snapshot(grouped),
            "the snapshot entries",
        ),
        (
            pushed(serial) != pushed(grouped),
            "the pending pushes' results",
        ),
    ]
    .into_iter()
    .find(|d| d.0)
    .map(|d| format!("{} differ", d.1))
}

/// `HORIZON_PARALLEL_TICK=verify`: log a grouped loop that differs from the serial one, and how
/// many were checked, every 100.
fn report_check(difference: Option<String>, groups: usize) {
    #[cfg(test)]
    CHECK.with(|c| {
        let (checked, differed) = c.get();
        c.set((checked + 1, differed + usize::from(difference.is_some())));
    });
    use std::sync::atomic::{AtomicU64, Ordering};
    static CHECKED: AtomicU64 = AtomicU64::new(0);
    static DIFFERED: AtomicU64 = AtomicU64::new(0);
    if let Some(diff) = difference {
        DIFFERED.fetch_add(1, Ordering::Relaxed);
        log::error!("mob islands: a tick in {groups} groups differs from the serial loop: {diff}");
    }
    let n = CHECKED.fetch_add(1, Ordering::Relaxed) + 1;
    if n == 1 || n.is_multiple_of(100) {
        log::info!(
            "mob islands: {n} grouped ticks checked against the serial loop, {} differed",
            DIFFERED.load(Ordering::Relaxed)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_workers_never_exceed_available_hardware() {
        assert_eq!(capped_lanes(64, Some(3)), 3);
        assert_eq!(capped_lanes(64, Some(1)), 1);
        assert_eq!(capped_lanes(64, None), 1);
        assert_eq!(capped_lanes(2, Some(64)), 2);
        assert_eq!(capped_lanes(0, Some(64)), 1);
        assert_eq!(capped_lanes(64, Some(64)), 8);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn startup_and_turn_failures_release_lanes_and_preserve_payload() {
        let (sent, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (at, expected) in [
                (
                    FailAt::Spawn,
                    "injected spawn failure at the OS-spawn error boundary",
                ),
                (FailAt::Setup, "injected worker setup failure"),
                (FailAt::Turn, "injected mob turn failure"),
            ] {
                let mut game = super::super::super::native_machines::tests::server_game();
                for x in [0.5, 1.5, 2.5] {
                    game.spawn_mob("minecraft:cow", [x, 65.0, 0.5], true)
                        .unwrap();
                }
                let mobs: Vec<Mob> = std::mem::take(&mut game.mobs).into();
                let snapshot = mobs.iter().map(|m| m.seen(None)).collect();
                let base = game.mob_world_base(&[]);
                let players = super::super::super::natural_spawning::NearestPlayers::new(&[]);
                let nearest = super::super::prep::MobsNearest::new(&players, &mobs, false);
                let mut turns = Turns {
                    base: &base,
                    despawn: base.despawn_view(),
                    ticking: &|_, _| true,
                    nearest: &nearest,
                    single_player: Some(None),
                    canceled: &|_| false,
                    difficulty: &|_| {
                        horizon_client::mobs::spawning::LocalDifficulty::new(2, 0, 0, 0.0)
                    },
                    cached_difficulty: None,
                };
                let start = Start {
                    events: Vec::new(),
                    random: horizon_core::random::LegacyRandom::new(7),
                    guests: Vec::new(),
                    player: None,
                    bindings: Default::default(),
                    died: Default::default(),
                    ticked: Default::default(),
                    targets: vec![None; 3],
                };
                let shared = Shared {
                    uuid: None,
                    level_id: "minecraft:overworld",
                    path_changes: &[],
                    guest_ids: &[],
                };
                FAIL_THREADS.with(|f| {
                    *f.borrow_mut() = Some(FailThreads {
                        at,
                        started: Arc::new(std::sync::Barrier::new(2)),
                    });
                });
                let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    Islands {
                        uuid: None,
                        ticking: &|| Box::new(|_, _| true),
                    }
                    .run(
                        Plan {
                            groups: vec![vec![0], vec![1], vec![2]],
                        },
                        mobs.clone(),
                        snapshot,
                        start,
                        &mut turns,
                        &shared,
                    );
                }));
                FAIL_THREADS.with(|f| *f.borrow_mut() = None);
                let payload = caught.expect_err("injected failure reaches the server caller");
                let text = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
                assert_eq!(
                    text,
                    Some(expected),
                    "the original panic payload reaches the caller"
                );
            }
            sent.send(()).unwrap();
        });
        finished
            .recv_timeout(std::time::Duration::from_secs(120))
            .expect("started workers finish after spawn/setup failure, with no stranded lane");
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn verification_detects_snapshot_and_gaussian_only_differences() {
        use horizon_core::random::RandomSource;
        let mut game = super::super::super::native_machines::tests::server_game();
        game.spawn_mob("minecraft:cow", [0.5, 65.0, 0.5], true)
            .unwrap();
        let mobs = game.mobs.to_vec();
        let snapshot = mobs.iter().map(|m| m.seen(None)).collect::<Vec<_>>();
        let mut random = horizon_core::random::LegacyRandom::new(7);
        random.next_gaussian();
        let start = Start {
            events: Vec::new(),
            random,
            guests: Vec::new(),
            player: None,
            bindings: Default::default(),
            died: Default::default(),
            ticked: Default::default(),
            targets: vec![None],
        };
        let serial = Group::new(mobs.clone(), snapshot.clone(), start.clone());
        let mut grouped = Group::new(mobs, snapshot, start);
        assert_eq!(difference(&serial, &grouped), None);
        grouped.snapshot[0].position.x += 1.0;
        assert_eq!(
            difference(&serial, &grouped),
            Some("the snapshot entries differ".into())
        );
        grouped.snapshot[0] = serial.snapshot[0].clone();
        // Consuming the cached Gaussian changes no LCG bits, only the stream's cache.
        grouped.random.next_gaussian();
        assert_eq!(serial.random.state(), grouped.random.state());
        assert_eq!(
            difference(&serial, &grouped),
            Some("the level random differ".into())
        );
    }

    #[test]
    fn extreme_plan_boxes_fall_back_without_overflow_or_visits() {
        let b = Aabb::new(Vec3::new(-1e30, 0.0, -1e30), Vec3::new(1e30, 1.0, 1e30));
        let mut budget = PLAN_BUDGET;
        assert!(Cells::visit(b, &mut budget, |_| panic!("over budget")).is_none());
    }

    /// A mob with a one-block body at (x, z), reading 34 blocks around it (`MIN_SENSE` + `SLACK`).
    fn at(id: u32, x: f64, z: f64) -> Reach {
        let feet = Vec3::new(x, 64.0, z);
        let body = grow(Aabb::new(feet, Vec3::new(x + 1.0, 65.0, z + 1.0)), SLACK);
        Reach {
            at: (x, z),
            ticking: true,
            body,
            sense: grow(body, MIN_SENSE + SLACK),
            village_reads: None,
            claims: None,
            id,
            links: Vec::new(),
            raid: None,
            away: None,
        }
    }

    /// `count` mobs around (x, z), ids from `first`.
    fn herd(first: u32, count: u32, x: f64, z: f64) -> Vec<Reach> {
        (0..count)
            .map(|k| at(first + k, x + f64::from(k % 8), z + f64::from(k / 8)))
            .collect()
    }

    /// The group holding each mob of the list.
    fn group_of(plan: &Plan, n: usize) -> Vec<usize> {
        let mut of = vec![usize::MAX; n];
        for (g, members) in plan.groups.iter().enumerate() {
            for &k in members {
                assert_eq!(of[k], usize::MAX, "mob {k} is in two groups");
                of[k] = g;
            }
        }
        assert!(of.iter().all(|&g| g != usize::MAX), "a mob is in no group");
        of
    }

    /// Herds far apart, their mobs interleaved in the list: one group each, every group in list
    /// order.
    #[test]
    fn far_herds_go_to_their_own_groups_in_list_order() {
        let (a, b) = (herd(0, 200, 0.0, 0.0), herd(1000, 200, 1000.0, 0.0));
        let list: Vec<Reach> = a.into_iter().zip(b).flat_map(|(a, b)| [a, b]).collect();
        let plan = islands(&list, 2).expect("two herds split");
        let of = group_of(&plan, list.len());
        for (k, r) in list.iter().enumerate() {
            assert_eq!(of[k], of[(r.id >= 1000) as usize], "mob {}", r.id);
        }
        assert_ne!(of[0], of[1]);
        assert!(plan.groups.iter().all(|g| g.is_sorted()));
    }

    /// A herd whose reads meet another's bodies joins it; a herd out of reach stays apart, and
    /// so does one past the reach a mob's move and sense add up to.
    #[test]
    fn herds_within_reach_share_an_island() {
        let mut list = herd(0, 150, 0.0, 0.0);
        // 30 blocks past the first herd's edge: within its 34 blocks of sense.
        list.extend(herd(1000, 150, 38.0, 0.0));
        list.extend(herd(2000, 150, 500.0, 0.0));
        list.extend(herd(3000, 150, 1000.0, 0.0));
        let plan = islands(&list, 4).expect("three islands");
        let of = group_of(&plan, list.len());
        assert_eq!(of[0], of[150], "within reach");
        assert_ne!(of[0], of[300]);
        assert_ne!(of[300], of[450]);
        assert_eq!(plan.groups.len(), 3);
    }

    /// Mobs linked by id (a target, a leash, a raid...) share an island at any distance.
    #[test]
    fn links_join_mobs_at_any_distance() {
        let mut list = herd(0, 150, 0.0, 0.0);
        list.extend(herd(1000, 150, 1000.0, 0.0));
        list.extend(herd(2000, 150, 2000.0, 0.0));
        list.extend(herd(3000, 150, 3000.0, 0.0));
        list[5].links.push(1007);
        list[460].raid = Some(9);
        list[301].raid = Some(9);
        let plan = islands(&list, 4).expect("two islands");
        let of = group_of(&plan, list.len());
        assert_eq!(of[0], of[150], "the link");
        assert_eq!(of[300], of[450], "the raid");
        assert_ne!(of[0], of[300]);
        // An id that isn't in the list joins nothing.
        list[5].links = vec![99_999];
        assert_eq!(islands(&list, 4).expect("three islands").groups.len(), 3);
    }

    /// A villager's points of interest join the mobs that read the village data near them.
    #[test]
    fn village_readers_join_the_villagers_they_read() {
        let mut list = herd(0, 150, 0.0, 0.0);
        list.extend(herd(1000, 150, 1000.0, 0.0));
        list.extend(herd(2000, 150, 2000.0, 0.0));
        // A villager of the first herd claims a bed 600 blocks away; a mob of the third herd
        // reads the villages around it.
        let bed = point_box([600, 64, 0]);
        list[3].claims = Some(union_box(grow(list[3].body, POI_CLAIM), bed));
        list[300].village_reads = Some(grow(point_box([600, 64, 0]), 4.0));
        let plan = islands(&list, 3).expect("two islands");
        let of = group_of(&plan, list.len());
        assert_eq!(of[0], of[300]);
        assert_ne!(of[0], of[150]);
    }

    /// Exercise the actual Mob/Seen-to-reach conversion, not just hand-built planner boxes.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn actual_reaches_cover_links_villages_and_bounded_moves() {
        let mut game = super::super::super::native_machines::tests::server_game();
        game.spawn_mob("minecraft:cow", [0.5, 65.0, 0.5], true)
            .unwrap();
        let kind = Arc::clone(&game.mobs[0].kind);
        let mut mobs: Vec<_> = (0..400)
            .map(|i| {
                let x = f64::from(i / 100 * 1000 + i % 8);
                let z = f64::from(i % 100 / 8);
                let key = (x as i32 >> 4, z as i32 >> 4);
                if !game.world.chunks.contains_key(&key) {
                    game.world.chunks.insert(
                        key,
                        Arc::new(horizon_client::chunk::Chunk::empty(key.0, key.1)),
                    );
                }
                Mob::new(i, Arc::clone(&kind), Vec3::new(x, 65.0, z), 0.0)
            })
            .collect();
        let mut snapshot: Vec<_> = mobs.iter().map(|m| m.seen(None)).collect();
        let base = game.mob_world_base(&[]);
        let players = super::super::super::natural_spawning::NearestPlayers::new(&[]);
        let nearest = super::super::prep::MobsNearest::new(&players, &mobs, false);
        let turns = Turns {
            base: &base,
            despawn: base.despawn_view(),
            ticking: &|_, _| true,
            nearest: &nearest,
            single_player: Some(None),
            canceled: &|_| false,
            difficulty: &|_| horizon_client::mobs::spawning::LocalDifficulty::new(2, 0, 0, 0.0),
            cached_difficulty: None,
        };
        // The host player stands in the third herd.
        let owners = [([0, 0, 0, 7], Vec3::new(2003.5, 65.0, 3.5))];
        let plan = |mobs: &[Mob], snapshot: &[Seen]| {
            islands(&reaches(mobs, snapshot, &turns, &owners).unwrap(), 4).unwrap()
        };
        assert_eq!(plan(&mobs, &snapshot).groups.len(), 4);
        for relation in 0..8 {
            let mut linked = snapshot.clone();
            match relation {
                0 => linked[0].target = Some(Who::Mob(100)),
                1 => linked[0].hurt_by = Some((Who::Mob(100), 1)),
                2 => linked[0].leash = Some(leash::Holder::Mob(100)),
                3 => linked[0].vehicle = Some(100),
                4 => linked[0].caravan = 101,
                5 => linked[0].school.0 = Some(100),
                6 => mobs[0].extra.mount = Some(100),
                _ => mobs[0].extra.rider = Some(100),
            }
            let of = group_of(&plan(&mobs, &linked), mobs.len());
            assert_eq!(of[0], of[100], "relation {relation}");
            assert_ne!(of[0], of[200]);
            mobs[0].extra.mount = None;
            mobs[0].extra.rider = None;
        }
        horizon_client::mobs::raider::state_mut(&mut mobs[0]).raid_id = Some(7);
        horizon_client::mobs::raider::state_mut(&mut mobs[100]).raid_id = Some(7);
        let of = group_of(&plan(&mobs, &snapshot), mobs.len());
        assert_eq!(of[0], of[100], "one raid cannot split");
        horizon_client::mobs::raider::state_mut(&mut mobs[0]).raid_id = None;
        horizon_client::mobs::raider::state_mut(&mut mobs[100]).raid_id = None;
        snapshot[0].species = Species::Villager;
        snapshot[0].bed = Some([2000, 65, 0]);
        snapshot[200].species = Species::IronGolem;
        let of = group_of(&plan(&mobs, &snapshot), mobs.len());
        assert_eq!(of[0], of[200], "a remote bed reaches its village reader");
        assert_ne!(of[0], of[100]);
        snapshot[0].species = Species::Cow;
        snapshot[0].bed = None;
        snapshot[200].species = Species::Cow;
        assert_eq!(plan(&mobs, &snapshot).groups.len(), 4);
        let unchanged = mobs[0].clone();
        // A wither's side head keeps its target by id (`wither::target_pos`).
        snapshot[0].species = Species::Wither;
        mobs[0].extra.ints[1] = 100;
        let of = group_of(&plan(&mobs, &snapshot), mobs.len());
        assert_eq!(of[0], of[100], "a wither's head target");
        assert_ne!(of[0], of[200]);
        mobs[0] = unchanged.clone();
        // A warden's suspect, kept in its anger.
        snapshot[0].species = Species::Warden;
        let mut warden = horizon_client::mobs::warden::Warden::default();
        warden.anger.increase(Who::Mob(300), 80);
        mobs[0].extra.warden = Some(Box::new(warden));
        let of = group_of(&plan(&mobs, &snapshot), mobs.len());
        assert_eq!(of[0], of[300], "a warden's suspect");
        assert_ne!(of[0], of[100]);
        snapshot[0].species = Species::Cow;
        mobs[0] = unchanged.clone();
        // A pet joins the mobs near its owner, and only those; another player's pet nothing.
        mobs[0].extra.owner = Some([0, 0, 0, 7]);
        let of = group_of(&plan(&mobs, &snapshot), mobs.len());
        assert_eq!(of[0], of[200], "the pet's owner stands in the third herd");
        assert_ne!(of[0], of[100]);
        assert_ne!(of[0], of[300]);
        mobs[0].extra.owner = Some([0, 0, 0, 8]);
        assert_eq!(plan(&mobs, &snapshot).groups.len(), 4, "no such owner here");
        mobs[0] = unchanged.clone();
        // Chorus fruit: half its 16-block diameter around the mob, and down to the world's floor.
        mobs[0]
            .extra
            .set_main_hand(Some("minecraft:chorus_fruit".into()));
        let r = &reaches(&mobs, &snapshot, &turns, &owners).unwrap()[0];
        let plain = &reaches(
            std::slice::from_ref(&unchanged),
            &snapshot[..1],
            &turns,
            &owners,
        )
        .unwrap()[0];
        assert!((r.body.max(Axis::X) - plain.body.max(Axis::X) - 9.0).abs() < 1e-9);
        assert!(r.body.min(Axis::Y) < -1e9);
        assert_eq!(plan(&mobs, &snapshot).groups.len(), 4, "its own herd only");
        mobs[0] = unchanged.clone();
        // Changed locomotion grows the reach by its bound, even from rest and with NoAI: a
        // Speed II effect by a few blocks, a speed of 128 to the next herd, not the one after.
        mobs[0].extra.effects.push(("minecraft:speed", 100, 1));
        let speedy = reaches(&mobs, &snapshot, &turns, &owners).unwrap()[0].body;
        let grown = speedy.max(Axis::X) - plain.body.max(Axis::X);
        assert!(grown > 1.0 && grown < 10.0, "{grown}");
        assert_eq!(plan(&mobs, &snapshot).groups.len(), 4);
        mobs[0] = unchanged.clone();
        for attr in [
            horizon_client::mobs::finalize::Attr::MovementSpeed,
            horizon_client::mobs::finalize::Attr::FlyingSpeed,
        ] {
            horizon_client::mobs::finalize::set_base(&mut mobs[0], attr, 128.0);
            mobs[0].no_ai = true;
            let of = group_of(&plan(&mobs, &snapshot), mobs.len());
            assert_eq!(of[0], of[100], "{attr:?}");
            assert_ne!(of[0], of[200], "{attr:?}");
            mobs[0] = unchanged.clone();
        }
        mobs[0].velocity.x = f64::NAN;
        assert!(reaches(&mobs, &snapshot, &turns, &owners).is_none());
        mobs[0] = unchanged;
        mobs[0].position.y = f64::NAN;
        assert!(reaches(&mobs, &snapshot, &turns, &owners).is_none());
        assert!(reaches(&mobs, &snapshot[..399], &turns, &owners).is_none());
    }

    /// A later id is visible frozen without waiting; an earlier id waits for its publication,
    /// and a lane that unwinds without publishing releases the lookup instead of hanging it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn foreign_lookup_observes_serial_visibility_and_unwind() {
        let mut game = super::super::super::native_machines::tests::server_game();
        game.spawn_mob("minecraft:cow", [0.5, 65.0, 0.5], true)
            .unwrap();
        let template = game.mobs[0].seen(None);
        let (sent, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let draws = OrderedDraws::new(&horizon_core::random::LegacyRandom::new(7), 2);
            let mut frozen = vec![template.clone(), template];
            frozen[0].id = 10;
            frozen[1].id = 11;
            let board = Board {
                index: [(10, 0), (11, 1)].into_iter().collect(),
                frozen,
                published: vec![OnceLock::new(), OnceLock::new()],
                lane_of: vec![0, 1],
                lanes: 2,
                draws,
            };
            let foreign = Foreign {
                board: &board,
                reader: Cell::new(0),
                defers: false,
            };
            assert_eq!(foreign.seen(11).unwrap().position, board.frozen[1].position);
            assert!(foreign.seen(999).is_none());
            std::thread::scope(|s| {
                s.spawn(|| {
                    let mut random = board.draws.lane(0);
                    let _done = random.lane_done();
                    random.begin_item(0);
                    let publisher = Foreign {
                        board: &board,
                        reader: Cell::new(0),
                        defers: false,
                    };
                    let mut moved = board.frozen[0].clone();
                    moved.position.x = 42.0;
                    publisher.publish(0, &moved);
                });
                let later = Foreign {
                    board: &board,
                    reader: Cell::new(1),
                    defers: false,
                };
                assert_eq!(later.seen(10).unwrap().position.x, 42.0);
            });
            let missing = Board {
                published: vec![OnceLock::new(), OnceLock::new()],
                draws: OrderedDraws::new(&horizon_core::random::LegacyRandom::new(7), 2),
                ..board
            };
            std::thread::scope(|s| {
                let earlier = s.spawn(|| {
                    let random = missing.draws.lane(0);
                    let _done = random.lane_done();
                    panic!("turn failed before publication");
                });
                let later = Foreign {
                    board: &missing,
                    reader: Cell::new(1),
                    defers: false,
                };
                assert!(later.seen(10).is_none());
                assert!(earlier.join().is_err());
            });
            let _ = sent.send(());
        });
        finished
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("foreign wait released");
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn moved_riders_use_loaded_and_ticking_state_at_their_turn() {
        let mut game = super::super::super::native_machines::tests::server_game();
        game.spawn_mob("minecraft:cow", [0.5, 65.0, 0.5], true)
            .unwrap();
        let kind = Arc::clone(&game.mobs[0].kind);
        game.world.chunks.insert(
            (62, 0),
            Arc::new(horizon_client::chunk::Chunk::empty(62, 0)),
        );
        let mut mobs: Vec<_> = [1.5, 0.5, 1001.5, 1001.5]
            .into_iter()
            .enumerate()
            .map(|(i, x)| {
                let mut m = Mob::new(
                    i as u32 + 10,
                    Arc::clone(&kind),
                    Vec3::new(x, 65.0, 0.5),
                    0.0,
                );
                m.no_ai = true;
                m.persistent = true;
                m
            })
            .collect();
        horizon_client::mobs::riders::link(&mut mobs, 11, 10);
        horizon_client::mobs::riders::link(&mut mobs, 13, 12);
        let snapshot: Vec<_> = mobs.iter().map(|m| m.seen(None)).collect();
        // A passenger is repositioned after the initial snapshot: one enters the ticking area,
        // another leaves the loaded area. The turn must not reuse those initial roles.
        mobs[1].position.x = 1.5;
        mobs[3].position.x = 1024.5;
        let base = game.mob_world_base(&[]);
        let players = super::super::super::natural_spawning::NearestPlayers::new(&[]);
        let nearest = super::super::prep::MobsNearest::new(&players, &mobs, false);
        let ticking = |x: i32, _: i32| x.rem_euclid(1000) >= 1;
        let mut turns = Turns {
            base: &base,
            despawn: base.despawn_view(),
            ticking: &ticking,
            nearest: &nearest,
            single_player: Some(None),
            canceled: &|_| false,
            difficulty: &|_| horizon_client::mobs::spawning::LocalDifficulty::new(2, 0, 0, 0.0),
            cached_difficulty: None,
        };
        let start = Start {
            events: Vec::new(),
            random: horizon_core::random::LegacyRandom::new(7),
            guests: Vec::new(),
            player: None,
            bindings: Default::default(),
            died: Default::default(),
            ticked: Default::default(),
            targets: vec![None; 4],
        };
        let shared = Shared {
            uuid: None,
            level_id: "minecraft:overworld",
            path_changes: &[],
            guest_ids: &[],
        };
        let mut serial = Group::new(mobs.clone(), snapshot.clone(), start.clone());
        serial.run(&mut turns, &shared, None);
        turns.cached_difficulty = None;
        let grouped = Islands {
            uuid: None,
            ticking: &|| Box::new(ticking),
        }
        .run(
            Plan {
                groups: vec![vec![0, 1], vec![2, 3]],
            },
            mobs,
            snapshot,
            start,
            &mut turns,
            &shared,
        );
        let expected: FxHashSet<_> = [10, 11, 12].into_iter().collect();
        assert_eq!(serial.ticked, expected);
        assert_eq!(grouped.ticked, expected);
        assert_eq!(difference(&serial, &grouped), None);
    }

    /// The loop stays serial with too few ticking mobs, one island holding most of them, or one
    /// lane; resting mobs weigh nothing.
    #[test]
    fn small_or_lopsided_loops_run_serially() {
        let mut list = herd(0, 100, 0.0, 0.0);
        list.extend(herd(1000, 100, 1000.0, 0.0));
        assert!(islands(&list, 2).is_none(), "200 ticking mobs");
        list.extend(herd(2000, 100, 2000.0, 0.0));
        assert!(islands(&list, 2).is_some());
        assert!(islands(&list, 1).is_none(), "one lane");
        let mut lopsided = herd(0, 400, 0.0, 0.0);
        lopsided.extend(herd(1000, 100, 1000.0, 0.0));
        assert!(islands(&lopsided, 2).is_none(), "one island holds 4 in 5");
        // The small herd's resting mobs leave the big one 400 of 400 ticking mobs.
        let mut resting = herd(0, 300, 0.0, 0.0);
        resting.extend(herd(1000, 300, 1000.0, 0.0).into_iter().map(|r| Reach {
            ticking: false,
            ..r
        }));
        assert!(islands(&resting, 2).is_none());
    }

    fn separated_herds_game() -> Game<'static> {
        let mut game = super::super::super::native_machines::tests::server_game();
        game.build_registries();
        game.set_allow_commands(true);
        let feet = Vec3::new(104.5, 65.0, 8.5);
        game.player = Some(horizon_client::motion::PlayerMotion::new(game.motion, feet).unwrap());
        for x in -3..=16 {
            for z in -3..=3 {
                let chunk = horizon_client::chunk::Chunk::empty(x, z);
                game.world.chunks.insert((x, z), Arc::new(chunk));
            }
        }
        game.update_chunk_tickets();
        let stone = game.pack.assets.registry.default_state("minecraft:stone");
        let kinds = [
            "pig",
            "cow",
            "sheep",
            "chicken",
            "wolf",
            "rabbit",
            "goat",
            "frog",
            "armadillo",
            "camel",
        ];
        for herd in 0..3 {
            let x0 = 96 * herd;
            for x in x0..x0 + 20 {
                for z in 0..20 {
                    game.world.set(x, 64, z, stone.unwrap()).unwrap();
                }
            }
            // Packed 0.6 blocks apart: they push each other.
            for k in 0..110 {
                let x = f64::from(x0 + 2) + f64::from(k % 10) * 0.6;
                let z = 2.0 + f64::from(k / 10) * 0.6;
                let kind = kinds[k as usize % kinds.len()];
                assert!(game.run_command(&format!("summon {kind} {x:.2} 66 {z:.2}")));
            }
        }
        game
    }

    /// Mob ticks in islands on several threads leave every mob, the level random and the death
    /// marks as the serial loop does: three herds of animals far apart, packed so they push each
    /// other, some with brains that draw from the level random (no villagers: their village reads
    /// reach the next herd).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn islands_tick_their_mobs_as_one_thread_does() {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                LANES.with(|l| l.set(None));
                VERIFY.with(|v| v.set(false));
            }
        }
        let _reset = Reset;
        let mut game = separated_herds_game();
        let (mobs, random, died) = (
            game.mobs.clone(),
            game.world.random.clone(),
            game.died.clone(),
        );
        let mut runs = Vec::new();
        let mut seeds = None;
        for lanes in [1, 3] {
            (game.mobs, game.world.random, game.died) =
                (mobs.clone(), random.clone(), died.clone());
            LANES.with(|l| l.set(Some(lanes)));
            RAN.with(|r| r.set(0));
            let tick = || {
                for _ in 0..20 {
                    game.tick_creature_entities().unwrap();
                }
            };
            if lanes == 1 {
                seeds = Some(horizon_client::random_source::record_seeds(tick).1);
            } else {
                let seeds = seeds.as_ref().unwrap();
                horizon_client::random_source::replay_seeds(Arc::clone(seeds), tick);
                assert!(seeds.complete(), "the same new RNGs were created");
            }
            // `section_seq`'s counter runs on across the runs (see `difference`).
            let left: Vec<String> = game
                .mobs
                .iter()
                .map(|m| {
                    let mut m = m.clone();
                    m.extra.section_seq = 0;
                    format!("{m:?}")
                })
                .collect();
            let mut died: Vec<u32> = game.died.iter().copied().collect();
            died.sort_unstable();
            runs.push((
                left,
                format!("{:?}", game.world.random),
                died,
                RAN.with(Cell::get),
            ));
        }
        assert_eq!(runs[0].3, 0, "one lane runs serially");
        let available = capped_lanes(
            3,
            std::thread::available_parallelism().ok().map(usize::from),
        );
        let grouped = if available > 1 { available } else { 0 };
        assert_eq!(
            runs[1].3, grouped,
            "the islands ran on available hardware, or fell back to serial"
        );
        assert_eq!(runs[0].0.len(), 330);
        assert!(
            runs[0]
                .0
                .iter()
                .zip(&mobs)
                .any(|(a, b)| *a != format!("{b:?}"))
        );
        for (a, b) in runs[0].0.iter().zip(&runs[1].0) {
            assert_eq!(a, b);
        }
        assert_eq!(runs[0].1, runs[1].1, "the level random");
        assert_eq!(runs[0].2, runs[1].2, "the death marks");
        (game.mobs, game.world.random, game.died) = (mobs, random, died);
        VERIFY.with(|v| v.set(true));
        CHECK.with(|c| c.set((0, 0)));
        for _ in 0..20 {
            game.tick_creature_entities().unwrap();
        }
        assert_eq!(
            CHECK.with(Cell::get),
            (if grouped > 1 { 20 } else { 0 }, 0),
            "verify checked every grouped tick, or the hardware cap kept it serial"
        );
    }

    /// The groups a plan of `islands` islands runs in on three requested lanes, on this machine.
    fn groups_for(islands: usize) -> usize {
        let lanes = capped_lanes(
            3,
            std::thread::available_parallelism().ok().map(usize::from),
        );
        if lanes > 1 { islands.min(lanes) } else { 0 }
    }

    /// Ticks `game`'s mobs `ticks` times serially and then on three lanes from the same start,
    /// once plain and once under verify. Each grouped run leaves the mobs, the level random and
    /// the death marks as its serial run does and creates the same RNGs, and verify finds no
    /// difference. Each grouped run replays its own serial run's seeds: the process-wide counter
    /// moves on between runs. `each` runs before every tick, as a goal would. Returns the groups
    /// the first grouped tick ran in; `game` is left as the last run left it.
    fn grouped_as_serial(
        game: &mut Game<'static>,
        ticks: usize,
        what: &str,
        each: fn(&mut Game<'static>),
    ) -> usize {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                LANES.with(|l| l.set(None));
                VERIFY.with(|v| v.set(false));
            }
        }
        let _reset = Reset;
        let (mobs, random, died) = (
            game.mobs.clone(),
            game.world.random.clone(),
            game.died.clone(),
        );
        let mut first = None;
        for verify in [false, true] {
            let mut expected = None;
            let mut seeds = None;
            for requested in [1, 3] {
                (game.mobs, game.world.random, game.died) =
                    (mobs.clone(), random.clone(), died.clone());
                LANES.with(|l| l.set(Some(requested)));
                VERIFY.with(|v| v.set(verify && requested > 1));
                CHECK.with(|c| c.set((0, 0)));
                let mut ran = Vec::new();
                let tick = || {
                    for _ in 0..ticks {
                        each(game);
                        RAN.with(|r| r.set(0));
                        game.tick_creature_entities().unwrap();
                        ran.push(RAN.with(Cell::get));
                    }
                };
                if requested == 1 {
                    seeds = Some(horizon_client::random_source::record_seeds(tick).1);
                    assert!(
                        ran.iter().all(|&g| g == 0),
                        "{what}: one lane runs serially"
                    );
                } else {
                    let seeds = seeds.take().unwrap();
                    horizon_client::random_source::replay_seeds(Arc::clone(&seeds), tick);
                    assert!(seeds.complete(), "{what}: the same new RNGs were created");
                    first.get_or_insert(ran[0]);
                    let grouped = ran.iter().filter(|&&g| g > 1).count();
                    let checked = if verify { grouped } else { 0 };
                    assert_eq!(CHECK.with(Cell::get), (checked, 0), "{what}: verify");
                }
                let left: Vec<String> = game
                    .mobs
                    .iter()
                    .map(|m| format!("{:?}", without_counters(m)))
                    .collect();
                let mut died: Vec<u32> = game.died.iter().copied().collect();
                died.sort_unstable();
                let result = (left, format!("{:?}", game.world.random), died);
                match &expected {
                    Some(expected) => assert_eq!(&result, expected, "{what}: grouped as serial"),
                    None => expected = Some(result),
                }
            }
        }
        first.unwrap()
    }

    /// A pet teleports to its owner (`FollowOwnerGoal`) from one herd into another: the plan joins
    /// it with the herd around its owner, and the herd far from both still ticks in a group of its
    /// own.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_pet_teleports_to_its_owner_while_a_far_herd_ticks_apart() {
        let mut game = separated_herds_game();
        // Keep the neighbors in place while pets cross the 96-block gap.
        for m in &mut game.mobs {
            m.no_ai = true;
        }
        let herds = game.mobs.clone();
        for species in ["wolf", "cat", "parrot"] {
            game.mobs = herds.clone();
            game.spawn_mob(&format!("minecraft:{species}"), [2.5, 65.0, 2.5], true)
                .unwrap();
            let mut pet = game.mobs.pop().unwrap();
            horizon_client::mobs::tame::set_tame(&mut pet, true);
            pet.extra.owner = Some(game.level.player_uuid.unwrap_or_default());
            pet.random =
                horizon_core::random::EntityRandom::own(horizon_core::random::LegacyRandom::new(7));
            // Its destination neighbors come later, so they must see its moved snapshot.
            game.mobs.insert(0, pet);
            // The pet's herd and its owner's herd, and the third one.
            let groups = grouped_as_serial(&mut game, 20, species, |_| {});
            assert_eq!(groups, groups_for(2), "{species}");
            let pet = &game.mobs[0];
            assert!(
                pet.position.x > 96.0 && pet.position.x < 112.0,
                "{species} really teleported from the origin herd: {:?}",
                pet.position
            );
        }
    }

    /// The raiders of one raid order their hurts across it (`raids.rs`): one in each of two herds
    /// joins them, and the herd between ticks in a group of its own.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_raid_shares_one_group_beside_a_herd() {
        let mut game = separated_herds_game();
        for x in [2.5, 194.5] {
            game.spawn_mob("minecraft:pillager", [x, 66.0, 2.5], true)
                .unwrap();
            let raider = game.mobs.len() - 1;
            let state = horizon_client::mobs::raider::state_mut(&mut game.mobs[raider]);
            state.raid_id = Some(42);
            state.active_raid = true;
        }
        let groups = grouped_as_serial(&mut game, 20, "raid", |_| {});
        assert_eq!(groups, groups_for(2));
    }

    /// A fox that finishes a chorus fruit teleports to the first ground below a point within the
    /// item's diameter, a fall of any length: 85 blocks above one herd, it joins that herd's
    /// island, and the two others tick apart.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_fox_eating_chorus_fruit_joins_the_herd_below() {
        let mut game = separated_herds_game();
        for m in &mut game.mobs {
            m.no_ai = true;
        }
        // The fox waits 85 blocks above the second herd, far outside every read of it, and
        // finishes its fruit this tick (`TICKS_SINCE_EATEN` is 600; the tick makes it 601).
        game.spawn_mob("minecraft:fox", [2.5, 65.0, 2.5], true)
            .unwrap();
        let mut fox = game.mobs.pop().unwrap();
        // It stands on a one-block pillar, so `travel` keeps it on the ground until it eats.
        let pillar = game.pack.assets.registry.default_state("minecraft:stone");
        game.world.set(106, 149, 10, pillar.unwrap()).unwrap();
        fox.position = Vec3::new(106.5, 150.0, 10.5);
        fox.previous = fox.position;
        fox.velocity = Vec3::ZERO;
        fox.on_ground = true;
        fox.ai.goals = Default::default();
        fox.ai.targets = Default::default();
        fox.extra
            .set_main_hand(Some("minecraft:chorus_fruit".into()));
        fox.random =
            horizon_core::random::EntityRandom::own(horizon_core::random::LegacyRandom::new(7));
        // The herd's mobs come later, so the serial loop shows them the landed fox.
        game.mobs.insert(0, fox);
        // One tick settles it on the pillar (the first tick of a mob does not run its step);
        // it has just started eating, so it does not finish yet.
        game.tick_creature_entities().unwrap();
        assert_eq!(game.mobs[0].position, Vec3::new(106.5, 150.0, 10.5));
        game.mobs[0].extra.ints[1] = 600;
        let groups = grouped_as_serial(&mut game, 1, "fox", |_| {});
        assert_eq!(groups, groups_for(3));
        let fox = &game.mobs[0];
        assert!(
            fox.position.y < 100.0 && fox.extra.main_hand.is_none(),
            "the fox ate its fruit and really teleported down to the herd: {:?}",
            fox.position
        );
    }

    /// `MoveControl` accelerates a stationary mob with a movement speed of 8 into the next herd,
    /// 96 blocks on: its locomotion bound joins that herd, not the one past it, and the pushes
    /// at the destination come out as serially.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_fast_mob_joins_only_the_herds_its_bound_reaches() {
        let mut game = separated_herds_game();
        for m in &mut game.mobs {
            m.no_ai = true;
            m.position.y = 65.0;
            m.on_ground = true;
            m.velocity = Vec3::ZERO;
        }
        game.spawn_mob("minecraft:cow", [2.5, 65.0, 2.5], true)
            .unwrap();
        let mut fast = game.mobs.pop().unwrap();
        // A road from its herd's floor to the next herd's, so it runs rather than falls.
        let stone = game.pack.assets.registry.default_state("minecraft:stone");
        for x in 20..212 {
            for z in 0..5 {
                game.world.set(x, 64, z, stone.unwrap()).unwrap();
            }
        }
        fast.on_ground = true;
        fast.velocity = Vec3::ZERO;
        fast.yaw = -90.0;
        fast.ai.goals = Default::default();
        fast.ai.targets = Default::default();
        horizon_client::mobs::finalize::set_base(
            &mut fast,
            horizon_client::mobs::finalize::Attr::MovementSpeed,
            8.0,
        );
        // Destination mobs come later, so the serial loop applies these pushes at their turn.
        game.mobs.insert(0, fast);
        // A goal sets the move control again every tick; `MoveControl.tick` lets it go after one.
        let groups = grouped_as_serial(&mut game, 20, "fast cow", |game| {
            game.mobs[0].ai.mov.set_wanted(110.0, 65.0, 2.5, 1.0);
        });
        assert_eq!(groups, groups_for(2));
        let fast = &game.mobs[0];
        assert!(
            fast.position.x > 96.0,
            "it really ran into the destination herd: {:?}",
            fast.position
        );
    }

    /// Islands are packed heaviest first, each into the lightest group.
    #[test]
    fn islands_are_packed_into_the_lightest_group() {
        let mut list = herd(0, 200, 0.0, 0.0);
        list.extend(herd(1000, 100, 1000.0, 0.0));
        list.extend(herd(2000, 100, 2000.0, 0.0));
        let plan = islands(&list, 2).expect("two groups");
        let of = group_of(&plan, list.len());
        assert_ne!(of[0], of[200]);
        assert_eq!(of[200], of[300], "both small herds fill the lighter group");
    }
}
