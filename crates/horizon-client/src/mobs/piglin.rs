//! Piglins, piglin brutes, hoglins and zoglins: gold-wearing players are
//! left alone, gold is admired and bartered, piglins and hoglins zombify
//! outside the Nether, hoglins headbutt their targets into the air.
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    brain::{Ttl, living_near},
    goals::{
        Cx, Float, Goal, HurtByTarget, LOOK, LookAtPlayer, MOVE, MeleeAttack, NearestTarget, Prey,
        RandomLookAround, Stroll, TARGET, TargetState, goal_clone, target_continue,
    },
};
use horizon_core::geometry::{Vec3, trig::Mth};
use horizon_core::random::{LegacyRandom, RandomSource};
use std::f32::consts::PI;

/// `PiglinAi.ADMIRE_DURATION`.
pub const ADMIRE: i32 = 119;
/// `PiglinAi.eat`: the `ATE_RECENTLY` memory lasts 200 ticks (kept in
/// `extra.ints[0]` of a piglin).
pub const ATE_RECENTLY: i32 = 200;
/// `AbstractPiglin.CONVERSION_TIME` (and the hoglin's).
const CONVERSION: i32 = 300;
/// `PiglinAi.CELEBRATION_TIME`: how long the `CELEBRATE_LOCATION` and
/// `DANCING` memories last.
const CELEBRATION: i32 = 300;

