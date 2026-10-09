//! Model animation: ports of each vanilla `EntityModel.setupAnim` family
//! (quadrupeds, humanoids, creepers, spiders, chickens...) applied to a map of
//! part poses before the model is drawn.
use super::{Mob, Part, keyframes::AnimationState, living::wrap};
use std::collections::HashMap;
use std::f32::consts::PI;

/// The setupAnim implementation a model uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anim {
    Quadruped,
    Sheep,
    Chicken,
    Zombie,
    Drowned,
    ZombieVillager,
    Skeleton,
    Creeper,
    Spider,
    Enderman,
    Silverfish,
    Slime,
    MagmaCube,
    Witch,
    Villager,
    IronGolem,
    SnowGolem,
    Illager,
    Piglin,
    ZombifiedPiglin,
    Hoglin,
    Wolf,
    Feline,
    Fox,
    Rabbit,
    Horse,
    Llama,
    Goat,
    Panda,
    PolarBear,
    Parrot,
    Bee,
    Turtle,
    Armadillo,
    Frog,
    Axolotl,
    Squid,
    Fish,
    Pufferfish,
    Dolphin,
    Bat,
    Phantom,
    Ghast,
    Blaze,
    Strider,
    /// The humanoid model of the earlier simple renderer.
    Humanoid {
        zombie_arms: bool,
    },
}
/// Client-side animation clocks advanced each tick.
#[derive(Clone, Debug, Default)]
pub struct AnimState {
    pub attack_anim: f32,
    pub attack_anim_o: f32,
    pub flap: f32,
    pub flap_o: f32,
    pub flap_speed: f32,
    pub flap_speed_o: f32,
    pub flapping: f32,
    /// Generic per-species keyframe animation states.
    pub states: [AnimationState; 4],
    /// Per-species animation values (tail wag, squid tentacles, squish...)
    /// with their previous-tick copies for interpolation.
    pub v: [f32; 8],
    pub vo: [f32; 8],
}
impl AnimState {
    pub fn begin_tick(&mut self) {
        self.attack_anim_o = self.attack_anim;
        self.vo = self.v;
    }
    /// Interpolated value `i` at the render partial tick.
    pub fn lerp(&self, i: usize, partial: f32) -> f32 {
        self.vo[i] + (self.v[i] - self.vo[i]) * partial
    }
    pub fn end_tick(&mut self, _swinging: bool) {}
}
/// Part poses for one draw, keyed by part name.
pub struct Poses {
    map: HashMap<String, [f32; 9]>,
    dummy: [f32; 9],
}
impl Poses {
    pub fn new(model: &Part) -> Self {
        let mut map = HashMap::new();
        fn walk(p: &Part, map: &mut HashMap<String, [f32; 9]>) {
            map.insert(p.name.clone(), p.pose);
            for c in &p.children {
                walk(c, map);
            }
        }
        walk(model, &mut map);
        Self {
            map,
            dummy: [0.0; 9],
        }
    }
    pub fn get(&mut self, name: &str) -> &mut [f32; 9] {
        match self.map.get_mut(name) {
            Some(p) => p,
            None => &mut self.dummy,
        }
    }
    pub fn pose(&self, part: &Part) -> [f32; 9] {
        self.map.get(&part.name).copied().unwrap_or(part.pose)
    }
    pub fn has(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }
    pub fn xrot(&mut self, n: &str, v: f32) {
        self.get(n)[3] = v;
    }
    pub fn yrot(&mut self, n: &str, v: f32) {
        self.get(n)[4] = v;
    }
    pub fn zrot(&mut self, n: &str, v: f32) {
        self.get(n)[5] = v;
    }
    pub fn visible(&mut self, n: &str, v: bool) {
        if !v {
            let p = self.get(n);
            p[6] = 0.0;
            p[7] = 0.0;
            p[8] = 0.0;
        }
    }
}
/// The LivingEntityRenderState values setupAnim reads.
#[derive(Clone, Copy, Debug)]
pub struct State {
    pub pos: f32,
    pub speed: f32,
    pub age: f32,
    /// Head yaw relative to the body, degrees.
    pub y_rot: f32,
    /// Head pitch, degrees.
    pub x_rot: f32,
    pub swing: f32,
    pub baby: bool,
    pub partial: f32,
}
pub fn state(m: &Mob, alpha: f32) -> State {
    let body = m.previous_body_yaw + wrap(m.body_yaw - m.previous_body_yaw) * alpha;
    let head = m.previous_head_yaw + wrap(m.head_yaw - m.previous_head_yaw) * alpha;
    let mut swing = m.anim.attack_anim - m.anim.attack_anim_o;
    if swing < 0.0 {
        swing += 1.0;
    }
    let swing = m.anim.attack_anim_o + swing * alpha;
    let alive = m.alive();
    State {
        pos: if alive {
            m.limb_position - m.limb_speed * (1.0 - alpha)
        } else {
            0.0
        },
        speed: if alive {
            (m.previous_limb_speed + (m.limb_speed - m.previous_limb_speed) * alpha).min(1.0)
        } else {
            0.0
        },
        age: m.age as f32 + alpha,
        y_rot: wrap(head - body),
        x_rot: m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha,
        swing,
        baby: m.is_baby(),
        partial: alpha,
    }
}
fn head(p: &mut Poses, s: &State) {
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
}
/// QuadrupedModel.setupAnim
fn quadruped(p: &mut Poses, s: &State) {
    head(p, s);
    let (pos, sp) = (s.pos, s.speed);
    p.xrot("right_hind_leg", (pos * 0.6662).cos() * 1.4 * sp);
    p.xrot("left_hind_leg", (pos * 0.6662 + PI).cos() * 1.4 * sp);
    p.xrot("right_front_leg", (pos * 0.6662 + PI).cos() * 1.4 * sp);
    p.xrot("left_front_leg", (pos * 0.6662).cos() * 1.4 * sp);
}
/// AnimationUtils.bobModelPart
fn bob(p: &mut Poses, n: &str, age: f32, scale: f32) {
    let q = p.get(n);
    q[5] += scale * ((age * 0.09).cos() * 0.05 + 0.05);
    q[3] += scale * ((age * 0.067).sin() * 0.05);
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ArmPose {
    Empty,
    Item,
    Bow,
    CrossbowHold,
    CrossbowCharge,
}
/// HumanoidModel.setupAnim holding an item or not (piglins).
pub fn humanoid_item(p: &mut Poses, s: &State, holding: bool) {
    let right = if holding {
        ArmPose::Item
    } else {
        ArmPose::Empty
    };
    humanoid(p, s, right, ArmPose::Empty, None, holding);
}
/// HumanoidModel.setupAnim with empty hands (endermen).
pub fn humanoid_pub(p: &mut Poses, s: &State, _m: &Mob) {
    humanoid(p, s, ArmPose::Empty, ArmPose::Empty, None, false);
}
/// HumanoidModel.setupAnim up to (and including) the attack animation hook.
fn humanoid(
    p: &mut Poses,
    s: &State,
    right: ArmPose,
    left: ArmPose,
    zombie: Option<bool>,
    holding: bool,
) {
    head(p, s);
    let (pos, sp) = (s.pos, s.speed);
    p.xrot("right_arm", (pos * 0.6662 + PI).cos() * 2.0 * sp * 0.5);
    p.xrot("left_arm", (pos * 0.6662).cos() * 2.0 * sp * 0.5);
    p.xrot("right_leg", (pos * 0.6662).cos() * 1.4 * sp);
    p.xrot("left_leg", (pos * 0.6662 + PI).cos() * 1.4 * sp);
    p.yrot("right_leg", 0.005);
    p.yrot("left_leg", -0.005);
    p.zrot("right_leg", 0.005);
    p.zrot("left_leg", -0.005);
    let (hx, hy) = (p.get("head")[3], p.get("head")[4]);
    for (arm, pose) in [("right_arm", right), ("left_arm", left)] {
        match pose {
            ArmPose::Empty => p.yrot(arm, 0.0),
            ArmPose::Item => {
                let x = p.get(arm)[3];
                p.xrot(arm, x * 0.5 - PI / 10.0);
                p.yrot(arm, 0.0);
            }
            ArmPose::Bow => {
                let right_main = arm == "right_arm";
                p.yrot("right_arm", -0.1 + hy - if right_main { 0.0 } else { 0.4 });
                p.yrot("left_arm", 0.1 + hy + if right_main { 0.4 } else { 0.0 });
                p.xrot("right_arm", -PI / 2.0 + hx);
                p.xrot("left_arm", -PI / 2.0 + hx);
            }
            ArmPose::CrossbowHold => {
                let r = arm == "right_arm";
                let (hold, shoot) = if r {
                    ("right_arm", "left_arm")
                } else {
                    ("left_arm", "right_arm")
                };
                p.yrot(hold, if r { -0.3 } else { 0.3 } + hy);
                p.yrot(shoot, if r { 0.6 } else { -0.6 } + hy);
                p.xrot(hold, -PI / 2.0 + hx + 0.1);
                p.xrot(shoot, -1.5 + hx);
            }
            ArmPose::CrossbowCharge => {
                let r = arm == "right_arm";
                let (hold, pull) = if r {
                    ("right_arm", "left_arm")
                } else {
                    ("left_arm", "right_arm")
                };
                p.yrot(hold, if r { -0.8 } else { 0.8 });
                p.xrot(hold, -0.970_796_35);
                p.xrot(pull, -0.970_796_35);
                p.yrot(pull, 0.6 * if r { 1.0 } else { -1.0 });
                p.xrot(pull, -PI / 2.0);
            }
        }
    }
    // setupAttackAnimation (WHACK swing with the right arm).
    let swing = s.swing;
    if swing > 0.0 {
        let body_y = (swing.sqrt() * PI * 2.0).sin() * 0.2;
        p.yrot("body", body_y);
        let age_scale = if s.baby { 0.5 } else { 1.0 };
        {
            let r = p.get("right_arm");
            r[2] = body_y.sin() * 5.0 * age_scale;
            r[0] = -body_y.cos() * 5.0 * age_scale;
        }
        {
            let l = p.get("left_arm");
            l[2] = -body_y.sin() * 5.0 * age_scale;
            l[0] = body_y.cos() * 5.0 * age_scale;
        }
        p.get("right_arm")[4] += body_y;
        p.get("left_arm")[4] += body_y;
        p.get("left_arm")[3] += body_y;
        let eased = 1.0 - (1.0 - swing).powi(4);
        let aa = (eased * PI).sin();
        let bb = (swing * PI).sin() * -(hx - 0.7) * 0.75;
        let r = p.get("right_arm");
        r[3] -= aa * 1.2 + bb;
        r[4] += body_y * 2.0;
        r[5] += (swing * PI).sin() * -0.4;
    }
    if let Some(aggressive) = zombie {
        // AnimationUtils.animateZombieArms
        let raise = !s.baby || !holding;
        let drop = if raise {
            -PI / if aggressive { 1.5 } else { 2.25 }
        } else {
            0.0
        };
        let ym = if raise { 1.0 } else { -1.0 } * (swing * PI).sin();
        let xm = ((1.0 - (1.0 - swing) * (1.0 - swing)) * PI).sin();
        let x = drop + ym * 1.2 - xm * 0.4;
        let y = 0.1 - ym * 0.6;
        p.xrot("right_arm", x);
        p.yrot("right_arm", if raise { -y } else { y });
        p.zrot("right_arm", 0.0);
        p.xrot("left_arm", x);
        p.yrot("left_arm", if raise { y } else { -y });
        p.zrot("left_arm", 0.0);
    }
    bob(p, "right_arm", s.age, 1.0);
    bob(p, "left_arm", s.age, -1.0);
}
/// Apply the mob's model animation to `p`.
pub fn setup(
    m: &Mob,
    s: &State,
    p: &mut Poses,
    w_anims: &HashMap<String, super::keyframes::Animation>,
) {
    let _ = w_anims;
    match m.kind.anim {
        Anim::Squid | Anim::Fish | Anim::Pufferfish | Anim::Dolphin => {
            super::aquatic::anim(m, s, p)
        }
        Anim::Quadruped | Anim::PolarBear | Anim::Panda | Anim::Goat => quadruped(p, s),
        Anim::Sheep => {
            quadruped(p, s);
            // SheepModel: lower the head while eating grass.
            let t = m.extra.anim_ticks;
            let partial = s.partial;
            let pos_scale = if t <= 0 {
                0.0
            } else if (4..=36).contains(&t) {
                1.0
            } else if t < 4 {
                (t as f32 - partial) / 4.0
            } else {
                -((t - 40) as f32 - partial) / 4.0
            };
            let angle = if t > 4 && t <= 36 {
                let f = ((t - 4) as f32 - partial) / 32.0;
                PI / 5.0 + 0.219_911_49 * (f * 28.7).sin()
            } else if t > 0 {
                PI / 5.0
            } else {
                s.x_rot.to_radians()
            };
            let age_scale = if s.baby { 0.5 } else { 1.0 };
            p.get("head")[1] += pos_scale * 9.0 * age_scale;
            p.xrot("head", angle);
        }
        Anim::Chicken => {
            let flap = m.anim.flap_o + (m.anim.flap - m.anim.flap_o) * s.partial;
            let flap_speed =
                m.anim.flap_speed_o + (m.anim.flap_speed - m.anim.flap_speed_o) * s.partial;
            let angle = (flap.sin() + 1.0) * flap_speed;
            p.xrot("right_leg", (s.pos * 0.6662).cos() * 1.4 * s.speed);
            p.xrot("left_leg", (s.pos * 0.6662 + PI).cos() * 1.4 * s.speed);
            p.zrot("right_wing", angle);
            p.zrot("left_wing", -angle);
            if !s.baby {
                head(p, s);
            }
        }
        Anim::Zombie
        | Anim::Drowned
        | Anim::ZombieVillager
        | Anim::ZombifiedPiglin
        | Anim::Humanoid { zombie_arms: true } => {
            let holding = m.extra.main_hand.is_some();
            let right = if holding {
                ArmPose::Item
            } else {
                ArmPose::Empty
            };
            humanoid(p, s, right, ArmPose::Empty, Some(m.ai.aggressive), holding);
        }
        Anim::Skeleton | Anim::Humanoid { zombie_arms: false } => {
            let bow = m.extra.main_hand.as_deref() == Some("minecraft:bow");
            let right = if bow && m.ai.aggressive {
                ArmPose::Bow
            } else if m.extra.main_hand.is_some() {
                ArmPose::Item
            } else {
                ArmPose::Empty
            };
            humanoid(
                p,
                s,
                right,
                ArmPose::Empty,
                None,
                m.extra.main_hand.is_some(),
            );
            if m.ai.aggressive && !bow {
                // SkeletonModel melee pose.
                let sw = s.swing;
                let s2 = (sw * PI).sin();
                let s1 = ((1.0 - (1.0 - sw) * (1.0 - sw)) * PI).sin();
                p.zrot("right_arm", 0.0);
                p.zrot("left_arm", 0.0);
                p.yrot("right_arm", -(0.1 - s2 * 0.6));
                p.yrot("left_arm", 0.1 - s2 * 0.6);
                p.xrot("right_arm", -PI / 2.0 - (s2 * 1.2 - s1 * 0.4));
                p.xrot("left_arm", -PI / 2.0 - (s2 * 1.2 - s1 * 0.4));
                bob(p, "right_arm", s.age, 1.0);
                bob(p, "left_arm", s.age, -1.0);
            }
        }
        Anim::Witch => super::witch::anim(m, s, p),
        Anim::Wolf => super::tame::anim(m, s, p),
        Anim::IronGolem => super::golem::iron_anim(m, s, p),
        Anim::Illager => super::illager::anim(m, s, p),
        Anim::Piglin => super::piglin::piglin_anim(m, s, p),
        Anim::Horse => {
            super::equine::anim(m, s, p);
            // DonkeyModel: the chests show once fitted.
            if !m.extra.chested {
                for part in ["left_chest", "right_chest"] {
                    if p.has(part) {
                        let v = p.get(part);
                        v[6] = 0.0;
                        v[7] = 0.0;
                        v[8] = 0.0;
                    }
                }
            }
        }
        Anim::Hoglin => super::piglin::hoglin_anim(m, s, p),
        Anim::Blaze => super::nether::blaze_anim(s, p),
        Anim::Ghast => super::nether::ghast_anim(s, p),
        Anim::Feline if m.species() == super::Species::Cat => super::tame::cat_anim(m, s, p),
        Anim::Enderman => super::enderman::anim(m, s, p),
        Anim::Slime | Anim::MagmaCube => super::cube::anim(m, s, p),
        Anim::Creeper => {
            head(p, s);
            p.xrot("right_hind_leg", (s.pos * 0.6662).cos() * 1.4 * s.speed);
            p.xrot("left_hind_leg", (s.pos * 0.6662 + PI).cos() * 1.4 * s.speed);
            p.xrot(
                "right_front_leg",
                (s.pos * 0.6662 + PI).cos() * 1.4 * s.speed,
            );
            p.xrot("left_front_leg", (s.pos * 0.6662).cos() * 1.4 * s.speed);
        }
        Anim::Spider => {
            head(p, s);
            let pos = s.pos * 0.6662;
            let sp = s.speed;
            let swing = |o: f32| -((pos * 2.0 + o).cos() * 0.4) * sp;
            let step = |o: f32| ((pos + o).sin() * 0.4).abs() * sp;
            let legs = [
                ("hind", 0.0f32, 0.0f32),
                ("middle_hind", PI, PI),
                ("middle_front", PI / 2.0, PI / 2.0),
                ("front", PI * 3.0 / 2.0, PI * 3.0 / 2.0),
            ];
            for (n, so, to) in legs {
                let (sw, st) = (swing(so), step(to));
                p.get(&format!("right_{n}_leg"))[4] += sw;
                p.get(&format!("left_{n}_leg"))[4] -= sw;
                p.get(&format!("right_{n}_leg"))[5] += st;
                p.get(&format!("left_{n}_leg"))[5] -= st;
            }
        }
        _ => {
            if p.has("head") {
                head(p, s);
            }
            if p.has("right_hind_leg") {
                quadruped(p, s);
            }
        }
    }
}
