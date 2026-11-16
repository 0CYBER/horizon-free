//! A mob's `CombatTracker`: the hits it took lately, which its death message
//! is built from (`TamableAnimal.die` sends it to the owner).
use super::{Damage, Mob, MobEvent, Species, Who, goals::Cx};
use crate::blocks::Fluid;
use horizon_core::{geometry::Vec3, random::RandomSource};

/// `CombatEntry`: one hit, with where the mob was falling from when it landed.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub source: Damage,
    pub damage: f32,
    /// `FallLocation.getCurrentFallLocation` is `WATER`. Horizon keeps no
    /// `lastClimbablePos` for mobs, so the climbable locations are not known.
    pub in_water: bool,
    pub fall_distance: f32,
}

/// `CombatTracker` for a mob, timed by its `tickCount` (`Mob::age`).
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    pub entries: Vec<Entry>,
    last_damage_time: u32,
    pub in_combat: bool,
    taking_damage: bool,
}

impl Tracker {
    /// `CombatTracker.recordDamage`: the hit after forgetting an old fight; a
    /// living cause (`shouldEnterCombat`) starts a fight. It runs before the
    /// hit takes the health (`actuallyHurt`), so the mob is still alive.
    pub fn record(&mut self, entry: Entry, tick: u32) {
        self.recheck(true, tick);
        let living_cause = entry.source.attacker.is_some();
        self.entries.push(entry);
        self.last_damage_time = tick;
        self.taking_damage = true;
        if !self.in_combat && living_cause {
            self.in_combat = true;
        }
    }
    /// `CombatTracker.recheckStatus`: 100 ticks without a hit (300 in a
    /// fight) forget them all. Vanilla also forgets them once the mob is
    /// dead, after `die` has read them; Horizon's `die` runs after the tick,
    /// so a dead mob's hits stay for it.
    pub fn recheck(&mut self, alive: bool, tick: u32) {
        let reset = if self.in_combat { 300 } else { 100 };
        if alive && self.taking_damage && tick.wrapping_sub(self.last_damage_time) > reset {
            self.taking_damage = false;
            self.in_combat = false;
            self.entries.clear();
        }
    }
}

