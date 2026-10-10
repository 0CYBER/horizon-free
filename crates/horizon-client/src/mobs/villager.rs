//! Villagers (`Villager` and its brain packages, as goals): taking the
//! profession of a free job site nearby (`AcquirePoi` with
//! `AssignProfessionFromJobSite`), losing a fresh profession when the site
//! goes (`ResetProfession`), claiming a bed and a bell, the daily schedule
//! (work and restock at the job site, meet at the bell, sleep in the bed),
//! fleeing hostiles (`VillagerPanicTrigger`) and the unhappy head shake.
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    goals::{
        Avoid, AvoidEntity, Cx, Float, Goal, LOOK, LookAtPlayer, MOVE, Panic, RandomLookAround,
        Stroll, goal_clone,
    },
};
use crate::items::ItemStack;
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
        Species::Illusioner,
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
    g.add(2, RaidBehavior::default());
    g.add(3, Social::default());
    g.add(0, GolemCall);
    g.add(3, GiftToHero::default());
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
    matches!(m.species(), Species::Villager | Species::WanderingTrader) && m.extra.ints[1] != 0
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
    ext_mut(m).last_slept = Some(w.game_time);
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
    if m.species() == Species::Villager {
        let now = w.game_time;
        let seen_golem = (i64::from(m.age) + i64::from(m.id)) % 200 == 0
            && w.others.iter().any(|s| {
                s.species == Species::IronGolem
                    && s.alive
                    && (s.position.x - m.position.x).abs() <= 16.0
                    && (s.position.y - m.position.y).abs() <= 16.0
                    && (s.position.z - m.position.z).abs() <= 16.0
            });
        let e = ext_mut(m);
        // Villager.maybeDecayGossip and the GolemSensor (every 200 ticks).
        e.maybe_decay(now);
        if seen_golem {
            e.golem_until = now + 599;
        }
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

// ---- Raids: the raid activities and the gift for a hero.

pub const RAID_NONE: u8 = 0;
/// A raid is near but no wave is on (`Activity.PRE_RAID`).
pub const PRE_RAID: u8 = 1;
/// A wave is on (`Activity.RAID`).
pub const RAID: u8 = 2;
/// The raid was won: the villagers celebrate.
pub const RAID_VICTORY: u8 = 3;

/// Villager state beyond `Extra`'s scratch fields.
#[derive(Clone, Debug, Default)]
pub struct Ext {
    /// Which raid package the villager follows.
    pub raid: u8,
    /// What the villager remembers of players (`gossips`).
    pub gossips: crate::gossip::Gossips,
    pub last_gossip_time: i64,
    pub last_gossip_decay: i64,
    /// `foodLevel`: eaten but not yet digested, in food points.
    pub food_level: i32,
    /// The eight-slot inventory (`InventoryCarrier`).
    pub inventory: Vec<ItemStack>,
    /// `LAST_SLEPT` memory.
    pub last_slept: Option<i64>,
    /// `GOLEM_DETECTED_RECENTLY` expires at this game time.
    pub golem_until: i64,
    /// The `last_hurt_by_time` of the player's hit already counted.
    pub hurt_seen: i32,
    /// A zombie villager's cure under way.
    pub cure: Option<super::zombie_villager::Cure>,
}
pub fn ext(m: &Mob) -> Option<&Ext> {
    m.extra.villager.as_deref()
}
pub fn ext_mut(m: &mut Mob) -> &mut Ext {
    m.extra.villager.get_or_insert_with(Box::default)
}
fn raid_status(m: &Mob) -> u8 {
    ext(m).map_or(RAID_NONE, |e| e.raid)
}

/// The raid packages of `VillagerGoalPackages`: in a raid villagers hide at
/// home (`LocateHidingPlace`), before one they gather at the bell and ring it
/// (`RingBell`), and after a victory they look for the sky to celebrate
/// (`MoveToSkySeeingSpot`, `CelebrateVillagersSurvivedRaid`).
#[derive(Clone, Debug, Default)]
struct RaidBehavior {
    next_move: i32,
}
impl Goal for RaidBehavior {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        raid_status(m) != RAID_NONE && !sleeping(m)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.can_use(m, cx)
    }
    fn start(&mut self, m: &mut Mob, _cx: &mut Cx) {
        self.next_move = 0;
        m.nav_stop();
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.nav_stop();
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.next_move -= 1;
        let p = m.position;
        match raid_status(m) {
            RAID => {
                // LocateHidingPlace(24, 0.7, 1): back to the bed.
                let Some(home) = m.extra.bed else { return };
                if distance(m, home) > 1.0 && (self.next_move <= 0 || m.ai.nav.is_done()) {
                    if distance(m, home) <= 24.0 {
                        m.nav_move_to(cx.w, center(home), 0.7);
                    }
                    self.next_move = 40;
                }
            }
            PRE_RAID => {
                // RingBell and SetWalkTargetFromBlockMemory(meeting, 0.75).
                let Some(bell) = m.extra.meeting else { return };
                if distance(m, bell) > 2.0 && (self.next_move <= 0 || m.ai.nav.is_done()) {
                    m.nav_move_to(cx.w, center(bell), 0.75);
                    self.next_move = 40;
                }
                if distance(m, bell) < 3.0
                    && m.random.next_float() > 0.95
                    && cx.w.block_name(bell[0], bell[1], bell[2]) == "minecraft:bell"
                {
                    cx.events.push(MobEvent::SoundId(
                        "minecraft:block.bell.use".into(),
                        center(bell),
                        2.0,
                        1.0,
                    ));
                }
            }
            RAID_VICTORY => {
                let (x, y, z) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
                if cx.w.can_see_sky(x, y, z) {
                    // CelebrateVillagersSurvivedRaid.tick.
                    if m.random.next_int_bound(100).unwrap_or(1) == 0 {
                        m.voice("entity.villager.celebrate", cx.events);
                    }
                    if m.random.next_int_bound(200).unwrap_or(1) == 0 {
                        let color = m.random.next_int_bound(16).unwrap_or(0) as usize;
                        let flight = m.random.next_int_bound(3).unwrap_or(0) as u8;
                        cx.events.push(MobEvent::Firework {
                            at: [p.x, m.eye_y(), p.z],
                            color,
                            flight,
                        });
                    }
                    m.nav_stop();
                } else if self.next_move <= 0 || m.ai.nav.is_done() {
                    // MoveToSkySeeingSpot: a random spot with a view of the sky.
                    for _ in 0..5 {
                        if let Some(at) = m.land_random_pos(cx.w, 15, 7)
                            && cx.w.can_see_sky(
                                at[0].floor() as i32,
                                at[1].floor() as i32,
                                at[2].floor() as i32,
                            )
                        {
                            m.nav_move_to(cx.w, at, 0.5);
                            break;
                        }
                    }
                    self.next_move = 40;
                }
            }
            _ => {}
        }
    }
}

