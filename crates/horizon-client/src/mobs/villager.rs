//! Villagers (`Villager` and its brain packages, as goals): taking the
//! profession of a free job site nearby (`AcquirePoi` with
//! `AssignProfessionFromJobSite`), losing a fresh profession when the site
//! goes (`ResetProfession`), fleeing hostiles (`VillagerPanicTrigger`) and
//! the unhappy head shake.
use super::{
    Mob, MobEvent, MobWorld, Species,
    anim::{Poses, State},
    goals::{
        Avoid, AvoidEntity, Cx, Float, Goal, LookAtPlayer, MOVE, Panic, RandomLookAround, Stroll,
        goal_clone,
    },
};
use horizon_core::random::RandomSource;
use std::{collections::HashMap, f32::consts::PI};

// Villager state: `variant` type, `variant2` profession (index into
// `render::PROFESSIONS`), `variant3` level, `home` the claimed job site,
// `ints[0]` the unhappy counter, `trade` the offers and experience.

pub const NONE: u8 = 0;
pub const NITWIT: u8 = 11;

/// `PoiTypes`: the job site block of each profession.
pub fn job_site_profession(block: &str) -> Option<u8> {
    Some(match block.trim_start_matches("minecraft:") {
        "blast_furnace" => 1,
        "smoker" => 2,
        "cartography_table" => 3,
        "brewing_stand" => 4,
        "composter" => 5,
        "barrel" => 6,
        "fletching_table" => 7,
        "cauldron" | "water_cauldron" | "lava_cauldron" | "powder_snow_cauldron" => 8,
        "lectern" => 9,
        "stonecutter" => 10,
        "loom" => 12,
        "smithing_table" => 13,
        "grindstone" => 14,
        _ => return None,
    })
}

pub fn register(m: &mut Mob) {
    if m.extra.variant3 == 0 {
        m.extra.variant3 = 1;
    }
    let g = &mut m.ai.goals;
    g.add(0, Float);
    // VillagerHostilesSensor: zombies within 8 blocks, illagers further.
    let zombies = vec![
        Species::Zombie,
        Species::ZombieVillager,
        Species::Husk,
        Species::Drowned,
        Species::Vex,
    ];
    let raiders = vec![
        Species::Pillager,
        Species::Vindicator,
        Species::Evoker,
        Species::Ravager,
        Species::Zoglin,
    ];
    g.add(
        1,
        AvoidEntity::new(Avoid::Species(zombies), 8.0, 0.75, 0.75),
    );
    g.add(
        1,
        AvoidEntity::new(Avoid::Species(raiders), 12.0, 0.75, 0.75),
    );
    g.add(1, Panic::new(0.75));
    g.add(3, JobSite::default());
    g.add(5, Stroll::avoiding_water(0.5));
    g.add(6, LookAtPlayer::new(8.0));
    g.add(7, RandomLookAround::default());
}

