//! PURPOSE: ticks experience orbs, primed TNT and falling blocks in the order
//! they joined the level, between the other kinds' batches.
//! WHY: `ServerLevel.tick` ticks every entity in join order, one kind among
//! the rest; these three used to tick as three fixed batches (orbs after the
//! mobs, falling blocks and TNT after the gateways), so a TNT that joined
//! before a boat blew up after the boat moved (#6509, tick order slice 2).
//! REFS: vanilla `ServerLevel.tick` (`entityTickList.forEach`),
//! `EntityTickList`; `join_seq.rs`; `tools/test_tick_order_parity.py`.
//! LOGS: none. READS-BY: `Game::tick_world`, `tick_mobs`.
//! ENDS-WHEN: every kind ticks from the join-order walk.

use super::{Game, Result};
use horizon_client::join_seq::{JoinSeq, Joined};
use horizon_client::random_source;
use horizon_core::geometry::Vec3;
use horizon_core::random::RandomSource as _;

/// What this tick has ticked of the three kinds.
#[derive(Default, Clone)]
pub(super) struct OrderedTicks {
    /// Joins at or above this started after the tick began; they tick next tick.
    /// The general rows' pass (`general/tick.rs`) shares the same cutoff.
    pub(in crate::game) born: JoinSeq,
    /// The last join number ticked, if any: later ones are still to come.
    pub(super) done: Option<JoinSeq>,
    /// Joins below this have had their spawn packet; the ones above joined
    /// since (a summoned falling block, a lit TNT) and tick after it goes out.
    pub(super) announced: JoinSeq,
    /// For each batch of another kind, in tick order (see [`Batch`]): the
    /// lowest join number among it and every batch after it.
    suffix_min: [JoinSeq; Batch::COUNT],
    /// The orbs' lava random, made on first use.
    lava_random: Option<horizon_core::random::LegacyRandom>,
    /// Scratch schedule reused by each ordered pass in a level tick. The
    /// entity lists can change between passes, so this only retains capacity;
    /// entries are rebuilt from the live lists every time: the place in the
    /// tick order, the kind and the join number.
    pending_buf: Vec<(JoinSeq, Pending, JoinSeq)>,
    /// The level's entities out of the tick order or back in it at its end:
    /// each level keeps its own (`levels.rs` swaps it in with the lists).
    pub(super) restarts: TickRestarts,
    /// The falling blocks (id, join) that ended a tick still falling with a part listening to
    /// `falling_tick`, delivered after the pass.
    pub(super) falling_ticked: Vec<(u32, JoinSeq)>,
}

/// `EntityTickList` beyond the join order, for one level's orbs, primed TNT
/// and falling blocks.
#[derive(Default, Clone)]
pub(super) struct TickRestarts {
    /// The entities (by join number) in chunks that do not tick entities,
    /// with when they were first found there: they are out of the tick list
    /// until their chunk ticks (`onTickingEnd`).
    idle: horizon_client::fast_hash::FxHashMap<JoinSeq, u64>,
    /// Counts the entities found idle, in the order they were.
    idle_clock: u64,
    /// The entities that started ticking again since they joined, by join
    /// number: `EntityTickList.add` put them at the end of the tick order,
    /// at this number.
    restarted: horizon_client::fast_hash::FxHashMap<JoinSeq, JoinSeq>,
}

impl OrderedTicks {
    /// Where the entity that joined as `join` is in the tick order: where it
    /// joined, or where it last started ticking again.
    pub(super) fn tick_key(&self, join: JoinSeq) -> JoinSeq {
        self.restarts.restarted.get(&join).copied().unwrap_or(join)
    }
}

/// The batches of the other kinds, in the order they tick.
#[derive(Clone, Copy)]
pub(super) enum Batch {
    ArmorStands,
    Boats,
    Carts,
    Tridents,
    Rockets,
    Items,
    Mobs,
    Projectiles,
    /// The general rows tick last of the kinds (`general/tick.rs`): a slot
    /// of its own keeps every earlier batch's `ordered_before` draining the
    /// orbs, TNT and falling blocks that joined before the earliest general
    /// row, so the mobile kinds keep their join order against them.
    General,
}