/// `GiveGiftToHero`: a villager that sees a hero of the village walks up and
/// throws a gift of its profession, every so often.
#[derive(Clone, Debug)]
struct GiftToHero {
    until_next: i32,
    started: i64,
    given: bool,
}
impl Default for GiftToHero {
    fn default() -> Self {
        Self {
            until_next: 600,
            started: 0,
            given: false,
        }
    }
}
/// The gift loot table of a villager (`GiveGiftToHero.getLootTableToThrow`).
fn gift_table(m: &Mob) -> String {
    if m.is_baby() {
        return "minecraft:gameplay/hero_of_the_village/baby_gift".into();
    }
    let profession = super::render::PROFESSIONS[m.extra.variant2 as usize % 15];
    let name = match profession {
        "none" | "nitwit" => "unemployed",
        other => other,
    };
    format!("minecraft:gameplay/hero_of_the_village/{name}_gift")
}
fn hero_in_view(m: &mut Mob, cx: &Cx) -> bool {
    let Some(p) = &cx.w.player else { return false };
    p.hero
        && p.alive
        && !p.spectator
        && m.position.distance_squared(p.position) <= 16.0 * 16.0
        && m.sees(cx.w, Who::Player)
}
impl Goal for GiftToHero {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !hero_in_view(m, cx) || sleeping(m) {
            return false;
        }
        if self.until_next > 0 {
            self.until_next -= 1;
            return false;
        }
        true
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        hero_in_view(m, cx) && !self.given
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.given = false;
        self.started = cx.w.game_time;
        if let Some(p) = &cx.w.player {
            m.look_at(p.eye);
        }
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        self.until_next = 600 + m.random.next_int_bound(6001).unwrap_or(0);
        m.nav_stop();
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(p) = cx.w.player.clone() else { return };
        m.look_at(p.eye);
        let near = m.position.distance_squared(p.position) < 25.0;
        if near {
            if cx.w.game_time - self.started > 20 {
                cx.events.push(MobEvent::ThrowGift {
                    table: gift_table(m),
                    from: [m.position.x, m.eye_y() - 0.3, m.position.z],
                    to: [p.position.x, p.position.y, p.position.z],
                });
                self.given = true;
            }
            m.nav_stop();
        } else if m.ai.nav.is_done() {
            m.nav_move_to(cx.w, [p.position.x, p.position.y, p.position.z], 0.5);
        }
    }
}

