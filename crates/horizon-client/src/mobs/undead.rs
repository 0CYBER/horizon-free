//! Zombies (zombie, husk, drowned, zombie villager) and skeletons (skeleton,
//! stray, bogged, wither skeleton): goals, bow attacks, water conversion.
use super::{
    Mob, MobEvent, Species, Who,
    goals::{
        Avoid, AvoidEntity, Cx, FleeSun, Goal, HurtByTarget, LOOK, LookAtPlayer, MOVE, MeleeAttack,
        NearestTarget, RandomLookAround, RestrictSun, Stroll, goal_clone,
    },
};
use crate::projectiles::{Projectile, ProjectileKind};
use horizon_core::{geometry::Vec3, random::RandomSource};

pub fn register(m: &mut Mob) {
    use Species as S;
    match m.species() {
        S::Zombie | S::Husk | S::ZombieVillager => {
            let g = &mut m.ai.goals;
            g.add(8, LookAtPlayer::new(8.0));
            g.add(8, RandomLookAround::default());
            g.add(3, MeleeAttack::new(1.0, false).zombie());
            g.add(7, Stroll::avoiding_water(1.0));
            let t = &mut m.ai.targets;
            let mut hurt = HurtByTarget::new().alert();
            hurt.alert_except = vec![S::ZombifiedPiglin];
            t.add(1, hurt);
            t.add(2, NearestTarget::player(true));
            t.add(
                3,
                NearestTarget::species(&[S::Villager, S::WanderingTrader], false),
            );
            t.add(3, NearestTarget::species(&[S::IronGolem], true));
            t.add(
                5,
                NearestTarget::new(
                    super::goals::Prey::Filter(|s| s.species == Species::Turtle && s.baby),
                    true,
                )
                .interval(10),
            );
        }
        S::ZombifiedPiglin => {
            // ZombifiedPiglin.addBehaviourGoals: neutral until provoked.
            let g = &mut m.ai.goals;
            g.add(8, LookAtPlayer::new(8.0));
            g.add(8, RandomLookAround::default());
            g.add(2, MeleeAttack::new(1.0, false).zombie());
            g.add(7, Stroll::avoiding_water(1.0));
            let t = &mut m.ai.targets;
            t.add(1, HurtByTarget::new().alert());
            t.add(
                2,
                NearestTarget::player(true)
                    .interval(10)
                    .when(|m, _| m.extra.anger > 0),
            );
        }
        S::Drowned => {
            let g = &mut m.ai.goals;
            g.add(1, DrownedGoToWater::default());
            g.add(2, MeleeAttack::new(1.0, false).zombie());
            g.add(5, Stroll::new(1.0));
            g.add(8, LookAtPlayer::new(8.0));
            g.add(8, RandomLookAround::default());
            let t = &mut m.ai.targets;
            let mut hurt = HurtByTarget::new().alert();
            hurt.alert_except = vec![S::ZombifiedPiglin];
            t.add(1, hurt);
            t.add(2, NearestTarget::player(false).when(drowned_can_attack));
            t.add(
                3,
                NearestTarget::species(&[S::Villager, S::WanderingTrader], false),
            );
            t.add(3, NearestTarget::species(&[S::IronGolem], true));
            t.add(3, NearestTarget::species(&[S::Axolotl], true).interval(10));
            m.ai.nav.can_float = false;
        }
        S::Skeleton | S::Stray | S::Bogged | S::Parched | S::WitherSkeleton => {
            let species = m.species();
            let g = &mut m.ai.goals;
            g.add(2, RestrictSun);
            g.add(3, FleeSun::new(1.0));
            g.add(
                3,
                AvoidEntity::new(Avoid::Species(vec![S::Wolf]), 6.0, 1.0, 1.2),
            );
            g.add(5, Stroll::avoiding_water(1.0));
            g.add(6, LookAtPlayer::new(8.0));
            g.add(6, RandomLookAround::default());
            let t = &mut m.ai.targets;
            t.add(1, HurtByTarget::new());
            t.add(2, NearestTarget::player(true));
            t.add(3, NearestTarget::species(&[S::IronGolem], true));
            t.add(
                3,
                NearestTarget::new(
                    super::goals::Prey::Filter(|s| s.species == Species::Turtle && s.baby),
                    true,
                )
                .interval(10),
            );
            if species == S::WitherSkeleton {
                t.add(
                    3,
                    NearestTarget::species(&[S::Piglin, S::PiglinBrute], true),
                );
                m.extra.main_hand = Some("minecraft:stone_sword".into());
                m.ai.goals.add(4, MeleeAttack::new(1.2, false));
            } else {
                m.extra.main_hand = Some("minecraft:bow".into());
                let interval = if matches!(m.species(), S::Bogged | S::Parched) {
                    70
                } else {
                    40
                };
                m.ai.goals.add(4, BowAttack::new(1.0, interval, 15.0));
            }
        }
        _ => {}
    }
}
fn drowned_can_attack(m: &Mob, w: &super::MobWorld) -> bool {
    // Drowned.okTarget: at night or when the target is in water.
    let Some(p) = &w.player else { return false };
    let (x, y, z) = (
        p.position.x.floor() as i32,
        p.position.y.floor() as i32,
        p.position.z.floor() as i32,
    );
    !w.daylight || w.fluid(x, y, z) == crate::blocks::Fluid::Water || m.fluids.in_water
}
/// Zombie drowning conversion (Zombie.DrowningTracker) and husk -> zombie.
/// `ZombifiedPiglin.customServerAiStep`: faster and loud while angry,
/// forgetting the player when the anger runs out, rallying the others.
fn zombified_piglin_step(m: &mut Mob, cx: &mut Cx) {
    let base = 0.23;
    // NeutralMob.updatePersistentAnger: a new target starts 20-39 s of anger.
    if m.ai.target.is_some() && !m.extra.flag2 {
        m.extra.flag2 = true;
        m.extra.anger = 400 + m.random.next_int_bound(380).unwrap_or(0);
        m.extra.counter = m.random.next_int_bound(21).unwrap_or(0).max(1);
        m.extra.counter2 = 80 + m.random.next_int_bound(41).unwrap_or(0);
    } else if m.ai.target.is_none() {
        m.extra.flag2 = false;
    }
    if m.extra.anger > 0 {
        m.extra.anger -= 1;
        if !m.is_baby() {
            m.attrs.speed = base + 0.05;
        }
        // playFirstAngerSoundIn
        if m.extra.counter > 0 {
            m.extra.counter -= 1;
            if m.extra.counter == 0 {
                let base = if m.is_baby() { 1.5 } else { 1.0 };
                let pitch = ((m.random.next_float() - m.random.next_float()) * 0.2 + base) * 1.8;
                cx.events.push(MobEvent::Sound(
                    "entity.zombified_piglin.angry",
                    m.center(),
                    2.0,
                    pitch,
                ));
            }
        }
        if m.extra.anger == 0 {
            m.ai.target = None;
            m.extra.flag2 = false;
        }
    } else {
        m.attrs.speed = base;
    }
    // maybeAlertOthers every 4 to 6 seconds while seeing the target.
    if let Some(target) = m.ai.target {
        if m.extra.counter2 > 0 {
            m.extra.counter2 -= 1;
        } else {
            if m.sees(cx.w, target) {
                let range = m.attrs.follow_range;
                let (id, pos) = (m.id, m.position);
                let others: Vec<u32> =
                    cx.w.others
                        .iter()
                        .filter(|s| {
                            s.id != id
                                && s.alive
                                && s.species == Species::ZombifiedPiglin
                                && s.target.is_none()
                                && (s.position.x - pos.x).abs() <= range + 1.0
                                && (s.position.y - pos.y).abs() <= 11.0
                                && (s.position.z - pos.z).abs() <= range + 1.0
                        })
                        .map(|s| s.id)
                        .collect();
                for other in others {
                    cx.events.push(MobEvent::SetTarget { mob: other, target });
                }
            }
            m.extra.counter2 = 80 + m.random.next_int_bound(41).unwrap_or(0);
        }
    }
}
pub fn custom_server_ai_step(m: &mut Mob, cx: &mut Cx) {
    if m.species() == Species::ZombieVillager {
        super::zombie_villager::step(m, cx);
    }
    if m.species() == Species::ZombifiedPiglin {
        return zombified_piglin_step(m, cx);
    }
    if matches!(m.species(), Species::Zombie | Species::Husk) && m.alive() {
        if m.extra.counter2 > 0 {
            m.extra.counter2 -= 1;
            if m.extra.counter2 == 0 {
                let (kind, sound) = if m.species() == Species::Husk {
                    ("minecraft:zombie", "entity.husk.converted_to_zombie")
                } else {
                    ("minecraft:drowned", "entity.zombie.converted_to_drowned")
                };
                cx.events.push(MobEvent::Sound(sound, m.center(), 1.0, 1.0));
                cx.events.push(MobEvent::SpawnKind {
                    kind: kind.into(),
                    at: [m.position.x, m.position.y, m.position.z],
                    setup: super::SpawnSetup {
                        baby: m.is_baby(),
                        yaw: m.yaw,
                        persistent: m.persistent,
                        ..Default::default()
                    },
                });
                m.removed = true;
            }
        } else if m.fluids.eye_in_water {
            m.extra.counter += 1;
            if m.extra.counter >= 600 {
                m.extra.counter2 = 300;
            }
        } else {
            m.extra.counter = -1;
        }
    }
    if m.species() == Species::Drowned {
        // Drowned.updateSwimming / travel: swim when the target is in water.
        let wants =
            m.ai.target
                .and_then(|t| cx.w.target_info(t))
                .is_some_and(|t| {
                    let (x, y, z) = (
                        t.position.x.floor() as i32,
                        t.position.y.floor() as i32,
                        t.position.z.floor() as i32,
                    );
                    cx.w.fluid(x, y, z) == crate::blocks::Fluid::Water
                });
        m.extra.flag = wants;
        if m.fluids.in_water && wants {
            m.ai.nav.kind = super::nav::NavKind::Water { breach: false };
        } else if m.ai.nav.kind != super::nav::NavKind::Ground {
            m.ai.nav.kind = super::nav::NavKind::Ground;
            m.ai.nav.stop();
        }
    }
}
/// RangedBowAttackGoal for skeletons.
#[derive(Clone, Debug)]
pub struct BowAttack {
    speed: f64,
    interval_min: i32,
    radius_sqr: f32,
    attack_time: i32,
    see_time: i32,
    strafe_clockwise: bool,
    strafe_back: bool,
    strafe_time: i32,
}
impl BowAttack {
    pub fn new(speed: f64, interval_min: i32, radius: f32) -> Self {
        Self {
            speed,
            interval_min,
            radius_sqr: radius * radius,
            attack_time: -1,
            see_time: 0,
            strafe_clockwise: false,
            strafe_back: false,
            strafe_time: -1,
        }
    }
}
fn holding_bow(m: &Mob) -> bool {
    m.extra.main_hand.as_deref() == Some("minecraft:bow")
}
impl Goal for BowAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.ai.target.is_some() && holding_bow(m)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        (self.can_use(m, cx) || !m.ai.nav.is_done()) && holding_bow(m)
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.aggressive = true;
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.aggressive = false;
        self.see_time = 0;
        self.attack_time = -1;
        m.extra.using_item = 0;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(who) = m.ai.target else { return };
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        let d = m.distance_sqr(t.position);
        let sees = m.sees(cx.w, who);
        if sees != (self.see_time > 0) {
            self.see_time = 0;
        }
        if sees {
            self.see_time += 1;
        } else {
            self.see_time -= 1;
        }
        if d <= f64::from(self.radius_sqr) && self.see_time >= 20 {
            m.nav_stop();
            self.strafe_time += 1;
        } else {
            m.nav_move_to_entity(
                cx.w,
                [t.position.x, t.position.y, t.position.z],
                1,
                self.speed,
            );
            self.strafe_time = -1;
        }
        if self.strafe_time >= 20 {
            if m.random.next_float() < 0.3 {
                self.strafe_clockwise = !self.strafe_clockwise;
            }
            if m.random.next_float() < 0.3 {
                self.strafe_back = !self.strafe_back;
            }
            self.strafe_time = 0;
        }
        let eye = [t.position.x, t.eye_y, t.position.z];
        if self.strafe_time > -1 {
            if d > f64::from(self.radius_sqr * 0.75) {
                self.strafe_back = false;
            } else if d < f64::from(self.radius_sqr * 0.25) {
                self.strafe_back = true;
            }
            m.ai.mov.strafe(
                if self.strafe_back { -0.5 } else { 0.5 },
                if self.strafe_clockwise { 0.5 } else { -0.5 },
            );
            m.turn_to(eye, 30.0, 30.0);
        } else {
            m.look_at_speed(eye, 30.0, 30.0);
        }
        if m.extra.using_item > 0 {
            if !sees && self.see_time < -60 {
                m.extra.using_item = 0;
            } else if sees {
                let pull = m.extra.using_item;
                if pull >= 20 {
                    m.extra.using_item = 0;
                    perform_ranged_attack(m, who, cx, bow_power(pull));
                    self.attack_time = self.interval_min;
                }
            }
            if m.extra.using_item > 0 {
                m.extra.using_item += 1;
            }
        } else {
            self.attack_time -= 1;
            if self.attack_time <= 0 && self.see_time >= -60 {
                m.extra.using_item = 1;
            }
        }
    }
}
/// BowItem.getPowerForTime
pub fn bow_power(ticks: i32) -> f32 {
    let t = ticks as f32 / 20.0;
    ((t * t + t * 2.0) / 3.0).min(1.0)
}
/// AbstractSkeleton.performRangedAttack: aim 20% above the target's lower
/// third, speed 1.6, uncertainty 14 - 4 * difficulty.
pub fn perform_ranged_attack(m: &mut Mob, who: Who, cx: &mut Cx, power: f32) {
    let Some(t) = cx.w.target_info(who) else {
        return;
    };
    let start = Vec3::new(m.position.x, m.eye_y() - 0.1, m.position.z);
    let height = t.bounds.max_position().y - t.bounds.min_position().y;
    let xd = t.position.x - m.position.x;
    let yd = t.position.y + height / 3.0 - start.y;
    let zd = t.position.z - m.position.z;
    let dist = (xd * xd + zd * zd).sqrt();
    // AbstractArrow.setBaseDamageFromMob: power * 2 + triangle(difficulty * 0.11, 0.57425).
    let base = f64::from(power) * 2.0 + m.random.triangle(2.0 * 0.11, 0.574_25);
    let effect = match m.species() {
        Species::Stray => Some(("minecraft:slowness", 600)),
        Species::Bogged => Some(("minecraft:poison", 100)),
        Species::Parched => Some(("minecraft:weakness", 600)),
        _ => None,
    };
    let mut p = Projectile::new(
        ProjectileKind::Arrow {
            base_damage: base,
            critical: false,
            effect,
        },
        start,
        Some(Who::Mob(m.id)),
    );
    p.shoot(
        &mut m.random,
        [xd, yd + dist * 0.2, zd],
        1.6,
        (14 - 2 * 4) as f32,
    );
    cx.events.push(MobEvent::Projectile(p));
    let pitch = 1.0 / (m.random.next_float() * 0.4 + 0.8);
    cx.events.push(MobEvent::Sound(
        "entity.skeleton.shoot",
        m.center(),
        1.0,
        pitch,
    ));
}
/// Drowned.DrownedGoToWaterGoal (simplified to "head for water by day").
#[derive(Clone, Debug, Default)]
pub struct DrownedGoToWater {
    wanted: [f64; 3],
}
impl Goal for DrownedGoToWater {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !cx.w.daylight || m.fluids.in_water {
            return false;
        }
        let (bx, by, bz) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        for _ in 0..10 {
            let p = [
                bx + m.random.next_int_bound(20).unwrap_or(0) - 10,
                by + 2 - m.random.next_int_bound(8).unwrap_or(0),
                bz + m.random.next_int_bound(20).unwrap_or(0) - 10,
            ];
            if cx.w.fluid(p[0], p[1], p[2]) == crate::blocks::Fluid::Water {
                self.wanted = [
                    f64::from(p[0]) + 0.5,
                    f64::from(p[1]),
                    f64::from(p[2]) + 0.5,
                ];
                return true;
            }
        }
        false
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.nav_move_to(cx.w, self.wanted, 1.0);
    }
}
