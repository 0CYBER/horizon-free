//! Resumable borrowed mob-tick region; the caller owns final restoration.
use super::*;
/// Route each player event to the player selected when it was emitted.
/// Untagged events retain the previous per-tick guest binding.
fn route_player_events(
    events: &mut Vec<MobEvent>,
    from: usize,
    seen_guest: Option<u32>,
    guest_ids: &[(u32, u32)],
    guest_events: &mut Vec<(u32, u32, MobEvent)>,
    mob: u32,
) {
    let mut k = from;
    while k < events.len() {
        let (conn, tagged) = match &events[k] {
            MobEvent::ForPlayer { conn, .. } => (*conn, true),
            _ => (seen_guest, false),
        };
        if tagged {
            let MobEvent::ForPlayer { event, .. } = events.remove(k) else {
                unreachable!();
            };
            events.insert(k, *event);
        }
        // A hoglin broadcast carries the source's player, not the receiving
        // hoglin's independently selected player.
        if let MobEvent::HoglinAttackTarget {
            target: Who::Player,
            player,
            ..
        } = &mut events[k]
        {
            *player = conn;
        }
        // ShulkerBullet stores a target for later projectile ticks. Preserve
        // the selected guest's entity ID instead of the host-player sentinel.
        if let Some(conn) = conn
            && let MobEvent::Projectile(p) = &mut events[k]
            && let ProjectileKind::ShulkerBullet { target, .. } = &mut p.kind
            && *target == Some(Who::Player)
        {
            *target = guest_ids
                .iter()
                .find(|g| g.0 == conn)
                .map(|g| Who::Mob(g.1));
        }
        if let Some(conn) = conn
            && super::super::lan_mobs::acts_on_player(&events[k])
        {
            guest_events.push((conn, mob, events.remove(k)));
        } else {
            k += 1;
        }
    }
}