// ---- Gossip, food, breeding and golems.

/// `VillagerFood` nutrition (`DataComponents.VILLAGER_FOOD`).
pub fn food_points(item: &str) -> i32 {
    match item {
        "minecraft:bread" => 4,
        "minecraft:potato" | "minecraft:carrot" | "minecraft:beetroot" => 1,
        _ => 0,
    }
}
/// The farmer's `requestedItems`.
pub fn farmer_items() -> &'static [&'static str] {
    &[
        "minecraft:bread",
        "minecraft:potato",
        "minecraft:carrot",
        "minecraft:wheat",
        "minecraft:wheat_seeds",
        "minecraft:beetroot",
        "minecraft:beetroot_seeds",
        "minecraft:torchflower_seeds",
        "minecraft:pitcher_pod",
    ]
}
/// A villager's `requestedItems` by profession.
pub fn requested_items(m: &Mob) -> &'static [&'static str] {
    if super::render::PROFESSIONS[m.extra.variant2 as usize % 15] == "farmer" {
        farmer_items()
    } else {
        &[]
    }
}
pub fn is_farmer(m: &Mob) -> bool {
    super::render::PROFESSIONS[m.extra.variant2 as usize % 15] == "farmer"
}
/// `SimpleContainer(8)`: stacks the villager carries.
const INVENTORY_SLOTS: usize = 8;

