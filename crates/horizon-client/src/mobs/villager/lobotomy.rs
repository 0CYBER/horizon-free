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
use horizon_core::geometry::{Aabb, Vec3};
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

/// Whether the villager cannot move: it rides something (`isPassenger`), or none of the four blocks
/// around its feet lets it in. A neighbour lets it in when its own box fits there at the same height
/// (a step down or a drop included), or one block higher with room above itself (a jump up), or
/// when a door villagers open (`mob_interactable_doors`) is in the way. A sleeping villager is
/// never stuck: the brain has to wake it. A trapdoor or slab over its head does not matter; only
/// the room around it does.
pub fn is_stuck(m: &Mob, w: &MobWorld) -> bool {
    if m.extra.mount.is_some() {
        return true;
    }
    if super::sleeping(m) {
        return false;
    }
    let own = m.bounds();
    let up = |b: Aabb| b.move_by(Vec3::new(0.0, 1.0, 0.0));
    // The room to jump up from here: its box one block higher.
    let headroom = w.no_collision(up(own));
    let feet = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    ![(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dz)| {
        let there = own.move_by(Vec3::new(f64::from(dx), 0.0, f64::from(dz)));
        if w.no_collision(there) || headroom && w.no_collision(up(there)) {
            return true;
        }
        [0, 1].iter().any(|dy| {
            let name = w.block_name(feet[0] + dx, feet[1] + dy, feet[2] + dz);
            name.ends_with("_door") && w.has_tag("minecraft:mob_interactable_doors", name)
        })
    })
}

/// `Brain.tick` for a villager, unless it is stuck (see [`skips_brain`]).
pub fn brain_tick(
    m: &mut Mob,
    w: &MobWorld,
    lr: &mut LegacyRandom,
    events: &mut Vec<super::super::MobEvent>,
) {
    if !skips_brain(m, w) {
        villager_brain::tick(m, w, lr, events);
    }
}

/// `WorkAtPoi.start`'s restock for a villager whose brain is off: the job site it remembers (the
/// brain's memory, else what it was saved or loaded with), if it is still a workstation.
fn restock_at_job_site(m: &mut Mob, w: &MobWorld) {
    let site = match villager_brain::mem_ref(m) {
        Some(mem) => mem.job_site.value(),
        None => match super::ext(m).and_then(|e| e.saved_memories.as_deref()) {
            Some(saved) => saved.job_site.value(),
            None => m.extra.home,
        },
    };
    if site.is_some_and(|p| villager_brain::poi_at(w, p[0], p[1], p[2]).is_some()) {
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
