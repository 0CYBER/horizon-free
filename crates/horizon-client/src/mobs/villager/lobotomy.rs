//! `server.toml` `[entities] villager_lobotomize` (docs/server.md): a villager that cannot move
//! skips its brain. Vanilla has no such rule (Purpur's `villager.lobotomize` is the model); off by
//! default, it changes nothing.
//!
//! A villager is *stuck* when it rides something, or when no horizontal neighbour is walkable (see
//! [`is_stuck`]). The rule is rechecked every `check_interval` ticks, staggered by the villager's id
//! so a hall of villagers does not all recheck on one tick. A stuck villager skips
//! `Villager.customServerAiStep`'s brain (sensors, behaviours, activity changes, gossip, sleep,
//! farming, breeding, panic, POI searches), but keeps gravity, damage, despawn, trading and its XP,
//! and `Villager.restock` on `WorkAtPoi`'s schedule (see [`skips_brain`]).
use super::super::villager_brain;
use super::super::{Mob, MobWorld};
use horizon_core::geometry::Vec3;
use horizon_core::random::LegacyRandom;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);
static INTERVAL: AtomicU32 = AtomicU32::new(DEFAULT_INTERVAL);

/// `villager_lobotomize_check_interval`'s default, in ticks.
pub const DEFAULT_INTERVAL: u32 = 100;
/// `WorkAtPoi`: a villager that works tries at most every 300 ticks.
const RESTOCK_EVERY: i64 = 300;