impl Ext {
    /// `countFoodPointsInInventory`.
    pub fn food_in_inventory(&self) -> i32 {
        self.inventory
            .iter()
            .map(|s| food_points(&s.item) * s.count as i32)
            .sum()
    }
    /// `hasExcessFood` and `wantsMoreFood`.
    pub fn has_excess_food(&self) -> bool {
        self.food_in_inventory() >= 24
    }
    pub fn wants_more_food(&self) -> bool {
        self.food_in_inventory() < 12
    }
    fn hungry(&self) -> bool {
        self.food_level < 12
    }
    /// `Villager.eatAndDigestFood`: eat until full, then digest 12 points.
    pub fn eat_and_digest(&mut self) {
        if self.hungry() {
            let mut slot = 0;
            while slot < self.inventory.len() && self.hungry() {
                let points = food_points(&self.inventory[slot].item);
                if points > 0 {
                    while self.inventory[slot].count > 0 && self.hungry() {
                        self.food_level += points;
                        self.inventory[slot].count -= 1;
                    }
                    if self.inventory[slot].count == 0 {
                        self.inventory.remove(slot);
                        continue;
                    }
                }
                slot += 1;
            }
        }
        self.food_level -= 12;
    }
    /// Whether the stack fits (`SimpleContainer.canAddItem`).
    pub fn can_add(&self, stack: &ItemStack, max_stack: u32) -> bool {
        self.inventory.len() < INVENTORY_SLOTS
            || self
                .inventory
                .iter()
                .any(|s| s.same_item_same_components(stack) && s.count < max_stack)
    }
    /// `SimpleContainer.addItem`: merge into stacks, then free slots; the
    /// part that did not fit is returned.
    pub fn add_item(&mut self, mut stack: ItemStack, max_stack: u32) -> Option<ItemStack> {
        for s in &mut self.inventory {
            if stack.count > 0 && s.same_item_same_components(&stack) && s.count < max_stack {
                let n = (max_stack - s.count).min(stack.count);
                s.count += n;
                stack.count -= n;
            }
        }
        while stack.count > 0 && self.inventory.len() < INVENTORY_SLOTS {
            let n = stack.count.min(max_stack);
            self.inventory.push(ItemStack {
                count: n,
                ..stack.clone()
            });
            stack.count -= n;
        }
        (stack.count > 0).then_some(stack)
    }
    pub fn count_item(&self, item: &str) -> u32 {
        self.inventory
            .iter()
            .filter(|s| &*s.item == item)
            .map(|s| s.count)
            .sum()
    }
    /// `TradeWithVillager.throwHalfStack`: the half of a big matching stack
    /// (or whatever is over 24) leaves the inventory.
    pub fn take_half_stack(
        &mut self,
        wanted: impl Fn(&ItemStack) -> bool,
        max_stack: impl Fn(&str) -> u32,
    ) -> Option<ItemStack> {
        for i in 0..self.inventory.len() {
            let s = &self.inventory[i];
            if s.count == 0 || !wanted(s) {
                continue;
            }
            let count = if s.count > max_stack(&s.item) / 2 {
                s.count / 2
            } else if s.count > 24 {
                s.count - 24
            } else {
                continue;
            };
            let taken = ItemStack { count, ..s.clone() };
            self.inventory[i].count -= count;
            if self.inventory[i].count == 0 {
                self.inventory.remove(i);
            }
            return Some(taken);
        }
        None
    }
    /// `Villager.wantsToSpawnGolem`: slept within the last day and no golem
    /// seen lately.
    pub fn wants_golem(&self, now: i64) -> bool {
        self.last_slept.is_some_and(|t| now - t < 24000) && now >= self.golem_until
    }
    /// `maybeDecayGossip`: once a day.
    pub fn maybe_decay(&mut self, now: i64) {
        if self.last_gossip_decay == 0 {
            self.last_gossip_decay = now;
        } else if now >= self.last_gossip_decay + 24000 {
            self.gossips.decay();
            self.last_gossip_decay = now;
        }
    }
    /// `Villager.gossip` timing: hearing again waits 1200 ticks.
    pub fn can_gossip(&self, now: i64) -> bool {
        now < self.last_gossip_time || now >= self.last_gossip_time + 1200
    }
}
/// `Villager.canBreed`: enough food, awake and an adult.
pub fn can_breed(m: &Mob) -> bool {
    m.species() == Species::Villager
        && m.extra.age == 0
        && !sleeping(m)
        && ext(m).map_or(0, |e| e.food_level + e.food_in_inventory()) >= 12
}

