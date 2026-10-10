//! Piglins, piglin brutes, hoglins and zoglins: gold-wearing players are
//! left alone, gold is admired and bartered, piglins and hoglins zombify
//! outside the Nether, hoglins headbutt their targets into the air.
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    goals::{
        Avoid, AvoidEntity, Cx, Float, Goal, HurtByTarget, LOOK, LookAtPlayer, MOVE, MeleeAttack,
        NearestTarget, Prey, RandomLookAround, Stroll, goal_clone,
    },
};
use horizon_core::geometry::trig::Mth;
use horizon_core::random::RandomSource;
use std::f32::consts::PI;

/// `PiglinAi.ADMIRE_DURATION`.
pub const ADMIRE: i32 = 119;
/// `AbstractPiglin.CONVERSION_TIME` (and the hoglin's).
const CONVERSION: i32 = 300;

fn hostile_to_player(m: &Mob, w: &MobWorld) -> bool {
    if m.is_baby() {
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
    g.add(
        2,
        AvoidEntity::new(
            Avoid::Species(vec![Species::ZombifiedPiglin, Species::Zoglin]),
            6.0,
            1.0,
            1.2,
        ),
    );
    if crossbow {
        g.add(3, super::illager::crossbow_goal(1.0, 8.0));
    }
    g.add(3, MeleeAttack::new(1.0, true));
    g.add(4, GoToWantedItem);
    g.add(5, Stroll::avoiding_water(0.6));
    g.add(6, LookAtPlayer::new(8.0));
    g.add(7, RandomLookAround::default());
    let mut hurt = HurtByTarget::new().alert();
    hurt.ignore = vec![Species::Piglin, Species::PiglinBrute];
    m.ai.targets.add(1, hurt);
    m.ai.targets
        .add(2, NearestTarget::player(true).when(hostile_to_player));
    m.ai.targets.add(
        3,
        NearestTarget::new(
            Prey::Filter(|s| s.species == Species::Hoglin && s.baby),
            true,
        )
        .interval(10)
        .when(|m, _| !m.is_baby()),
    );
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
    g.add(1, BabyPanic(super::goals::Panic::new(1.25)));
    g.add(2, MeleeAttack::new(1.0, true));
    g.add(3, super::goals::Breed::new(1.0));
    g.add(4, super::goals::FollowParent::new(1.1));
    g.add(5, Stroll::avoiding_water(0.4));
    g.add(6, LookAtPlayer::new(8.0));
    m.ai.targets.add(1, HurtByTarget::new().alert());
    m.ai.targets
        .add(2, NearestTarget::player(true).when(|m, _| !m.is_baby()));
}
pub fn register_zoglin(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(2, MeleeAttack::new(1.0, true));
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
/// Baby hoglins flee what hurts them; adults stand their ground.
#[derive(Clone, Debug)]
struct BabyPanic(super::goals::Panic);
impl Goal for BabyPanic {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.is_baby() && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
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
/// Per-tick piglin and hoglin state: admiring, anger, zombification and the
/// hoglin attack animation. Returns a conversion when it happens.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
    if m.extra.anger > 0 {
        m.extra.anger -= 1;
        if m.extra.anger == 0 && m.ai.target == Some(Who::Player) && !hostile_to_player(m, w) {
            m.ai.target = None;
        }
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
}
/// `HoglinBase.hurtAndThrowTarget`.
pub fn hoglin_hurt_target(m: &mut Mob, who: Who, cx: &mut Cx) {
    m.extra.anim_ticks = 10;
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
        // throwTarget: a strong shove up and away.
        cx.events.push(MobEvent::Launch {
            target: who,
            up: 0.5 * f64::from(m.random.next_float()) + 0.3,
        });
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
    let pose = if m.extra.using_item > 0 {
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
    piglin_pose(p, s, pose, 25.0, m.extra.using_item as f32);
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
/// `PiglinModel.setupAnim` for a piglin facing right-handed.
pub fn piglin_pose(p: &mut Poses, s: &State, pose: PiglinPose, max_charge: f32, ticks: f32) {
    use super::anim::{ArmPose, Hum, humanoid};
    let mut h = Hum::new(ArmPose::Empty, ArmPose::Empty);
    h.weapon_swing = pose == PiglinPose::MeleeWeapon;
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
        PiglinPose::MeleeWeapon if s.swing == 0.0 => p.xrot("right_arm", -1.8),
        PiglinPose::CrossbowHold => {
            p.yrot("right_arm", -0.3 + hy);
            p.yrot("left_arm", 0.6 + hy);
            p.xrot("right_arm", -PI / 2.0 + hx + 0.1);
            p.xrot("left_arm", -1.5 + hx);
        }
        PiglinPose::CrossbowCharge => {
            p.yrot("right_arm", -0.8);
            p.xrot("right_arm", -0.970_796_35);
            p.xrot("left_arm", -0.970_796_35);
            let alpha = ticks.clamp(0.0, max_charge) / max_charge;
            p.yrot("left_arm", 0.4 + alpha * (0.85 - 0.4));
            let x = -0.970_796_35 + alpha * (-PI / 2.0 - -0.970_796_35);
            p.xrot("left_arm", x);
        }
        PiglinPose::AdmiringItem => {
            p.xrot("head", 0.5);
            p.yrot("head", 0.0);
            p.yrot("left_arm", 0.5);
            p.xrot("left_arm", -0.9);
        }
        _ => {}
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
