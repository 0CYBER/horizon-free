//! The per-tick mob update in vanilla order (`Entity.baseTick`,
//! `LivingEntity.baseTick/tick/aiStep`, `Mob.serverAiStep`) and the
//! dispatch to each species family's goals and special behaviour.
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who, aquatic,
    control::{BodyRotation, LookControl, MoveControl},
    farm,
    goals::{Cx, GoalSelector},
    hostile,
    nav::Navigation,
    path::{PATH_TYPES, Path},
    undead,
};
use crate::blocks::Fluid;
use horizon_core::{geometry::Vec3, random::RandomSource};

#[derive(Clone, Debug, Default)]
pub struct Ai {
    pub goals: GoalSelector,
    pub targets: GoalSelector,
    pub nav: Navigation,
    pub mov: MoveControl,
    pub look: LookControl,
    pub body: BodyRotation,
    /// JumpControl.jump (requested this tick).
    pub jump: bool,
    /// LivingEntity.jumping
    pub jumping: bool,
    pub no_jump_delay: i32,
    pub xxa: f32,
    pub yya: f32,
    pub zza: f32,
    pub speed: f32,
    pub target: Option<Who>,
    pub last_hurt_by: Option<Who>,
    pub last_hurt_by_time: i32,
    /// Ticks left during which kills count as player kills.
    pub last_hurt_by_player: i32,
    pub no_action_time: i32,
    pub aggressive: bool,
    pub panicking: bool,
    pub malus: [Option<f32>; PATH_TYPES],
    /// A path computed in canUse and consumed by start.
    pub pending_path: Option<Path>,
    /// Sensing: line-of-sight results cached for one tick.
    pub seen_cache: Vec<(Who, bool)>,
    /// Swing animation progress (attackAnim) and ticks.
    pub swing_time: i32,
    pub swinging: bool,
}
impl Mob {
    /// LivingEntity.swing
    pub fn swing(&mut self) {
        if !self.ai.swinging || self.ai.swing_time >= 3 || self.ai.swing_time < 0 {
            self.ai.swing_time = -1;
            self.ai.swinging = true;
        }
    }
}
/// The module implementing a species' behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Farm,
    Undead,
    Hostile,
    Aquatic,
    Generic,
    /// No goals at all (Giant).
    Inert,
}
pub fn family(s: Species) -> Family {
    use Species as S;
    match s {
        S::Pig | S::Cow | S::Mooshroom | S::Sheep | S::Chicken => Family::Farm,
        S::Zombie
        | S::Husk
        | S::Drowned
        | S::ZombieVillager
        | S::Skeleton
        | S::Stray
        | S::Bogged
        | S::WitherSkeleton => Family::Undead,
        S::Creeper | S::Spider | S::CaveSpider => Family::Hostile,
        S::Squid
        | S::GlowSquid
        | S::Cod
        | S::Tadpole
        | S::Salmon
        | S::TropicalFish
        | S::Pufferfish
        | S::Dolphin => Family::Aquatic,
        S::Giant => Family::Inert,
        _ => Family::Generic,
    }
}
pub fn register(m: &mut Mob) {
    match family(m.species()) {
        Family::Farm => farm::register(m),
        Family::Undead => undead::register(m),
        Family::Hostile => hostile::register(m),
        Family::Aquatic => aquatic::register(m),
        Family::Generic => generic(m),
        Family::Inert => {}
    }
}
/// Mob.tick additions that run before the entity tick (creeper fuse, puffing).
fn pre_tick(m: &mut Mob, events: &mut Vec<MobEvent>) {
    match family(m.species()) {
        Family::Hostile => {
            if m.species() == Species::Creeper {
                hostile::creeper_tick(m, events)
            }
        }
        Family::Aquatic => aquatic::tick(m, events),
        _ => {}
    }
}
/// Move controls implemented by the species (MoveKind::Custom).
pub fn custom_move_control(m: &mut Mob) {
    if family(m.species()) == Family::Aquatic {
        aquatic::fish_move_control(m)
    }
}
/// A fallback for species without bespoke goals yet: animals stroll and
/// panic, monsters hunt the player in melee.
fn generic(m: &mut Mob) {
    use super::goals::*;
    let hostile = m.hostile();
    let g = &mut m.ai.goals;
    if hostile {
        g.add(1, Float);
        g.add(2, MeleeAttack::new(1.0, false));
        g.add(5, Stroll::avoiding_water(0.8));
        g.add(6, LookAtPlayer::new(8.0));
        g.add(6, RandomLookAround::default());
        m.ai.targets.add(1, HurtByTarget::new());
        m.ai.targets.add(2, NearestTarget::player(true));
    } else {
        g.add(0, Float);
        g.add(1, Panic::new(1.25));
        g.add(5, Stroll::avoiding_water(1.0));
        g.add(6, LookAtPlayer::new(6.0));
        g.add(7, RandomLookAround::default());
    }
}
/// One full entity tick. Returns false when the mob is gone.
pub fn tick(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) -> bool {
    if m.removed {
        return false;
    }
    m.previous = m.position;
    m.previous_yaw = m.yaw;
    m.previous_body_yaw = m.body_yaw;
    m.previous_head_yaw = m.head_yaw;
    m.previous_head_pitch = m.head_pitch;
    m.previous_swell = m.swell;
    m.anim.begin_tick();
    m.age += 1;
    m.ai.seen_cache.clear();
    if m.ai.last_hurt_by_player > 0 {
        m.ai.last_hurt_by_player -= 1;
    }
    // Mob.tick pre-update hooks (Creeper.tick swells before super.tick).
    pre_tick(m, events);
    if m.removed {
        return false;
    }
    base_tick(m, w, events);
    if m.death_time >= 20 {
        return false;
    }
    ai_step(m, w, events);
    // Mob.tickHeadTurn -> BodyRotationControl.clientTick
    if m.alive() {
        m.body_rotation_tick();
    }
    m.update_walk_animation();
    m.anim.end_tick(m.ai.swinging);
    tick_swing(m);
    if !m.alive() {
        return true;
    }
    if !check_despawn(m, w) {
        m.removed = true;
        return false;
    }
    m.position.y > -128.0
}
/// LivingEntity.updateSwingTime (6-tick swing duration).
fn tick_swing(m: &mut Mob) {
    if m.ai.swinging {
        m.ai.swing_time += 1;
        if m.ai.swing_time >= 6 {
            m.ai.swing_time = 0;
            m.ai.swinging = false;
        }
    } else {
        m.ai.swing_time = 0;
    }
    m.anim.attack_anim = m.ai.swing_time as f32 / 6.0;
}
/// Entity.baseTick + LivingEntity.baseTick + Mob.baseTick.
fn base_tick(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    m.fluids = w.fluids(m.bounds(), m.eye_y());
    // Fire: water extinguishes, lava ignites, burning hurts every second.
    if m.fluids.in_water && m.fire_ticks > 0 {
        m.fire_ticks = 0;
        events.push(MobEvent::Sound(
            "entity.generic.extinguish_fire",
            m.center(),
            0.7,
            1.6 + (m.random.next_float() - m.random.next_float()) * 0.4,
        ));
    }
    if m.fluids.in_lava && !m.kind.flags.fire_immune {
        m.fire_ticks = m.fire_ticks.max(300);
        m.damage(4.0, Damage::of(DamageKind::Lava));
    }
    if m.fire_ticks > 0 {
        if m.kind.flags.fire_immune {
            m.fire_ticks = (m.fire_ticks - 4).max(0);
        } else {
            if m.fire_ticks % 20 == 0 && !m.fluids.in_lava {
                m.damage(1.0, Damage::of(DamageKind::OnFire));
            }
            m.fire_ticks -= 1;
        }
    }
    if m.alive() {
        // LivingEntity.baseTick: suffocation inside full blocks.
        let (ex, ey, ez) = (
            m.position.x.floor() as i32,
            m.eye_y().floor() as i32,
            m.position.z.floor() as i32,
        );
        let at_eye = w.facts.get(w.world.block(ex, ey, ez));
        if at_eye.full_collision && at_eye.solid_render && m.width() < 1.0 {
            m.damage(1.0, Damage::of(DamageKind::InWall));
        }
        // Air supply and drowning (WaterAnimal.handleAirSupply for aquatic mobs).
        if m.kind.flags.needs_water {
            if !m.fluids.in_water {
                m.air -= 1;
                if m.air == -20 {
                    m.air = 0;
                    m.damage(2.0, Damage::of(DamageKind::Drown));
                }
            } else {
                m.air = 300;
            }
        } else if m.fluids.eye_in_water && !m.kind.flags.breathes_water {
            m.air -= 1;
            if m.air == -20 {
                m.air = 0;
                m.damage(2.0, Damage::of(DamageKind::Drown));
            }
        } else if m.air < 300 {
            m.air = (m.air + 4).min(300);
        }
        if m.kind.flags.water_sensitive
            && (m.fluids.in_water || w.raining && w.can_see_sky(ex, ey, ez))
        {
            m.damage(1.0, Damage::of(DamageKind::Drown));
        }
    }
    tick_effects(m);
    if m.hurt_time > 0 {
        m.hurt_time -= 1;
    }
    if m.invulnerable > 0 {
        m.invulnerable -= 1;
    }
    if !m.alive() {
        m.death_time += 1;
    }
    // Mob.baseTick: ambient sounds.
    if m.alive() {
        let roll = m.random.next_int_bound(1000).unwrap_or(999);
        let t = m.ambient_timer;
        m.ambient_timer += 1;
        if roll < t {
            m.ambient_timer = -m.kind.ambient_interval;
            play_ambient(m, w, events);
        }
    }
}
fn play_ambient(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let sound = match m.species() {
        Species::Zombie | Species::Husk if m.is_baby() => m.kind.sounds[0],
        Species::Drowned if m.fluids.in_water => "entity.drowned.ambient_water",
        Species::Enderman if m.ai.target.is_some() => "entity.enderman.scream",
        Species::Wolf if m.ai.target.is_some() => "entity.wolf.growl",
        Species::Wolf if m.random.next_int_bound(3).unwrap_or(1) == 0 => {
            if m.extra.tame && m.health < m.attrs.max_health / 2.0 {
                "entity.wolf.whine"
            } else {
                "entity.wolf.pant"
            }
        }
        Species::Cat if m.extra.tame && m.extra.in_love > 0 => "entity.cat.purr",
        Species::Cat if !m.extra.tame => "entity.cat.stray_ambient",
        Species::PolarBear if m.is_baby() => "entity.polar_bear.ambient_baby",
        Species::Dolphin if !m.fluids.in_water => "entity.dolphin.ambient",
        Species::Axolotl if !m.fluids.in_water => "entity.axolotl.idle_air",
        Species::Bat if m.extra.flag && m.random.next_int_bound(4).unwrap_or(1) != 0 => return,
        Species::ZombifiedPiglin if m.ai.target.is_some() => "entity.zombified_piglin.angry",
        Species::Piglin if m.ai.target.is_some() => "entity.piglin.angry",
        Species::Hoglin if m.ai.target.is_some() => "entity.hoglin.angry",
        _ => m.kind.sounds[0],
    };
    let _ = w;
    if !sound.is_empty() {
        m.voice(sound, events);
    }
}
/// LivingEntity.aiStep (and Mob/AgeableMob/Animal additions).
fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.ai.no_jump_delay > 0 {
        m.ai.no_jump_delay -= 1;
    }
    let v = m.velocity;
    m.velocity = Vec3::new(
        if v.x.abs() < 0.003 { 0.0 } else { v.x },
        if v.y.abs() < 0.003 { 0.0 } else { v.y },
        if v.z.abs() < 0.003 { 0.0 } else { v.z },
    );
    m.ai.xxa *= 0.98;
    m.ai.zza *= 0.98;
    if m.alive() {
        server_ai_step(m, w, events);
    } else {
        m.ai.jumping = false;
        m.ai.xxa = 0.0;
        m.ai.zza = 0.0;
    }
    m.handle_jumping();
    let input = Vec3::new(
        f64::from(m.ai.xxa),
        f64::from(m.ai.yya),
        f64::from(m.ai.zza),
    );
    m.travel(w, input, events);
    // Spider.tick: climbing while pressed against a wall.
    if m.kind.flags.climber {
        m.climbing = m.horizontal_collision;
    }
    // AgeableMob.aiStep
    if m.alive() {
        if m.extra.age < 0 {
            m.extra.age += 1;
        } else if m.extra.age > 0 {
            m.extra.age -= 1;
        }
        // Animal.aiStep
        if m.extra.age != 0 {
            m.extra.in_love = 0;
        }
        if m.extra.in_love > 0 {
            m.extra.in_love -= 1;
            if m.extra.in_love % 10 == 0 {
                events.push(MobEvent::Particles {
                    kind: "heart",
                    at: [
                        m.position.x,
                        m.position.y + f64::from(m.height()) + 0.5,
                        m.position.z,
                    ],
                    count: 1,
                });
            }
        }
    }
    // Mob.aiStep: undead burn in daylight.
    if m.kind.flags.burns_in_day && m.alive() && sun_burn_tick(m, w) && m.extra.head.is_none() {
        m.fire_ticks = m.fire_ticks.max(160);
    }
    species_ai_step(m, w, events);
}
fn sun_burn_tick(m: &mut Mob, w: &MobWorld) -> bool {
    if !w.daylight || w.nether {
        return false;
    }
    let (x, y, z) = (
        m.position.x.floor() as i32,
        m.eye_y().floor() as i32,
        m.position.z.floor() as i32,
    );
    let br = w.brightness(x, y, z);
    let wet = m.fluids.in_water || w.raining && w.can_see_sky(x, y, z);
    br > 0.5 && m.random.next_float() * 30.0 < (br - 0.4) * 2.0 && !wet && w.can_see_sky(x, y, z)
}
/// Mob.serverAiStep
fn server_ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    m.ai.no_action_time += 1;
    let tick_count = m.age as i64;
    let id_based = tick_count + i64::from(m.id);
    let mut targets = std::mem::take(&mut m.ai.targets);
    let mut goals = std::mem::take(&mut m.ai.goals);
    {
        let mut cx = Cx { w, events };
        if id_based % 2 != 0 && tick_count > 1 {
            targets.tick_running(m, &mut cx, false);
            goals.tick_running(m, &mut cx, false);
        } else {
            targets.tick(m, &mut cx);
            goals.tick(m, &mut cx);
        }
    }
    m.ai.targets = targets;
    m.ai.goals = goals;
    m.nav_tick(w);
    {
        let mut cx = Cx { w, events };
        custom_server_ai_step(m, &mut cx);
    }
    m.move_control_tick(w);
    m.look_control_tick();
    m.jump_control_tick();
}
fn custom_server_ai_step(m: &mut Mob, cx: &mut Cx) {
    match m.species() {
        Species::Sheep => {}
        Species::Zombie
        | Species::Husk
        | Species::Drowned
        | Species::ZombieVillager
        | Species::Skeleton
        | Species::Stray
        | Species::Bogged
        | Species::WitherSkeleton => undead::custom_server_ai_step(m, cx),
        _ => {}
    }
}
fn species_ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if family(m.species()) == Family::Aquatic {
        aquatic::ai_step(m, w, events);
    }
    match m.species() {
        Species::Chicken => farm::chicken_ai_step(m, w, events),
        Species::Sheep if m.extra.anim_ticks > 0 => {
            m.extra.anim_ticks -= 1;
        }
        _ => {}
    }
}
/// Mob.checkDespawn (players are the only despawn reference).
fn check_despawn(m: &mut Mob, w: &MobWorld) -> bool {
    let creature = m.kind.category == "creature";
    if m.persistent || creature && m.kind.flags.animal {
        m.ai.no_action_time = 0;
        return true;
    }
    let Some(p) = &w.player else { return true };
    let d = m.distance_sqr(p.position);
    let despawn = if m.kind.category == "water_ambient" {
        64.0
    } else {
        128.0
    };
    if d > despawn * despawn {
        return false;
    }
    if m.ai.no_action_time > 600
        && m.random.next_int_bound(800).unwrap_or(1) == 0
        && d > 32.0 * 32.0
    {
        return false;
    }
    if d < 32.0 * 32.0 {
        m.ai.no_action_time = 0;
    }
    true
}
/// Movement speed bonus multiplier (baby zombies are 50% faster, etc.).
pub fn speed_bonus(m: &Mob) -> f64 {
    match m.species() {
        Species::Zombie
        | Species::Husk
        | Species::Drowned
        | Species::ZombieVillager
        | Species::ZombifiedPiglin
        | Species::Piglin
            if m.is_baby() =>
        {
            1.5
        }
        _ => 1.0,
    }
}
pub fn sound_volume(m: &Mob) -> f32 {
    match m.species() {
        Species::Ghast => 5.0,
        Species::Slime | Species::MagmaCube => 0.4 * m.extra.size.max(1) as f32,
        Species::Cod
        | Species::Salmon
        | Species::TropicalFish
        | Species::Pufferfish
        | Species::Bat => 0.1,
        Species::Squid | Species::GlowSquid => 0.4,
        Species::Horse | Species::Donkey | Species::Mule => 0.8,
        _ => 1.0,
    }
}
/// PathfinderMob.getWalkTargetValue (Animal and Monster overrides).
pub fn walk_target_value(m: &Mob, w: &MobWorld, p: [i32; 3]) -> f32 {
    let light = || w.brightness(p[0], p[1], p[2]) - 0.5;
    if m.hostile()
        && !matches!(
            m.species(),
            Species::Slime | Species::MagmaCube | Species::Ghast | Species::Phantom
        )
    {
        return -light();
    }
    if m.kind.flags.animal {
        if w.block_name(p[0], p[1] - 1, p[2]) == "minecraft:grass_block" {
            return 10.0;
        }
        return light();
    }
    0.0
}
/// Species damage filters (LivingEntity.hurtServer overrides).
pub fn accepts_damage(m: &mut Mob, source: &Damage, _amount: f32) -> bool {
    match m.species() {
        Species::Enderman if source.is_projectile() => false,
        Species::WitherSkeleton | Species::Blaze | Species::MagmaCube | Species::Strider
            if source.is_fire() =>
        {
            false
        }
        _ => true,
    }
}
/// Reactions after taking damage (sounds, anger).
pub fn after_hurt(m: &mut Mob, _source: &Damage) {
    m.ai.no_action_time = 0;
}
/// Mob.doHurtTarget: attack damage (+ species extras) to the target.
pub fn do_hurt_target(m: &mut Mob, who: Who, cx: &mut Cx) {
    let mut amount = m.attrs.attack_damage;
    let from = [m.position.x, m.position.y, m.position.z];
    let source = Damage::mob(m.id, from);
    match m.species() {
        // Difficulty NORMAL: cave spiders poison for 7 seconds.
        Species::CaveSpider if who == Who::Player => cx.events.push(MobEvent::EffectPlayer {
            effect: "minecraft:poison",
            ticks: 7 * 20,
            amplifier: 0,
        }),
        Species::Husk if who == Who::Player => cx.events.push(MobEvent::EffectPlayer {
            effect: "minecraft:hunger",
            ticks: 140,
            amplifier: 0,
        }),
        Species::WitherSkeleton if who == Who::Player => cx.events.push(MobEvent::EffectPlayer {
            effect: "minecraft:wither",
            ticks: 200,
            amplifier: 0,
        }),
        _ => {}
    }
    if amount <= 0.0 {
        amount = 1.0;
    }
    // Zombies set their target on fire while burning (Zombie.doHurtTarget).
    if m.fire_ticks > 0 && matches!(m.species(), Species::Zombie) && m.random.next_float() < 0.3 {
        cx.events.push(MobEvent::Ignite {
            target: who,
            seconds: 2,
        });
    }
    match who {
        Who::Player => cx.events.push(MobEvent::AttackPlayer(amount, from)),
        Who::Mob(id) => cx.events.push(MobEvent::AttackMob {
            target: id,
            amount,
            source,
        }),
    }
    let _ = Fluid::None;
}
/// Mob.ate (sheep regrow wool, babies grow).
pub fn ate(m: &mut Mob) {
    if m.species() == Species::Sheep {
        m.extra.sheared = false;
        if m.is_baby() {
            // AgeableMob.ageUp(60)
            m.extra.age = (m.extra.age + 60 * 20).min(0);
        }
    }
}
/// Species-specific travel (fish, squid, dolphins swim with their own rules).
pub fn custom_travel(m: &mut Mob, w: &MobWorld, input: Vec3, events: &mut Vec<MobEvent>) -> bool {
    match family(m.species()) {
        Family::Aquatic => aquatic::travel(m, w, input, events),
        _ => false,
    }
}

/// MobEffect ticking for the effects mobs receive (poison, wither, regeneration).
fn tick_effects(m: &mut Mob) {
    let mut effects = std::mem::take(&mut m.extra.effects);
    for (effect, ticks, amp) in &mut effects {
        let interval = |base: i32| (base >> (*amp).max(0)).max(1);
        match *effect {
            "minecraft:poison"
                if *ticks % interval(25) == 0 && !m.kind.flags.undead && m.health > 1.0 =>
            {
                m.damage(1.0, Damage::of(DamageKind::Magic));
            }
            "minecraft:wither" if *ticks % interval(40) == 0 => {
                m.damage(1.0, Damage::of(DamageKind::Magic));
            }
            "minecraft:regeneration"
                if *ticks % interval(50) == 0 && !m.kind.flags.undead && m.alive() =>
            {
                m.health = (m.health + 1.0).min(m.attrs.max_health);
            }
            _ => {}
        }
        *ticks -= 1;
    }
    effects.retain(|e| e.1 > 0);
    m.extra.effects = effects;
}
