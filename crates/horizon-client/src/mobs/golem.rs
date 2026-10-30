//! Golems: iron golems (fighting monsters, the heavy swing that throws its
//! target up, cracks as they weaken, iron repairs and poppies for villagers)
//! and snow golems (snowballs at monsters, the snow trail, melting).
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    goals::{
        Cx, Goal, HurtByTarget, LOOK, LookAtPlayer, MOVE, MeleeAttack, NearestTarget, Prey,
        RandomLookAround, Stroll, goal_clone, reduced,
    },
    villager_brain::{
        closest_section, occupied_pois_in_range, others_tickets, sections_to_village,
        village_sections,
    },
};
use crate::projectiles::{Projectile, ProjectileKind};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// `Enemy` monsters an iron golem goes after (never creepers).
fn golem_prey(s: &super::Seen) -> bool {
    s.hostile && s.species != Species::Creeper
}
/// `IronGolem.registerGoals`, without `ResetUniversalAngerTargetGoal`
/// (nothing reads the `universal_anger` rule yet).
pub fn register_iron(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(1, MeleeAttack::new(1.0, true));
    g.add(
        2,
        MoveTowardsTarget {
            speed: 0.9,
            within: 32.0,
        },
    );
    let mut back = Stroll::new(0.6)
        .interval(10)
        .named("MoveBackToVillageGoal")
        .gate(outside_village)
        .position(back_to_village_position);
    back.check_no_action = false;
    g.add(2, back);
    let mut stroll = Stroll::new(0.6)
        .interval(240)
        .named("GolemRandomStrollInVillageGoal")
        .position(village_stroll_position);
    stroll.check_no_action = false;
    g.add(4, stroll);
    g.add(
        5,
        OfferFlower {
            ticks: 0,
            villager: None,
        },
    );
    g.add(7, LookAtPlayer::new(6.0));
    g.add(8, RandomLookAround::default());
    m.ai.targets.add(1, DefendVillage);
    m.ai.targets.add(2, HurtByTarget::new());
    m.ai.targets.add(
        3,
        NearestTarget::player(true)
            .interval(10)
            .when(|m, _| m.extra.anger > 0),
    );
    m.ai.targets.add(
        3,
        NearestTarget::new(Prey::Filter(golem_prey), false).interval(5),
    );
}
/// `SectionPos.of`: the section a position is in.
fn section_of(p: Vec3) -> [i32; 3] {
    [
        (p.x.floor() as i32) >> 4,
        (p.y.floor() as i32) >> 4,
        (p.z.floor() as i32) >> 4,
    ]
}
/// `SectionPos.center` as `Vec3.atBottomCenterOf` reads it.
fn section_center(s: [i32; 3]) -> [f64; 3] {
    [
        f64::from(s[0] * 16 + 8) + 0.5,
        f64::from(s[1] * 16 + 8),
        f64::from(s[2] * 16 + 8) + 0.5,
    ]
}
/// `MoveBackToVillageGoal.canUse`: only a golem outside the village
/// (`ServerLevel.isVillage` at its feet, at most 1 section from a village
/// section) goes back to it.
fn outside_village(m: &Mob, w: &MobWorld) -> bool {
    sections_to_village(&village_sections(m, w), section_of(m.position)) > 1
}
/// `MoveBackToVillageGoal.getPosition`: toward the section within 2 that
/// is closest to a village, a quarter turn either side; none when the
/// golem's own section already is.
fn back_to_village_position(m: &mut Mob, cx: &mut Cx) -> Option<[f64; 3]> {
    let villages = village_sections(m, cx.w);
    let here = section_of(m.position);
    let best = closest_section(&villages, here, 2);
    if best == here {
        return None;
    }
    m.default_random_pos_towards(
        cx.w,
        10,
        7,
        section_center(best),
        f64::from(std::f32::consts::FRAC_PI_2),
    )
}
/// `GolemRandomStrollInVillageGoal.getPosition`: now and then anywhere
/// (3 in 10); else toward a villager who wants a golem or toward a spot in
/// a village section (7 in 10 the villager first), and anywhere if neither
/// is found. The picks draw from the level's random.
fn village_stroll_position(m: &mut Mob, cx: &mut Cx) -> Option<[f64; 3]> {
    if cx.lr.next_float() < 0.3 {
        return m.land_random_pos(cx.w, 10, 7);
    }
    let target = if cx.lr.next_float() < 0.7 {
        towards_villager_who_wants_golem(m, cx).or_else(|| towards_poi(m, cx))
    } else {
        towards_poi(m, cx).or_else(|| towards_villager_who_wants_golem(m, cx))
    };
    target.or_else(|| m.land_random_pos(cx.w, 10, 7))
}
/// `GolemRandomStrollInVillageGoal.getPositionTowardsVillagerWhoWantsGolem`:
/// one of the villagers within 32 blocks that want a golem, picked from
/// the level's random.
fn towards_villager_who_wants_golem(m: &mut Mob, cx: &mut Cx) -> Option<[f64; 3]> {
    let area = m.bounds().inflate(Vec3::new(32.0, 32.0, 32.0));
    let now = cx.w.game_time;
    let villagers: Vec<Vec3> =
        cx.w.others_near(area)
            .filter(|s| {
                s.species == Species::Villager
                    && s.bounds.intersects(area)
                    && s.villager.as_ref().is_some_and(|v| v.wants_golem(now))
            })
            .map(|s| s.position)
            .collect();
    if villagers.is_empty() {
        return None;
    }
    let at = villagers[cx.lr.next_int_bound(villagers.len() as i32).unwrap_or(0) as usize];
    m.land_random_pos_towards(cx.w, 10, 7, [at.x, at.y, at.z])
}
/// `GolemRandomStrollInVillageGoal.getPositionTowardsPoi`: a taken point of
/// interest within 8 blocks of the centre of a random village section
/// (`getRandomVillageSection`, `getRandomPoiWithinSection`) among those
/// within 2 sections of the golem.
fn towards_poi(m: &mut Mob, cx: &mut Cx) -> Option<[f64; 3]> {
    let villages = village_sections(m, cx.w);
    let here = section_of(m.position);
    let mut sections = Vec::new();
    for z in here[2] - 2..=here[2] + 2 {
        for y in here[1] - 2..=here[1] + 2 {
            for x in here[0] - 2..=here[0] + 2 {
                if sections_to_village(&villages, [x, y, z]) == 0 {
                    sections.push([x, y, z]);
                }
            }
        }
    }
    if sections.is_empty() {
        return None;
    }
    let s = sections[cx.lr.next_int_bound(sections.len() as i32).unwrap_or(0) as usize];
    let taken = others_tickets(m, cx.w);
    let pois = occupied_pois_in_range(
        cx.w,
        &taken,
        [s[0] * 16 + 8, s[1] * 16 + 8, s[2] * 16 + 8],
        8,
    );
    if pois.is_empty() {
        return None;
    }
    let p = pois[cx.lr.next_int_bound(pois.len() as i32).unwrap_or(0) as usize];
    m.land_random_pos_towards(
        cx.w,
        10,
        7,
        [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]),
            f64::from(p[2]) + 0.5,
        ],
    )
}
/// `DefendVillageTargetGoal`: a player that a villager nearby has no
/// good word for (reputation of -100 or worse) is attacked on sight.
#[derive(Clone, Debug)]
struct DefendVillage;
impl Goal for DefendVillage {
    goal_clone!();
    fn flags(&self) -> u8 {
        super::goals::TARGET
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(p) = &cx.w.player else { return false };
        if !p.alive || p.creative || p.spectator || m.spares_players() {
            return false;
        }
        // The box around the golem: 10 across, 8 up and down.
        let b = m.bounds().inflate(Vec3::new(10.0, 8.0, 10.0));
        if !b.intersects(p.bounds) || m.position.distance_squared(p.position) > 64.0 * 64.0 {
            return false;
        }
        // A position inside `b` puts the box's bottom face there (one block
        // wider, as the box test is strict).
        let near = b.inflate(Vec3::new(1.0, 1.0, 1.0));
        let bad = cx.w.others_near(near).any(|s| {
            s.species == Species::Villager
                && s.alive
                && s.player_rep <= -100
                && b.contains(s.position)
                && s.position.distance_squared(m.position) <= 64.0 * 64.0
        });
        bad && m.sees(cx.w, Who::Player)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let spares = m.spares_players();
        m.target_info(cx.w).is_some_and(|t| {
            t.alive
                && !(t.is_player && (t.creative || spares))
                && m.position.distance_squared(t.position) <= 64.0 * 64.0
        })
    }
    fn start(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.ai.target = Some(Who::Player);
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.ai.target = None;
    }
}
/// `MoveTowardsTargetGoal`: close in on a target within range.
#[derive(Clone, Debug)]
struct MoveTowardsTarget {
    speed: f64,
    within: f64,
}
impl Goal for MoveTowardsTarget {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(t) = m.ai.target.and_then(|t| cx.w.target_info(t)) else {
            return false;
        };
        if m.distance_sqr(t.position) > self.within * self.within {
            return false;
        }
        m.nav_move_to_entity(
            cx.w,
            [t.position.x, t.position.y, t.position.z],
            0,
            self.speed,
        )
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !m.ai.nav.is_done()
            && m.ai
                .target
                .and_then(|t| cx.w.target_info(t))
                .is_some_and(|t| t.alive && m.distance_sqr(t.position) < self.within * self.within)
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
    }
}
/// `OfferFlowerGoal`: by day, now and then, hold out a poppy to a villager.
#[derive(Clone, Debug)]
struct OfferFlower {
    ticks: i32,
    villager: Option<u32>,
}
impl Goal for OfferFlower {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !cx.w.daylight || m.random.next_int_bound(8000).unwrap_or(1) != 0 {
            return false;
        }
        // The nearest of #candidate_for_iron_golem_gift: villagers and copper golems.
        self.villager =
            cx.w.nearby(m.position, 6.0, |s| {
                matches!(s.species, Species::Villager | Species::CopperGolem) && s.alive
            })
            .map(|s| s.id)
            .next();
        self.villager.is_some()
    }
    fn can_continue(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        self.ticks > 0
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        // adjustedTickDelay(400): the goal ticks every other game tick.
        self.ticks = reduced(400);
        m.extra.counter = 400;
        // IronGolem.offerFlower(true): entity event 11.
        entity_event(m, 11, cx);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.extra.counter = 0;
        // IronGolem.offerFlower(false): entity event 34.
        entity_event(m, 34, cx);
        // A copper golem the offer ran out beside takes the poppy on its
        // antenna (#accepts_iron_golem_gift) when it has none.
        if self.ticks == 0
            && let Some(v) = self.villager.and_then(|v| cx.w.seen(v))
            && v.species == Species::CopperGolem
        {
            let b = m.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            let (vlo, vhi) = (v.bounds.min_position(), v.bounds.max_position());
            if lo.x - 6.0 < vhi.x
                && hi.x + 6.0 > vlo.x
                && lo.y - 2.0 < vhi.y
                && hi.y + 2.0 > vlo.y
                && lo.z - 6.0 < vhi.z
                && hi.z + 6.0 > vlo.z
            {
                cx.events.push(MobEvent::Gift {
                    target: v.id,
                    item: "minecraft:poppy",
                });
            }
        }
        self.villager = None;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(v) = self.villager.and_then(|v| cx.w.seen(v)) {
            m.look_at_speed([v.position.x, v.eye_y, v.position.z], 30.0, 30.0);
        }
        self.ticks -= 1;
    }
}
/// `IronGolem.doHurtTarget`: half to one and a half times the damage, and
/// the target is thrown upward.
pub fn iron_hurt_target(m: &mut Mob, who: Who, cx: &mut Cx) {
    m.extra.anim_ticks = 10;
    entity_event(m, 4, cx);
    let attack = m.attrs.attack_damage;
    let amount = if attack as i32 > 0 {
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
    cx.events.push(MobEvent::Launch {
        target: who,
        up: 0.4,
    });
    cx.events.push(MobEvent::Sound(
        "entity.iron_golem.attack",
        m.center(),
        1.0,
        1.0,
    ));
}
/// `Level.broadcastEntityEvent` for an iron golem (guests run `client_event`).
fn entity_event(m: &Mob, event: i8, cx: &mut Cx) {
    cx.events.push(MobEvent::EntityEvent {
        mob: m.id,
        event,
        at: [m.position.x, m.position.y, m.position.z],
    });
}
/// `IronGolem.handleEntityEvent` on a client: 4 the swing, 11 and 34 the
/// poppy offer starting and stopping.
pub fn client_event(m: &mut Mob, event: u8) {
    match event {
        4 => m.extra.anim_ticks = 10,
        11 => m.extra.counter = 400,
        34 => m.extra.counter = 0,
        _ => {}
    }
}
/// The swing and offer clocks of `IronGolem.aiStep` on a client.
pub fn client_tick(m: &mut Mob) {
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
    if m.extra.counter > 0 {
        m.extra.counter -= 1;
    }
}
/// `IronGolem.aiStep` timers and anger.
pub fn iron_ai_step(m: &mut Mob) {
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
    if m.extra.counter > 0 {
        m.extra.counter -= 1;
    }
    if m.extra.anger > 0 {
        m.extra.anger -= 1;
        if m.extra.anger == 0 && m.ai.target == Some(Who::Player) {
            m.ai.target = None;
        }
    }
}
/// `Crackiness.GOLEM` texture for the health left.
pub fn crackiness(m: &Mob) -> Option<&'static str> {
    let f = m.health / m.attrs.max_health;
    if f < 0.25 {
        Some("entity/iron_golem/iron_golem_crackiness_high")
    } else if f < 0.5 {
        Some("entity/iron_golem/iron_golem_crackiness_medium")
    } else if f < 0.75 {
        Some("entity/iron_golem/iron_golem_crackiness_low")
    } else {
        None
    }
}
fn triangle_wave(x: f32, period: f32) -> f32 {
    ((x % period) - period * 0.5).abs() / (period * 0.5) * 2.0 - 1.0
}
/// `IronGolemModel.setupAnim`.
pub fn iron_anim(m: &Mob, s: &State, p: &mut Poses) {
    let attack = if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks as f32 - s.partial
    } else {
        0.0
    };
    let (pos, speed) = (s.pos, s.speed);
    if attack > 0.0 {
        p.xrot("right_arm", -2.0 + 1.5 * triangle_wave(attack, 10.0));
        p.xrot("left_arm", -2.0 + 1.5 * triangle_wave(attack, 10.0));
    } else if m.extra.counter > 0 {
        p.xrot(
            "right_arm",
            -0.8 + 0.025 * triangle_wave(m.extra.counter as f32, 70.0),
        );
        p.xrot("left_arm", 0.0);
    } else {
        p.xrot("right_arm", (-0.2 + 1.5 * triangle_wave(pos, 13.0)) * speed);
        p.xrot("left_arm", (-0.2 - 1.5 * triangle_wave(pos, 13.0)) * speed);
    }
    p.yrot("head", s.y_rot.to_radians());
    p.xrot("head", s.x_rot.to_radians());
    p.xrot("right_leg", -1.5 * triangle_wave(pos, 13.0) * speed);
    p.xrot("left_leg", 1.5 * triangle_wave(pos, 13.0) * speed);
    p.yrot("right_leg", 0.0);
    p.yrot("left_leg", 0.0);
}

