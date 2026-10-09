//! Creakings (`Creaking`, `CreakingAi`): they freeze while any player looks at
//! them, wake up when one looks from within 12 blocks, then hunt the nearest
//! visible player and hit every 40 ticks. One bound to a creaking heart takes
//! no damage (it only sways and hurts the heart's resin spreader), dies when
//! the heart no longer protects it and falls apart with the heart.
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    goals::{Cx, Float, Goal, LookAtPlayer, MeleeAttack, Stroll, TARGET, goal_clone},
    keyframes::Animation,
    path::PathType,
};
use crate::containers::BlockEntity;
use horizon_core::{geometry::Vec3, random::RandomSource};
use std::collections::HashMap;

/// `Extra.ints` slots.
const INVULNERABLE: usize = 0;
const ATTACKING: usize = 1;
const STUCK: usize = 2;
const TEARING_DOWN: usize = 3;
const EYES: usize = 4;
const NEXT_FLICKER: usize = 5;
/// `Extra.counter` bits: hurt while protected, and by a player.
const HURT: i32 = 1;
const HURT_BY_PLAYER: i32 = 2;

/// `Creaking.ACTIVATION_RANGE_SQ`.
const ACTIVATION_RANGE_SQ: f64 = 144.0;
/// `Creaking.INVULNERABILITY_ANIMATION_DURATION` and the attack animation.
const INVULNERABILITY_TICKS: i32 = 8;
const ATTACK_TICKS: i32 = 15;
/// `Creaking.TWITCH_DEATH_DURATION`.
const TWITCH_TICKS: i32 = 45;