pub fn do_hurt_target(m: &mut Mob, who: Who, cx: &mut Cx) {
    if m.species() == Species::IronGolem {
        return super::golem::iron_hurt_target(m, who, cx);
    }
    if m.species() == Species::Bee {
        return super::bee::sting(m, who, cx);
    }
    if matches!(m.species(), Species::Hoglin | Species::Zoglin) {
        return super::piglin::hoglin_hurt_target(m, who, cx);
    }
    if m.species() == Species::Ravager {
        super::ravager::on_attack(m, cx.events);
    }
    if m.species() == Species::Panda {
        super::panda::bit(m);
    }
    if super::creaking::is_creaking(m) {
        super::creaking::on_attack(m, cx.events);
    }
    let mut amount = super::gear::attack_damage(m, cx.w.items);
    let from = [m.position.x, m.position.y, m.position.z];
    let source = Damage::mob(m.id, from).with_species(m.species());
    let first_event = cx.events.len();
    // `CaveSpider`, `Husk` and `WitherSkeleton.doHurtTarget` all test
    // `target instanceof LivingEntity`, so the effect reaches a mob the hit
    // lands on and not only the player.
    match m.species() {
        // CaveSpider.doHurtTarget: poison for 7 s on normal, 15 s on hard.
        Species::CaveSpider if cx.w.difficulty.base >= 2 => {
            cx.events.push(MobEvent::EffectPlayer {
                effect: "minecraft:poison",
                ticks: if cx.w.difficulty.base == 2 { 7 } else { 15 } * 20,
                amplifier: 0,
                ambient: false,
            })
        }
        // Husk.doHurtTarget: hunger for 140 ticks per whole point of the
        // local difficulty, only with an empty main hand.
        Species::Husk if m.extra.main_hand_id().is_none() => {
            let ticks = 140 * cx.w.difficulty.effective as i32;
            if ticks > 0 {
                cx.events.push(MobEvent::EffectPlayer {
                    effect: "minecraft:hunger",
                    ticks,
                    amplifier: 0,
                    ambient: false,
                })
            }
        }
        Species::WitherSkeleton => cx.events.push(MobEvent::EffectPlayer {
            effect: "minecraft:wither",
            ticks: 200,
            amplifier: 0,
            ambient: false,
        }),
        _ => {}
    }
    if amount <= 0.0 {
        amount = 1.0;
    }
    // The held weapon's enchantments: Sharpness, Smite, Bane of Arthropods,
    // Fire Aspect and Knockback.
    let target_species = match who {
        Who::Mob(id) => cx.w.seen(id).map(|s| s.species),
        Who::Player => None,
    };
    amount += super::gear::melee_bonus(m, target_species);
    let burn = super::gear::fire_aspect(m);
    if burn > 0 {
        cx.events
            .push(cx.w.route_player_event(MobEvent::Ignite(who, burn)));
    }
    // `Mob.doHurtTarget`: `causeExtraKnockback(target, getKnockback(...))`. `getKnockback` is the
    // `attack_knockback` attribute plus the Knockback enchantment, halved; the push goes along the
    // attacker's facing through `LivingEntity.knockback` (so the target's resistance applies), and
    // slows the attacker.
    let power = (m.attrs.attack_knockback as f32 + super::gear::knockback(m) as f32) / 2.0;
    if power > 0.0 {
        use horizon_core::geometry::trig::Mth;
        cx.events.push(cx.w.route_player_event(MobEvent::Knockback {
            target: who,
            power: f64::from(power),
            x: f64::from(m.yaw.to_radians().msin()),
            z: f64::from(-m.yaw.to_radians().mcos()),
            source,
            apply_blocking: None,
        }));
        m.velocity = Vec3::new(m.velocity.x * 0.6, m.velocity.y, m.velocity.z * 0.6);
    }
    // The effect picked above goes with the hit: it applies only when the
    // target takes damage (a shield-blocked hit gives none), for a mob as for
    // the player, since those `doHurtTarget`s take any `LivingEntity`.
    let effect = cx.events[first_event..]
        .iter()
        .position(|e| matches!(e, MobEvent::EffectPlayer { .. }))
        .and_then(|k| match cx.events.remove(first_event + k) {
            MobEvent::EffectPlayer {
                effect,
                ticks,
                amplifier,
                ..
            } => Some((effect, ticks, amplifier)),
            _ => None,
        });
    match who {
        Who::Player => cx.events.push(cx.w.route_player_event(match effect {
            Some((effect, ticks, amplifier)) => MobEvent::AttackPlayerWithEffect {
                amount,
                from,
                effect,
                ticks,
                amplifier,
            },
            None => MobEvent::AttackPlayer(amount, from),
        })),
        Who::Mob(id) => cx.events.push(match effect {
            Some((effect, ticks, amplifier)) => MobEvent::AttackMobWithEffect {
                target: id,
                amount,
                source,
                effect,
                ticks,
                amplifier,
            },
            None => MobEvent::AttackMob {
                target: id,
                amount,
                source,
            },
        }),
    }
    // Mob.doHurtTarget ends with playAttackSound once the hit landed.
    if super::creaking::is_creaking(m) {
        m.voice("entity.creaking.attack", cx.events);
    }
    // Rabbit.playAttackSound: only the killer bunny.
    if m.species() == Species::Rabbit {
        super::rabbit::attack_sound(m, cx.events);
    }
    // Panda.playAttackSound: playSound(PANDA_BITE, 1.0, 1.0).
    if m.species() == Species::Panda && !m.silent {
        let at = [m.position.x, m.position.y, m.position.z];
        cx.events
            .push(MobEvent::Sound("entity.panda.bite", at, 1.0, 1.0));
    }
    // Axolotl.playAttackSound: playSound(AXOLOTL_ATTACK, 1.0, 1.0).
    if m.species() == Species::Axolotl && !m.silent {
        let at = [m.position.x, m.position.y, m.position.z];
        cx.events
            .push(MobEvent::Sound("entity.axolotl.attack", at, 1.0, 1.0));
    }
    // Dolphin.playAttackSound: playSound(DOLPHIN_ATTACK, 1.0, 1.0), once
    // the blow landed. A dolphin that is hurt takes its attacker as a target
    // and strikes with the melee goal, so its bite is heard as it lands.
    if m.species() == Species::Dolphin && !m.silent {
        let at = [m.position.x, m.position.y, m.position.z];
        cx.events
            .push(MobEvent::Sound("entity.dolphin.attack", at, 1.0, 1.0));
    }
    // `Zombie.doHurtTarget` (every zombie): drawn here, lit if the blow hurt.
    // Queue this after the attack so the handler reads this blow's hit outcome.
    let difficulty = cx.w.difficulty.effective;
    if m.is_on_fire()
        && m.extra.main_hand_id().is_none()
        && super::doors::is_zombie(m.species())
        && m.random.next_float() < difficulty * 0.3
    {
        cx.events
            .push(cx.w.route_player_event(MobEvent::ZombieIgnite(who, 2 * difficulty as i32)));
    }
    let _ = Fluid::None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::{DamageKind, Who};

    fn hit(attacker: Option<Who>) -> Entry {
        Entry {
            source: Damage {
                kind: DamageKind::MobAttack,
                attacker,
                from: None,
                attacker_species: None,
            },
            damage: 2.0,
            in_water: false,
            fall_distance: 0.0,
        }
    }

    #[test]
    fn old_hits_are_forgotten_after_100_ticks_or_300_in_a_fight() {
        let mut t = Tracker::default();
        t.record(hit(None), 10);
        t.recheck(true, 110);
        assert_eq!(t.entries.len(), 1, "100 ticks later it still remembers");
        t.recheck(true, 111);
        assert!(t.entries.is_empty());
        t.record(hit(Some(Who::Mob(7))), 200);
        assert!(t.in_combat);
        t.recheck(true, 500);
        assert_eq!(t.entries.len(), 1, "a fight is remembered for 300 ticks");
        t.record(hit(None), 501);
        assert_eq!(t.entries.len(), 1, "the new hit comes after forgetting");
        assert!(!t.in_combat);
    }

    #[test]
    fn a_dead_mob_keeps_its_hits_for_its_death_message() {
        let mut t = Tracker::default();
        t.record(hit(Some(Who::Mob(7))), 0);
        t.recheck(false, 1000);
        assert_eq!(t.entries.len(), 1);
    }
}