/// Sets the server's `[entities]` settings (the dedicated server reads them at start).
pub fn configure(enabled: bool, check_interval: u32) {
    ENABLED.store(enabled, Ordering::Relaxed);
    INTERVAL.store(check_interval.max(1), Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The most cells a stuck villager can reach. A trading hall's cell often holds a bed or a
/// workstation's neighbour, so "no free neighbour" is too strict: a villager that can only shuffle
/// around a small pocket is as good as stuck.
pub const MAX_ROOM: usize = 6;

/// Whether the villager cannot go anywhere: it rides something (`isPassenger`), or the cells it can
/// walk to from where it stands (see [`room_exceeds`]) number at most [`MAX_ROOM`]. A sleeping
/// villager is never stuck: the brain has to wake it. A trapdoor or slab over its head does not
/// matter; only the room around it does.
pub fn is_stuck(m: &Mob, w: &MobWorld) -> bool {
    if m.extra.mount.is_some() {
        return true;
    }
    !super::sleeping(m) && !room_exceeds(m, w, MAX_ROOM)
}

/// Whether the villager can reach more than `cap` cells, or leave the pocket at all. A search from
/// its box over the four horizontal moves, as its navigation takes them: to a cell its box fits in,
/// then down while it falls (a drop of more than 3 blocks, or a way out through a door villagers
/// open (`mob_interactable_doors`), counts as leaving), or one block up when there is room above
/// both cells (a jump). Beds (0.56 high) and slabs are cells it can step onto; walls, fences and
/// closed trapdoors are not.
fn room_exceeds(m: &Mob, w: &MobWorld, cap: usize) -> bool {
    let own = m.bounds();
    let at =
        |c: (i32, i32, i32)| own.move_by(Vec3::new(f64::from(c.0), f64::from(c.1), f64::from(c.2)));
    let feet = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    let mut seen = vec![(0, 0, 0)];
    // Cells it stands on a half block (a bed, a slab) in: one block up from there is not a jump.
    let mut low = std::collections::HashSet::new();
    let mut next = 0;
    while next < seen.len() {
        let cell = seen[next];
        next += 1;
        let head_room = !low.contains(&cell) && w.no_collision(at((cell.0, cell.1 + 1, cell.2)));
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let side = (cell.0 + dx, cell.1, cell.2 + dz);
            let door = [0, 1].iter().any(|dy| {
                let name = w.block_name(feet[0] + side.0, feet[1] + side.1 + dy, feet[2] + side.2);
                name.ends_with("_door") && w.has_tag("minecraft:mob_interactable_doors", name)
            });
            if door {
                return true;
            }
            let mut to = if w.no_collision(at(side)) {
                Some(side)
            } else if head_room && w.no_collision(at((side.0, side.1 + 1, side.2))) {
                Some((side.0, side.1 + 1, side.2))
            } else {
                None
            };
            if let Some(c) = &mut to {
                // Falls to the floor: a drop of more than 3 blocks leaves the pocket.
                let mut drop = 0;
                while w.no_collision(at((c.0, c.1 - 1, c.2))) {
                    c.1 -= 1;
                    drop += 1;
                    if drop > 3 {
                        return true;
                    }
                }
                // Standing higher than where it started, on a surface at least a quarter block below the cell's
                // top: a bed or a slab, not a full step.
                let quarter = Vec3::new(0.0, -0.25, 0.0);
                if c.1 > 0 && w.no_collision(at(*c).move_by(quarter)) {
                    low.insert(*c);
                }
                if !seen.contains(c) {
                    seen.push(*c);
                    if seen.len() > cap {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// `Brain.tick` for a villager, unless it is stuck (see [`skips_brain`]).
pub fn brain_tick(
    m: &mut Mob,
    w: &MobWorld,
    lr: &mut LegacyRandom,
    events: &mut Vec<super::super::MobEvent>,
) {
    if !skips_brain(m, w) {
        super::tick_brain(m, w, lr, events);
    }
}

/// `WorkAtPoi.start`'s restock for a villager whose brain is off: the job site it remembers (the
/// brain's memory, else what it was saved or loaded with), if it is still a workstation of its own
/// profession (`PoiFilter::Held`) and within `WorkAtPoi`'s 1.73 blocks of the villager.
fn restock_at_job_site(m: &mut Mob, w: &MobWorld) {
    let site = match villager_brain::mem_ref(m) {
        Some(mem) => mem.job_site.value(),
        None => match super::ext(m).and_then(|e| e.saved_memories.as_deref()) {
            Some(saved) => saved.job_site.value(),
            None => m.extra.home,
        },
    };
    let own_job_site = |p: [i32; 3]| {
        matches!(villager_brain::poi_at(w, p[0], p[1], p[2]),
            Some(villager_brain::Poi::Job(j)) if j == m.extra.variant2)
            && villager_brain::closer_to_center(p, m.position, 1.73)
    };
    if site.is_some_and(own_job_site) {
        villager_brain::restock_if_due(m, w);
    }
}

/// Whether this tick's brain is skipped. Rechecks the villager when its turn comes (its id staggers
/// the turns), and restocks a stuck villager that has a job site on `WorkAtPoi`'s schedule.
pub fn skips_brain(m: &mut Mob, w: &MobWorld) -> bool {
    if !enabled() {
        // Off, or turned off: forget the state, so a later start rechecks at once.
        if let Some(e) = m.extra.villager.as_deref_mut() {
            e.lobotomized = None;
        }
        return false;
    }
    step(m, w, i64::from(INTERVAL.load(Ordering::Relaxed)))
}

/// `skips_brain` with the setting on, rechecking every `interval` ticks.
fn step(m: &mut Mob, w: &MobWorld, interval: i64) -> bool {
    let phase = (w.game_time + i64::from(m.id)) % interval == 0;
    let stuck = match super::ext(m).and_then(|e| e.lobotomized) {
        Some(stuck) if !phase => stuck,
        _ => {
            let stuck = is_stuck(m, w);
            super::ext_mut(m).lobotomized = Some(stuck);
            stuck
        }
    };
    if stuck && (w.game_time + i64::from(m.id)) % RESTOCK_EVERY == 0 {
        // The brain is off, so `WorkAtPoi` never runs: do its restock here, in the same window
        // (an adult's work hours) and with a job site that is still there, without the walk to it.
        let works = super::activity_at(m.is_baby(), w.day_time) == super::Activity::Work;
        if works {
            restock_at_job_site(m, w);
        }
    }
    stuck
}

#[cfg(test)]
#[path = "lobotomy_tests.rs"]
mod tests;
