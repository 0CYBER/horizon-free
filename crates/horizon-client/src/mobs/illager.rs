//! Illagers outside raids: pillagers (crossbow charging and volleys) and
//! vindicators (axes, Johnny), with `IllagerModel`'s arm poses.
use super::{
    Mob, MobEvent, Species, Who,
    anim::{Poses, State},
    goals::{
        Cx, Float, Goal, HurtByTarget, LOOK, LookAtPlayer, MOVE, MeleeAttack, NearestTarget, Prey,
        Stroll, goal_clone,
    },
};
use crate::projectiles::{Projectile, ProjectileKind};
use horizon_core::{geometry::Vec3, random::RandomSource};
use std::f32::consts::PI;

/// The species raiders do not fight (`Raider`'s allies).
fn raider(s: Species) -> bool {
    matches!(s, Species::Pillager | Species::Vindicator | Species::Witch)
}
fn targets(m: &mut Mob, villagers_seen: bool) {
    let mut hurt = HurtByTarget::new().alert();
    hurt.ignore = vec![Species::Pillager, Species::Vindicator, Species::Witch];
    m.ai.targets.add(1, hurt);
    m.ai.targets.add(2, NearestTarget::player(true));
    m.ai.targets.add(
        3,
        NearestTarget::species(
            &[Species::Villager, Species::WanderingTrader],
            villagers_seen,
        ),
    );
    m.ai.targets
        .add(3, NearestTarget::species(&[Species::IronGolem], true));
}
pub fn register_pillager(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(3, CrossbowAttack::new(1.0, 8.0));
    g.add(8, Stroll::new(0.6));
    g.add(9, LookAtPlayer::new(15.0));
    targets(m, false);
}
pub fn register_vindicator(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(5, MeleeAttack::new(1.0, false));
    g.add(8, Stroll::new(0.6));
    g.add(9, LookAtPlayer::new(3.0));
    targets(m, true);
    // VindicatorJohnnyAttackGoal: a vindicator named Johnny attacks anything.
    m.ai.targets.add(
        4,
        NearestTarget::new(Prey::Filter(|s| !raider(s.species)), true)
            .interval(0)
            .when(|m, _| m.custom_name.as_deref() == Some("Johnny")),
    );
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Crossbow {
    Uncharged,
    Charging,
    Charged,
    Ready,
}
/// `RangedCrossbowAttackGoal`.
#[derive(Clone, Debug)]
struct CrossbowAttack {
    speed: f64,
    radius_sqr: f64,
    state: Crossbow,
    see_time: i32,
    attack_delay: i32,
    path_delay: i32,
}
impl CrossbowAttack {
    fn new(speed: f64, radius: f64) -> Self {
        Self {
            speed,
            radius_sqr: radius * radius,
            state: Crossbow::Uncharged,
            see_time: 0,
            attack_delay: 0,
            path_delay: 0,
        }
    }
}
/// `CrossbowItem.getChargeDuration` without Quick Charge.
const CHARGE: i32 = 25;
fn holding_crossbow(m: &Mob) -> bool {
    m.extra.main_hand.as_deref() == Some("minecraft:crossbow")
}
impl Goal for CrossbowAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        holding_crossbow(m)
            && m.ai
                .target
                .and_then(|t| cx.w.target_info(t))
                .is_some_and(|t| t.alive && !(t.is_player && t.creative))
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let valid =
            m.ai.target
                .and_then(|t| cx.w.target_info(t))
                .is_some_and(|t| t.alive && !(t.is_player && t.creative));
        valid && holding_crossbow(m)
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.aggressive = false;
        m.ai.target = None;
        self.see_time = 0;
        m.extra.flag = false;
        m.extra.using_item = 0;
        self.state = Crossbow::Uncharged;
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.aggressive = true;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(who) = m.ai.target else {
            return;
        };
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        let sees = m.sees(cx.w, who);
        if sees != (self.see_time > 0) {
            self.see_time = 0;
        }
        self.see_time += if sees { 1 } else { -1 };
        let d2 = m.distance_sqr(t.position);
        let needs_to_move = (d2 > self.radius_sqr || self.see_time < 5) && self.attack_delay == 0;
        if needs_to_move {
            self.path_delay -= 1;
            if self.path_delay <= 0 {
                let speed = if self.state == Crossbow::Uncharged {
                    self.speed
                } else {
                    self.speed * 0.5
                };
                m.nav_move_to_entity(cx.w, [t.position.x, t.position.y, t.position.z], 0, speed);
                self.path_delay = 20 + m.random.next_int_bound(21).unwrap_or(0);
            }
        } else {
            self.path_delay = 0;
            m.nav_stop();
        }
        m.look_at_speed([t.position.x, t.eye_y, t.position.z], 30.0, 30.0);
        match self.state {
            Crossbow::Uncharged => {
                if !needs_to_move {
                    self.state = Crossbow::Charging;
                    m.extra.flag = true;
                    m.extra.using_item = 0;
                    let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
                    cx.events.push(MobEvent::Sound(
                        "item.crossbow.loading_start",
                        m.center(),
                        1.0,
                        pitch,
                    ));
                }
            }
            Crossbow::Charging => {
                m.extra.using_item += 1;
                if m.extra.using_item >= CHARGE {
                    m.extra.using_item = 0;
                    m.extra.flag = false;
                    self.state = Crossbow::Charged;
                    self.attack_delay = 20 + m.random.next_int_bound(20).unwrap_or(0);
                    cx.events.push(MobEvent::Sound(
                        "item.crossbow.loading_end",
                        m.center(),
                        1.0,
                        1.0 / (m.random.next_float() * 0.5 + 1.0) + 0.2,
                    ));
                }
            }
            Crossbow::Charged => {
                self.attack_delay -= 1;
                if self.attack_delay == 0 {
                    self.state = Crossbow::Ready;
                }
            }
            Crossbow::Ready => {
                if sees {
                    shoot_crossbow(m, who, cx);
                    self.state = Crossbow::Uncharged;
                }
            }
        }
    }
}
/// `CrossbowAttackMob.performCrossbowAttack` at power 1.6: one arrow aimed a
/// third of the way up the target, lifted by a fifth of the distance.
fn shoot_crossbow(m: &mut Mob, who: Who, cx: &mut Cx) {
    let Some(t) = cx.w.target_info(who) else {
        return;
    };
    let start = Vec3::new(m.position.x, m.eye_y() - 0.1, m.position.z);
    let dx = t.position.x - m.position.x;
    let dz = t.position.z - m.position.z;
    let dist = (dx * dx + dz * dz).sqrt();
    let third =
        t.bounds.min_position().y + (t.bounds.max_position().y - t.bounds.min_position().y) / 3.0;
    let dy = third - start.y + dist * 0.2;
    let mut p = Projectile::new(
        ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        start,
        Some(Who::Mob(m.id)),
    );
    // rangedAttackUncertainty at normal difficulty: 14 - 2 * 4.
    p.shoot(&mut m.random, [dx, dy, dz], 1.6, 6.0);
    cx.events.push(MobEvent::Projectile(p));
    let pitch = 1.0 / (m.random.next_float() * 0.5 + 1.8) + 0.53;
    cx.events.push(MobEvent::Sound(
        "item.crossbow.shoot",
        m.center(),
        1.0,
        pitch,
    ));
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ArmPose {
    Crossed,
    Neutral,
    Attacking,
    CrossbowHold,
    CrossbowCharge,
}
/// `Pillager` / `Vindicator.getArmPose`.
fn arm_pose(m: &Mob) -> ArmPose {
    match m.species() {
        Species::Pillager => {
            if m.extra.flag {
                ArmPose::CrossbowCharge
            } else if holding_crossbow(m) {
                ArmPose::CrossbowHold
            } else if m.ai.aggressive {
                ArmPose::Attacking
            } else {
                ArmPose::Neutral
            }
        }
        _ => {
            if m.ai.aggressive {
                ArmPose::Attacking
            } else {
                ArmPose::Crossed
            }
        }
    }
}
/// Whether the held item shows (vindicators only draw their axe when angry).
pub fn shows_item(m: &Mob) -> bool {
    m.species() != Species::Vindicator || m.ai.aggressive
}
fn hide(p: &mut Poses, part: &str) {
    if p.has(part) {
        let v = p.get(part);
        v[6] = 0.0;
        v[7] = 0.0;
        v[8] = 0.0;
    }
}
/// `IllagerModel.setupAnim`.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    let (hy, hx) = (s.y_rot.to_radians(), s.x_rot.to_radians());
    p.yrot("head", hy);
    p.xrot("head", hx);
    let (pos, sp) = (s.pos, s.speed);
    p.xrot("right_arm", (pos * 0.6662 + PI).cos() * 2.0 * sp * 0.5);
    p.yrot("right_arm", 0.0);
    p.zrot("right_arm", 0.0);
    p.xrot("left_arm", (pos * 0.6662).cos() * 2.0 * sp * 0.5);
    p.yrot("left_arm", 0.0);
    p.zrot("left_arm", 0.0);
    p.xrot("right_leg", (pos * 0.6662).cos() * 1.4 * sp * 0.5);
    p.xrot("left_leg", (pos * 0.6662 + PI).cos() * 1.4 * sp * 0.5);
    let pose = arm_pose(m);
    match pose {
        ArmPose::Attacking => {
            if m.extra.main_hand.is_none() {
                // animateZombieArms + bobArms
                let swing = s.swing;
                let a = (swing * PI).sin();
                let b = ((1.0 - (1.0 - swing) * (1.0 - swing)) * PI).sin();
                p.zrot("right_arm", 0.0);
                p.zrot("left_arm", 0.0);
                p.yrot("right_arm", -(0.1 - a * 0.6));
                p.yrot("left_arm", 0.1 - a * 0.6);
                let x = -PI / 1.5;
                p.xrot("right_arm", x + a * 1.2 - b * 0.4);
                p.xrot("left_arm", x + a * 1.2 - b * 0.4);
            } else {
                // swingWeaponDown (right-handed).
                let swing = s.swing;
                let a2 = (swing * PI).sin();
                let a = ((1.0 - (1.0 - swing) * (1.0 - swing)) * PI).sin();
                p.zrot("right_arm", 0.0);
                p.zrot("left_arm", 0.0);
                p.yrot("right_arm", PI / 20.0);
                p.yrot("left_arm", -PI / 20.0);
                p.xrot(
                    "right_arm",
                    -1.884_955_8 + (s.age * 0.09).cos() * 0.15 + (a2 * 2.2 - a * 0.4),
                );
                p.xrot("left_arm", (s.age * 0.19).cos() * 0.5);
            }
            let bob = |p: &mut Poses, part: &str, sign: f32| {
                let v = p.get(part);
                v[5] += sign * ((s.age * 0.09).cos() * 0.05 + 0.05);
                v[3] += sign * (s.age * 0.067).sin() * 0.05;
            };
            bob(p, "right_arm", 1.0);
            bob(p, "left_arm", -1.0);
        }
        ArmPose::CrossbowHold => {
            p.yrot("right_arm", -0.3 + hy);
            p.yrot("left_arm", 0.6 + hy);
            p.xrot("right_arm", -PI / 2.0 + hx + 0.1);
            p.xrot("left_arm", -1.5 + hx);
        }
        ArmPose::CrossbowCharge => {
            p.yrot("right_arm", -0.8);
            p.xrot("right_arm", -0.970_796_35);
            let a =
                (m.extra.using_item as f32 + s.partial).clamp(0.0, CHARGE as f32) / CHARGE as f32;
            p.yrot("left_arm", 0.4 + (0.85 - 0.4) * a);
            p.xrot("left_arm", -0.970_796_35 + (-PI / 2.0 + 0.970_796_35) * a);
        }
        ArmPose::Crossed | ArmPose::Neutral => {}
    }
    if pose == ArmPose::Crossed {
        hide(p, "left_arm");
        hide(p, "right_arm");
    } else {
        hide(p, "arms");
    }
}
