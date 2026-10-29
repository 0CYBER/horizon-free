//! Mobs using a spear (`KineticWeapon`): `startUsingItem` of the main hand
//! and the stab of every tick of the use (`KineticWeapon.damageEntities`).
//! A mob's `using_item` counts the ticks of the use up from 1, which the
//! living entity flags tell the clients (`mob_meta`), so the arm and the
//! spear are drawn charging for them as well.
use super::{Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who, gear};
use crate::spear::{Kinetic, REACH, hits_in_reach};
use horizon_core::geometry::{Aabb, Vec3, trig::Mth};

/// The kinetic weapon in the main hand (`getMainHandItem().get(KINETIC_WEAPON)`).
pub fn main_hand_kinetic(m: &Mob) -> Option<Kinetic> {
    m.extra.main_hand.as_deref().and_then(Kinetic::of)
}

/// `LivingEntity.isUsingItem`. A piglin admiring what it holds in its off
/// hand counts `using_item` down as well; vanilla keeps that in a memory.
pub fn is_using_item(m: &Mob) -> bool {
    m.extra.using_item > 0 && !(m.species() == Species::Piglin && m.extra.off_hand.is_some())
}

/// `LivingEntity.startUsingItem(MAIN_HAND)`: the use of a kinetic weapon
/// starts with a new `recentKineticEnemies`.
pub fn start_using_item(m: &mut Mob) {
    if m.extra.main_hand.is_none() || is_using_item(m) {
        return;
    }
    m.extra.using_item = 1;
    m.extra.using_off_hand = false;
    if main_hand_kinetic(m).is_some() {
        m.extra.stabbed = Some(Vec::new());
    }
}

/// `LivingEntity.stopUsingItem`: `recentKineticEnemies` is gone.
pub fn stop_using_item(m: &mut Mob) {
    if is_using_item(m) {
        m.extra.using_item = 0;
    }
    m.extra.stabbed = None;
}

/// `LivingEntity.updatingUsingItem` of a kinetic weapon, in `LivingEntity.tick`
/// before the AI: the stab of this tick, then one more tick used. A use whose
/// weapon left the main hand ends. `known_speed` is what the mob moved last
/// tick (`Entity.lastKnownSpeed`).
pub fn update_using_item(m: &mut Mob, w: &MobWorld, known_speed: Vec3, events: &mut Vec<MobEvent>) {
    if m.extra.stabbed.is_none() {
        return;
    }
    let Some(kinetic) = main_hand_kinetic(m).filter(|_| is_using_item(m)) else {
        stop_using_item(m);
        return;
    };
    // `ItemStack.onUseTick` gets the ticks left; the use began at 1.
    let ticks_used = m.extra.using_item - 1;
    damage_entities(m, &kinetic, ticks_used, w, known_speed, events);
    if is_using_item(m) {
        m.extra.using_item += 1;
    }
}

/// What a stab can meet: who, where and how fast it moves per second.
struct Candidate {
    who: Who,
    bounds: Aabb,
    speed: Vec3,
}

/// The entities a stab may hit (`PiercingWeapon.canHitEntity`): the player
/// the mob sees and the other living mobs near it, but not the mount it sits
/// on or the rider it carries.
fn candidates(m: &Mob, w: &MobWorld) -> Vec<Candidate> {
    let mut out = Vec::new();
    if let Some(p) = w.player.as_ref().filter(|p| p.alive && !p.spectator) {
        out.push(Candidate {
            who: Who::Player,
            bounds: p.bounds,
            speed: p.velocity.scale(20.0),
        });
    }
    let r = 6.0;
    let area = m.bounds().inflate(Vec3::new(r, r, r));
    for s in w.others_near(area) {
        if s.id == m.id || !s.alive || m.extra.mount == Some(s.id) || m.extra.rider == Some(s.id) {
            continue;
        }
        out.push(Candidate {
            who: Who::Mob(s.id),
            bounds: s.bounds,
            speed: s.known_speed.scale(20.0),
        });
    }
    out
}

/// `Entity.getHeadLookAngle`: `calculateViewVector(xRot, yHeadRot)`.
fn head_look_angle(m: &Mob) -> Vec3 {
    let to_rad = std::f32::consts::PI / 180.0;
    let x = m.head_pitch * to_rad;
    let y = -m.head_yaw * to_rad;
    let (y_cos, y_sin, x_cos, x_sin) = (y.mcos(), y.msin(), x.mcos(), x.msin());
    Vec3::new(
        f64::from(y_sin * x_cos),
        f64::from(-x_sin),
        f64::from(y_cos * x_cos),
    )
}

/// `KineticWeapon.getMotion`: the known speed per second of the attacker, or
/// of the mount it rides.
fn motion(m: &Mob, w: &MobWorld, known_speed: Vec3) -> Vec3 {
    m.extra
        .mount
        .and_then(|id| w.seen(id))
        .map_or(known_speed, |s| s.known_speed)
        .scale(20.0)
}