pub fn register_snow(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(
        1,
        super::witch::RangedAttack::new(1.25, 20, 20, 10.0, snowball_attack),
    );
    g.add(2, Stroll::avoiding_water(1.0));
    g.add(3, LookAtPlayer::new(6.0));
    g.add(4, RandomLookAround::default());
    m.ai.targets.add(
        1,
        NearestTarget::new(Prey::Filter(|s| s.hostile), true).interval(10),
    );
}
/// `SnowGolem.performRangedAttack`.
fn snowball_attack(m: &mut Mob, who: Who, _power: f32, cx: &mut Cx) {
    let Some(t) = cx.w.target_info(who) else {
        return;
    };
    let start = Vec3::new(m.position.x, m.eye_y() - 0.1, m.position.z);
    let xd = t.position.x - m.position.x;
    let yd = t.eye_y - 1.1;
    let zd = t.position.z - m.position.z;
    let yo = (xd * xd + zd * zd).sqrt() * 0.2;
    let mut p = Projectile::new(ProjectileKind::Snowball, start, Some(Who::Mob(m.id)));
    // SnowGolem.rangedAttackUncertainty: 12 on every difficulty.
    p.shoot(
        &mut crate::projectiles::own_random(m),
        [xd, yd + yo - start.y, zd],
        1.6,
        12.0,
    );
    cx.events.push(MobEvent::Projectile(p));
    let pitch = 0.4 / (m.random.next_float() * 0.4 + 0.8);
    cx.events.push(MobEvent::Sound(
        "entity.snow_golem.shoot",
        m.center(),
        1.0,
        pitch,
    ));
}
/// `SnowGolem.aiStep`: melting in hot biomes, the snow trail.
pub fn snow_ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let (x, y, z) = (
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    );
    // EnvironmentAttributes.SNOW_GOLEM_MELTS: the hot biomes and the Nether.
    if w.nether || w.temperature(x, y, z) > 1.0 {
        events.push(MobEvent::HurtSelf {
            mob: m.id,
            kind: super::DamageKind::OnFire,
            amount: 1.0,
        });
    }
    if !w.mob_griefing {
        return;
    }
    for i in 0..4 {
        let xx = (m.position.x + f64::from((i % 2 * 2 - 1) as f32 * 0.25)).floor() as i32;
        let zz = (m.position.z + f64::from((i / 2 % 2 * 2 - 1) as f32 * 0.25)).floor() as i32;
        let here = w.facts.get(w.world.block(xx, y, zz));
        let below = w.facts.get(w.world.block(xx, y - 1, zz));
        // SnowLayerBlock.canSurvive: a full top face, not ice.
        let ice = matches!(
            below.name.as_ref(),
            "minecraft:ice" | "minecraft:packed_ice" | "minecraft:barrier"
        );
        if here.name.as_ref() == "minecraft:air" && below.full_collision && !ice {
            events.push(MobEvent::SetBlockName {
                pos: [xx, y, zz],
                name: "minecraft:snow",
                effect: false,
            });
        }
    }
    let _ = z;
}
