//! Creepers (swelling, explosions) and spiders (wall climbing, daylight
//! neutrality, leaping).
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    goals::{
        Avoid, AvoidEntity, Cx, Float, Goal, HurtByTarget, LeapAtTarget, LookAtPlayer, MOVE,
        MeleeAttack, NearestTarget, RandomLookAround, Stroll, goal_clone,
    },
};
use horizon_core::random::RandomSource;

pub fn register(m: &mut Mob) {
    use Species as S;
    match m.species() {
        S::Creeper => {
            let g = &mut m.ai.goals;
            g.add(1, Float);
            g.add(2, Swell::default());
            g.add(
                3,
                AvoidEntity::new(Avoid::Species(vec![S::Ocelot]), 6.0, 1.0, 1.2),
            );
            g.add(
                3,
                AvoidEntity::new(Avoid::Species(vec![S::Cat]), 6.0, 1.0, 1.2),
            );
            g.add(4, MeleeAttack::new(1.0, false));
            g.add(5, Stroll::avoiding_water(0.8));
            g.add(6, LookAtPlayer::new(8.0));
            g.add(6, RandomLookAround::default());
            m.ai.targets.add(1, NearestTarget::player(true));
            m.ai.targets.add(2, HurtByTarget::new());
            // Creeper DATA_SWELL_DIR starts at -1.
            m.extra.counter = -1;
        }
        S::Spider | S::CaveSpider => {
            let g = &mut m.ai.goals;
            g.add(1, Float);
            g.add(
                2,
                AvoidEntity::new(Avoid::Species(vec![S::Armadillo]), 6.0, 1.0, 1.2),
            );
            g.add(3, LeapAtTarget::new(0.4));
            g.add(4, SpiderAttack(MeleeAttack::new(1.0, true)));
            g.add(5, Stroll::avoiding_water(0.8));
            g.add(6, LookAtPlayer::new(8.0));
            g.add(6, RandomLookAround::default());
            m.ai.targets.add(1, HurtByTarget::new());
            m.ai.targets
                .add(2, NearestTarget::player(true).gate(dark_enough));
            m.ai.targets.add(
                3,
                NearestTarget::species(&[S::IronGolem], true).gate(dark_enough),
            );
            m.ai.nav.kind = super::nav::NavKind::WallClimber;
        }
        _ => {}
    }
}
/// Entity.getLightLevelDependentMagicValue at the eye.
pub fn mob_brightness(m: &Mob, w: &MobWorld) -> f32 {
    w.brightness(
        m.position.x.floor() as i32,
        m.eye_y().floor() as i32,
        m.position.z.floor() as i32,
    )
}
fn dark_enough(m: &Mob, w: &MobWorld) -> bool {
    mob_brightness(m, w) < 0.5
}
/// Spider.SpiderAttackGoal: gives up in bright light 1 in 100 ticks.
#[derive(Clone, Debug)]
pub struct SpiderAttack(MeleeAttack);
impl Goal for SpiderAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // SpiderAttackGoal.canUse: not while it carries a rider.
        self.0.can_use(m, cx) && m.extra.rider.is_none()
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if mob_brightness(m, cx.w) >= 0.5 && m.random.next_int_bound(100).unwrap_or(1) == 0 {
            m.ai.target = None;
            return false;
        }
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx)
    }
}
/// SwellGoal: fuse within 3 blocks, defuse beyond 7 or out of sight.
#[derive(Clone, Debug, Default)]
pub struct Swell {
    target: Option<Who>,
}
impl Goal for Swell {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let close = m
            .target_info(cx.w)
            .is_some_and(|t| t.alive && m.distance_sqr(t.position) < 9.0);
        m.extra.counter > 0 || close
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
        self.target = m.ai.target;
    }
    fn stop(&mut self, _: &mut Mob, _: &mut Cx) {
        self.target = None;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let dir = match self
            .target
            .and_then(|t| cx.w.target_info(t).map(|i| (t, i)))
        {
            Some((who, t)) if t.alive => {
                if m.distance_sqr(t.position) > 49.0 || !m.sees(cx.w, who) {
                    -1
                } else {
                    1
                }
            }
            _ => -1,
        };
        m.extra.counter = dir;
    }
}
/// Creeper.tick on a client of a server: the synced fuse direction
/// (DATA_SWELL_DIR, DATA_IS_IGNITED) swells the creeper the renderer draws;
/// the server explodes it.
pub fn creeper_client_tick(m: &mut Mob) {
    if !m.alive() {
        return;
    }
    m.previous_swell = m.swell;
    if m.extra.flag {
        m.extra.counter = 1;
    }
    m.swell = (m.swell + m.extra.counter).clamp(0, 30);
}
/// Creeper.tick: advance the fuse and explode at 30.
pub fn creeper_tick(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if !m.alive() {
        return;
    }
    if m.extra.flag {
        // Ignited by flint and steel.
        m.extra.counter = 1;
    }
    let dir = m.extra.counter;
    if dir > 0 && m.swell == 0 {
        events.push(MobEvent::Sound(
            "entity.creeper.primed",
            m.center(),
            1.0,
            0.5,
        ));
    }
    m.swell = (m.swell + dir).max(0);
    if m.swell >= 30 {
        m.swell = 30;
        let radius = if m.extra.charged { 6.0 } else { 3.0 };
        events.push(MobEvent::ExplosionSource {
            mob: m.id,
            charged: m.extra.charged,
        });
        events.push(MobEvent::Explode(
            [m.position.x, m.position.y, m.position.z],
            radius,
            m.species(),
        ));
        m.removed = true;
    }
}