/// `AcquirePoi` for job sites with `AssignProfessionFromJobSite`: walk to the
/// nearest unclaimed job site and take its profession once within 2 blocks.
#[derive(Clone, Debug, Default)]
struct JobSite {
    target: Option<[i32; 3]>,
    next_search: i32,
    ticks: i32,
}
fn claimed(w: &MobWorld, me: u32, p: [i32; 3]) -> bool {
    w.others
        .iter()
        .any(|s| s.id != me && s.alive && s.job == Some(p))
}
fn job_seeker(m: &Mob) -> bool {
    !m.is_baby() && m.extra.home.is_none() && m.extra.variant2 != NITWIT
}
/// The nearest free job site (a profession's own one once employed) within
/// 48 blocks horizontally and 8 vertically.
fn find_site(m: &Mob, w: &MobWorld) -> Option<[i32; 3]> {
    let (bx, by, bz) = (
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    );
    let mut known: HashMap<u32, Option<u8>> = HashMap::new();
    let mut best: Option<(i32, [i32; 3])> = None;
    for y in by - 8..=by + 8 {
        for z in bz - 48..=bz + 48 {
            for x in bx - 48..=bx + 48 {
                let state = w.world.block(x, y, z);
                if w.facts.is_air(state) {
                    continue;
                }
                let profession = *known
                    .entry(state.0)
                    .or_insert_with(|| job_site_profession(&w.facts.get(state).name));
                let Some(profession) = profession else {
                    continue;
                };
                if m.extra.variant2 != NONE && profession != m.extra.variant2 {
                    continue;
                }
                let d = (x - bx).pow(2) + (y - by).pow(2) + (z - bz).pow(2);
                if best.is_none_or(|(b, _)| d < b) && !claimed(w, m.id, [x, y, z]) {
                    best = Some((d, [x, y, z]));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}
impl Goal for JobSite {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !job_seeker(m) {
            return false;
        }
        if self.next_search > 0 {
            self.next_search -= 1;
            return false;
        }
        // AcquirePoi's scan pacing: 20 to 40 ticks between scans.
        self.next_search = 20 + m.random.next_int_bound(20).unwrap_or(0);
        self.target = find_site(m, cx.w);
        self.target.is_some()
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        job_seeker(m)
            && self.ticks < 1200
            && self.target.is_some_and(|p| {
                job_site_profession(cx.w.block_name(p[0], p[1], p[2])).is_some()
                    && !claimed(cx.w, m.id, p)
            })
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.ticks = 0;
        if let Some(p) = self.target {
            let path = m.create_path_to(cx.w, p, 1);
            m.nav_move_to_path(path, 0.5);
        }
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.nav_stop();
        self.target = None;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.ticks += 1;
        let Some(p) = self.target else { return };
        let c = [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]) + 0.5,
            f64::from(p[2]) + 0.5,
        ];
        if m.position
            .distance_squared(horizon_core::geometry::Vec3::new(c[0], c[1], c[2]))
            < 4.0
        {
            take_job(m, p, cx.w, cx.events);
            return;
        }
        if m.ai.nav.is_done() && self.ticks % 40 == 0 {
            let path = m.create_path_to(cx.w, p, 1);
            m.nav_move_to_path(path, 0.5);
        }
    }
}
/// `AssignProfessionFromJobSite`: claim the site, take its profession if
/// unemployed, and show the happy particles (entity event 14).
fn take_job(m: &mut Mob, p: [i32; 3], w: &MobWorld, events: &mut Vec<MobEvent>) {
    m.extra.home = Some(p);
    if m.extra.variant2 == NONE
        && let Some(profession) = job_site_profession(w.block_name(p[0], p[1], p[2]))
    {
        m.extra.variant2 = profession;
        m.extra.trade = None;
    }
    particles_around(m, "happy_villager", events);
}
/// `LivingEntity.addParticlesAroundSelf`: five particles about the body.
pub fn particles_around(m: &mut Mob, kind: &'static str, events: &mut Vec<MobEvent>) {
    let (w, h) = (f64::from(m.width()), f64::from(m.height()));
    for _ in 0..5 {
        let at = [
            m.position.x + w * (2.0 * m.random.next_double() - 1.0),
            m.position.y + h * m.random.next_double() + 1.0,
            m.position.z + w * (2.0 * m.random.next_double() - 1.0),
        ];
        events.push(MobEvent::Particles { kind, at, count: 0 });
    }
}
/// `Villager.tick` / brain upkeep: the unhappy counter, and
/// `ResetProfession` / job site loss when the site's block is gone.
pub fn ai_step(m: &mut Mob, w: &MobWorld) {
    if m.extra.ints[0] > 0 {
        m.extra.ints[0] -= 1;
    }
    if m.species() != Species::Villager || !m.age.is_multiple_of(20) {
        return;
    }
    let Some(p) = m.extra.home else { return };
    if !w.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4)) {
        return;
    }
    if job_site_profession(w.block_name(p[0], p[1], p[2])).is_none() {
        m.extra.home = None;
        let fresh = m.extra.trade.as_ref().is_none_or(|t| t.xp == 0);
        if m.extra.variant2 != NONE && m.extra.variant2 != NITWIT && fresh && m.extra.variant3 <= 1
        {
            m.extra.variant2 = NONE;
            m.extra.trade = None;
        }
    }
}
/// `Villager.setUnhappy`: shake the head for two seconds and say no.
pub fn set_unhappy(m: &mut Mob, events: &mut Vec<MobEvent>) {
    m.extra.ints[0] = 40;
    let sound = if m.species() == Species::WanderingTrader {
        "entity.wandering_trader.no"
    } else {
        "entity.villager.no"
    };
    events.push(MobEvent::Sound(
        sound,
        [m.position.x, m.position.y, m.position.z],
        1.0,
        1.0,
    ));
}
/// `VillagerModel.setupAnim`.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    p.yrot("head", s.y_rot.to_radians());
    p.xrot("head", s.x_rot.to_radians());
    if m.extra.ints[0] > 0 {
        p.zrot("head", 0.3 * (0.45 * s.age).sin());
        p.xrot("head", 0.4);
    } else {
        p.zrot("head", 0.0);
    }
    let leg = (s.pos * 0.6662).cos() * 1.4 * s.speed * 0.5;
    p.xrot("right_leg", leg);
    p.xrot(
        "left_leg",
        (s.pos * 0.6662 + PI).cos() * 1.4 * s.speed * 0.5,
    );
    p.yrot("right_leg", 0.0);
    p.yrot("left_leg", 0.0);
}