/// The saved fields (`FoodLevel`, `Gossips`, `LastGossipDecay`, `Inventory`
/// and the sleep and golem memories).
pub fn write_nbt(m: &Mob, map: &mut horizon_world::nbt::Compound) {
    use horizon_world::nbt::Tag;
    let Some(e) = ext(m) else { return };
    map.insert(
        "FoodLevel".into(),
        Tag::Byte(e.food_level.clamp(-128, 127) as i8),
    );
    map.insert("Gossips".into(), e.gossips.to_nbt());
    map.insert("LastGossipDecay".into(), Tag::Long(e.last_gossip_decay));
    map.insert(
        "Inventory".into(),
        Tag::List(e.inventory.iter().map(crate::save::item_to_nbt).collect()),
    );
}
/// The memories of `Brain.memories` the villager keeps besides positions.
pub fn write_memories(m: &Mob, memories: &mut Vec<(&'static str, horizon_world::nbt::Tag)>) {
    use horizon_world::nbt::Tag;
    let Some(e) = ext(m) else { return };
    if let Some(t) = e.last_slept {
        memories.push((
            "minecraft:last_slept",
            Tag::compound([("value", Tag::Long(t))]),
        ));
    }
}
pub fn read_nbt(m: &mut Mob, tag: &horizon_world::nbt::Tag) {
    use horizon_world::nbt::Tag;
    let int = |t: Option<&Tag>| match t {
        Some(Tag::Int(v)) => Some(i64::from(*v)),
        Some(Tag::Byte(v)) => Some(i64::from(*v)),
        Some(Tag::Short(v)) => Some(i64::from(*v)),
        Some(Tag::Long(v)) => Some(*v),
        _ => None,
    };
    let last_slept = int(tag
        .get("Brain")
        .and_then(|b| b.get("memories"))
        .and_then(|b| b.get("minecraft:last_slept"))
        .and_then(|j| j.get("value")));
    let e = ext_mut(m);
    e.food_level = int(tag.get("FoodLevel")).unwrap_or(0) as i32;
    e.gossips = tag
        .get("Gossips")
        .map(crate::gossip::Gossips::from_nbt)
        .unwrap_or_default();
    e.last_gossip_decay = int(tag.get("LastGossipDecay")).unwrap_or(0);
    e.last_slept = last_slept;
    if let Some(Tag::List(items)) = tag.get("Inventory") {
        e.inventory = items
            .iter()
            .filter_map(crate::save::item_from_nbt)
            .take(INVENTORY_SLOTS)
            .collect();
    }
}

/// `InteractWith(VILLAGER)`, `TradeWithVillager` and `VillagerMakeLove` while
/// idle, and `SocializeAtBell` at the meeting: walk up to a villager, swap
/// gossip and food, and make a baby with a willing partner.
#[derive(Clone, Debug, Default)]
struct Social {
    target: Option<u32>,
    breed: bool,
    ticks: i32,
    cooldown: i32,
    birth_at: i64,
}
impl Social {
    fn pick(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let breedable = can_breed(m);
        let id = m.id;
        let nearest =
            cx.w.nearby(m.position, 8.0, move |s| {
                s.species == Species::Villager && s.id != id
            })
            .filter(|s| m.position.distance_squared(s.position) <= 64.0)
            .map(|s| (s.id, s.in_love))
            .collect::<Vec<_>>();
        let visible: Vec<(u32, bool)> = nearest
            .into_iter()
            .filter(|(i, _)| m.sees(cx.w, Who::Mob(*i)))
            .collect();
        let Some(&(target, partner_willing)) = visible.first() else {
            return false;
        };
        // RunOne weights: interact 2, breed 1 (when both are willing).
        let breed = breedable && partner_willing && m.random.next_int_bound(3).unwrap_or(0) == 0;
        self.target = Some(target);
        self.breed = breed;
        true
    }
}
impl Goal for Social {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.is_baby() || sleeping(m) || raid_status(m) != RAID_NONE {
            return false;
        }
        if self.cooldown > 0 {
            self.cooldown -= 1;
            return false;
        }
        match activity(m, cx.w.day_time) {
            Activity::Idle => self.pick(m, cx),
            Activity::Meet => {
                // SocializeAtBell: one tick in a hundred, near the bell.
                let near_bell = m.extra.meeting.is_some_and(|p| distance(m, p) < 4.0);
                near_bell && m.random.next_int_bound(100).unwrap_or(1) == 0 && {
                    let ok = self.pick(m, cx);
                    self.breed = false;
                    ok
                }
            }
            _ => false,
        }
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(t) = self.target else { return false };
        let Some(s) = cx.w.seen(t) else { return false };
        if !s.alive || sleeping(m) || raid_status(m) != RAID_NONE {
            return false;
        }
        if self.breed {
            self.ticks < 350 && can_breed(m) && s.in_love
        } else {
            self.ticks < 60
        }
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.ticks = 0;
        self.birth_at = 0;
        if let Some(s) = self.target.and_then(|t| cx.w.seen(t)) {
            let at = [s.position.x, s.position.y, s.position.z];
            m.nav_move_to(cx.w, at, 0.5);
        }
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.nav_stop();
        self.target = None;
        self.cooldown = 20 + m.random.next_int_bound(40).unwrap_or(0);
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.ticks += 1;
        let Some(t) = self.target else { return };
        let Some(s) = cx.w.seen(t) else { return };
        let (at, eye) = ([s.position.x, s.position.y, s.position.z], s.eye_y);
        m.look_at([at[0], eye, at[2]]);
        if m.position.distance_squared(s.position) > 5.0 {
            if m.ai.nav.is_done() && self.ticks % 10 == 1 {
                m.nav_move_to(cx.w, at, 0.5);
            }
            return;
        }
        m.nav_stop();
        if !self.breed {
            cx.events.push(MobEvent::VillagerMeet {
                mob: m.id,
                other: t,
            });
            return;
        }
        let now = cx.w.game_time;
        if self.birth_at == 0 {
            self.birth_at = now + 275 + i64::from(m.random.next_int_bound(50).unwrap_or(0));
        }
        if now >= self.birth_at {
            cx.events.push(MobEvent::VillagerBirth {
                parent: m.id,
                partner: t,
            });
            self.ticks = 1000;
        } else if m.random.next_int_bound(35).unwrap_or(1) == 0 {
            for p in [m.position, s.position] {
                cx.events.push(MobEvent::Particles {
                    kind: "heart",
                    at: [p.x, p.y + 1.8, p.z],
                    count: 1,
                });
            }
        }
    }
}

