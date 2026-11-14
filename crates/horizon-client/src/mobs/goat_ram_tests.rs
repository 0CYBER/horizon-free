//! `RamTarget.tick` hurts with `DamageSources.noAggroMobAttack`, the one type in
//! `DamageTypeTags.NO_ANGER`: a mob the goat rams loses health but never names
//! the goat as its last attacker (`LivingEntity.resolveMobResponsibleForDamage`),
//! so `HurtByTargetGoal`, which turns a mob on whoever last hurt it, never
//! starts. A plain `mob_attack` still angers its victim.
use super::ram_tick;
use crate::mobs::goals::Cx;
use crate::mobs::world::with_test_world;
use crate::mobs::{Damage, DamageKind, Mob, MobEvent, MobTypes, MobWorld, Who};
use horizon_core::geometry::Vec3;
use horizon_core::random::LegacyRandom;
use std::sync::Arc;

fn summon(types: &MobTypes, id: &str, mob_id: u32) -> Mob {
    Mob::new(
        mob_id,
        Arc::new(types.types[id].clone()),
        Vec3::new(0.5, 64.0, 0.5),
        0.0,
    )
}

/// What a hit did to a mob: the health it took, who it blames, and whether its
/// `HurtByTargetGoal` started on that attacker.
#[derive(Debug, PartialEq)]
struct Hurt {
    lost: f32,
    by: Option<Who>,
    retaliates: bool,
    target: Option<Who>,
}

/// Hits `victim` with `source`, then ticks its target selector once, as the
/// mob's own tick does.
fn hit(victim: &mut Mob, w: &MobWorld, amount: f32, source: Damage) -> Hurt {
    // `HurtByTargetGoal.canUse` refuses a hurt stamped at the goal's own
    // timestamp (0), and a hit records `age` as its time.
    victim.age = 100;
    let before = victim.health;
    assert!(victim.damage(amount, source), "the hit lands");
    let mut lr = LegacyRandom::new(0);
    let mut events = Vec::new();
    let mut cx = Cx {
        w,
        lr: &mut lr,
        events: &mut events,
    };
    let mut targets = std::mem::take(&mut victim.ai.targets);
    targets.tick(victim, &mut cx);
    let hurt = Hurt {
        lost: before - victim.health,
        by: victim.ai.last_hurt_by,
        retaliates: targets.is_running("HurtByTarget"),
        target: victim.ai.target,
    };
    victim.ai.targets = targets;
    hurt
}

/// A goat's ram hurts a creeper and a zombie with `mob_attack_no_aggro`: they
/// lose health and blame nobody, so `HurtByTargetGoal` leaves them alone,
/// while the same creeper hit by a zombie's `mob_attack` turns on the zombie.
#[test]
fn a_ram_angers_no_mob_but_a_plain_mob_attack_does() {
    let mut outcomes = None;
    let ran = with_test_world(&[], |w, types| {
        let mut out = Vec::new();
        for victim_id in ["minecraft:creeper", "minecraft:zombie"] {
            let mut goat = summon(types, "minecraft:goat", 1);
            goat.attrs.attack_damage = 2.0;
            let mut victim = summon(types, victim_id, 2);
            let zombie = summon(types, "minecraft:zombie", 3);
            let seen = [goat.seen(None), victim.seen(None), zombie.seen(None)];
            let mut scene = w.clone();
            scene.others = &seen;
            let mut lr = LegacyRandom::new(0);
            let mut events = Vec::new();
            ram_tick(
                &mut goat,
                &mut Cx {
                    w: &scene,
                    lr: &mut lr,
                    events: &mut events,
                },
            );
            let (amount, source) = events
                .iter()
                .find_map(|e| match e {
                    MobEvent::AttackMob {
                        target: 2,
                        amount,
                        source,
                    } => Some((*amount, *source)),
                    _ => None,
                })
                .expect("the ram hits the mob in front of it");
            assert_eq!(
                source.kind,
                DamageKind::MobAttackNoAggro,
                "{victim_id} takes the ram's no-aggro type"
            );
            out.push((victim_id, hit(&mut victim, &scene, amount, source)));
        }
        // A plain `mob_attack` (a zombie's blow) still angers its victim.
        let mut plain = summon(types, "minecraft:creeper", 4);
        let zombie = summon(types, "minecraft:zombie", 3);
        let seen = [zombie.seen(None)];
        let mut scene = w.clone();
        scene.others = &seen;
        out.push((
            "minecraft:creeper",
            hit(&mut plain, &scene, 2.0, Damage::mob(3, [0.5, 64.0, 0.5])),
        ));
        outcomes = Some(out);
    });
    if !ran {
        return;
    }
    let Some(mut outcomes) = outcomes else { return };
    let (_, plain) = outcomes.pop().expect("the plain hit");
    for (victim_id, rammed) in outcomes {
        assert!(rammed.lost > 0.0, "{victim_id} lost health");
        assert_eq!(
            rammed.by, None,
            "{victim_id} has nobody to retaliate against"
        );
        assert!(!rammed.retaliates, "{victim_id} does not turn on the goat");
        assert_eq!(rammed.target, None, "{victim_id} keeps its target");
    }
    assert!(plain.lost > 0.0, "the zombie's blow lands");
    assert_eq!(
        plain.by,
        Some(Who::Mob(3)),
        "a mob_attack names its attacker"
    );
    assert!(plain.retaliates, "a mob_attack angers the creeper");
    assert_eq!(plain.target, Some(Who::Mob(3)));
}