pub(super) struct BorrowedMobTick<'a> {
    pub(super) mobs: &'a mut horizon_client::join_seq::JoinList<Mob>,
    pub(super) snapshot: &'a mut Vec<Seen>,
    pub(super) passenger_objects: &'a [Seen],
    pub(super) blocked: &'a std::collections::HashSet<u32>,
    pub(super) events: &'a mut Vec<MobEvent>,
    pub(super) pushes: &'a mut horizon_client::mobs::PendingPushes,
    pub(super) dead: &'a mut Vec<Mob>,
    pub(super) corpses: &'a mut Vec<Mob>,
    pub(super) died: &'a mut horizon_client::fast_hash::FxHashSet<u32>,
    pub(super) ticked: &'a mut horizon_client::fast_hash::FxHashSet<u32>,
    pub(super) guests: &'a mut super::super::lan_mobs::GuestViews,
    pub(super) spare: &'a mut Option<PlayerView>,
    pub(super) bindings: &'a mut horizon_client::fast_hash::FxHashMap<u32, u32>,
    pub(super) guest_ids: &'a mut Vec<(u32, u32)>,
    pub(super) guest_events: &'a mut Vec<(u32, u32, MobEvent)>,
    pub(super) player_credit:
        &'a mut std::collections::HashMap<u32, super::super::combat::PlayerCredit>,
    pub(super) path_writes: &'a mut Vec<((i32, i32, i32), u32, u32)>,
    pub(super) gone: &'a mut Vec<(u32, Vec3, f64, f64, Option<Mob>)>,
    pub(super) lent: horizon_client::health::LentLevel,
    pub(super) laps: &'a mut horizon_client::spans::Laps,
    pub(super) uuid: Option<[i32; 4]>,
    pub(super) level_id: &'static str,
    pub(super) round: &'a super::super::native_ticks::TickRound,
}
impl Game<'_> {
    pub(super) fn tick_borrowed_creatures(
        &mut self,
        state: BorrowedMobTick<'_>,
    ) -> (
        horizon_client::health::LentLevel,
        Option<Box<dyn std::any::Any + Send>>,
    ) {
        let passenger_objects = state.passenger_objects;
        let mut blocked = std::borrow::Cow::Borrowed(state.blocked);
        let mut mobs = state.mobs.take_owned_pass();
        let mut snapshot = std::mem::take(state.snapshot);
        let mut events = std::mem::take(state.events);
        let mut dead = std::mem::take(state.dead);
        let mut corpses = std::mem::take(state.corpses);
        let mut died = std::mem::take(state.died);
        let mut ticked = std::mem::take(state.ticked);
        let mut guests = std::mem::take(state.guests);
        let mut spare = state.spare.take();
        let mut bindings = std::mem::take(state.bindings);
        let mut guest_ids = std::mem::take(state.guest_ids);
        let mut guest_events = std::mem::take(state.guest_events);
        let mut player_credit = std::mem::take(state.player_credit);
        let mut path_writes = std::mem::take(state.path_writes);
        let mut gone = std::mem::take(state.gone);
        let mut lent = state.lent;
        let laps = state.laps;
        let uuid = state.uuid;
        let level_id = state.level_id;
        let round = state.round;
        // Each mob's position updates in its tick, in order, filed and told before the next mob
        // is examined. The small record remains separate from `Mob` because teleports update
        // the same position field as ordinary movement.
        let section_table = round.sections().then(|| {
            std::rc::Rc::new(std::cell::RefCell::new(std::mem::take(
                &mut self.native_lifecycle.entity_sections,
            )))
        });
        let mut section_moves = section_table
            .as_ref()
            .map(|sections| std::mem::take(&mut sections.borrow_mut().ticked))
            .unwrap_or_default();
        crate::mobs::living::section_writes::begin(section_table.is_some());
        let section_scope = round
            .sections()
            .then(|| self.entity_section_scope())
            .flatten();
        let live_section_observer: Option<crate::mobs::living::section_writes::Observer> =
            section_scope
                .zip(section_table.as_ref())
                .map(|(scope, sections)| {
                    let observer: crate::mobs::living::section_writes::Observer =
                        std::rc::Rc::new(std::cell::RefCell::new(
                            crate::game::entity_sections::LiveSectionObserver::new(
                                std::rc::Rc::clone(sections),
                                scope,
                            ),
                        ));
                    observer
                });
        let mut dispatch_panic = None;
        let mut pushes = std::mem::take(state.pushes);
        let original_len = snapshot.len();
        let mut keep = vec![true; original_len];
        let mut grid =
            horizon_client::mobs::seen_grid::SeenGrid::new(snapshot.iter().map(|s| s.bounds));
        // Snapshot slots stay stable even if a full-host listener removes an
        // earlier row or adds a new one. Only that rare boundary needs a map
        // from stable identities to the owning list's current positions.
        let mut positions: Option<horizon_client::fast_hash::FxHashMap<u32, usize>> = None;
        let mut index = horizon_client::fast_hash::FxHashMap::<u32, u32>::default();
        let mut villager_slots = Vec::new();
        let mut solid_slots = Vec::new();
        for (i, s) in snapshot.iter().enumerate() {
            index.entry(s.id).or_insert(i as u32);
            if s.may_block_mobs() {
                solid_slots.push(i as u32);
            }
            if s.species == horizon_client::mobs::Species::Villager {
                villager_slots.push(i as u32);
            }
        }
        let mut cursor = 0;
        let mut resuming = false;
        // An invalid removed vehicle is retried once at this rider's own
        // eligible entity tick, never from the later global positioning pass.
        let mut retried_rider = None;
        let mut resume_from = 0;
        let mut resume_credit_expiring = false;
        let mut resume_was_on_fire = false;
        let mut resume_sampled = None;
        let mut resume_guest = None;
        loop {
            let mut pause = None;
            let mut retry_rider = None;
            // A mob's list is lifted while its world view is borrowed.  Once a
            // position write was captured, stop after this mob, restore the
            // list, and tell its writes while the live row is readable.
            let mut section_yield = false;
            {
                let ticking = self.entity_ticking_area();
                let players = self.level_players();
                let nearest = super::super::natural_spawning::NearestPlayers::new(&players);
                // All but the snapshot is the same for every mob of the tick, so
                // build it once: the player view and the teams scan every UUID.
                self.leash_boats = self
                    .boats
                    .iter()
                    .map(super::super::boat_leads::boat_state)
                    .collect();
                let mut base = self.mob_world_base(&[]);
                // Install the shared observer only on the per-mob view below; keeping the
                // reusable base empty lets its reference fields remain copyable between rows.
                base.section_writer = None;
                base.passenger_objects = passenger_objects;
                let mut level = horizon_client::health::DamageLevel::lent(
                    &self.world,
                    self.pack.assets.registry,
                    &mut lent,
                    Some(world_items!(self)),
                    Some(&self.pack.block_registries),
                    &self.damage_hooks,
                );
                let path_changes = horizon_client::mobs::collision_changes(&base, &path_writes);
                laps.lap("creature_prepare");
                let mut cached_difficulty: Option<((i32, i32), spawning::LocalDifficulty)> = None;
                while cursor < original_len {
                    let slot = cursor;
                    let i = match &positions {
                        None => slot,
                        Some(positions) => {
                            let Some(&i) = positions.get(&snapshot[slot].id) else {
                                keep[slot] = false;
                                snapshot[slot].alive = false;
                                resuming = false;
                                resume_sampled = None;
                                cursor += 1;
                                continue;
                            };
                            i
                        }
                    };
                    let m = &mut mobs[i];
                    let retried = retried_rider.take() == Some(m.id);
                    if blocked.contains(&m.id) {
                        cursor += 1;
                        resuming = false;
                        continue;
                    }
                    if m.removed && !resuming {
                        keep[slot] = false;
                        snapshot[slot].alive = false;
                        cursor += 1;
                        continue;
                    }
                    let (bx, bz) = (m.position.x.floor() as i32, m.position.z.floor() as i32);
                    let seen_guest = if resuming {
                        resume_guest
                    } else if guests.is_empty() {
                        None
                    } else {
                        let bound = bindings.get(&m.id).copied();
                        let conn = super::super::lan_mobs::player_for(
                            m,
                            base.player.as_ref(),
                            &guests,
                            bound,
                        );
                        match conn {
                            Some(conn) => {
                                bindings.insert(m.id, conn);
                            }
                            None => {
                                bindings.remove(&m.id);
                            }
                        }
                        conn
                    };
                    if !resuming && !base.world.loaded(bx, bz) {
                        keep[slot] = !m.hostile() || m.persistent;
                        cursor += 1;
                        continue;
                    }
                    let nearest_player = nearest
                        .nearest(m.position.x, m.position.y, m.position.z)
                        .map(|p| p.position);
                    // `MobDespawnEvent`, before `checkDespawn`'s own draws; one branch
                    // with no listener. Cleared below: the tick may not reach the check.
                    // The wither overrides `checkDespawn` without `super`, so the
                    // event, patched into `Mob.checkDespawn`, never fires for it;
                    // a mannequin is no `Mob` and never checks despawning.
                    if !resuming
                        && !retried
                        && mod_api::__private::listened(mod_api::HookId::MobDespawn)
                        && !matches!(
                            m.species(),
                            horizon_client::mobs::Species::Wither
                                | horizon_client::mobs::Species::Mannequin
                        )
                        && let Some(epoch) = self.native_lifecycle.epoch()
                        && let Err(payload) =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                self.native_mob_despawn(
                                    self.dim.id(),
                                    mod_api::MobRef::new(mod_api::EntityRef::new(epoch, m.id)),
                                    &m.kind.id,
                                    [m.position.x, m.position.y, m.position.z],
                                    &mut m.despawn,
                                );
                            }))
                    {
                        m.despawn = mod_api::Despawn::default();
                        dispatch_panic = Some(payload);
                        break;
                    }
                    // ServerLevel.tick: every loaded entity checks despawning, but
                    // only those in entity-ticking chunks (within the simulation
                    // distance) tick. The despawn check reads the nearest player
                    // and the difficulty, not the player the mob sees.
                    if !resuming && !ticking(bx, bz) {
                        let w = MobWorld {
                            others: &snapshot,
                            others_grid: Some(&grid),
                            others_index: Some(&index),
                            others_villagers: Some(&villager_slots),
                            others_solid: Some(&solid_slots),
                            nearest_player,
                            player: None,
                            teams: std::sync::Arc::clone(&base.teams),
                            section_writer: None,
                            ..base
                        };
                        let kept = horizon_client::mobs::ai::despawn_check(m, &w);
                        m.despawn = mod_api::Despawn::default();
                        if !kept {
                            died.remove(&m.id);
                            snapshot[slot].alive = false;
                        }
                        keep[slot] = kept;
                        cursor += 1;
                        continue;
                    }
                    let controlling_player_movement = horizon_client::mobs::equine::controlled(m)
                        .then(|| {
                            base.player
                                .as_ref()
                                .filter(|p| p.riding_vehicle == Some(m.id))
                                .map(|p| p.velocity)
                                .or_else(|| {
                                    guests
                                        .iter()
                                        .find(|(_, p)| p.riding_vehicle == Some(m.id))
                                        .map(|(_, p)| p.velocity)
                                })
                        })
                        .flatten();
                    // A mod canceled its tick: it stays as it is.
                    if !resuming && round.canceled(m.id) {
                        m.despawn = mod_api::Despawn::default();
                        cursor += 1;
                        continue;
                    }
                    if !resuming
                        && !retried
                        && m.extra.mount.is_some_and(|vehicle| {
                            self.persist.removed_vehicles.contains_key(&vehicle)
                        })
                    {
                        retry_rider = Some(m.id);
                        break;
                    }
                    // getCurrentDifficultyAt(blockPosition()): the mob's own chunk,
                    // not the local player's (a server thread has no player).
                    let chunk_key = (bx >> 4, bz >> 4);
                    let difficulty = match cached_difficulty {
                        Some((key, difficulty)) if key == chunk_key => difficulty,
                        _ => {
                            let difficulty =
                                self.local_difficulty([m.position.x, m.position.y, m.position.z]);
                            cached_difficulty = Some((chunk_key, difficulty));
                            difficulty
                        }
                    };
                    horizon_client::mobs::apply_mob_pushes(m, &mut pushes);
                    horizon_client::mobs::blocks_changed(m, &path_changes);
                    if !resuming {
                        // LivingEntity.aiStep -> EntityEquipment.tick, before movement.
                        super::super::inventory_tick::equipment_inventory_tick(
                            &self.world,
                            self.res.facts(),
                            level_id,
                            &mut m.extra.equipment,
                        );
                    }
                    let from = if resuming { resume_from } else { events.len() };
                    // The guest's view is lent to the mob for its tick, not copied:
                    // the tick only reads it (`&MobWorld`), and it goes back to the
                    // guest's slot straight after. Its identity is captured with
                    // it, once: the player a goal resolves is the one the tick
                    // sees — a guest on a server, the game's own otherwise (#1077).
                    let lent = seen_guest.and_then(|conn| guests.iter().position(|g| g.0 == conn));
                    let active_player = match (seen_guest, lent) {
                        (None, _) => base.player.take(),
                        (Some(_), Some(k)) => spare
                            .take()
                            .map(|stand_in| std::mem::replace(&mut guests[k].1, stand_in)),
                        (Some(_), None) => None,
                    };
                    let active_identity = match seen_guest {
                        None => base.player_identity,
                        Some(conn) => self.guest_identity(conn),
                    };
                    // A listener can redirect to any real player in this level. Keep
                    // immutable views for this tick so changing selection never
                    // invalidates a view a goal already borrowed. No allocation is
                    // made when nobody listens for target changes.
                    let selection =
                        if mod_api::__private::listened(mod_api::HookId::LivingChangeTarget) {
                            use horizon_client::mobs::player_selection::{
                                PlayerSelection, TickPlayer,
                            };
                            let mut players = Vec::new();
                            if let (Some(view), Some((_, entity_id))) =
                                (&active_player, active_identity)
                            {
                                players.push(TickPlayer {
                                    entity_id,
                                    connection: seen_guest,
                                    view: view.clone(),
                                });
                            }
                            if seen_guest.is_some()
                                && let (Some(view), Some((_, entity_id))) =
                                    (&base.player, base.player_identity)
                            {
                                players.push(TickPlayer {
                                    entity_id,
                                    connection: None,
                                    view: view.clone(),
                                });
                            }
                            for (index, (conn, view)) in guests.iter().enumerate() {
                                if Some(index) == lent {
                                    continue;
                                }
                                if let Some((_, entity_id)) = self.guest_identity(*conn) {
                                    players.push(TickPlayer {
                                        entity_id,
                                        connection: Some(*conn),
                                        view: view.clone(),
                                    });
                                }
                            }
                            PlayerSelection::new(players, active_identity.map(|(_, id)| id))
                        } else {
                            None
                        };

                    let mut w = MobWorld {
                        others: &snapshot,
                        others_grid: Some(&grid),
                        others_index: Some(&index),
                        others_villagers: Some(&villager_slots),
                        others_solid: Some(&solid_slots),
                        difficulty,
                        nearest_player,
                        player: active_player,
                        player_identity: active_identity,
                        player_selection: selection.as_ref(),
                        teams: std::sync::Arc::clone(&base.teams),
                        section_writer: None,
                        ..base
                    };
                    w.section_writer = live_section_observer.clone();
                    w.controlling_player_movement = controlling_player_movement;
                    let credit_expiring = if resuming {
                        resume_credit_expiring
                    } else {
                        m.ai.last_hurt_by_player == 1
                    };
                    ticked.insert(m.id);
                    // `horizon-report`'s cost by mob type (tick_profile.rs).
                    let sampled = if resuming {
                        horizon_threads::work::set_entity(horizon_threads::work::entity_id(
                            &mobs[i].kind.id,
                        ));
                        resume_sampled.take()
                    } else {
                        super::super::tick_profile::start_mob(&mobs[i].kind)
                    };
                    let run_tick = || {
                        if resuming {
                            resuming = false;
                            horizon_client::mobs::ai::TickProgress::Complete(
                                horizon_client::mobs::riders::resume_after_consumption(
                                    &mut mobs,
                                    i,
                                    &w,
                                    &mut level,
                                    resume_was_on_fire,
                                    &mut events,
                                ),
                            )
                        } else {
                            horizon_client::mobs::riders::tick_until_consumption(
                                &mut mobs,
                                i,
                                &w,
                                &mut level,
                                &mut events,
                            )
                        }
                    };
                    let progress =
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run_tick)) {
                            Ok(progress) => progress,
                            Err(payload) => {
                                mobs[i].despawn = mod_api::Despawn::default();
                                if seen_guest.is_none() {
                                    base.player = w.player;
                                } else if let Some(k) = lent {
                                    spare =
                                        w.player.map(|v| std::mem::replace(&mut guests[k].1, v));
                                }
                                horizon_client::mobs::take_mob_pushes(
                                    &mut events,
                                    from,
                                    &mut pushes,
                                );
                                route_player_events(
                                    &mut events,
                                    from,
                                    seen_guest,
                                    &guest_ids,
                                    &mut guest_events,
                                    mobs[i].id,
                                );
                                dispatch_panic = Some(payload);
                                break;
                            }
                        };
                    let kept = match progress {
                        horizon_client::mobs::ai::TickProgress::Complete(kept) => kept,
                        horizon_client::mobs::ai::TickProgress::Consumption {
                            decision,
                            was_on_fire,
                        } => {
                            // Return every player loan before ending the world borrow.
                            if seen_guest.is_none() {
                                base.player = w.player;
                            } else if let Some(k) = lent
                                && let Some(player) = w.player
                            {
                                guests[k].1 = player;
                            }
                            resume_from = from;
                            resume_credit_expiring = credit_expiring;
                            resume_was_on_fire = was_on_fire;
                            resume_sampled = sampled;
                            resume_guest = seen_guest;
                            pause = Some(decision);
                            horizon_threads::work::clear_work();
                            break;
                        }
                    };
                    super::super::tick_profile::end_mob(level_id, sampled);
                    let m = &mut mobs[i];
                    // Its `xo` (`previous`) is where its tick began, unless a teleport in the
                    // tick (`snapTo`) moved it along: that teleport, then where the tick left
                    // it, are two position updates (`EntityInLevelCallback.onMove` for each).
                    if round.sections() {
                        let captured = crate::mobs::living::section_writes::drain_for(
                            m.id,
                            &mut section_moves,
                        );
                        if kept && !m.removed {
                            let changed = captured != 0 || m.previous != m.position;
                            if changed && captured == 0 {
                                // A position changed before the lifted tick (or a raw test fixture)
                                // still has the old filed section as its first write.
                                section_moves.push(
                                    crate::mobs::living::section_writes::PositionWrite::baseline(
                                        m.id, m.join, m.previous,
                                    ),
                                );
                            }
                            if changed && captured == 0 {
                                section_moves.push(
                                    crate::mobs::living::section_writes::PositionWrite::captured(
                                        m.id, m.join, m.previous, m.position,
                                    ),
                                );
                                section_yield = true;
                            }
                        }
                    }
                    if let Some(selection) = &selection
                        && selection.has_switched()
                        && let Some(player) = selection.selected()
                    {
                        if let Some(conn) = player.connection {
                            bindings.insert(m.id, conn);
                        } else {
                            bindings.remove(&m.id);
                        }
                    }
                    m.despawn = mod_api::Despawn::default();
                    if credit_expiring && m.ai.last_hurt_by_player == 0 {
                        player_credit.remove(&m.id);
                    }
                    if seen_guest.is_none() {
                        base.player = w.player;
                    } else if let Some(k) = lent {
                        spare = w.player.map(|v| std::mem::replace(&mut guests[k].1, v));
                    }
                    // A killed mob's body is removed: its effects that act on
                    // death run (`LivingEntity.remove` with KILLED).
                    if !kept && !m.alive() && !m.removed && !m.extra.effects.is_empty() {
                        corpses.push(m.clone());
                    }
                    horizon_client::mobs::take_mob_pushes(&mut events, from, &mut pushes);
                    // LivingEntity.tickDeath: after twenty ticks the body is removed.
                    if !kept && !m.removed && !m.alive() && m.death_time >= 20 {
                        let (width, height) = (f64::from(m.width()), f64::from(m.height()));
                        // 🤖 Agent: `AbstractCubeMob.remove` splits a grown cube into
                        // its smaller halves when it is removed, twenty ticks after
                        // the death. The parent leaves the list before the split
                        // runs, so its size, position, kind and persistence are
                        // captured here, as they are at the removal.
                        let split = if matches!(
                            m.species(),
                            Species::Slime | Species::MagmaCube | Species::SulfurCube
                        ) && m.extra.size > 1
                        {
                            Some(m.clone())
                        } else {
                            None
                        };
                        gone.push((m.id, m.position, width, height, split));
                    }
                    route_player_events(
                        &mut events,
                        from,
                        seen_guest,
                        &guest_ids,
                        &mut guest_events,
                        m.id,
                    );
                    // Killed during its own tick (drowning, burning, falling...).
                    if !m.alive() && !m.removed && died.insert(m.id) {
                        dead.push(m.clone());
                    }
                    if !kept {
                        died.remove(&m.id);
                    }
                    keep[slot] = kept;
                    // ServerLevel ticks entities one after another: the ones
                    // after this mob see it as it is now.
                    snapshot[slot] = seen(m, uuid, world_items!(self));
                    snapshot[slot].alive &= kept;
                    grid.update(slot, snapshot[slot].bounds);
                    horizon_client::mobs::apply_brain_events(
                        &mut mobs,
                        &mut level,
                        &mut events,
                        from,
                    );
                    cursor += 1;
                    if section_yield {
                        break;
                    }
                }
            }
            if pause.is_none() && retry_rider.is_none() && !section_yield {
                break;
            }
            // No MobWorld, DamageLevel or navigation loan crosses this boundary.
            // The callback sees the real owning rows and level RNG/attachments,
            // and its writes are the state the continuation will resume from.
            lent.give_back(&mut self.world);
            // Earlier mobs' pushes become visible before a real pause callback
            // can read or replace another actor's velocity. A section-only
            // yield must not change the normal end-of-pass push ordering.
            if pause.is_some() || retry_rider.is_some() {
                for m in &mut mobs {
                    horizon_client::mobs::apply_mob_pushes(m, &mut pushes);
                }
            }
            mobs.retain(|m| {
                index
                    .get(&m.id)
                    .and_then(|&slot| keep.get(slot as usize))
                    .copied()
                    .unwrap_or(true)
            });
            self.mobs
                .restore(mobs, horizon_client::join_seq::RestoreOrder::SpawnsFirst);
            std::mem::swap(&mut self.last_hurt_by_player, &mut player_credit);
            std::mem::swap(&mut self.died, &mut died);
            std::mem::swap(&mut self.creatures.ticked, &mut ticked);
            if let Some(host) = self.lan.as_mut() {
                std::mem::swap(&mut host.mob_players, &mut bindings);
            }
            let callback_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if section_yield && let Some(sections) = section_table.as_ref() {
                    self.entity_sections_ticked_live(
                        &mut sections.borrow_mut(),
                        &mut section_moves,
                    );
                }
                if let Some(id) = retry_rider {
                    self.dismount_mob(id);
                } else if let Some(decision) = pause.as_ref() {
                    self.apply_fox_consumption(decision);
                }
            }))
            .err();
            if retry_rider.is_some() {
                blocked = std::borrow::Cow::Owned(self.mobs_below_off_level());
            }
            mobs = self.mobs.lift();
            std::mem::swap(&mut self.last_hurt_by_player, &mut player_credit);
            std::mem::swap(&mut self.died, &mut died);
            std::mem::swap(&mut self.creatures.ticked, &mut ticked);
            if let Some(host) = self.lan.as_mut() {
                std::mem::swap(&mut host.mob_players, &mut bindings);
            }
            if callback_panic.is_some() {
                lent = horizon_client::health::LentLevel::take(&mut self.world);
                dispatch_panic = callback_panic;
                break;
            }
            guests = self.lan_guest_views();
            guest_ids = self
                .lan
                .as_ref()
                .map(|host| {
                    host.hub
                        .guests
                        .iter()
                        .map(|g| (g.conn, g.entity_id as u32))
                        .collect()
                })
                .unwrap_or_default();
            spare = guests.first().map(|g| g.1.clone());
            let current: horizon_client::fast_hash::FxHashMap<u32, usize> =
                mobs.iter().enumerate().map(|(i, m)| (m.id, i)).collect();
            for (slot, s) in snapshot.iter_mut().enumerate() {
                if let Some(&i) = current.get(&s.id) {
                    *s = seen(&mobs[i], uuid, world_items!(self));
                    s.alive &= keep.get(slot).copied().unwrap_or(true);
                } else {
                    s.alive = false;
                }
            }
            for m in &mobs {
                if let std::collections::hash_map::Entry::Vacant(entry) = index.entry(m.id) {
                    let slot = snapshot.len();
                    entry.insert(slot as u32);
                    snapshot.push(seen(m, uuid, world_items!(self)));
                }
            }
            villager_slots.clear();
            solid_slots.clear();
            for (slot, s) in snapshot.iter().enumerate() {
                if s.species == Species::Villager {
                    villager_slots.push(slot as u32);
                }
                if s.may_block_mobs() {
                    solid_slots.push(slot as u32);
                }
            }
            grid =
                horizon_client::mobs::seen_grid::SeenGrid::new(snapshot.iter().map(|s| s.bounds));
            positions = Some(current);
            if let Some(writes) = self.world.path_writes.as_mut() {
                path_writes.append(writes);
            }
            lent = horizon_client::health::LentLevel::take(&mut self.world);
            resuming = retry_rider.is_none() && !section_yield;
            retried_rider = retry_rider;
        }
        laps.lap("creature_loop");
        if dispatch_panic.is_none() {
            if !self.persist.off_level.is_empty() {
                let mounted: std::collections::HashSet<_> = mobs
                    .iter()
                    .chain(self.mob_objects())
                    .filter_map(|passenger| passenger.extra.mount)
                    .collect();
                for mob in &mobs {
                    let kept = index
                        .get(&mob.id)
                        .and_then(|&slot| keep.get(slot as usize))
                        .copied()
                        .unwrap_or(true);
                    if !kept && mounted.contains(&mob.id) {
                        let mut removed = mob.clone();
                        removed.removed = true;
                        self.persist
                            .off_level
                            .insert(mob.id, crate::game::persist::OffLevel::Mob(removed));
                    }
                }
            }
            if positions.is_none() {
                let mut flags = keep.into_iter();
                mobs.retain(|_| flags.next().unwrap_or(true));
            } else {
                mobs.retain(|m| {
                    index
                        .get(&m.id)
                        .and_then(|&slot| keep.get(slot as usize))
                        .copied()
                        .unwrap_or(true)
                });
            }
            for m in &mut mobs {
                horizon_client::mobs::apply_mob_pushes(m, &mut pushes);
            }
        }
        crate::mobs::living::section_writes::finish();
        drop(live_section_observer);
        // Keep the reusable write storage for the next pass; the live-row callback has already
        // consumed every record that belonged to this family tick.
        section_moves.clear();
        if let Some(table) = section_table {
            let mut sections = std::rc::Rc::try_unwrap(table)
                .ok()
                .expect("live section observer still held");
            sections.get_mut().ticked = section_moves;
            self.native_lifecycle.entity_sections = sections.into_inner();
        } else {
            self.native_lifecycle.entity_sections.ticked = section_moves;
        }
        *state.pushes = pushes;
        *state.mobs = mobs;
        *state.snapshot = snapshot;
        *state.events = events;
        *state.dead = dead;
        *state.corpses = corpses;
        *state.died = died;
        *state.ticked = ticked;
        *state.guests = guests;
        *state.spare = spare;
        *state.bindings = bindings;
        *state.guest_ids = guest_ids;
        *state.guest_events = guest_events;
        *state.player_credit = player_credit;
        *state.path_writes = path_writes;
        *state.gone = gone;
        (lent, dispatch_panic)
    }
}

