//! Dynamic activation of brain (DAB), an opt-in server setting (`server.toml` `[entities]`, off by
//! default): a mob farther from the nearest player runs its AI on fewer ticks, like Pufferfish's
//! setting of that name. This is not vanilla behaviour, so nothing here runs unless the host turns
//! it on (`HORIZON_DAB=1`, set by `horizon-server` from `[entities] dab`).
//!
//! Only the AI decisions are skipped (`Mob.serverAiStep`: target and goal selectors, the brains,
//! `customServerAiStep`). Navigation, move, look and jump controls, gravity, fluids, pushes, damage,
//! effects, despawn checks, riding and leashes tick as always, and the mob's own random draws of the
//! skipped decisions are not made.
use super::{Mob, MobEvent, MobWorld, Species};
use horizon_core::geometry::Vec3;

/// The four settings. `start` is in blocks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dab {
    start_squared: i64,
    max_tick_freq: i64,
    dist_mod: u32,
}

impl Dab {
    /// Pufferfish's defaults are 12, 20 and 8. A frequency below 1 counts as 1, a shift past 62 as 62.
    pub fn new(start_distance: f64, max_tick_freq: i64, activation_dist_mod: u32) -> Self {
        let start = start_distance.max(0.0);
        Self {
            start_squared: (start * start) as i64,
            max_tick_freq: max_tick_freq.max(1),
            dist_mod: activation_dist_mod.min(62),
        }
    }

    /// How many ticks pass between two AI ticks of a mob whose squared distance to the nearest
    /// player is `distance_squared`: 1 within the start distance, else `d² >> mod` between 1 and
    /// the maximum (Pufferfish's formula, on whole squared distances).
    pub fn interval(&self, distance_squared: f64) -> i64 {
        let d2 = distance_squared as i64; // saturates for huge and NaN values
        if d2 <= self.start_squared {
            return 1;
        }
        (d2 >> self.dist_mod).clamp(1, self.max_tick_freq)
    }
}

/// The host's setting: `HORIZON_DAB` on (`1`, `on`, `true`), with `HORIZON_DAB_START_DISTANCE`,
/// `HORIZON_DAB_MAX_TICK_FREQ` and `HORIZON_DAB_ACTIVATION_DIST_MOD` over the defaults. Read once.
pub fn settings() -> Option<Dab> {
    static ON: std::sync::OnceLock<Option<Dab>> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let var = |name: &str| std::env::var(name).ok();
        if !matches!(var("HORIZON_DAB").as_deref(), Some("1" | "on" | "true")) {
            return None;
        }
        let num = |name: &str, default: f64| {
            var(name)
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|v| v.is_finite())
                .unwrap_or(default)
        };
        Some(Dab::new(
            num("HORIZON_DAB_START_DISTANCE", 12.0),
            num("HORIZON_DAB_MAX_TICK_FREQ", 20.0) as i64,
            num("HORIZON_DAB_ACTIVATION_DIST_MOD", 8.0) as u32,
        ))
    })
}

/// A mob hurt this recently keeps its full AI: ~2 s (`Mob.getLastHurtByMobTimestamp`).
const HURT_TICKS: i32 = 40;

/// Whether the mob's AI always runs, however far the player is. Each one keeps a behaviour a
/// player sees or relies on at any distance:
/// - Wither and warden: bosses whose fights and anger would stutter.
/// - A target or anger: it is hunting or being hunted, and a slowed attack is a visible lag.
/// - Hurt in the last ~2 s: it flees, retaliates or calls for help at once.
/// - In a fluid deep enough to float in: `FloatGoal` and the brains' `Swim` jump every tick, or it
///   sinks, drowns or burns.
/// - In a raid: raids are balanced around raiders that act every tick.
/// - Ridden, riding, with passengers, or led: a rider's input and a lead's pull expect a reply.
/// - Tamed and not sitting: it follows or defends its owner, and teleports to them.
pub fn exempt(m: &Mob) -> bool {
    matches!(m.species(), Species::Wither | Species::Warden)
        || m.ai.target.is_some()
        || m.extra.anger > 0
        || m.extra.angry_at.is_some()
        || m.hurt_time > 0
        || m.floats()
        || (m.ai.last_hurt_by.is_some()
            && (m.age as i32).wrapping_sub(m.ai.last_hurt_by_time) <= HURT_TICKS)
        || super::raider::in_raid(m)
        || m.rider.is_some()
        || m.extra.rider.is_some()
        || m.extra.mount.is_some()
        || m.extra.leash.is_some()
        || m.extra.leash_pending.is_some()
        || m.extra.leash_player.is_some()
        || ((m.extra.tame || m.extra.owner.is_some()) && !m.extra.sitting)
}

/// Whether the mob runs its AI this tick (`tick_count % interval == 0`). Always, with the setting
/// off. A level without a player counts as far away: the longest interval.
pub fn runs_ai(m: &Mob, w: &MobWorld) -> bool {
    let Some(dab) = settings() else {
        return true;
    };
    runs_ai_with(&dab, m, w.nearest_player)
}