/// `KineticWeapon.damageEntities` for a mob: from `delay_ticks` of the use
/// on, every entity along the view that the speed allows is stabbed, once in
/// `contact_cooldown_ticks` each. A mob's conditions are scaled by 0.2 and
/// its reach by the `mob_factor` of the spear's `AttackRange`.
fn damage_entities(
    m: &mut Mob,
    k: &Kinetic,
    ticks_used: i32,
    w: &MobWorld,
    known_speed: Vec3,
    events: &mut Vec<MobEvent>,
) {
    if ticks_used < k.delay_ticks {
        return;
    }
    let ticks_used = ticks_used - k.delay_ticks;
    let look = m.look_angle();
    let look = Vec3::new(look[0], look[1], look[2]);
    let attacker_speed = look.dot(motion(m, w, known_speed));
    // `AttackRange.effectiveMinRange` and `effectiveMaxRange` of a mob.
    let reach = (
        REACH.min * REACH.mob_factor,
        REACH.max * REACH.mob_factor,
        REACH.hitbox_margin,
    );
    let targets = candidates(m, w);
    let boxes: Vec<(usize, Aabb)> = targets
        .iter()
        .enumerate()
        .map(|(i, t)| (i, t.bounds))
        .collect();
    let eye = Vec3::new(m.position.x, m.eye_y(), m.position.z);
    let hits = hits_in_reach(
        &boxes,
        eye,
        head_look_angle(m),
        m.velocity,
        reach,
        |from, to| w.clip_block(from, to),
    );
    let now = w.game_time;
    let action_factor = f64::from(0.2_f32);
    // The attack damage attribute's base value.
    let base = m.attrs.attack_damage;
    let mut affected = false;
    for (i, _) in hits {
        let target = &targets[i];
        let recent = m.extra.stabbed.as_ref().is_some_and(|stabbed| {
            stabbed.iter().any(|&(who, at)| {
                who == target.who && now - at < i64::from(k.contact_cooldown_ticks)
            })
        });
        if recent {
            continue;
        }
        // `LivingEntity.rememberStabbedEntity`.
        if let Some(stabbed) = m.extra.stabbed.as_mut() {
            match stabbed.iter_mut().find(|(who, _)| *who == target.who) {
                Some(entry) => entry.1 = now,
                None => stabbed.push((target.who, now)),
            }
        }
        let relative = (attacker_speed - look.dot(target.speed)).max(0.0);
        let dismounts = k
            .dismount
            .test(ticks_used, attacker_speed, relative, action_factor);
        let pushes = k
            .knockback
            .test(ticks_used, attacker_speed, relative, action_factor);
        let damages = k
            .damage
            .test(ticks_used, attacker_speed, relative, action_factor);
        if dismounts || pushes || damages {
            let amount = base + (relative * f64::from(k.damage_multiplier)).floor() as i32 as f32;
            affected |= stab_attack(
                m,
                target.who,
                amount,
                (damages, pushes, dismounts),
                w,
                events,
            );
        }
    }
    if affected {
        // `Level.broadcastEntityEvent(mob, 2)`: the hit is felt and heard.
        events.push(MobEvent::EntityEvent {
            mob: m.id,
            event: 2,
            at: [m.position.x, m.position.y, m.position.z],
        });
    }
}

/// `LivingEntity.stabAttack` of a mob on `target`, with what the conditions
/// allow (`deals_damage`, `deals_knockback`, `dismounts`): whether it was
/// affected. The damage and the pushes are the game's events; a hit that
/// the target's damage cooldown refuses still counts as landed here.
fn stab_attack(
    m: &mut Mob,
    target: Who,
    base_damage: f32,
    (deals_damage, deals_knockback, dismounts): (bool, bool, bool),
    w: &MobWorld,
    events: &mut Vec<MobEvent>,
) -> bool {
    let species = match target {
        Who::Mob(id) => w.seen(id).map(|s| s.species),
        Who::Player => None,
    };
    // `EnchantmentHelper.modifyDamage`.
    let damage = base_damage + gear::melee_bonus(m, species);
    let from = [m.position.x, m.position.y, m.position.z];
    let mut affected = deals_knockback;
    if deals_damage {
        match target {
            Who::Player => events.push(MobEvent::DamagePlayer {
                amount: damage,
                from,
                kind: "minecraft:spear",
            }),
            Who::Mob(id) => events.push(MobEvent::AttackMob {
                target: id,
                amount: damage,
                source: Damage {
                    kind: DamageKind::Spear,
                    attacker: Some(Who::Mob(m.id)),
                    from: Some(from),
                },
            }),
        }
        affected = true;
        // `EnchantmentHelper.doPostAttackEffects`: Fire Aspect.
        let burn = gear::fire_aspect(m);
        if burn > 0 {
            events.push(MobEvent::Ignite {
                target,
                seconds: burn,
            });
        }
    }
    if deals_knockback {
        cause_extra_knockback(m, target, 0.4_f32, events);
        // `LivingEntity.getKnockback`: the Knockback enchantment, halved.
        cause_extra_knockback(m, target, gear::knockback(m) as f32 / 2.0, events);
    }
    if dismounts && w.target_info(target).is_some_and(|t| t.passenger) {
        affected = true;
        events.push(MobEvent::StabDismount { target });
    }
    affected
}

/// `LivingEntity.causeExtraKnockback`: a push along the attacker's facing,
/// which slows the attacker.
fn cause_extra_knockback(m: &mut Mob, target: Who, power: f32, events: &mut Vec<MobEvent>) {
    if power <= 0.0 {
        return;
    }
    let to_rad = std::f32::consts::PI / 180.0;
    events.push(MobEvent::Knockback {
        target,
        power: f64::from(power),
        x: f64::from((m.yaw * to_rad).msin()),
        z: f64::from(-(m.yaw * to_rad).mcos()),
    });
    m.velocity = Vec3::new(m.velocity.x * 0.6, m.velocity.y, m.velocity.z * 0.6);
}