impl Game<'_> {
    /// The killing blow of a mob as the death hooks name it: its damage
    /// type and the causing entity's id (0 for none).
    pub(in crate::game) fn mob_damage_source(&self, m: &Mob) -> (&'static str, u32) {
        let (damage_type, attacker) = match m.last_damage {
            Some((d, _)) => (source_damage_type(d), d.attacker),
            None => ("minecraft:generic", None),
        };
        let attacker = match attacker {
            Some(Who::Mob(id)) => id,
            Some(Who::Player) => self
                .last_hurt_by_player
                .get(&m.id)
                .and_then(|credit| self.resolve_player_credit(*credit))
                .and_then(|conn| self.player_entity_id_by_connection(conn))
                .map_or(0, |id| id as u32),
            None => 0,
        };
        (damage_type, attacker)
    }
}

impl Game<'_> {
    pub(super) fn retained_passenger_snapshot(&self, uuid: Option<[i32; 4]>) -> Vec<Seen> {
        if self.persist.off_level.is_empty() {
            Vec::new()
        } else {
            self.persist
                .off_level
                .values()
                .filter_map(crate::game::persist::OffLevel::as_mob)
                .map(|m| seen(m, uuid, self.items()))
                .collect()
        }
    }
}

#[cfg(test)]
mod off_level_tests {
    use super::*;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn borrowed_tick_resolves_the_retained_controller() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            use horizon_client::mobs::goals::{JUMP, LOOK, MOVE};
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let spider = game
                .res
                .mob_types()
                .get("minecraft:spider")
                .unwrap()
                .clone();
            let skeleton = game
                .res
                .mob_types()
                .get("minecraft:skeleton")
                .unwrap()
                .clone();
            let at = Vec3::new(1.0, 80.0, 0.5);
            let mut vehicle = Mob::new(71, spider, at, 0.0);
            vehicle.extra.rider = Some(72);
            let mut rider = Mob::new(72, skeleton, at, 0.0);
            rider.extra.mount = Some(71);
            rider.age = 19;
            game.mobs.push(vehicle);
            game.persist
                .off_level
                .insert(72, crate::game::persist::OffLevel::Mob(rider));
            game.tick_creature_entities().unwrap();
            assert_eq!(
                game.mob_object(71).unwrap().ai.goals.disabled & (MOVE | JUMP | LOOK),
                MOVE | JUMP | LOOK
            );
            assert_eq!(game.mob_object(72).unwrap().age, 19);
            assert_eq!(game.mobs.len(), 1);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn resident_descendant_does_not_tick_until_detached() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut parent = Mob::new(72, pig.clone(), Vec3::new(1.0, 80.0, 0.5), 0.0);
            parent.extra.mount = Some(71);
            let mut child = Mob::new(73, pig, parent.position, 0.0);
            child.extra.mount = Some(72);
            child.no_ai = true;
            child.age = 19;
            child.fire_ticks = 20;
            game.persist
                .off_level
                .insert(72, crate::game::persist::OffLevel::Mob(parent));
            game.mobs.push(child);
            game.tick_creature_entities().unwrap();
            let child = game.mob_object(73).unwrap();
            assert_eq!(child.age, 19);
            assert_eq!(child.fire_ticks, 20);
            game.mob_object_mut(73).unwrap().extra.mount = None;
            game.tick_creature_entities().unwrap();
            assert_eq!(game.mob_object(73).unwrap().age, 20);
        });
    }
}

#[cfg(test)]
mod hoglin_routing_tests {
    use super::*;

    #[test]
    fn broadcast_keeps_source_player_connection() {
        for (seen_guest, tag, expected) in [
            (Some(12), None, Some(12)),
            (None, None, None),
            (Some(12), Some(Some(13)), Some(13)),
            (Some(12), Some(None), None),
        ] {
            let event = MobEvent::HoglinAttackTarget {
                mob: 9001,
                target: Who::Player,
                player: None,
            };
            let mut events = vec![match tag {
                Some(conn) => MobEvent::ForPlayer {
                    conn,
                    event: Box::new(event),
                },
                None => event,
            }];
            route_player_events(&mut events, 0, seen_guest, &[], &mut Vec::new(), 9000);
            assert!(matches!(events[0], MobEvent::HoglinAttackTarget {
                player, ..
            } if player == expected));
        }
    }
}
