//! Happy ghasts and ghastlings (`HappyGhast`, `HappyGhastAi`): the adult
//! floats about following torchlight of snowballs and harnesses, the
//! ghastling follows players and adults, a harnessed adult can be ridden and
//! flown, healing is slow (fast in the rain) and `HappyGhastModel`.
use super::{
    Mob, MobEvent, MobWorld, Species,
    anim::{Poses, State},
    control::{MoveKind, Operation},
    goals::{Cx, Float, Goal, LOOK, MOVE, Panic, Stroll, Tempt, goal_clone},
    nav::NavKind,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};

// Happy ghast state: `head` the harness, `scratch[0]` 1 once set up as a
// baby and 2 as an adult, `home` and `ints[4]` the restriction, `counter`
// the end tick of the follow-an-adult range pick.
pub const BABY_SCALE: f64 = 0.2375;
/// `Attributes.FLYING_SPEED` and the speed `travel` uses (`* 5 / 3`).
pub const FLYING_SPEED: f64 = 0.05;
pub const AIR_SPEED: f32 = 0.05 * 5.0 / 3.0;

pub fn register(m: &mut Mob) {
    setup(m, m.is_baby());
}
/// `adultGhastSetup` and `babyGhastSetup`.
fn setup(m: &mut Mob, baby: bool) {
    m.extra.scratch[0] = if baby { 1 } else { 2 };
    m.ai.goals.clear();
    m.ai.nav.stop();
    m.ai.mov.operation = Operation::Wait;
    if baby {
        m.ai.mov.kind = MoveKind::Flying {
            max_turn: 180.0,
            hovers: true,
        };
        m.ai.nav.kind = NavKind::Flying;
        m.ai.nav.can_float = true;
        m.ai.nav.required_path_length = 48.0;
        let tempt = m.kind.food.clone();
        let g = &mut m.ai.goals;
        g.add(0, Float);
        g.add(0, Panic::new(2.0));
        let mut t = Tempt::new(1.25, tempt, false);
        t.stop_distance = 3.0;
        g.add(1, t);
        g.add(2, FollowAdult::default());
        g.add(4, Stroll::flying(1.0));
        return;
    }
    m.ai.mov.kind = MoveKind::Custom;
    m.ai.nav.kind = NavKind::Ground;
    let g = &mut m.ai.goals;
    g.add(3, Float);
    g.add(4, GhastTempt::default());
    g.add(5, FloatAround { blocks: 16 });
}
fn wearing_harness(m: &Mob) -> bool {
    m.extra.head.is_some()
}
/// `getControllingPassenger`: a player on a harnessed adult.
pub fn controlled(m: &Mob) -> bool {
    m.rider.is_some() && wearing_harness(m) && !m.is_baby()
}
/// `getHappyGhastRestrictionRadius`.
fn restriction_radius(m: &Mob) -> i32 {
    if !m.is_baby() && m.extra.head.is_none() {
        64
    } else {
        32
    }
}
/// `Mob.isWithinHome` for the ghast's restriction.
fn within_home(m: &Mob, p: Vec3) -> bool {
    let Some(h) = m.extra.home else { return true };
    let r = f64::from(m.extra.ints[4]);
    let (dx, dy, dz) = (
        f64::from(h[0]) + 0.5 - p.x,
        f64::from(h[1]) + 0.5 - p.y,
        f64::from(h[2]) + 0.5 - p.z,
    );
    dx * dx + dy * dy + dz * dz < r * r
}
/// `Ghast.GhastMoveControl` with the careful block checks: a push toward
/// the wanted point every few ticks while the way is clear.
pub fn move_control(m: &mut Mob, w: &MobWorld) {
    if m.ai.mov.operation != Operation::MoveTo {
        return;
    }
    m.extra.counter2 -= 1;
    if m.extra.counter2 > 0 {
        return;
    }
    m.extra.counter2 += m.random.next_int_bound(5).unwrap_or(0) + 2;
    let [wx, wy, wz] = m.ai.mov.wanted;
    let travel = Vec3::new(wx - m.position.x, wy - m.position.y, wz - m.position.z);
    let len = travel.length();
    if len < 1.0e-6 {
        return;
    }
    let b = m.bounds();
    let steps = len.ceil() as i32;
    let clear = (1..=steps).all(|i| {
        let f = f64::from(i) / f64::from(steps);
        let d = Vec3::new(travel.x * f, travel.y * f, travel.z * f);
        let at = Aabb::new(
            b.min_position().add_vector(d),
            b.max_position().add_vector(d),
        );
        w.no_collision(at) && !touches_fluid(w, at)
    });
    if clear {
        let speed = AIR_SPEED;
        m.velocity = m.velocity.add_vector(Vec3::new(
            travel.x / len * f64::from(speed),
            travel.y / len * f64::from(speed),
            travel.z / len * f64::from(speed),
        ));
    } else {
        m.ai.mov.operation = Operation::Wait;
    }
}
/// A careful ghast does not float into water or lava.
fn touches_fluid(w: &MobWorld, b: Aabb) -> bool {
    let (lo, hi) = (b.min_position(), b.max_position());
    for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
        for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
            for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                if w.fluid(x, y, z) != crate::blocks::Fluid::None {
                    return true;
                }
            }
        }
    }
    false
}
/// `TemptGoal.ForNonPathfinders`: drift toward a player holding food, or
/// food and harnesses for a bare adult.
#[derive(Clone, Debug, Default)]
struct GhastTempt {
    calm_down: i32,
}
impl GhastTempt {
    fn wants(m: &Mob, item: &Option<std::sync::Arc<str>>) -> bool {
        let Some(i) = item else { return false };
        let harness = !wearing_harness(m) && !m.is_baby() && i.ends_with("_harness");
        harness || m.kind.food.iter().any(|f| f.as_str() == &**i)
    }
}
impl Goal for GhastTempt {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if self.calm_down > 0 {
            self.calm_down -= 1;
            return false;
        }
        let Some(p) = &cx.w.player else { return false };
        // Attributes.TEMPT_RANGE 16.
        p.alive
            && !p.creative
            && (Self::wants(m, &p.main_hand) || Self::wants(m, &p.off_hand))
            && m.distance_sqr(p.position) <= 256.0
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.mov.operation = Operation::Wait;
        self.calm_down = 50;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(p) = cx.w.player.clone() else { return };
        m.look_at(p.eye);
        if m.distance_sqr(p.position) < 49.0 {
            m.ai.mov.operation = Operation::Wait;
        } else {
            let f = m.random.next_double();
            let at = [
                (p.eye[0] - m.position.x) * f + m.position.x,
                (p.eye[1] - m.position.y) * f + m.position.y,
                (p.eye[2] - m.position.z) * f + m.position.z,
            ];
            m.ai.mov.set_wanted(at[0], at[1], at[2], 1.0);
        }
    }
}
/// `Ghast.RandomFloatAroundGoal` with a restriction and a wish for open
/// air next to something solid.
#[derive(Clone, Debug)]
struct FloatAround {
    blocks: i32,
}
impl FloatAround {
    fn good(&self, w: &MobWorld, p: Vec3) -> bool {
        if self.blocks <= 0 {
            return true;
        }
        let at = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
        if !w.facts.is_air(w.world.block(at[0], at[1], at[2])) {
            return false;
        }
        for d in [
            [1, 0, 0],
            [-1, 0, 0],
            [0, 1, 0],
            [0, -1, 0],
            [0, 0, 1],
            [0, 0, -1],
        ] {
            for i in 1..self.blocks {
                if !w.facts.is_air(w.world.block(
                    at[0] + d[0] * i,
                    at[1] + d[1] * i,
                    at[2] + d[2] * i,
                )) {
                    return true;
                }
            }
        }
        false
    }
}
impl Goal for FloatAround {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        if !m.ai.mov.has_wanted() {
            return true;
        }
        let [x, y, z] = m.ai.mov.wanted;
        let d =
            (x - m.position.x).powi(2) + (y - m.position.y).powi(2) + (z - m.position.z).powi(2);
        !(1.0..=3600.0).contains(&d)
    }
    fn can_continue(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        false
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let c = m.position;
        let mut result: Option<Vec3> = None;
        for _ in 0..64 {
            let r = &mut m.random;
            let t = Vec3::new(
                c.x + f64::from(r.next_float() * 2.0 - 1.0) * 16.0,
                c.y + f64::from(r.next_float() * 2.0 - 1.0) * 16.0,
                c.z + f64::from(r.next_float() * 2.0 - 1.0) * 16.0,
            );
            result = Some(t);
            if within_home(m, t) && self.good(cx.w, t) {
                break;
            }
            result = None;
        }
        let mut t = result.unwrap_or_else(|| {
            let r = &mut m.random;
            Vec3::new(
                c.x + f64::from(r.next_float() * 2.0 - 1.0) * 16.0,
                c.y + f64::from(r.next_float() * 2.0 - 1.0) * 16.0,
                c.z + f64::from(r.next_float() * 2.0 - 1.0) * 16.0,
            )
        });
        let top =
            cx.w.motion_blocking_height(t.x.floor() as i32, t.z.floor() as i32);
        if top < t.y.floor() as i32 && top > crate::chunk::MIN_Y {
            t.y = c.y - (c.y - t.y).abs();
        }
        m.ai.mov.set_wanted(t.x, t.y, t.z, 1.0);
    }
}
/// `BabyFollowAdult`: a ghastling drifts toward the nearest player or adult
/// until it is a few blocks away.
#[derive(Clone, Debug, Default)]
struct FollowAdult {
    close: f64,
}
impl FollowAdult {
    fn target(m: &Mob, cx: &Cx) -> Option<Vec3> {
        let player =
            cx.w.player
                .as_ref()
                .filter(|p| p.alive && !p.spectator && m.distance_sqr(p.position) <= 32.0 * 32.0)
                .map(|p| p.position);
        player.or_else(|| {
            cx.w.nearby(m.position, 32.0, |s| {
                s.species == Species::HappyGhast && !s.baby
            })
            .next()
            .map(|s| s.position)
        })
    }
}
impl Goal for FollowAdult {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.is_baby()
            && Self::target(m, cx).is_some_and(|t| {
                let close = if self.close > 0.0 { self.close } else { 16.0 };
                m.position.distance_squared(t) > close * close
            })
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        Self::target(m, cx).is_some_and(|t| m.position.distance_squared(t) > 9.0)
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        // UniformInt.of(3, 16).
        self.close = f64::from(3 + m.random.next_int_bound(14).unwrap_or(0));
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(t) = Self::target(m, cx) {
            m.nav_move_to(cx.w, [t.x, t.y, t.z], 1.1);
        }
    }
}
/// What a rider asks (`getRiddenInput`, `tickRidden`).
pub fn ridden(m: &mut Mob) {
    let Some(r) = m.rider else { return };
    // tickRidden: turn 8% of the way toward the rider's yaw; pitch half of theirs.
    let diff = super::living::wrap(r.yaw - m.yaw);
    m.yaw += diff * 0.08;
    m.head_pitch = r.pitch * 0.5;
    m.body_yaw = m.yaw;
    m.head_yaw = m.yaw;
    m.ai.nav.stop();
    let (mut forward, mut up) = (0.0, 0.0);
    if r.forward != 0.0 {
        let mut f = r.pitch.to_radians().cos();
        let mut u = -r.pitch.to_radians().sin();
        if r.forward < 0.0 {
            f *= -0.5;
            u *= -0.5;
        }
        forward = f;
        up = u;
    }
    if r.jumping {
        up += 0.5;
    }
    let scale = 3.9 * FLYING_SPEED as f32;
    m.ai.xxa = r.strafe * scale;
    m.ai.yya = up * scale;
    m.ai.zza = forward * scale;
    m.ai.mov.operation = Operation::Wait;
}
/// `HappyGhast.aiStep`, `tick` and `checkRestriction`: the adult/baby
/// setup at the age boundary, the home, face-the-motion and slow healing.
pub fn ai_step(m: &mut Mob, w: &MobWorld) {
    let baby = m.is_baby();
    if m.extra.scratch[0] != if baby { 1 } else { 2 } {
        setup(m, baby);
    }
    // checkRestriction.
    if m.extra.leash.is_none() && m.rider.is_none() {
        let radius = restriction_radius(m);
        let here = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        let stale = match m.extra.home {
            Some(h) => {
                (h[0] - here[0]).pow(2) + (h[1] - here[1]).pow(2) + (h[2] - here[2]).pow(2)
                    >= (radius + 16).pow(2)
                    || radius != m.extra.ints[4]
            }
            None => true,
        };
        if stale {
            m.extra.home = Some(here);
            m.extra.ints[4] = radius;
        }
    }
    // LookControl: face where it is going.
    if !baby && m.rider.is_none() {
        let v = m.velocity;
        if m.ai.target.is_none() && v.x * v.x + v.z * v.z > 1.0e-6 {
            m.yaw = -(v.x.atan2(v.z) as f32).to_degrees();
            m.body_yaw = m.yaw;
            m.head_yaw = m.yaw;
        }
    }
    // continuousHeal: a point every 30 seconds, every second in rain.
    if m.alive() && m.death_time == 0 && m.health < m.attrs.max_health {
        let (x, y, z) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        let wet = w.raining && w.can_see_sky(x, y, z);
        let interval = if wet { 20 } else { 600 };
        if m.age.is_multiple_of(interval) {
            m.health = (m.health + 1.0).min(m.attrs.max_health);
        }
    }
}
/// Sounds: the ghastling has its own.
pub fn sounds(m: &Mob) -> [&'static str; 3] {
    if m.is_baby() {
        [
            "entity.ghastling.ambient",
            "entity.ghastling.hurt",
            "entity.ghastling.death",
        ]
    } else {
        [
            "entity.happy_ghast.ambient",
            "entity.happy_ghast.hurt",
            "entity.happy_ghast.death",
        ]
    }
}
/// `getSoundVolume`: 4 for adults, 1 for ghastlings.
pub fn volume(m: &Mob) -> f32 {
    if m.is_baby() { 1.0 } else { 4.0 }
}
/// `HappyGhastModel.setupAnim`: the tentacles, and the body squeezed under a harness.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    // HappyGhastHarnessModel.setupAnim: the goggles down while ridden
    // (vanilla's literal -0.7854, not -PI/4).
    #[allow(clippy::approx_constant)]
    if p.has("goggles") {
        let g = p.get("goggles");
        if m.rider.is_some() {
            g[3] = 0.0;
            g[1] = 14.0;
        } else {
            g[3] = -0.7854;
            g[1] = 9.0;
        }
    }
    if wearing_harness(m) {
        let b = p.get("body");
        b[6] = 0.9375;
        b[7] = 0.9375;
        b[8] = 0.9375;
    }
    super::nether::ghast_anim(s, p);
}
/// Texture of the harness layer for the worn item.
pub fn harness_texture(item: &str) -> Option<String> {
    let name = item.strip_prefix("minecraft:")?;
    name.ends_with("_harness")
        .then(|| format!("entity/equipment/happy_ghast_body/{name}"))
}
/// Events for the player getting on or off.
pub fn goggles(down: bool, m: &Mob) -> MobEvent {
    MobEvent::Sound(
        if down {
            "entity.happy_ghast.harness_goggles_down"
        } else {
            "entity.happy_ghast.harness_goggles_up"
        },
        m.center(),
        1.0,
        1.0,
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_harness_names_its_texture() {
        assert_eq!(
            harness_texture("minecraft:red_harness").as_deref(),
            Some("entity/equipment/happy_ghast_body/red_harness")
        );
        assert_eq!(harness_texture("minecraft:saddle"), None);
    }
    #[test]
    fn travel_speed_is_five_thirds_of_the_flying_speed() {
        assert!((f64::from(AIR_SPEED) - FLYING_SPEED * 5.0 / 3.0).abs() < 1e-7);
    }
}