impl Batch {
    const COUNT: usize = 9;
}

#[derive(Clone, Copy)]
enum Pending {
    Orb,
    Tnt,
    Falling,
}

/// The lowest join number in a batch, or none when it is empty.
pub(super) fn first_join<T: Joined>(list: &[T]) -> Option<JoinSeq> {
    list.iter().map(Joined::join_seq).min()
}

impl Game<'_> {
    /// `PersistentEntitySectionManager.startTicking`: an idle orb, primed TNT
    /// or falling block whose chunk now ticks entities (a piston pushed it into
    /// one, or the chunk's tickets changed) goes to the end of the tick order
    /// (`EntityTickList.add`), behind everything that ticked meanwhile. Those
    /// that start together keep the order they went idle in: the order they
    /// came into their section, which `getEntities` (the piston's push, the
    /// chunk's status change) walks.
    fn restart_woken_entities(&mut self) {
        if self.ordered.restarts.idle.is_empty() && self.ordered.restarts.restarted.is_empty() {
            return;
        }
        let entity_ticking = self.entity_ticking_area();
        let position = |game: &Self, join: JoinSeq| {
            game.tnts
                .locate(join)
                .map(|i| game.tnts[i].position)
                .or_else(|| game.falling.locate(join).map(|i| game.falling[i].position))
                .or_else(|| game.orbs.locate(join).map(|i| game.orbs[i].position))
        };
        let idle = std::mem::take(&mut self.ordered.restarts.idle);
        let mut woken = Vec::new();
        for (join, since) in idle {
            match position(self, join) {
                Some(at) if entity_ticking(at.x.floor() as i32, at.z.floor() as i32) => {
                    woken.push((since, join));
                }
                Some(_) => {
                    self.ordered.restarts.idle.insert(join, since);
                }
                None => {}
            }
        }
        woken.sort_unstable();
        for (_, join) in woken {
            self.ordered
                .restarts
                .restarted
                .insert(join, JoinSeq::restart());
        }
        let gone: Vec<JoinSeq> = self
            .ordered
            .restarts
            .restarted
            .keys()
            .copied()
            .filter(|&join| position(self, join).is_none())
            .collect();
        for join in gone {
            self.ordered.restarts.restarted.remove(&join);
        }
    }

    /// `onTickingEnd`: the entity that joined as `join` is in a chunk that
    /// does not tick entities, out of the tick order until it ticks again.
    fn went_idle(&mut self, join: JoinSeq) {
        let restarts = &mut self.ordered.restarts;
        if !restarts.idle.contains_key(&join) {
            restarts.idle_clock += 1;
            restarts.idle.insert(join, restarts.idle_clock);
        }
    }

    /// A level tick starts: what has joined so far ticks this tick.
    pub(super) fn begin_ordered_ticks(&mut self) {
        let _span = horizon_client::spans::guard("ordered_ticks");
        self.restart_woken_entities();
        let pending_buf = std::mem::take(&mut self.ordered.pending_buf);
        let mut falling_ticked = std::mem::take(&mut self.ordered.falling_ticked);
        falling_ticked.clear();
        let firsts = [
            first_join(&self.armor_stands),
            first_join(&self.boats),
            first_join(&self.carts),
            first_join(&self.tridents),
            first_join(&self.rockets),
            first_join(&self.dropped),
            first_join(&self.mobs),
            first_join(&self.creatures.projectiles),
            first_join(&self.resident_entities.general.rows),
        ];
        // A batch ticks whole, so an entity of the three kinds goes after the
        // last batch that holds anything that joined before it.
        let mut suffix_min = [JoinSeq::MAX; Batch::COUNT];
        let mut lowest = JoinSeq::MAX;
        for (slot, first) in suffix_min.iter_mut().zip(firsts).rev() {
            lowest = lowest.min(first.unwrap_or(JoinSeq::MAX));
            *slot = lowest;
        }
        self.ordered = OrderedTicks {
            announced: self.ordered.announced,
            born: JoinSeq::upcoming(),
            done: None,
            suffix_min,
            lava_random: None,
            pending_buf,
            restarts: std::mem::take(&mut self.ordered.restarts),
            falling_ticked,
        };
    }

    /// Ticks, one at a time in join order, every orb, primed TNT and falling
    /// block that joined before `cut` and has not ticked this tick.
    pub(super) fn run_ordered_ticks(&mut self, cut: JoinSeq, late: bool) -> Result<()> {
        let _span = horizon_client::spans::guard("ordered_ticks");
        let mut first_error = None;
        let mut followed = None;
        let mut events = false;
        let entity_ticking = self.entity_ticking_area();
        // Who ticks in this run, sorted once: entities that join meanwhile are
        // at or above `born`, so the set cannot grow, only lose the removed.
        let (done, born, announced) =
            (self.ordered.done, self.ordered.born, self.ordered.announced);
        let wanted = |j: JoinSeq| done.is_none_or(|d| j > d) && j < cut && j < born;
        let mut pending = std::mem::take(&mut self.ordered.pending_buf);
        pending.clear();
        let ordered = &self.ordered;
        let mut offer = |join: JoinSeq, kind: Pending| {
            let j = ordered.tick_key(join);
            // Orbs are announced after their tick, always; TNT and falling
            // blocks that joined since the last packets wait for them.
            let right_pass = match kind {
                Pending::Orb => !late,
                _ => late == (j >= announced),
            };
            if right_pass && wanted(j) {
                pending.push((j, kind, join));
            }
        };
        self.orbs.iter().for_each(|e| offer(e.join, Pending::Orb));
        self.tnts.iter().for_each(|e| offer(e.join, Pending::Tnt));
        self.falling
            .iter()
            .for_each(|e| offer(e.join, Pending::Falling));
        pending.sort_unstable_by_key(|&(j, _, _)| j);
        // EntityGetter.getEntityCollisions: the boxes that stop these moves (a
        // boat, a shulker, a happy ghast held still), gathered once for the
        // run and again after an explosion, which can move or remove them.
        let mut obstacles: Option<horizon_client::entities::ObstacleGrid> = None;
        let mut nearby = Vec::new();
        for (key, kind, join) in pending.drain(..) {
            self.ordered.done = Some(key);
            // ServerLevel.tick: only entities in entity-ticking chunks tick.
            match kind {
                Pending::Tnt => {
                    let Some(i) = self.tnts.locate(join) else {
                        continue;
                    };
                    let at = self.tnts[i].position;
                    if entity_ticking(at.x.floor() as i32, at.z.floor() as i32) {
                        let t = &self.tnts[i];
                        let area = t.bounds().expand_towards(t.velocity);
                        obstacles
                            .get_or_insert_with(|| {
                                horizon_client::entities::ObstacleGrid::new(
                                    self.item_entity_collisions(),
                                )
                            })
                            .near(area.inflate(Vec3::new(1.0, 1.0, 1.0)), &mut nearby);
                        let count = self.tnts.len();
                        if let Err(e) = self.tick_one_tnt(i, &nearby) {
                            first_error.get_or_insert(e);
                        }
                        if self.tnts.len() < count {
                            obstacles = None;
                        }
                    } else {
                        self.went_idle(join);
                    }
                }
                Pending::Falling => {
                    let Some(i) = self.falling.locate(join) else {
                        continue;
                    };
                    let at = self.falling[i].position;
                    if entity_ticking(at.x.floor() as i32, at.z.floor() as i32) {
                        events = true;
                        let f = &self.falling[i];
                        let area = horizon_client::falling_block::bounds(f.position)
                            .expand_towards(f.velocity);
                        obstacles
                            .get_or_insert_with(|| {
                                horizon_client::entities::ObstacleGrid::new(
                                    self.item_entity_collisions(),
                                )
                            })
                            .near(area.inflate(Vec3::new(1.0, 1.0, 1.0)), &mut nearby);
                        if let Err(e) = self.tick_one_falling(i, &nearby) {
                            first_error.get_or_insert(e);
                        }
                    } else {
                        self.went_idle(join);
                    }
                }
                Pending::Orb => {
                    let Some(i) = self.orbs.locate(join) else {
                        continue;
                    };
                    let at = self.orbs[i].position;
                    if !entity_ticking(at.x.floor() as i32, at.z.floor() as i32) {
                        self.went_idle(join);
                        continue;
                    }
                    // ExperienceOrb.tick runs scanForMerges before it moves.
                    // An orb absorbed here is gone before its own turn's
                    // lookup finds it.
                    if self.remote.is_none() {
                        self.merge_orb(i);
                    }
                    let Some(i) = self.orbs.locate(join) else {
                        continue;
                    };
                    let followed = followed.get_or_insert_with(|| self.orb_followed());
                    let target = self.orb_target(i, followed);
                    let Some(i) = self.orbs.locate(join) else {
                        continue;
                    };
                    // A block of slack around the move: gravity and the pull
                    // towards a player change it before it is made.
                    let o = &self.orbs[i];
                    let area = o.bounds().expand_towards(o.velocity);
                    obstacles
                        .get_or_insert_with(|| {
                            horizon_client::entities::ObstacleGrid::new(
                                self.item_entity_collisions(),
                            )
                        })
                        .near(area.inflate(Vec3::new(1.0, 1.0, 1.0)), &mut nearby);
                    let orb_id = self.orbs[i].id;
                    let vanilla = self.saved_refuses_damage(orb_id, "minecraft:lava", false);
                    let registry = self.pack.assets.registry;
                    let tags = world_items!(self);
                    let block_registries = &self.pack.block_registries;
                    let hooks = &self.damage_hooks;
                    let o = &mut self.orbs[i];
                    // An orb has no random of its own here: lava draws from one
                    // made on first use, apart from the level stream.
                    let random = &mut self.ordered.lava_random;
                    let mut next_float = || {
                        random
                            .get_or_insert_with(random_source::entity)
                            .next_float()
                    };
                    let (stays, step) = {
                        let audio = &mut self.audio;
                        let mut on_sound = |pos: [f64; 3], pitch: f32| {
                            audio.play("minecraft:entity.generic.burn", Some(pos), 0.4, pitch);
                        };
                        o.tick_among_with_hook(
                            &mut self.world,
                            self.res.facts(),
                            self.collision,
                            target,
                            self.remote.is_none(),
                            &mut next_float,
                            Some(self.border.geometry()),
                            &self.res.motion().fluids,
                            &self.loop_context,
                            &mut on_sound,
                            &nearby,
                            &mut |world| {
                                let source = horizon_client::health::HurtSource {
                                    damage_type: "minecraft:lava",
                                    causing: 0,
                                    direct: 0,
                                    position: None,
                                };
                                horizon_client::health::DamageLevel::of(
                                    world,
                                    registry,
                                    Some(tags),
                                    Some(block_registries),
                                    hooks,
                                )
                                .invulnerability_check(
                                    horizon_client::health::Hurting::Entity(
                                        orb_id as i32,
                                        "minecraft:experience_orb",
                                        source,
                                    ),
                                    vanilla,
                                )
                            },
                        )
                    };
                    // PowderSnowBlock.entityInside during the move, before an
                    // orb that is done is removed.
                    let snow = (o.previous, o.position, o.bounds(), o.velocity);
                    self.entity_powder_snow_particles(snow.0, snow.1, snow.2, snow.3, false);
                    // Its move is filed at once, also for an orb about to go.
                    let (from, to) = (snow.0, snow.1);
                    self.entity_section_step(orb_id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
                    self.orb_moved(i, step);
                    // `ExperienceOrb.tick`'s `applyEffectsFromBlocks`: a cactus pricks it.
                    if !stays || self.orb_cactus_hits(i) {
                        self.orbs.remove(i);
                    }
                }
            }
        }
        self.ordered.pending_buf = pending;
        self.deliver_falling_ticks();
        if events && let Err(e) = self.process_level_events() {
            first_error.get_or_insert(e);
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Before a batch of another kind ticks: the three kinds' entities that
    /// joined before everything still to tick in the batches from this one on.
    pub(super) fn ordered_before(&mut self, batch: Batch) -> Result<()> {
        let cut = self.ordered.suffix_min[batch as usize];
        if cut == JoinSeq::MAX {
            return Ok(());
        }
        self.run_ordered_ticks(cut, false)
    }
}

impl Game<'_> {
    /// `Entity.move`'s fall bookkeeping after orb `i`'s move: a move of a
    /// block or more casts for a reset block or water, `checkFallDamage` adds
    /// the drop outside water and, on the ground after a fall, runs the
    /// block's `fallOn` (farmland's random draw, honey's slide), posts
    /// HIT_GROUND, then forgets it.
    pub(super) fn orb_moved(&mut self, i: usize, step: horizon_client::entities::OrbMove) {
        let o = &self.orbs[i];
        let mut fall = self.vehicle_fall_reset(o.fall_distance, step.from, step.moved);
        if !step.in_water && step.moved.y < 0.0 {
            fall -= f64::from(step.moved.y as f32);
        }
        if o.on_ground {
            if fall > 0.0 {
                // Entity.checkFallDamage: the block's fallOn, then HIT_GROUND.
                let (id, position, bounds) = (o.id, o.position, o.bounds());
                self.non_living_fall_on(id, position, bounds, fall);
                self.post_landing_event(position, bounds, false);
            }
            fall = 0.0;
        }
        self.orbs[i].fall_distance = fall;
    }
}

#[cfg(test)]
mod tests {
    use super::first_join;
    use horizon_client::chunk::Chunk;
    use horizon_client::join_seq::{JoinList, JoinSeq, Joined as _};
    use horizon_core::position::ChunkPos;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn dispatch_ticks_both_owners_of_each_located_kind() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            for x in [4, 12] {
                assert!(game.run_command(&format!("summon tnt {x} 70 4 {{fuse:80}}")));
                assert!(game.run_command(&format!(
                    "summon falling_block {x} 70 8 {{BlockState:{{Name:\"minecraft:sand\"}},Time:1}}"
                )));
                assert!(game.run_command(&format!("summon experience_orb {x} 70 12 {{Value:1}}")));
            }
            // Existing residents whose spawn packets were already announced.
            game.ordered.announced = horizon_client::join_seq::JoinSeq::MAX;
            game.tickets.update(None);
            game.begin_ordered_ticks();
            game.run_ordered_ticks(horizon_client::join_seq::JoinSeq::MAX, false)
                .unwrap();
            assert_eq!(
                game.tnts.iter().map(|t| t.fuse).collect::<Vec<_>>(),
                [79, 79]
            );
            assert_eq!(
                game.falling.iter().map(|f| f.age).collect::<Vec<_>>(),
                [2, 2]
            );
            assert_eq!(game.orbs.iter().map(|o| o.age).collect::<Vec<_>>(), [1, 1]);
        });
    }

    #[test]
    fn first_join_uses_join_sequence_when_storage_is_reordered() {
        struct E {
            join: JoinSeq,
        }
        impl horizon_client::join_seq::Joined for E {
            fn join_seq(&self) -> JoinSeq {
                self.join
            }
            fn set_join_seq(&mut self, join: JoinSeq) {
                self.join = join;
            }
        }
        let mut list: JoinList<E> = JoinList::default();
        for _ in 0..3 {
            list.push(E {
                join: JoinSeq::UNJOINED,
            });
        }
        let first = list[0].join_seq();
        list.swap(0, 2);
        assert_eq!(first_join(&list), Some(first));
    }
}