/// `Mob.serverAiStep` for a mob in `LivingEntity.aiStep`: all of it on an AI tick, and on the others only
/// what keeps the mob moving along its decisions of the last one.
pub(super) fn step(
    m: &mut Mob,
    w: &MobWorld,
    level_random: &mut crate::health::DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) {
    if runs_ai(m, w) {
        if settings().is_some() {
            count_turn(m);
        }
        super::ai::server_ai_step(m, w, level_random, events);
        return;
    }
    // Neither the goals and targets, nor the brains, nor `customServerAiStep`, so no draw of theirs.
    m.ai.no_action_time += 1;
    if m.species() == Species::HappyGhast {
        super::happy_ghast::ensure_setup(m, Some(w));
    }
    // Then the rest of `serverAiStep`: the navigation follows its path, the controls carry it out.
    let frozen = super::creaking::is_creaking(m) && !super::creaking::can_move(m);
    if !frozen && !super::bee::pollinating(m) {
        m.nav_tick(w);
    }
    if frozen {
        m.ai.jumping = false;
        return;
    }
    if !super::axolotl::is_playing_dead(m) {
        m.move_control_tick(w, events);
        if !(m.species() == Species::Bee && super::bee::angry(m, w)) {
            m.look_control_tick();
        }
    }
    m.jump_control_tick();
}

/// The tick `Mob.serverAiStep` alternates its full and partial selector updates by (`tickCount`,
/// plus the id). With the setting on it counts the mob's AI turns instead (`count_turn`): a far
/// mob's turns fall on ages that are all even at an even interval, which would leave an odd-id mob
/// on the partial update forever.
pub(super) fn selector_tick(m: &Mob) -> i64 {
    if m.ai.dab_turns == 0 {
        i64::from(m.age)
    } else {
        i64::from(m.ai.dab_turns)
    }
}

fn count_turn(m: &mut Mob) {
    m.ai.dab_turns = m.ai.dab_turns.checked_add(1).unwrap_or(1);
}

fn runs_ai_with(dab: &Dab, m: &Mob, nearest: Option<Vec3>) -> bool {
    let distance_squared = nearest.map_or(f64::INFINITY, |p| p.distance_squared(m.position));
    let interval = dab.interval(distance_squared);
    // The exemptions are looked up only for a mob that would skip.
    interval == 1 || i64::from(m.age) % interval == 0 || exempt(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::Who;
    use std::sync::Arc;

    const DEFAULTS: Dab = Dab {
        start_squared: 144,
        max_tick_freq: 20,
        dist_mod: 8,
    };

    #[test]
    fn interval_follows_pufferfishs_formula() {
        let d = |blocks: f64| DEFAULTS.interval(blocks * blocks);
        assert_eq!(d(5.0), 1);
        assert_eq!(d(12.0), 1); // at the start distance
        assert_eq!(d(13.0), 1); // 169 >> 8 = 0, raised to 1
        assert_eq!(d(60.0), 14); // 3600 >> 8
        assert_eq!(d(200.0), 20); // 40000 >> 8 = 156, capped
        assert_eq!(DEFAULTS.interval(f64::INFINITY), 20);
        assert_eq!(Dab::new(12.0, 0, 99).interval(1e9), 1);
    }

    /// A data-free evoker (`sensor_scan_tests::kind`) `distance` blocks from the nearest player.
    fn evoker() -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let def = Arc::make_mut(&mut kind);
        def.id = "minecraft:evoker".into();
        def.species = Species::Evoker;
        Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0)
    }
    fn deciding(m: &mut Mob, distance: f64) -> i32 {
        let nearest = Some(Vec3::new(0.5 + distance, 64.0, 0.5));
        (0..280)
            .filter(|&tick| {
                m.age = tick;
                runs_ai_with(&DEFAULTS, m, nearest)
            })
            .count() as i32
    }

    #[test]
    fn a_far_mob_runs_its_ai_one_tick_in_n_and_a_near_one_every_tick() {
        let mut m = evoker();
        // 60 blocks: interval 14, so ticks 0, 14, ..., 266 of 280.
        assert_eq!(deciding(&mut m, 60.0), 20);
        assert_eq!(deciding(&mut m, 5.0), 280);
    }

    /// Every AI turn over 280 ticks, as `step` would number them: whether its selectors update in full.
    fn full_selector_turns(m: &mut Mob, distance: f64) -> Vec<bool> {
        let nearest = Some(Vec3::new(0.5 + distance, 64.0, 0.5));
        let mut turns = Vec::new();
        for age in 0..280 {
            m.age = age;
            if runs_ai_with(&DEFAULTS, m, nearest) {
                count_turn(m);
                turns.push((selector_tick(m) + i64::from(m.id)) % 2 == 0 || m.age <= 1);
            }
        }
        turns
    }

    #[test]
    fn an_odd_id_far_mob_still_gets_full_selector_updates() {
        let mut m = evoker(); // id 1
        // 60 blocks: interval 14, every AI turn on an even age.
        let turns = full_selector_turns(&mut m, 60.0);
        assert_eq!(turns.len(), 20);
        assert_eq!(turns.iter().filter(|full| **full).count(), 10);
    }

    #[test]
    fn a_far_mob_swimming_keeps_its_ai_every_tick() {
        let mut m = evoker();
        m.fluids.in_lava = true;
        assert_eq!(deciding(&mut m, 60.0), 280);
    }

    #[test]
    fn a_mob_with_a_target_keeps_its_ai_every_tick() {
        let mut m = evoker();
        m.ai.target = Some(Who::Player);
        assert_eq!(deciding(&mut m, 60.0), 280);
    }
}
