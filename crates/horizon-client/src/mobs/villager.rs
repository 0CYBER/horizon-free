//! Villagers (`Villager` and its brain packages, as goals): taking the
//! profession of a free job site nearby (`AcquirePoi` with
//! `AssignProfessionFromJobSite`), losing a fresh profession when the site
//! goes (`ResetProfession`), claiming a bed and a bell, the daily schedule
//! (work and restock at the job site, meet at the bell, sleep in the bed),
//! fleeing hostiles (`VillagerPanicTrigger`) and the unhappy head shake.
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
// `bed` and `meeting` the claimed bed head and bell, `ints[0]` the unhappy
// counter, `ints[1]` the bed's facing while asleep (1 north, 2 south,
// 3 west, 4 east), `trade` the offers, experience and restocks.

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
    g.add(3, AcquireHome::default());
    g.add(4, Schedule::default());
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
#[derive(Clone, Copy, PartialEq)]
enum Poi {
    Job(u8),
    Bed,
    Bell,
}
fn poi_at(w: &MobWorld, state: horizon_world::registry::BlockStateId) -> Option<Poi> {
    let name = &w.facts.get(state).name;
    if name.as_ref() == "minecraft:bell" {
        return Some(Poi::Bell);
    }
    if name.ends_with("_bed") {
        let s = w.registry.state(state)?;
        let free = s.properties.get("part").map(String::as_str) == Some("head")
            && s.properties.get("occupied").map(String::as_str) != Some("true");
        return free.then_some(Poi::Bed);
    }
    job_site_profession(name).map(Poi::Job)
}
/// `PoiManager.findClosest` (`AcquirePoi.SCAN_RANGE` 48 blocks across, 8
/// up and down): the nearest position whose POI passes `want`.
fn scan(m: &Mob, w: &MobWorld, want: impl Fn(Poi, [i32; 3]) -> bool) -> Option<[i32; 3]> {
    let (bx, by, bz) = (
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    );
    let mut known: HashMap<u32, Option<Poi>> = HashMap::new();
    let mut best: Option<(i32, [i32; 3])> = None;
    for y in by - 8..=by + 8 {
        for z in bz - 48..=bz + 48 {
            for x in bx - 48..=bx + 48 {
                let state = w.world.block(x, y, z);
                if w.facts.is_air(state) {
                    continue;
                }
                let poi = *known.entry(state.0).or_insert_with(|| poi_at(w, state));
                let Some(poi) = poi else { continue };
                let d = (x - bx).pow(2) + (y - by).pow(2) + (z - bz).pow(2);
                if best.is_none_or(|(b, _)| d < b) && want(poi, [x, y, z]) {
                    best = Some((d, [x, y, z]));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}
/// The nearest free job site (a profession's own one once employed).
fn find_site(m: &Mob, w: &MobWorld) -> Option<[i32; 3]> {
    scan(m, w, |poi, p| match poi {
        Poi::Job(profession) => {
            (m.extra.variant2 == NONE || profession == m.extra.variant2) && !claimed(w, m.id, p)
        }
        _ => false,
    })
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
/// The `villager_activity` / `baby_villager_activity` timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Activity {
    Idle,
    Work,
    Play,
    Meet,
    Rest,
}
pub fn activity(m: &Mob, day_time: i64) -> Activity {
    let t = day_time.rem_euclid(24000);
    if m.is_baby() {
        match t {
            10..3000 | 6000..10000 => Activity::Idle,
            3000..6000 | 10000..12000 => Activity::Play,
            _ => Activity::Rest,
        }
    } else {
        match t {
            10..2000 | 11000..12000 => Activity::Idle,
            2000..9000 => Activity::Work,
            9000..11000 => Activity::Meet,
            _ => Activity::Rest,
        }
    }
}
fn center(p: [i32; 3]) -> [f64; 3] {
    [
        f64::from(p[0]) + 0.5,
        f64::from(p[1]) + 0.5,
        f64::from(p[2]) + 0.5,
    ]
}
fn distance(m: &Mob, p: [i32; 3]) -> f64 {
    let c = center(p);
    m.position
        .distance_squared(horizon_core::geometry::Vec3::new(c[0], c[1], c[2]))
        .sqrt()
}
/// `AcquirePoi` for `HOME` (a free bed head) and `MEETING` (a bell), with
/// the happy particles on success; failed scans back off.
#[derive(Clone, Debug, Default)]
struct AcquireHome {
    next_search: i32,
}
impl Goal for AcquireHome {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.extra.bed.is_some() && m.extra.meeting.is_some() {
            return false;
        }
        if self.next_search > 0 {
            self.next_search -= 1;
            return false;
        }
        let w = cx.w;
        let mut found = false;
        if m.extra.bed.is_none()
            && let Some(p) = scan(m, w, |poi, p| {
                poi == Poi::Bed
                    && !w
                        .others
                        .iter()
                        .any(|s| s.id != m.id && s.alive && s.bed == Some(p))
            })
        {
            m.extra.bed = Some(p);
            found = true;
        }
        if m.extra.meeting.is_none()
            && !m.is_baby()
            && let Some(p) = scan(m, w, |poi, _| poi == Poi::Bell)
        {
            m.extra.meeting = Some(p);
            found = true;
        }
        if found {
            particles_around(m, "happy_villager", cx.events);
        }
        self.next_search = if found {
            20 + m.random.next_int_bound(20).unwrap_or(0)
        } else {
            200 + m.random.next_int_bound(200).unwrap_or(0)
        };
        false
    }
}
/// The schedule's walk targets (`SetWalkTargetFromBlockMemory`,
/// `StrollToPoi`, `StrollAroundPoi`), `WorkAtPoi` with restocking, and
/// `SleepInBed`.
#[derive(Clone, Debug, Default)]
struct Schedule {
    cooldown: i32,
    target: Option<[f64; 3]>,
    to_site: bool,
    last_work: i64,
    ticks: i32,
}
impl Schedule {
    fn anchor(m: &Mob, a: Activity) -> Option<([i32; 3], f64)> {
        match a {
            Activity::Work if m.extra.variant2 != NONE && m.extra.variant2 != NITWIT => {
                m.extra.home.map(|p| (p, 9.0))
            }
            Activity::Meet => m.extra.meeting.map(|p| (p, 6.0)),
            Activity::Rest => m.extra.bed.map(|p| (p, 1.0)),
            _ => None,
        }
    }
}
impl Goal for Schedule {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if sleeping(m) {
            return false;
        }
        if self.cooldown > 0 {
            self.cooldown -= 1;
            return false;
        }
        let a = activity(m, cx.w.day_time);
        let Some((p, close)) = Self::anchor(m, a) else {
            return false;
        };
        self.to_site = false;
        let c = center(p);
        self.target = if a == Activity::Rest || distance(m, p) > close {
            self.to_site = true;
            Some(c)
        } else if m.random.next_int_bound(2).unwrap_or(0) == 0 && a == Activity::Work {
            // StrollToPoi / WorkAtPoi.
            self.to_site = true;
            Some(c)
        } else {
            // StrollAroundPoi: somewhere near the site.
            let r = &mut m.random;
            let spread = if a == Activity::Meet { 6 } else { 4 };
            let mut d = || f64::from(r.next_int_bound(2 * spread + 1).unwrap_or(0) - spread);
            Some([c[0] + d(), c[1], c[2] + d()])
        };
        self.cooldown = 40 + m.random.next_int_bound(80).unwrap_or(0);
        true
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.ticks < 400
            && !sleeping(m)
            && Self::anchor(m, activity(m, cx.w.day_time)).is_some()
            && !(m.ai.nav.is_done() && self.ticks > 20)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.ticks = 0;
        if let Some(t) = self.target {
            m.nav_move_to(cx.w, t, 0.5);
        }
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.nav_stop();
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.ticks += 1;
        let a = activity(m, cx.w.day_time);
        let Some((p, _)) = Self::anchor(m, a) else {
            return;
        };
        if !self.to_site {
            return;
        }
        match a {
            Activity::Work if distance(m, p) < 1.73 => {
                // WorkAtPoi: every 300 ticks at most, half the time.
                let now = cx.w.game_time;
                m.look_at(center(p));
                if now - self.last_work >= 300 && m.random.next_int_bound(2).unwrap_or(1) == 0 {
                    self.last_work = now;
                    work(m, cx.w, cx.events);
                }
                m.nav_stop();
            }
            Activity::Rest if distance(m, p) < 2.0 => start_sleeping(m, p, cx.w),
            _ => {}
        }
    }
}
/// `WorkAtPoi.start`: the profession's work sound, and a restock when due.
fn work(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let profession = super::render::PROFESSIONS[m.extra.variant2 as usize % 15];
    events.push(MobEvent::SoundId(
        format!("minecraft:entity.villager.work_{profession}"),
        [m.position.x, m.position.y, m.position.z],
        1.0,
        1.0,
    ));
    let day = w.day_time.div_euclid(24000);
    if let Some(t) = m.extra.trade.as_mut()
        && t.should_restock(w.game_time, day)
    {
        t.restock(w.game_time);
    }
}
pub fn sleeping(m: &Mob) -> bool {
    m.extra.ints[1] != 0
}
/// `LivingEntity.startSleeping`: lie in the bed (`setPosToBed`).
fn start_sleeping(m: &mut Mob, p: [i32; 3], w: &MobWorld) {
    if w.prop(p[0], p[1], p[2], "occupied") == Some("true") {
        return;
    }
    m.extra.ints[1] = match w.prop(p[0], p[1], p[2], "facing") {
        Some("north") => 1,
        Some("south") => 2,
        Some("west") => 3,
        _ => 4,
    };
    m.nav_stop();
    m.velocity = horizon_core::geometry::Vec3::ZERO;
    m.position = horizon_core::geometry::Vec3::new(
        f64::from(p[0]) + 0.5,
        f64::from(p[1]) + 0.6875,
        f64::from(p[2]) + 0.5,
    );
}
/// `LivingEntity.stopSleeping` with `BedBlock.findStandUpPosition`: beside
/// the bed where there is room, else on top of it.
pub fn stop_sleeping(m: &mut Mob, w: &MobWorld) {
    m.extra.ints[1] = 0;
    let Some(p) = m.extra.bed else { return };
    for (dx, dz) in [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (-1, -1),
        (1, -1),
        (-1, 1),
    ] {
        let (x, y, z) = (p[0] + dx, p[1], p[2] + dz);
        let free = |y| w.data.state(w.world.block(x, y, z)).top <= 0.0;
        if free(y) && free(y + 1) && w.solid(x, y - 1, z) {
            m.position = horizon_core::geometry::Vec3::new(
                f64::from(x) + 0.5,
                f64::from(y),
                f64::from(z) + 0.5,
            );
            return;
        }
    }
    m.position.y = f64::from(p[1]) + 0.5625;
}
/// While asleep the villager stays in the bed.
pub fn travel(m: &mut Mob) -> bool {
    if !sleeping(m) {
        return false;
    }
    m.velocity = horizon_core::geometry::Vec3::ZERO;
    true
}
/// `LivingEntityRenderer.sleepDirectionToRotation` and the bed's direction
/// (the body is shifted back from the pillow by the standing eye height).
pub fn sleep_pose(m: &Mob) -> Option<(f32, [f32; 2])> {
    Some(match m.extra.ints[1] {
        1 => (270.0, [0.0, -1.0]),
        2 => (90.0, [0.0, 1.0]),
        3 => (0.0, [-1.0, 0.0]),
        4 => (180.0, [1.0, 0.0]),
        _ => return None,
    })
}
/// `Villager.tick` / brain upkeep: the unhappy counter, and
/// `ResetProfession` / job site loss when the site's block is gone.
pub fn ai_step(m: &mut Mob, w: &MobWorld) {
    if m.extra.ints[0] > 0 {
        m.extra.ints[0] -= 1;
    }
    // WakeUp / LivingEntity.hurt: morning, a hit or a missing bed.
    if sleeping(m) {
        let bed_gone = m
            .extra
            .bed
            .is_none_or(|p| !w.block_name(p[0], p[1], p[2]).ends_with("_bed"));
        if activity(m, w.day_time) != Activity::Rest || m.hurt_time > 0 || bed_gone {
            stop_sleeping(m, w);
        }
    }
    // ValidateNearbyPoi for the bed and bell.
    if m.age.is_multiple_of(20) {
        let loaded = |p: [i32; 3]| w.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4));
        if let Some(p) = m.extra.bed
            && loaded(p)
            && !w.block_name(p[0], p[1], p[2]).ends_with("_bed")
        {
            m.extra.bed = None;
        }
        if let Some(p) = m.extra.meeting
            && loaded(p)
            && w.block_name(p[0], p[1], p[2]) != "minecraft:bell"
        {
            m.extra.meeting = None;
        }
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
    let awake = !sleeping(m);
    p.yrot("head", if awake { s.y_rot.to_radians() } else { 0.0 });
    p.xrot("head", if awake { s.x_rot.to_radians() } else { 0.0 });
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