/// `VillagerPanicTrigger.tick`: a frightened villager calls for a golem
/// every five seconds.
#[derive(Clone, Debug, Default)]
struct GolemCall;
/// `VillagerHostilesSensor`: how close each hostile must be to scare.
fn hostile_range(s: Species) -> f64 {
    match s {
        Species::Drowned
        | Species::Husk
        | Species::Vex
        | Species::Zombie
        | Species::ZombieVillager => 8.0,
        Species::Evoker | Species::Illusioner | Species::Ravager => 12.0,
        Species::Pillager => 15.0,
        Species::Vindicator | Species::Zoglin => 10.0,
        _ => 0.0,
    }
}
fn frightened(m: &Mob, cx: &Cx) -> bool {
    // HURT_BY lasts 100 ticks.
    let hurt = m.ai.last_hurt_by.is_some() && m.age as i32 - m.ai.last_hurt_by_time <= 100;
    hurt || cx.w.others.iter().any(|s| {
        s.alive && {
            let r = hostile_range(s.species);
            r > 0.0 && s.position.distance_squared(m.position) <= r * r
        }
    })
}
impl Goal for GolemCall {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !m.is_baby() && frightened(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.can_use(m, cx)
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if cx.w.game_time % 100 == 0 {
            cx.events.push(MobEvent::GolemCheck {
                mob: m.id,
                needed: 3,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack::new(item, count)
    }
    #[test]
    fn eating_fills_the_food_level_then_digests() {
        let mut e = Ext {
            inventory: vec![stack("minecraft:bread", 3), stack("minecraft:carrot", 5)],
            ..Default::default()
        };
        assert_eq!(e.food_in_inventory(), 17);
        // Hungry below 12: three breads fill it exactly, then 12 digest away.
        e.eat_and_digest();
        assert_eq!(e.food_level, 0);
        assert_eq!(e.inventory.len(), 1);
        assert_eq!(e.inventory[0].count, 5);
        // Five carrots are not enough to be full again, and are all eaten.
        e.eat_and_digest();
        assert_eq!(e.food_level, 5 - 12);
        assert!(e.inventory.is_empty());
    }
    #[test]
    fn willingness_to_breed_needs_twelve_food_points() {
        let mut e = Ext::default();
        assert!(e.wants_more_food() && !e.has_excess_food());
        e.inventory.push(stack("minecraft:bread", 6));
        assert!(e.has_excess_food() && !e.wants_more_food());
        e.food_level = -5;
        assert_eq!(e.food_level + e.food_in_inventory(), 19);
    }
    #[test]
    fn the_inventory_has_eight_stacking_slots() {
        let mut e = Ext::default();
        assert!(e.add_item(stack("minecraft:bread", 100), 64).is_none());
        assert_eq!(e.inventory.len(), 2);
        assert_eq!(e.inventory[0].count, 64);
        for i in 0..6 {
            assert!(
                e.add_item(stack(&format!("minecraft:item_{i}"), 1), 64)
                    .is_none()
            );
        }
        assert_eq!(e.inventory.len(), 8);
        // Full: another kind is refused, the same kind tops up the stack.
        assert!(!e.can_add(&stack("minecraft:stick", 1), 64));
        let left = e.add_item(stack("minecraft:bread", 40), 64).unwrap();
        assert_eq!(left.count, 40 - 28);
    }
    #[test]
    fn half_stacks_are_thrown() {
        let mut e = Ext {
            inventory: vec![
                stack("minecraft:bread", 10),
                stack("minecraft:wheat", 50),
                stack("minecraft:wheat", 20),
            ],
            ..Default::default()
        };
        let max = |_: &str| 64;
        // Bread is under half a stack and under 24: nothing.
        assert!(
            e.take_half_stack(|s| &*s.item == "minecraft:bread", max)
                .is_none()
        );
        // 50 wheat is over half a stack: 25 go.
        let t = e
            .take_half_stack(|s| &*s.item == "minecraft:wheat", max)
            .unwrap();
        assert_eq!(t.count, 25);
        assert_eq!(e.count_item("minecraft:wheat"), 25 + 20);
    }
    #[test]
    fn golems_need_sleep_and_no_golem_nearby() {
        let mut e = Ext::default();
        assert!(!e.wants_golem(100));
        e.last_slept = Some(0);
        assert!(e.wants_golem(23_999));
        assert!(!e.wants_golem(24_000));
        e.last_slept = Some(1000);
        e.golem_until = 2000 + 599;
        assert!(!e.wants_golem(2500));
        assert!(e.wants_golem(2700));
    }
    #[test]
    fn gossip_decays_once_a_day_and_heard_gossip_waits() {
        let player = [1, 2, 3, 4];
        let mut e = Ext::default();
        e.gossips
            .add(player, crate::gossip::GossipType::MajorNegative, 25);
        e.maybe_decay(100);
        assert_eq!(e.last_gossip_decay, 100);
        e.maybe_decay(24_099);
        assert_eq!(
            e.gossips
                .get(player, crate::gossip::GossipType::MajorNegative),
            25
        );
        e.maybe_decay(24_100);
        assert_eq!(
            e.gossips
                .get(player, crate::gossip::GossipType::MajorNegative),
            15
        );
        e.last_gossip_time = 5000;
        assert!(!e.can_gossip(5100));
        assert!(e.can_gossip(6200));
        assert!(e.can_gossip(4000));
    }
    #[test]
    fn villager_food_values() {
        assert_eq!(food_points("minecraft:bread"), 4);
        assert_eq!(food_points("minecraft:beetroot"), 1);
        assert_eq!(food_points("minecraft:apple"), 0);
    }
}