/// `Activity`: what a piglin's or hoglin's brain is busy with. `Admire` and
/// `Celebrate` are a piglin's only; the others are both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Activity {
    #[default]
    Idle,
    Fight,
    Avoid,
    Admire,
    Celebrate,
}
/// `MemoryModuleType.AVOID_TARGET`: who to keep away from, where it was last
/// seen (a target that is gone leaves its spot behind) and when the memory
/// expires.
#[derive(Clone, Copy, Debug)]
pub struct AvoidTarget {
    pub who: Who,
    pub at: Vec3,
    pub ttl: Ttl,
}
/// What a piglin's or hoglin's brain remembers of its surroundings and of
/// what it retreats from. None of it is saved: these memories have no codec.
#[derive(Clone, Debug, Default)]
pub struct Memories {
    /// `AVOID_TARGET`.
    pub avoid: Option<AvoidTarget>,
    /// `Brain.getActiveNonCoreActivity` as `updateActivity` last left it.
    pub activity: Activity,
    /// The `last_hurt_by_time` of the hit `wasHurtBy` last reacted to.
    pub hurt_seen: i32,
    /// `VISIBLE_ADULT_PIGLIN_COUNT`: a piglin's counts its brutes too, a
    /// hoglin's only the piglins.
    pub piglins: i32,
    /// `VISIBLE_ADULT_HOGLIN_COUNT`.
    pub hoglins: i32,
    /// A hoglin's `NEAREST_VISIBLE_ADULT_PIGLIN`.
    pub nearest_piglin: Option<u32>,
    /// A hoglin's `NEAREST_VISIBLE_ADULT_HOGLINS` and a piglin's
    /// `NEAREST_VISIBLE_ADULT_PIGLINS` (brutes too), nearest first.
    pub adults: Vec<u32>,
    /// A piglin's `NEAREST_VISIBLE_ZOMBIFIED`.
    pub zombified: Option<Who>,
    /// A piglin's `NEAREST_VISIBLE_NEMESIS`: a wither skeleton or a wither.
    pub nemesis: Option<Who>,
    /// A piglin's `NEAREST_PLAYER_HOLDING_WANTED_ITEM` is set.
    pub sees_loved: bool,
    /// Retreats other mobs' `broadcastRetreat` asked of this one, which its
    /// next tick starts with: who to retreat from and where it stands.
    pub retreats: Vec<(Who, Vec3)>,
}
/// `Sensor.isEntityTargetable`: `TargetingConditions.forNonCombat()` within
/// the follow range and in sight, which a `NearestVisibleLivingEntities`
/// applies to every entity it lists.
fn visible(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    let range = m.attrs.follow_range;
    match who {
        Who::Player => {
            let Some(p) = &w.player else { return false };
            let r = (range * p.visibility_percent()).max(2.0);
            p.alive && !p.spectator && m.distance_sqr(p.position) <= r * r && m.sees(w, who)
        }
        Who::Mob(id) => {
            let Some(s) = w.seen(id) else { return false };
            s.alive && m.distance_sqr(s.position) <= range * range && m.sees(w, who)
        }
    }
}
/// `PiglinAi.isPlayerHoldingLovedItem`: gold or anything else in
/// `#piglin_loved`, in either hand.
fn holds_loved(w: &MobWorld, p: &super::PlayerView) -> bool {
    let Some(tag) = w
        .items
        .and_then(|i| i.item_tags.get("minecraft:piglin_loved"))
    else {
        return false;
    };
    [&p.main_hand, &p.off_hand]
        .into_iter()
        .flatten()
        .any(|item| tag.contains(&**item))
}
/// `HoglinSpecificSensor.doTick`: the adult piglins and hoglins in sight,
/// counted, with the nearest piglin and the hoglins themselves. (The sensor
/// also looks up the nearest repellent; `hoglin_repellent` does that.)
fn hoglin_sense(m: &mut Mob, w: &MobWorld) {
    let mut piglins = 0;
    let mut nearest = None;
    let mut hoglins = Vec::new();
    for who in living_near(m, w) {
        let Who::Mob(id) = who else { continue };
        let Some(s) = w.seen(id) else { continue };
        if s.baby || !matches!(s.species, Species::Piglin | Species::Hoglin) || !visible(m, w, who)
        {
            continue;
        }
        if s.species == Species::Piglin {
            piglins += 1;
            nearest = nearest.or(Some(id));
        } else {
            hoglins.push(id);
        }
    }
    let b = &mut m.extra.piglin;
    b.piglins = piglins;
    b.hoglins = hoglins.len() as i32;
    b.nearest_piglin = nearest;
    b.adults = hoglins;
}
/// `PiglinSpecificSensor.doTick`: what a piglin sees: adult piglins (brutes
/// too) and hoglins counted, the nearest zombified piglin or zoglin, the
/// nearest wither skeleton or wither, and a player holding gold.
fn piglin_sense(m: &mut Mob, w: &MobWorld) {
    let mut adults = Vec::new();
    let mut hoglins = 0;
    let mut zombified = None;
    let mut nemesis = None;
    let mut loved = false;
    for who in living_near(m, w) {
        match who {
            Who::Player => {
                if !loved
                    && w.player
                        .as_ref()
                        .is_some_and(|p| !p.spectator && holds_loved(w, p))
                    && visible(m, w, who)
                {
                    loved = true;
                }
            }
            Who::Mob(id) => {
                let Some(s) = w.seen(id) else { continue };
                let (species, baby) = (s.species, s.baby);
                match species {
                    Species::Hoglin if !baby => {
                        if visible(m, w, who) {
                            hoglins += 1;
                        }
                    }
                    Species::Piglin | Species::PiglinBrute if !baby => {
                        if visible(m, w, who) {
                            adults.push(id);
                        }
                    }
                    Species::WitherSkeleton | Species::Wither if nemesis.is_none() => {
                        if visible(m, w, who) {
                            nemesis = Some(who);
                        }
                    }
                    Species::ZombifiedPiglin | Species::Zoglin if zombified.is_none() => {
                        if visible(m, w, who) {
                            zombified = Some(who);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let b = &mut m.extra.piglin;
    b.piglins = adults.len() as i32;
    b.adults = adults;
    b.hoglins = hoglins;
    b.zombified = zombified;
    b.nemesis = nemesis;
    b.sees_loved = loved;
}
/// `Brain.getActiveNonCoreActivity` after `setActiveActivityToFirstValid`:
/// a piglin's list is admiring, fighting, avoiding, celebrating and idle, a
/// hoglin's fighting, avoiding and idle. An activity is valid while the
/// memory it requires is there.
pub fn activity(m: &Mob) -> Activity {
    let b = &m.extra.piglin;
    let piglin = m.species() == Species::Piglin;
    // `ADMIRING_ITEM`: a gold item in the off hand being admired (a charging
    // crossbow counts `using_item` up too).
    if piglin && m.extra.off_hand.is_some() && m.extra.using_item > 0 {
        Activity::Admire
    } else if m.ai.target.is_some() {
        Activity::Fight
    } else if b.avoid.is_some() {
        Activity::Avoid
    } else if piglin && m.extra.scratch[1] > 0 && m.extra.home.is_some() {
        Activity::Celebrate
    } else {
        Activity::Idle
    }
}
/// `Brain.forgetOutdatedMemories` for the avoid target, which also keeps
/// track of where it stands, then the avoid target's copies of the core
/// activity, the retreat's `EraseMemoryIf`, and `updateActivity`'s record of
/// the activity, at the end of `customServerAiStep`'s brain.
pub fn brain_step(m: &mut Mob, cx: &mut Cx) {
    let expired = match m.extra.piglin.avoid.as_mut() {
        Some(a) => {
            a.ttl.tick();
            !a.ttl.present()
        }
        None => false,
    };
    if expired {
        m.extra.piglin.avoid = None;
    }
    if let Some(a) = m.extra.piglin.avoid.as_mut()
        && let Some(p) = cx.w.target_info(a.who)
    {
        a.at = p.position;
    }
    if m.species() == Species::Piglin {
        copy_to_avoid(m, cx);
    }
    if m.extra.piglin.activity == Activity::Avoid && wants_to_stop_fleeing(m, cx.w) {
        m.extra.piglin.avoid = None;
    }
    m.extra.piglin.activity = activity(m);
}
/// `PiglinAi.getAvoidTarget`: the avoid target while the memory is there.
pub fn avoid_target(m: &Mob) -> Option<Who> {
    m.extra.piglin.avoid.map(|a| a.who)
}
/// `AVOID_TARGET` for `ticks` more ticks (`Brain.setMemoryWithExpiry`).
fn remember_avoid(m: &mut Mob, who: Who, at: Vec3, ticks: i32) {
    let mut ttl = Ttl::default();
    ttl.set(ticks);
    m.extra.piglin.avoid = Some(AvoidTarget { who, at, ttl });
}
/// `TimeUtil.rangeOfSeconds(5, 7)` of `PiglinAi.AVOID_ZOMBIFIED_DURATION` and
/// `BABY_AVOID_NEMESIS_DURATION`, sampled from the level random.
fn avoid_duration(lr: &mut LegacyRandom) -> i32 {
    lr.next_int_between_inclusive(100, 140).unwrap_or(100)
}
/// `PiglinAi.isNearZombified`: the nearest visible zombified piglin or zoglin
/// when it is closer than 6 blocks, and where it stands.
fn near_zombified(m: &Mob, w: &MobWorld) -> Option<(Who, Vec3)> {
    let who = m.extra.piglin.zombified?;
    let at = w.target_info(who)?.position;
    (m.distance_sqr(at) < 36.0).then_some((who, at))
}
/// `PiglinAi.babyAvoidNemesis` and `PiglinAi.avoidZombified`, the core
/// activity's `CopyMemoryWithExpiry` into `AVOID_TARGET`: a baby avoids a
/// wither skeleton or wither it sees, anyone a zombified piglin or zoglin
/// that is near. Each tick it applies draws its expiry from the level random.
fn copy_to_avoid(m: &mut Mob, cx: &mut Cx) {
    if m.is_baby()
        && let Some(who) = m.extra.piglin.nemesis
        && let Some(p) = cx.w.target_info(who)
    {
        let ticks = avoid_duration(cx.lr);
        remember_avoid(m, who, p.position, ticks);
    }
    if let Some((who, at)) = near_zombified(m, cx.w) {
        let ticks = avoid_duration(cx.lr);
        remember_avoid(m, who, at, ticks);
    }
}
/// `HoglinAi.piglinsOutnumberHoglins`: an adult hoglin counts itself and
/// sees more adult piglins than hoglins.
pub fn piglins_outnumber_hoglins(m: &Mob) -> bool {
    !m.is_baby() && m.extra.piglin.piglins > m.extra.piglin.hoglins + 1
}
/// `PiglinAi.hoglinsOutnumberPiglins`: a piglin counts itself and sees more
/// adult hoglins than adult piglins.
pub fn hoglins_outnumber_piglins(m: &Mob) -> bool {
    m.extra.piglin.hoglins > m.extra.piglin.piglins + 1
}
/// `HoglinAi.wantsToStopFleeing` and `PiglinAi.wantsToStopFleeing`, the
/// retreat activity's `EraseMemoryIf`. A hoglin stops once it is an adult
/// and piglins no longer outnumber hoglins; a piglin once the hoglins it
/// avoids no longer outnumber piglins, or the zombified mob it avoids is not
/// the nearest one it sees any more. Any other avoid target waits out its
/// expiry.
fn wants_to_stop_fleeing(m: &Mob, w: &MobWorld) -> bool {
    let b = &m.extra.piglin;
    let Some(a) = b.avoid else { return true };
    if m.species() == Species::Hoglin {
        return !m.is_baby() && !piglins_outnumber_hoglins(m);
    }
    match species_of(w, a.who) {
        Some(Species::Hoglin) => !hoglins_outnumber_piglins(m),
        Some(Species::ZombifiedPiglin | Species::Zoglin) => b.zombified != Some(a.who),
        _ => false,
    }
}
/// `PiglinAi.RETREAT_DURATION` and `HoglinAi.RETREAT_DURATION`:
/// `TimeUtil.rangeOfSeconds(5, 20)`, sampled from the level random.
fn retreat_duration(lr: &mut LegacyRandom) -> i32 {
    lr.next_int_between_inclusive(100, 400).unwrap_or(100)
}
/// `PiglinAi.TIME_BETWEEN_HUNTS` (30 to 120 seconds) sampled from the level
/// random.
fn hunt_wait(lr: &mut LegacyRandom) -> i32 {
    lr.next_int_between_inclusive(600, 2400).unwrap_or(600)
}
/// `HoglinAi.setAvoidTarget`: forget the attack target and the walk, and
/// avoid `who`, standing at `at`, for a while.
pub fn set_avoid_target(m: &mut Mob, lr: &mut LegacyRandom, who: Who, at: Vec3) {
    m.ai.target = None;
    m.nav_stop();
    let ticks = retreat_duration(lr);
    remember_avoid(m, who, at, ticks);
}
/// `PiglinAi.setAvoidTargetAndDontHuntForAWhile`: forget the anger, the
/// attack target and the walk, avoid `who` for a while and start the
/// `HUNTED_RECENTLY` wait (`dontKillAnyMoreHoglinsForAWhile`).
pub fn set_avoid_target_and_dont_hunt(m: &mut Mob, lr: &mut LegacyRandom, who: Who, at: Vec3) {
    m.extra.anger = 0;
    set_avoid_target(m, lr, who, at);
    m.extra.ints[0] = hunt_wait(lr);
}
/// `HoglinAi.retreatFromNearestTarget` and `PiglinAi.retreatFromNearestTarget`:
/// retreat from the nearest of the new target, the avoid target and the
/// attack target (`BehaviorUtils.getNearestTarget`: the first one only when
/// it is strictly closer).
pub fn retreat_from_nearest(m: &mut Mob, w: &MobWorld, lr: &mut LegacyRandom, who: Who, at: Vec3) {
    let mut nearest = (who, at);
    let avoid = m.extra.piglin.avoid.map(|a| (a.who, a.at));
    let attack =
        m.ai.target
            .and_then(|t| w.target_info(t).map(|i| (t, i.position)));
    for other in [avoid, attack].into_iter().flatten() {
        if m.distance_sqr(other.1) < m.distance_sqr(nearest.1) {
            nearest = other;
        }
    }
    if m.species() == Species::Hoglin {
        set_avoid_target(m, lr, nearest.0, nearest.1);
    } else {
        set_avoid_target_and_dont_hunt(m, lr, nearest.0, nearest.1);
    }
}
/// `HoglinAi.broadcastRetreat` and `PiglinAi.broadcastRetreat`: every adult
/// hoglin (for a piglin, every adult piglin but the brutes) this one sees
/// retreats from `who` too, once its tick starts.
pub fn broadcast_retreat(m: &Mob, w: &MobWorld, who: Who, at: Vec3, events: &mut Vec<MobEvent>) {
    for &id in &m.extra.piglin.adults {
        if m.species() == Species::Piglin
            && !w.seen(id).is_some_and(|s| s.species == Species::Piglin)
        {
            continue;
        }
        events.push(MobEvent::Retreat {
            mob: id,
            from: who,
            at: [at.x, at.y, at.z],
        });
    }
}
/// What `broadcastRetreat` asks of a mob: it handles the retreat when its
/// next tick starts.
pub fn receive_retreat(m: &mut Mob, from: Who, at: [f64; 3]) {
    m.extra
        .piglin
        .retreats
        .push((from, Vec3::new(at[0], at[1], at[2])));
}
/// What a piglin or hoglin handles before its goals run: the retreats other
/// mobs broadcast to it and, for a piglin, its celebration.
pub fn before_goals(m: &mut Mob, w: &MobWorld, lr: &mut LegacyRandom) {
    for (from, at) in std::mem::take(&mut m.extra.piglin.retreats) {
        retreat_from_nearest(m, w, lr, from, at);
    }
    if m.species() == Species::Piglin {
        celebrate(m, w);
    }
}
/// What kind of mob `who` is: none for the player.
fn species_of(w: &MobWorld, who: Who) -> Option<Species> {
    match who {
        Who::Player => None,
        Who::Mob(id) => w.seen(id).map(|s| s.species),
    }
}
/// What `SetWalkTargetAwayFrom.entity` walks away from.
#[derive(Clone, Copy, Debug)]
enum AwayFrom {
    /// The retreat activity's `AVOID_TARGET`.
    AvoidTarget,
    /// The idle activity's `NEAREST_VISIBLE_ADULT_PIGLIN` (a hoglin's).
    AdultPiglin,
}
/// `SetWalkTargetAwayFrom.entity(memory, speed, desiredDistance, interruptCurrentWalk)`:
/// closer than the desired distance to what it avoids, pick a spot away
/// from it and walk there, unless a walk is on and may not be interrupted.
/// A walk at the same speed whose spot lies away from it is left alone.
#[derive(Clone, Debug)]
struct WalkAway {
    from: AwayFrom,
    speed: f64,
    desired: f64,
    interrupt: bool,
}
impl WalkAway {
    fn retreat(speed: f64, desired: f64, interrupt: bool) -> Self {
        Self {
            from: AwayFrom::AvoidTarget,
            speed,
            desired,
            interrupt,
        }
    }
    /// Where the avoided entity stands, while the activity this behaviour
    /// belongs to is the active one.
    fn source(&self, m: &Mob, w: &MobWorld) -> Option<Vec3> {
        let b = &m.extra.piglin;
        match self.from {
            AwayFrom::AvoidTarget if b.activity == Activity::Avoid => b.avoid.map(|a| a.at),
            AwayFrom::AdultPiglin if b.activity == Activity::Idle && !m.is_baby() => b
                .nearest_piglin
                .and_then(|id| w.seen(id))
                .map(|s| s.position),
            _ => None,
        }
    }
    /// One run of the behaviour: whether it sets a new walk target, which
    /// it leaves as the path in `pending_path`.
    fn step(&self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(from) = self.source(m, cx.w) else {
            return false;
        };
        let walking = !m.ai.nav.is_done();
        if walking && !self.interrupt {
            return false;
        }
        if m.distance_sqr(from) >= self.desired * self.desired {
            return false;
        }
        if walking
            && m.ai.nav.speed_modifier == self.speed
            && let Some(end) = m.ai.nav.path.as_ref().and_then(|p| p.end())
        {
            let (cx0, cy0, cz0) = (
                f64::from(end.x) + 0.5 - m.position.x,
                f64::from(end.y) - m.position.y,
                f64::from(end.z) + 0.5 - m.position.z,
            );
            let (ax, ay, az) = (
                from.x - m.position.x,
                from.y - m.position.y,
                from.z - m.position.z,
            );
            if cx0 * ax + cy0 * ay + cz0 * az < 0.0 {
                return false;
            }
        }
        let away = [from.x, from.y, from.z];
        for _ in 0..10 {
            if let Some(p) = m.land_random_pos_away(cx.w, 16, 7, away) {
                let at = [
                    p[0].floor() as i32,
                    p[1].floor() as i32,
                    p[2].floor() as i32,
                ];
                m.ai.pending_path = m.create_path_to(cx.w, at, 0);
                return m.ai.pending_path.is_some();
            }
        }
        false
    }
}
impl Goal for WalkAway {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.step(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.source(m, cx.w).is_some() && !m.ai.nav.is_done()
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        let path = m.ai.pending_path.take();
        m.nav_move_to_path(path, self.speed);
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if self.step(m, cx) {
            let path = m.ai.pending_path.take();
            m.nav_move_to_path(path, self.speed);
        }
    }
}
/// A goal of the idle and fight activities: it waits while the brain is in
/// the retreat activity.
#[derive(Clone, Debug)]
struct NotRetreating<G>(G);
impl<G: Goal + Clone + 'static> Goal for NotRetreating<G> {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.piglin.activity != Activity::Avoid && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.piglin.activity != Activity::Avoid && self.0.can_continue(m, cx)
    }
    fn interruptable(&self) -> bool {
        self.0.interruptable()
    }
    fn every_tick(&self) -> bool {
        self.0.every_tick()
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx)
    }
}

fn hostile_to_player(m: &Mob, w: &MobWorld) -> bool {
    // `StartAttacking` is in the idle and celebrate activities, not in the retreat.
    if m.is_baby() || m.extra.piglin.activity == Activity::Avoid {
        return false;
    }
    match m.species() {
        Species::Piglin => m.extra.anger > 0 || w.player.as_ref().is_some_and(|p| !p.gold_armor),
        _ => true,
    }
}
pub fn register_piglin(m: &mut Mob) {
    let crossbow = m.extra.main_hand.as_deref() == Some("minecraft:crossbow");
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(1, Admire);
    // `PiglinAi.initRetreatActivity`: `SetWalkTargetAwayFrom.entity(AVOID_TARGET, 1.0, 12, true)`.
    g.add(2, WalkAway::retreat(1.0, 12.0, true));
    if crossbow {
        g.add(3, super::illager::crossbow_goal(1.0, 8.0));
    }
    g.add(3, MeleeAttack::new(1.0, true));
    g.add(4, PiglinAvoidRepellent);
    g.add(4, GoToWantedItem);
    g.add(4, Celebrate);
    g.add(5, Stroll::avoiding_water(0.6));
    g.add(6, LookAtPlayer::new(8.0));
    g.add(7, RandomLookAround::default());
    let mut hurt = HurtByTarget::new().alert();
    hurt.ignore = vec![Species::Piglin, Species::PiglinBrute];
    m.ai.targets.add(1, hurt);
    m.ai.targets
        .add(2, NearestTarget::player(true).when(hostile_to_player));
    m.ai.targets.add(3, HuntHoglin::default());
    // `PiglinAi.initMemories`: the first hunt waits too.
    m.extra.ints[0] = time_between_hunts(m);
}
/// `PiglinAi.TIME_BETWEEN_HUNTS`: 30 to 120 seconds.
fn time_between_hunts(m: &mut Mob) -> i32 {
    m.random
        .next_int_between_inclusive(600, 2400)
        .unwrap_or(600)
}
/// `StartHuntingHoglin`: an adult piglin that is not angry, has not hunted
/// recently and sees no adult piglin that has, goes for the nearest adult
/// hoglin it sees; then it and the adult piglins near it wait before the next
/// hunt, and the others join in (`broadcastAngerTarget`).
#[derive(Clone, Debug)]
struct HuntHoglin {
    found: Option<u32>,
    st: TargetState,
}
impl Default for HuntHoglin {
    fn default() -> Self {
        let mut st = TargetState::default();
        st.unseen_memory = 60;
        Self { found: None, st }
    }
}
impl Goal for HuntHoglin {
    goal_clone!();
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.is_baby()
            || m.extra.ints[0] > 0
            || m.extra.anger > 0
            || m.ai.target.is_some()
            || m.extra.piglin.activity == Activity::Avoid
        {
            return false;
        }
        let range = m.attrs.follow_range;
        let pos = Vec3::new(m.position.x, m.eye_y(), m.position.z);
        let id = m.id;
        if cx
            .w
            .nearby(pos, range, |s| {
                s.id != id && s.species == Species::Piglin && !s.baby && s.hunted_recently
            })
            .next()
            .is_some()
        {
            return false;
        }
        let hoglins: Vec<(u32, Vec3)> =
            cx.w.nearby(pos, range, |s| s.species == Species::Hoglin && !s.baby)
                .map(|s| (s.id, s.position))
                .collect();
        self.found = hoglins
            .into_iter()
            .find(|&(hoglin, at)| {
                m.distance_sqr(at) <= range * range && m.sees(cx.w, Who::Mob(hoglin))
            })
            .map(|(hoglin, _)| hoglin);
        self.found.is_some()
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        target_continue(&mut self.st, true, m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(hoglin) = self.found else { return };
        m.ai.target = Some(Who::Mob(hoglin));
        m.extra.ints[0] = time_between_hunts(m);
        let range = m.attrs.follow_range;
        let pos = Vec3::new(m.position.x, m.eye_y(), m.position.z);
        let id = m.id;
        let others: Vec<u32> =
            cx.w.nearby(pos, range, |s| {
                s.id != id && s.species == Species::Piglin && !s.baby
            })
            .map(|s| s.id)
            .collect();
        for mob in others {
            let ticks = time_between_hunts(m);
            cx.events.push(MobEvent::HuntHoglin {
                mob,
                target: hoglin,
                ticks,
            });
        }
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.target = None;
    }
}
pub fn register_brute(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(2, MeleeAttack::new(1.0, true));
    g.add(5, Stroll::avoiding_water(0.6));
    g.add(6, LookAtPlayer::new(8.0));
    let mut hurt = HurtByTarget::new().alert();
    hurt.ignore = vec![Species::Piglin, Species::PiglinBrute];
    m.ai.targets.add(1, hurt);
    m.ai.targets.add(2, NearestTarget::player(true));
    m.ai.targets
        .add(3, NearestTarget::species(&[Species::WitherSkeleton], true));
}
pub fn register_hoglin(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    // `HoglinAi.initRetreatActivity`: `SetWalkTargetAwayFrom.entity(AVOID_TARGET, 1.3, 15, false)`.
    g.add(1, WalkAway::retreat(1.3, 15.0, false));
    // HoglinAi: MeleeAttack.create(40) for adults, 15 for babies.
    g.add(2, MeleeAttack::new(1.0, true).intervals(40, 15));
    g.add(3, NotRetreating(super::goals::Breed::new(1.0)));
    g.add(3, HoglinAvoidRepellent);
    g.add(4, NotRetreating(super::goals::FollowParent::new(1.1)));
    g.add(5, Stroll::avoiding_water(0.4));
    g.add(6, LookAtPlayer::new(8.0));
    m.ai.targets.add(1, HurtByTarget::new().alert());
    // `HoglinAi.findNearestValidAttackTarget`: nothing while pacified, and
    // `StartAttacking` is not in the retreat activity.
    m.ai.targets.add(
        2,
        NearestTarget::player(true).when(|m, _| {
            !m.is_baby() && m.extra.ints[0] <= 0 && m.extra.piglin.activity != Activity::Avoid
        }),
    );
}
pub fn register_zoglin(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    // Zoglin.initMemories: MeleeAttack.create(40) for adults, 15 for babies.
    g.add(2, MeleeAttack::new(1.0, true).intervals(40, 15));
    g.add(5, Stroll::avoiding_water(0.4));
    g.add(6, LookAtPlayer::new(8.0));
    m.ai.targets.add(1, HurtByTarget::new());
    m.ai.targets.add(2, NearestTarget::player(true));
    // Zoglin.isTargetable: anything but creepers and other zoglins.
    m.ai.targets.add(
        3,
        NearestTarget::new(
            Prey::Filter(|s| !matches!(s.species, Species::Creeper | Species::Zoglin)),
            true,
        )
        .interval(10),
    );
}
/// `SetWalkTargetAwayFrom.pos(NEAREST_REPELLENT, 1.0, 8, true)` of the hoglin's
/// idle activity: closer than 8 blocks to a repellent, walk away from it.
#[derive(Clone, Debug)]
struct HoglinAvoidRepellent;
impl Goal for HoglinAvoidRepellent {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(r) = m.extra.repellent else {
            return false;
        };
        // The idle activity's behaviour: not in the retreat.
        if m.extra.piglin.activity == Activity::Avoid {
            return false;
        }
        // Vec3.atBottomCenterOf.
        let avoid = [
            f64::from(r[0]) + 0.5,
            f64::from(r[1]),
            f64::from(r[2]) + 0.5,
        ];
        if m.distance_sqr(Vec3::new(avoid[0], avoid[1], avoid[2])) >= 64.0 {
            return false;
        }
        for _ in 0..10 {
            if let Some(p) = m.land_random_pos_away(cx.w, 16, 7, avoid) {
                let at = [
                    p[0].floor() as i32,
                    p[1].floor() as i32,
                    p[2].floor() as i32,
                ];
                m.ai.pending_path = m.create_path_to(cx.w, at, 0);
                return m.ai.pending_path.is_some();
            }
        }
        false
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        let path = m.ai.pending_path.take();
        m.nav_move_to_path(path, 1.0);
    }
}
/// `PiglinAi.avoidRepellent`: `SetWalkTargetAwayFrom.pos(NEAREST_REPELLENT, 1.0, 8, false)`
/// of the idle activity. Closer than 8 blocks to a repellent and not walking
/// anywhere, walk away from it.
#[derive(Clone, Debug)]
struct PiglinAvoidRepellent;
impl Goal for PiglinAvoidRepellent {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(r) = m.extra.repellent else {
            return false;
        };
        // The idle and celebrate activities: not fighting, admiring or
        // retreating, and no walk target yet.
        if m.ai.target.is_some()
            || m.extra.using_item > 0
            || m.extra.piglin.activity == Activity::Avoid
            || !m.ai.nav.is_done()
        {
            return false;
        }
        // Vec3.atBottomCenterOf.
        let avoid = [
            f64::from(r[0]) + 0.5,
            f64::from(r[1]),
            f64::from(r[2]) + 0.5,
        ];
        if m.distance_sqr(Vec3::new(avoid[0], avoid[1], avoid[2])) >= 64.0 {
            return false;
        }
        for _ in 0..10 {
            if let Some(p) = m.land_random_pos_away(cx.w, 16, 7, avoid) {
                let at = [
                    p[0].floor() as i32,
                    p[1].floor() as i32,
                    p[2].floor() as i32,
                ];
                m.ai.pending_path = m.create_path_to(cx.w, at, 0);
                return m.ai.pending_path.is_some();
            }
        }
        false
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        let path = m.ai.pending_path.take();
        m.nav_move_to_path(path, 1.0);
    }
}
/// Admiring a gold item held in the off hand: stand and look at it.
#[derive(Clone, Debug)]
struct Admire;
impl Goal for Admire {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.using_item > 0
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
    }
}
/// The celebrate activity: `GoToTargetLocation` on the spot a killed target
/// fell (within 2 blocks, or 4 and slower while dancing).
#[derive(Clone, Debug)]
struct Celebrate;
impl Celebrate {
    fn walk(m: &mut Mob, cx: &mut Cx) {
        let Some(at) = m.extra.home else { return };
        let (close, speed) = if m.extra.scratch[0] > 0 {
            (4.0, 0.6)
        } else {
            (2.0, 1.0)
        };
        let center = Vec3::new(
            f64::from(at[0]) + 0.5,
            f64::from(at[1]),
            f64::from(at[2]) + 0.5,
        );
        if m.distance_sqr(center) >= close * close {
            m.nav_move_to(cx.w, [center.x, center.y, center.z], speed);
        }
    }
}
impl Goal for Celebrate {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.scratch[1] > 0 && m.extra.home.is_some() && m.extra.using_item == 0
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        Self::walk(m, cx);
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if m.ai.nav.is_done() {
            Self::walk(m, cx);
        }
    }
}
/// `GoToWantedItem`: walk to the gold the game spotted.
#[derive(Clone, Debug)]
struct GoToWantedItem;
impl Goal for GoToWantedItem {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(p) = m.extra.point else {
            return false;
        };
        m.extra.using_item == 0 && m.nav_move_to(cx.w, p, 1.0)
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.point.is_some() && !m.ai.nav.is_done()
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.point = None;
    }
}
/// `HoglinSpecificSensor.findNearestRepellent`: the nearest `#hoglin_repellents`
/// block within 8 blocks sideways and 4 up or down.
fn find_hoglin_repellent(m: &Mob, w: &MobWorld) -> Option<[i32; 3]> {
    let tag = w.block_tags.get("minecraft:hoglin_repellents")?;
    let at = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    w.nearest_block(at, 8, 4, |_, name| tag.contains(name))
}
/// The hoglin's repellent memories: the sensor (every 20 ticks), the end of
/// `PACIFIED` (`ints[0]`), a hit ending it (`HoglinAi.wasHurtBy`, `ints[1]`
/// remembers the hit seen) and `BecomePassiveIfMemoryPresent(NEAREST_REPELLENT, 200)`.
fn hoglin_repellent(m: &mut Mob, w: &MobWorld) {
    if m.extra.ints[0] > 0 {
        m.extra.ints[0] -= 1;
    }
    if m.ai.last_hurt_by.is_some() && m.ai.last_hurt_by_time != m.extra.ints[1] {
        m.extra.ints[1] = m.ai.last_hurt_by_time;
        m.extra.ints[0] = 0;
    }
    if m.age.wrapping_add(m.id).is_multiple_of(20) {
        m.extra.repellent = find_hoglin_repellent(m, w);
        hoglin_sense(m, w);
    }
    // `BecomePassiveIfMemoryPresent` is in the idle and fight activities.
    if m.extra.ints[0] <= 0
        && m.extra.repellent.is_some()
        && m.extra.piglin.activity != Activity::Avoid
    {
        m.extra.ints[0] = 200;
        m.ai.target = None;
    }
}
/// `StartCelebratingIfTargetDead` for a piglin (`scratch[0]` dancing,
/// `scratch[1]` ticks of celebration left, `scratch[2]` the hit seen, `home`
/// the `CELEBRATE_LOCATION`): a killed target starts a celebration at its
/// spot, and a hoglin's death makes the piglin dance one time in ten.
/// `PiglinAi.wasHurtBy` ends both. Runs before the target goals drop the
/// dead target.
pub fn celebrate(m: &mut Mob, w: &MobWorld) {
    if m.extra.scratch[1] > 0 {
        m.extra.scratch[1] -= 1;
        if m.extra.scratch[1] == 0 {
            m.extra.scratch[0] = 0;
            m.extra.home = None;
        }
    }
    if let Some(by) = m.ai.last_hurt_by
        && m.ai.last_hurt_by_time != m.extra.scratch[2]
    {
        m.extra.scratch[2] = m.ai.last_hurt_by_time;
        // PiglinAi.wasHurtBy: another piglin's hit ends nothing.
        let piglin =
            matches!(by, Who::Mob(id) if w.seen(id).is_some_and(|s| s.species == Species::Piglin));
        if !piglin {
            m.extra.scratch[0] = 0;
            m.extra.scratch[1] = 0;
            m.extra.home = None;
        }
    }
    if m.extra.scratch[1] > 0 {
        return;
    }
    let Some(target) = m.ai.target else { return };
    let Some(t) = w.target_info(target).filter(|t| !t.alive) else {
        return;
    };
    let hoglin =
        matches!(target, Who::Mob(id) if w.seen(id).is_some_and(|s| s.species == Species::Hoglin));
    // PiglinAi.wantsToDance: a fresh random seeded with the game time, not
    // the piglin's own.
    if hoglin && LegacyRandom::new(w.game_time).next_float() < 0.1 {
        m.extra.scratch[0] = 1;
    }
    m.extra.scratch[1] = CELEBRATION;
    m.extra.home = Some([
        t.position.x.floor() as i32,
        t.position.y.floor() as i32,
        t.position.z.floor() as i32,
    ]);
}
/// `Hoglin.isConverting` and `AbstractPiglin.isConverting` on a client: in a
/// dimension that is not piglin safe (`nether` is false), a mob that is not
/// immune (`flag2`) and has AI shakes. `counter2 > 0` draws the shake, as it
/// does on the server while the zombification counts up.
pub fn client_converting(m: &mut Mob, nether: bool) {
    m.extra.counter2 = i32::from(!nether && !m.extra.flag2 && !m.no_ai);
}
/// Per-tick piglin and hoglin state: admiring, anger, zombification and the
/// hoglin attack animation. Returns a conversion when it happens.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
    if m.species() == Species::Hoglin && m.alive() {
        hoglin_repellent(m, w);
    }
    if m.species() == Species::Piglin {
        if m.extra.ints[0] > 0 {
            m.extra.ints[0] -= 1;
        }
        // `StopAttackingIfTargetInvalid`: a hoglin handed over by a hunt
        // broadcast has no target goal of its own to drop it when it dies.
        if let Some(who @ Who::Mob(_)) = m.ai.target
            && w.target_info(who)
                .is_none_or(|t| !t.alive || !t.can_be_seen_as_enemy)
        {
            m.ai.target = None;
        }
    }
    if m.extra.anger > 0 {
        m.extra.anger -= 1;
        if m.extra.anger == 0 && m.ai.target == Some(Who::Player) && !hostile_to_player(m, w) {
            m.ai.target = None;
        }
    }
    if m.species() == Species::Piglin && m.extra.ints[0] > 0 {
        m.extra.ints[0] -= 1;
    }
    // Admiring disabled after being hit by a player.
    if m.extra.egg_time > 0 {
        m.extra.egg_time -= 1;
    }
    if m.extra.using_item > 0 {
        m.extra.using_item -= 1;
        if m.extra.using_item == 0 {
            events.push(MobEvent::FinishAdmiring { mob: m.id });
        }
    }
    // Zombification outside the Nether (piglins without immunity, hoglins).
    if !w.nether && !m.extra.flag2 && m.alive() {
        m.extra.counter2 += 1;
        if m.extra.counter2 > CONVERSION {
            let (kind, sound) = match m.species() {
                Species::Hoglin => ("minecraft:zoglin", "entity.hoglin.converted_to_zombified"),
                Species::PiglinBrute => (
                    "minecraft:zombified_piglin",
                    "entity.piglin_brute.converted_to_zombified",
                ),
                _ => (
                    "minecraft:zombified_piglin",
                    "entity.piglin.converted_to_zombified",
                ),
            };
            events.push(MobEvent::Sound(sound, m.center(), 1.0, 1.0));
            events.push(MobEvent::SpawnKind {
                kind: kind.into(),
                at: [m.position.x, m.position.y, m.position.z],
                setup: super::SpawnSetup {
                    baby: m.is_baby(),
                    yaw: m.yaw,
                    persistent: m.persistent,
                    nausea: true,
                    ..Default::default()
                },
            });
            m.removed = true;
        }
    } else {
        m.extra.counter2 = 0;
    }
    // PiglinSpecificSensor (every 20 ticks): the nearest repelling block.
    if m.species() == Species::Piglin && m.alive() && m.age.wrapping_add(m.id).is_multiple_of(20) {
        m.extra.repellent = find_piglin_repellent(m, w);
        piglin_sense(m, w);
    }
}
/// `PiglinSpecificSensor.findNearestRepellent`: the nearest `#piglin_repellents`
/// block within 8 blocks sideways and 4 up or down; a soul campfire only
/// counts while lit.
fn find_piglin_repellent(m: &Mob, w: &MobWorld) -> Option<[i32; 3]> {
    let tag = w.block_tags.get("minecraft:piglin_repellents")?;
    let at = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    w.nearest_block(at, 8, 4, |p, name| {
        tag.contains(name)
            && (name != "minecraft:soul_campfire"
                || w.prop(p[0], p[1], p[2], "lit") == Some("true"))
    })
}
/// `HoglinBase.throwTarget`: a shove away and up of the attack knockback less
/// the target's knockback resistance (none at all when that is used up).
fn throw_target(m: &mut Mob, who: Who, cx: &mut Cx) {
    let resistance = match who {
        Who::Mob(id) => cx.w.seen(id).map_or(0.0, |s| s.knockback_resistance),
        Who::Player => 0.0,
    };
    let power = m.kind.attack_knockback - resistance;
    let Some(t) = cx.w.target_info(who) else {
        return;
    };
    if power <= 0.0 {
        return;
    }
    let away = Vec3::new(
        t.position.x - m.position.x,
        0.0,
        t.position.z - m.position.z,
    );
    // `yRot` takes the random whole number of degrees as radians, as vanilla does.
    let angle = (m.random.next_int_bound(21).unwrap_or(10) - 10) as f32;
    let scale = power * f64::from(m.random.next_float() * 0.5 + 0.2);
    let push = away.normalize().scale(scale).rotate_y(angle);
    let up = power * f64::from(m.random.next_float()) * 0.5;
    cx.events.push(MobEvent::Push {
        target: who,
        velocity: [push.x, up, push.z],
    });
}
/// `HoglinAi.onHitTarget`: an adult that hits a piglin while piglins
/// outnumber the hoglins around it retreats from that piglin and tells the
/// hoglins it sees to. (The broadcast of the attack target otherwise is not
/// ported.)
fn on_hit_target(m: &mut Mob, who: Who, cx: &mut Cx) {
    if m.is_baby() {
        return;
    }
    let Some(at) = cx.w.target_info(who).map(|t| t.position) else {
        return;
    };
    if species_of(cx.w, who) == Some(Species::Piglin) && piglins_outnumber_hoglins(m) {
        set_avoid_target(m, cx.lr, who, at);
        broadcast_retreat(m, cx.w, who, at, cx.events);
    }
}
/// `HoglinBase.hurtAndThrowTarget`.
pub fn hoglin_hurt_target(m: &mut Mob, who: Who, cx: &mut Cx) {
    m.extra.anim_ticks = 10;
    // Entity event 4: clients start the headbutt.
    cx.events.push(MobEvent::EntityEvent {
        mob: m.id,
        event: 4,
        at: [m.position.x, m.position.y, m.position.z],
    });
    if m.species() == Species::Hoglin {
        on_hit_target(m, who, cx);
    }
    let attack = m.attrs.attack_damage;
    let amount = if !m.is_baby() && attack as i32 > 0 {
        attack / 2.0 + m.random.next_int_bound(attack as i32).unwrap_or(0) as f32
    } else {
        attack
    };
    let from = [m.position.x, m.position.y, m.position.z];
    match who {
        Who::Player => cx.events.push(MobEvent::AttackPlayer(amount, from)),
        Who::Mob(id) => cx.events.push(MobEvent::AttackMob {
            target: id,
            amount,
            source: super::Damage::mob(m.id, from),
        }),
    }
    if !m.is_baby() {
        throw_target(m, who, cx);
    }
    let sound = if m.species() == Species::Zoglin {
        "entity.zoglin.attack"
    } else {
        "entity.hoglin.attack"
    };
    cx.events.push(MobEvent::Sound(sound, m.center(), 1.0, 1.0));
}
/// `AbstractPiglinModel.setupAnim`: the ears flap around their default angle
/// (30 degrees for adults, 5 for babies).
pub fn ears(p: &mut Poses, s: &State) {
    use horizon_core::geometry::trig::DEG_TO_RAD;
    let default = if s.baby { 5.0 } else { 30.0 } * DEG_TO_RAD;
    let freq = s.age * 0.1 + s.pos * 0.5;
    let amp = 0.08 + s.speed * 0.4;
    p.zrot("left_ear", -default - (freq * 1.2).mcos() * amp);
    p.zrot("right_ear", default + freq.mcos() * amp);
}
/// Piglins' and brutes' ears flap (`AbstractPiglinModel`), arms follow the
/// arm pose (`PiglinModel`).
pub fn piglin_anim(m: &Mob, s: &State, p: &mut Poses) {
    // Piglin.getArmPose: dancing comes first.
    let pose = if m.extra.scratch[0] > 0 {
        PiglinPose::Dancing
    } else if m.extra.using_item > 0 {
        PiglinPose::AdmiringItem
    } else if m.extra.flag {
        PiglinPose::CrossbowCharge
    } else if m.extra.main_hand.as_deref() == Some("minecraft:crossbow") && m.ai.aggressive {
        PiglinPose::CrossbowHold
    } else if m.ai.aggressive && m.extra.main_hand.is_some() {
        PiglinPose::MeleeWeapon
    } else {
        PiglinPose::Default
    };
    piglin_pose(
        p,
        s,
        pose,
        25.0,
        m.extra.using_item as f32,
        m.main_arm_left(),
    );
}
/// `PiglinArmPose`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PiglinPose {
    Default,
    Dancing,
    MeleeWeapon,
    CrossbowHold,
    CrossbowCharge,
    AdmiringItem,
}
/// `PiglinModel.setupAnim`; `left` when the piglin's main arm is the left one.
pub fn piglin_pose(
    p: &mut Poses,
    s: &State,
    pose: PiglinPose,
    max_charge: f32,
    ticks: f32,
    left: bool,
) {
    use super::anim::{ArmPose, Hum, crossbow_charge, crossbow_hold, humanoid};
    let mut h = Hum::new(ArmPose::Empty, ArmPose::Empty);
    h.weapon_swing = pose == PiglinPose::MeleeWeapon;
    h.left_main = left;
    h.charge = (max_charge, ticks);
    humanoid(p, s, &h);
    ears(p, s);
    let (hy, hx) = (s.y_rot.to_radians(), s.x_rot.to_radians());
    let rad = |v: f64| v as f32;
    let deg = horizon_core::geometry::trig::DEG_TO_RAD;
    match pose {
        PiglinPose::Dancing => {
            let dance = s.age / 60.0;
            p.zrot(
                "right_ear",
                rad(std::f64::consts::PI / 6.0) + deg * (dance * 30.0).msin() * 10.0,
            );
            p.zrot(
                "left_ear",
                rad(-std::f64::consts::PI / 6.0) - deg * (dance * 30.0).mcos() * 10.0,
            );
            p.get("head")[0] += (dance * 10.0).msin();
            p.get("head")[1] += (dance * 40.0).msin() + 0.4;
            let z = deg * (70.0 + (dance * 40.0).mcos() * 10.0);
            p.zrot("right_arm", z);
            p.zrot("left_arm", -z);
            p.get("right_arm")[1] += (dance * 40.0).msin() * 0.5 - 0.5;
            p.get("left_arm")[1] += (dance * 40.0).msin() * 0.5 + 0.5;
            p.get("body")[1] += (dance * 40.0).msin() * 0.35;
        }
        // holdWeaponHigh
        PiglinPose::MeleeWeapon if s.swing == 0.0 => {
            p.xrot(if left { "left_arm" } else { "right_arm" }, -1.8)
        }
        PiglinPose::CrossbowHold => crossbow_hold(p, hx, hy, !left),
        PiglinPose::CrossbowCharge => crossbow_charge(p, max_charge, ticks, !left),
        PiglinPose::AdmiringItem => {
            p.xrot("head", 0.5);
            p.yrot("head", 0.0);
            if left {
                p.yrot("right_arm", -0.5);
                p.xrot("right_arm", -0.9);
            } else {
                p.yrot("left_arm", 0.5);
                p.xrot("left_arm", -0.9);
            }
        }
        _ => {}
    }
}
/// `Hoglin.handleEntityEvent` and `Zoglin`'s event 4 on a client: the
/// headbutt, which `aiStep` then counts down.
pub fn client_event(m: &mut Mob, event: u8) {
    if event == 4 {
        m.extra.anim_ticks = 10;
    }
}
/// The headbutt's countdown (`Hoglin.aiStep`, `Zoglin.aiStep`) on a client.
pub fn client_tick(m: &mut Mob) {
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
}
/// `HoglinModel.setupAnim`.
pub fn hoglin_anim(m: &Mob, s: &State, p: &mut Poses) {
    let (pos, sp) = (s.pos, s.speed);
    p.zrot("right_ear", -PI * 2.0 / 9.0 - sp * pos.msin());
    p.zrot("left_ear", PI * 2.0 / 9.0 + sp * pos.msin());
    p.yrot("head", s.y_rot.to_radians());
    let remaining = m.extra.anim_ticks as f32;
    let f = 1.0 - (10.0 - 2.0 * remaining).abs() / 10.0;
    p.xrot("head", 0.872_664_63 + (-PI / 9.0 - 0.872_664_63) * f);
    if s.baby {
        p.get("head")[1] += f * 2.5;
    }
    p.xrot("right_front_leg", pos.mcos() * 1.2 * sp);
    p.xrot("left_front_leg", (pos + PI).mcos() * 1.2 * sp);
    p.xrot("right_hind_leg", (pos + PI).mcos() * 1.2 * sp);
    p.xrot("left_hind_leg", pos.mcos() * 1.2 * sp);
}