pub fn register(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(2, MeleeAttack::new(1.0, false).interval(40));
    g.add(5, Stroll::avoiding_water(0.3));
    g.add(6, LookAtPlayer::new(8.0));
    m.ai.targets.add(1, HuntPlayer);
}
/// Bound to a heart (`getHomePos() != null`).
pub fn heart_bound(m: &Mob) -> bool {
    is_creaking(m) && m.extra.home.is_some()
}
pub fn tearing_down(m: &Mob) -> bool {
    is_creaking(m) && m.extra.ints[TEARING_DOWN] != 0
}
/// `Creaking.canMove`: false while a player looks at it.
pub fn can_move(m: &Mob) -> bool {
    !m.extra.flag2
}
/// `Creaking.isActive`.
pub fn is_active(m: &Mob) -> bool {
    m.extra.flag
}
/// `Creaking.setTransient`: bind to the heart at `pos` and avoid damage.
pub fn set_transient(m: &mut Mob, pos: [i32; 3]) {
    m.extra.home = Some(pos);
    m.set_malus(PathType::Damaging, 8.0);
    m.set_malus(PathType::PowderSnow, 8.0);
    m.set_malus(PathType::Lava, 8.0);
    m.set_malus(PathType::Fire, 0.0);
    m.set_malus(PathType::FireInNeighbor, 0.0);
}
/// `Creaking.setTearingDown`: the heart was broken by someone.
pub fn set_tearing_down(m: &mut Mob) {
    m.extra.ints[TEARING_DOWN] = 1;
}
/// `Creaking.hasGlowingEyes` / `isActive`: the eyes layer shows.
pub fn eyes_glowing(m: &Mob) -> bool {
    if tearing_down(m) {
        m.extra.ints[EYES] != 0
    } else {
        is_active(m)
    }
}
/// Ticks of death animation before the mob is removed (`tickDeath`).
pub fn death_limit(m: &Mob) -> i32 {
    if heart_bound(m) && tearing_down(m) {
        TWITCH_TICKS + 1
    } else {
        20
    }
}
/// `Creaking.tearDown`: crumbles away.
pub fn tear_down(m: &mut Mob, events: &mut Vec<MobEvent>) {
    let b = m.bounds();
    let (lo, hi) = (b.min_position(), b.max_position());
    events.push(MobEvent::CreakingTearDown {
        center: [
            (lo.x + hi.x) / 2.0,
            (lo.y + hi.y) / 2.0,
            (lo.z + hi.z) / 2.0,
        ],
        spread: [
            (hi.x - lo.x) * 0.3,
            (hi.y - lo.y) * 0.3,
            (hi.z - lo.z) * 0.3,
        ],
    });
    m.voice("entity.creaking.death", events);
    m.removed = true;
}
/// `Creaking.hurtServer` for a heart-bound creaking: no damage, only the
/// invulnerability animation and the heart's reaction. Returns whether the
/// normal damage path should run.
pub fn accepts_damage(m: &mut Mob, source: &Damage) -> bool {
    if !heart_bound(m) || source.kind == DamageKind::Generic {
        return true;
    }
    if source.is_fire() || !m.alive() || m.extra.ints[INVULNERABLE] > 0 {
        return false;
    }
    let by_player = source.attacker == Some(Who::Player);
    // The direct entity must be living or a projectile (or a player must be to blame).
    let direct = matches!(
        source.kind,
        DamageKind::PlayerAttack
            | DamageKind::MobAttack
            | DamageKind::Arrow
            | DamageKind::Thrown
            | DamageKind::Fireball
            | DamageKind::Thorns
    );
    if !direct && !by_player {
        return false;
    }
    m.extra.ints[INVULNERABLE] = INVULNERABILITY_TICKS;
    m.extra.counter |= HURT | if by_player { HURT_BY_PLAYER } else { 0 };
    false
}
/// `Creaking.doHurtTarget` extras: the attack animation and sound.
pub fn on_attack(m: &mut Mob, events: &mut Vec<MobEvent>) {
    m.extra.ints[ATTACKING] = ATTACK_TICKS;
    m.voice("entity.creaking.attack", events);
}
/// The heart this creaking is bound to protects it (`isProtector`); a link
/// read from a save is trusted until the heart resolves it.
fn protected(m: &Mob, w: &MobWorld) -> bool {
    let Some(home) = m.extra.home else {
        return true;
    };
    match w.world.block_entities.get(&(home[0], home[1], home[2])) {
        Some(BlockEntity::CreakingHeart(h)) => match h.creaking {
            Some((Some(id), _)) => id == m.id,
            Some((None, _)) => true,
            None => false,
        },
        _ => false,
    }
}
/// `Creaking.tick` and the start of `aiStep`: protection, timers, freezing.
pub fn update(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    // Creaking.tick: without its heart's protection it dies.
    if heart_bound(m) && m.alive() && !protected(m, w) {
        m.health = 0.0;
    }
    for slot in [INVULNERABLE, ATTACKING] {
        if m.extra.ints[slot] > 0 {
            m.extra.ints[slot] -= 1;
        }
    }
    let age = m.age as f32;
    m.anim.states[0].animate_when(m.extra.ints[ATTACKING] > 0, age);
    m.anim.states[1].animate_when(m.extra.ints[INVULNERABLE] > 0, age);
    m.anim.states[2].animate_when(tearing_down(m), age);
    if m.extra.counter & HURT != 0 {
        // hurtServer: sway, and the heart reacts to a player's hit.
        if protected(m, w) {
            let by_player = m.extra.counter & HURT_BY_PLAYER != 0;
            if let Some(home) = m.extra.home {
                events.push(MobEvent::CreakingHurt {
                    home,
                    by_player,
                    id: m.id,
                });
            }
            m.voice("entity.creaking.sway", events);
        }
        m.extra.counter &= !(HURT | HURT_BY_PLAYER);
    }
    // checkEyeBlink: the eyes of a creaking falling apart flicker.
    if m.death_time > m.extra.ints[NEXT_FLICKER] {
        let glowing = m.extra.ints[EYES] != 0;
        let (lo, hi) = if glowing {
            (2, 8)
        } else {
            (m.death_time / 4, m.death_time / 2)
        };
        let span = (hi - lo + 1).max(1);
        m.extra.ints[NEXT_FLICKER] = m.death_time + lo + m.random.next_int_bound(span).unwrap_or(0);
        m.extra.ints[EYES] = i32::from(!glowing);
    }
    if !m.alive() {
        return;
    }
    let now = check_can_move(m, w, events);
    if now != can_move(m) {
        if now {
            m.voice("entity.creaking.unfreeze", events);
        } else {
            stop_in_place(m);
            m.voice("entity.creaking.freeze", events);
        }
        m.extra.flag2 = !now;
    }
}
/// `Mob.stopInPlace`.
pub fn stop_in_place(m: &mut Mob) {
    m.nav_stop();
    m.ai.xxa = 0.0;
    m.ai.yya = 0.0;
    m.ai.zza = 0.0;
    m.ai.speed = 0.0;
}
fn activate(m: &mut Mob, events: &mut Vec<MobEvent>) {
    m.ai.target = Some(Who::Player);
    m.voice("entity.creaking.activate", events);
    m.extra.flag = true;
}
fn deactivate(m: &mut Mob, events: &mut Vec<MobEvent>) {
    m.ai.target = None;
    m.voice("entity.creaking.deactivate", events);
    m.extra.flag = false;
}
/// `LivingEntity.isLookingAtMe(player, 0.5, false, true, eyeY, y + 0.5, mid)`.
fn looked_at_by(m: &Mob, w: &MobWorld) -> bool {
    let Some(p) = &w.player else { return false };
    let look = p.look;
    let eye = m.eye_y();
    [eye, m.position.y + 0.5, (eye + m.position.y) / 2.0]
        .into_iter()
        .any(|y| {
            let d = [
                m.position.x - p.position.x,
                y - p.eye[1],
                m.position.z - p.position.z,
            ];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if len < 1.0e-6 {
                return true;
            }
            let dot = (look[0] * d[0] + look[1] * d[1] + look[2] * d[2]) / len;
            dot > 0.5
                && w.line_of_sight(
                    Vec3::new(p.eye[0], p.eye[1], p.eye[2]),
                    Vec3::new(m.position.x, y, m.position.z),
                )
        })
}
/// `Creaking.checkCanMove`: false while looked at; also wakes and calms it.
fn check_can_move(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) -> bool {
    let active = is_active(m);
    // NEAREST_PLAYERS: players within the follow range.
    let near = w.player.as_ref().filter(|p| {
        let d = Vec3::new(p.position.x, p.position.y, p.position.z);
        m.distance_sqr(d) < 32.0 * 32.0
    });
    let Some(p) = near else {
        if active {
            deactivate(m, events);
        }
        return true;
    };
    // canAttack: alive and not creative or spectator.
    let attackable = p.alive && !p.creative;
    if attackable {
        // GAZE_DISGUISE_EQUIPMENT hides the player once the creaking is awake.
        let disguised = p.head.as_deref() == Some("minecraft:carved_pumpkin");
        if (!active || !disguised) && looked_at_by(m, w) {
            if active {
                return false;
            }
            if m.distance_sqr(p.position) < ACTIVATION_RANGE_SQ {
                activate(m, events);
                return false;
            }
        }
    } else if active {
        deactivate(m, events);
    }
    true
}
/// `Creaking.playerIsStuckInYou`: the player's eyes inside it for more than 4 ticks.
pub fn player_is_stuck(m: &mut Mob, eye: Option<[f64; 3]>) -> bool {
    let Some(eye) = eye else {
        m.extra.ints[STUCK] = 0;
        return false;
    };
    let b = m.bounds();
    let (lo, hi) = (b.min_position(), b.max_position());
    let inside = eye[0] >= lo.x
        && eye[0] <= hi.x
        && eye[1] >= lo.y
        && eye[1] <= hi.y
        && eye[2] >= lo.z
        && eye[2] <= hi.z;
    if inside {
        m.extra.ints[STUCK] += 1;
        m.extra.ints[STUCK] > 4
    } else {
        m.extra.ints[STUCK] = 0;
        false
    }
}
/// The nearest attackable player the creaking can see.
fn sees_attackable_player(m: &mut Mob, cx: &mut Cx) -> bool {
    let Some(p) = &cx.w.player else {
        return false;
    };
    let near = m.distance_sqr(p.position) < m.attrs.follow_range.powi(2);
    p.alive && !p.creative && near && m.sees(cx.w, Who::Player)
}
/// `StartAttacking` / `StopAttackingIfTargetInvalid`: while awake, hunt the
/// nearest visible player.
#[derive(Clone, Debug)]
struct HuntPlayer;
impl Goal for HuntPlayer {
    goal_clone!();
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !is_active(m) || !sees_attackable_player(m, cx) {
            return false;
        }
        m.ai.target = Some(Who::Player);
        true
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        is_active(m) && m.ai.target == Some(Who::Player) && sees_attackable_player(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        if m.ai.target == Some(Who::Player) {
            m.ai.target = None;
        }
    }
}
/// `CreakingModel.setupAnim`.
pub fn anim(m: &Mob, s: &State, p: &mut Poses, animations: &HashMap<String, Animation>) {
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
    let get = |n: &str| animations.get(&format!("CreakingAnimation.CREAKING_{n}"));
    if can_move(m)
        && let Some(walk) = get("WALK")
    {
        walk.apply_walk(s.pos, s.speed, 1.0, 1.0, p);
    }
    for (state, name) in [
        (&m.anim.states[0], "ATTACK"),
        (&m.anim.states[1], "INVULNERABLE"),
        (&m.anim.states[2], "DEATH"),
    ] {
        if let (Some(ms), Some(a)) = (state.millis(s.age), get(name)) {
            a.apply(ms, 1.0, p);
        }
    }
}
/// The creaking species exists in the species table.
pub fn is_creaking(m: &Mob) -> bool {
    m.species() == Species::Creaking
}
