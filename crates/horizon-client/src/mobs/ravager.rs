//! Ravagers (`Ravager`): charging beasts that trample leaves, bite for 12,
//! get stunned by a raised shield half the time and answer with a roar
//! that hurts and flings everything within 4 blocks.
use super::{
    Damage, Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    goals::{Float, HurtByTarget, LookAtPlayer, MeleeAttack, NearestTarget, Stroll},
    path::PathType,
};
use horizon_core::geometry::trig::Mth;
use horizon_core::random::RandomSource;
use std::f32::consts::PI;

// Ravager state: `counter` attackTick, `counter2` stunnedTick,
// `anim_ticks` roarTick.

pub fn register(m: &mut Mob) {
    m.set_malus(PathType::Leaves, 0.0);
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(4, MeleeAttack::new(1.0, true));
    g.add(5, Stroll::avoiding_water(0.4));
    g.add(6, LookAtPlayer::new(6.0));
    g.add(10, LookAtPlayer::mobs(8.0));
    let mut hurt = HurtByTarget::new().alert();
    hurt.ignore = vec![
        Species::Pillager,
        Species::Vindicator,
        Species::Evoker,
        Species::Illusioner,
        Species::Witch,
        Species::Ravager,
    ];
    m.ai.targets.add(2, hurt);
    m.ai.targets.add(3, NearestTarget::player(true));
    m.ai.targets.add(
        4,
        NearestTarget::new(
            super::goals::Prey::Filter(|s| {
                matches!(s.species, Species::Villager | Species::WanderingTrader) && !s.baby
            }),
            true,
        ),
    );
    m.ai.targets
        .add(4, NearestTarget::species(&[Species::IronGolem], true));
}
/// `Ravager.isImmobile`: biting, stunned or roaring.
pub fn immobile(m: &Mob) -> bool {
    m.extra.counter > 0 || m.extra.counter2 > 0 || m.extra.anim_ticks > 0
}
/// `Ravager.hasLineOfSight`: false while stunned or roaring (not while biting, unlike [`immobile`]).
pub fn blind(m: &Mob) -> bool {
    m.species() == Species::Ravager && (m.extra.counter2 > 0 || m.extra.anim_ticks > 0)
}
/// `Ravager.doHurtTarget`: the bite animation (entity event 4 for clients).
pub fn on_attack(m: &mut Mob, events: &mut Vec<MobEvent>) {
    m.extra.counter = 10;
    events.push(MobEvent::EntityEvent {
        mob: m.id,
        event: 4,
        at: [m.position.x, m.position.y, m.position.z],
    });
    events.push(MobEvent::Sound(
        "entity.ravager.attack",
        [m.position.x, m.position.y, m.position.z],
        1.0,
        1.0,
    ));
}
/// `Ravager.blockedByItem`: a raised shield stuns it half the time (else
/// flings the defender); returns whether it was stunned.
pub fn blocked(m: &mut Mob, events: &mut Vec<MobEvent>) -> bool {
    if m.extra.anim_ticks != 0 {
        return false;
    }
    if m.random.next_double() < 0.5 {
        m.extra.counter2 = 40;
        // Entity event 39: the stun starts on clients.
        events.push(MobEvent::EntityEvent {
            mob: m.id,
            event: 39,
            at: [m.position.x, m.position.y, m.position.z],
        });
        events.push(MobEvent::Sound(
            "entity.ravager.stunned",
            [m.position.x, m.position.y, m.position.z],
            1.0,
            1.0,
        ));
        true
    } else {
        false
    }
}
/// `Ravager.aiStep`: speed toward 0.35 while hunting, leaf and crop trampling, and
/// the bite, stun and roar clocks.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if !m.alive() {
        return;
    }
    // The speed lives in the attribute's base, which the save writes and the
    // load restores (AttributeMap.pack and apply).
    let speed = if immobile(m) {
        0.0
    } else {
        let max = if m.ai.target.is_some() { 0.35 } else { 0.3 };
        let base = super::finalize::base_value(m, super::finalize::Attr::MovementSpeed);
        base + (max - base) * 0.1
    };
    super::finalize::set_base(m, super::finalize::Attr::MovementSpeed, speed);
    if m.horizontal_collision && w.can_grief(m.id) {
        let b = m
            .bounds()
            .inflate(horizon_core::geometry::Vec3::new(0.2, 0.2, 0.2));
        let (lo, hi) = (b.min_position(), b.max_position());
        let mut destroyed = false;
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    // `block instanceof LeavesBlock`: the class chain, which a
                    // content mod's block copies from its template.
                    let facts = w.facts.get(w.world.block(x, y, z));
                    if facts.has_class("LeavesBlock") {
                        events.push(MobEvent::BreakBlock {
                            pos: [x, y, z],
                            drop: true,
                            by: Some(m.id),
                            breaker: None,
                        });
                        destroyed = true;
                    }
                }
            }
        }
        if m.on_ground {
            if !destroyed {
                m.ai.jumping = true;
            } else if w.asks_living_destroy_block {
                // The jump follows only when the game kept every leaf.
                events.push(MobEvent::IfBroke {
                    mob: m.id,
                    effects: Vec::new(),
                    otherwise: vec![MobEvent::Jump { mob: m.id }],
                });
            }
        }
    }
    // Entity.checkInsideBlocks -> CropBlock.entityInside: a ravager walking
    // through a crop destroys it, drops and all, with mobGriefing; a pitcher
    // crop (PitcherCropBlock.entityInside) goes the same way, both halves.
    // Mods are asked through `can_grief` for each crop, the pitcher crop too.
    if w.griefing_possible() {
        let b = m
            .bounds()
            .deflate(horizon_core::geometry::Vec3::new(1.0e-5, 1.0e-5, 1.0e-5));
        let (lo, hi) = (b.min_position(), b.max_position());
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    // `state.getBlock() instanceof CropBlock`: the class chain,
                    // so a mod crop that copies `wheat` breaks too.
                    let facts = w.facts.get(w.world.block(x, y, z));
                    // Mods are asked through `can_grief` for each crop, the pitcher crop too.
                    if (facts.has_class("CropBlock") || facts.has_class("PitcherCropBlock"))
                        && w.can_grief(m.id)
                    {
                        events.push(MobEvent::BreakBlock {
                            pos: [x, y, z],
                            drop: true,
                            by: None,
                            breaker: Some(m.id),
                        });
                    }
                }
            }
        }
    }
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
        if m.extra.anim_ticks == 10 {
            roar(m, w, events);
        }
    }
    if m.extra.counter > 0 {
        m.extra.counter -= 1;
    }
    if m.extra.counter2 > 0 {
        m.extra.counter2 -= 1;
        if m.random.next_int_bound(6).unwrap_or(1) == 0 {
            let yaw = m.body_yaw.to_radians();
            let width = f64::from(m.width());
            events.push(MobEvent::Particles {
                kind: "entity_effect",
                at: [
                    m.position.x - width * f64::from(yaw.msin()),
                    m.position.y + f64::from(m.height()) - 0.3,
                    m.position.z + width * f64::from(yaw.mcos()),
                ],
                count: 0,
            });
        }
        if m.extra.counter2 == 0 {
            events.push(MobEvent::Sound(
                "entity.ravager.roar",
                [m.position.x, m.position.y, m.position.z],
                1.0,
                1.0,
            ));
            m.extra.anim_ticks = 20;
        }
    }
}
/// `Ravager.handleEntityEvent` 4 and 39 on a client.
pub fn entity_event(m: &mut Mob, event: u8) {
    match event {
        4 => m.extra.counter = 10,
        39 => m.extra.counter2 = 40,
        _ => {}
    }
}
/// `Ravager.aiStep`'s clocks on a client: the bite and roar run down, and
/// the end of the stun starts the roar (the sounds, particles and the roar's
/// damage are the server's).
pub fn client_tick(m: &mut Mob) {
    if !m.alive() {
        return;
    }
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
    if m.extra.counter > 0 {
        m.extra.counter -= 1;
    }
    if m.extra.counter2 > 0 {
        m.extra.counter2 -= 1;
        if m.extra.counter2 == 0 {
            m.extra.anim_ticks = 20;
        }
    }
}
/// `Ravager.roar`: 6 damage to everything but illagers within 4 blocks,
/// and a strong knockback (players too, through the client).
fn roar(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let area = m
        .bounds()
        .inflate(horizon_core::geometry::Vec3::new(4.0, 4.0, 4.0));
    let from = [m.position.x, m.position.y, m.position.z];
    let fling = |p: horizon_core::geometry::Vec3| {
        let (xd, zd) = (p.x - m.position.x, p.z - m.position.z);
        let dd = (xd * xd + zd * zd).max(0.001);
        [xd / dd * 4.0, 0.2, zd / dd * 4.0]
    };
    if let Some(p) = w.selected_player()
        && p.alive
        && p.bounds.intersects(area)
    {
        events.push(w.route_player_event(MobEvent::AttackPlayer(6.0, from)));
        events.push(w.route_player_event(MobEvent::Push {
            target: Who::Player,
            velocity: fling(p.position),
        }));
    }
    for s in w.others_near(area) {
        if s.id == m.id || !s.alive || s.species == Species::Ravager || !s.bounds.intersects(area) {
            continue;
        }
        let illager = matches!(
            s.species,
            Species::Pillager | Species::Vindicator | Species::Evoker | Species::Illusioner
        );
        if !illager {
            events.push(MobEvent::DamageMob {
                target: s.id,
                amount: 6.0,
                source: Damage::mob(m.id, from),
            });
        }
        events.push(MobEvent::Push {
            target: Who::Mob(s.id),
            velocity: fling(s.position),
        });
    }
    // Entity event 69: a puff of smoke, and the client's own push. A client
    // of this server (the window of the server thread) gets the event;
    // this window draws the puff itself.
    events.push(MobEvent::EntityEvent {
        mob: m.id,
        event: 69,
        at: from,
    });
    events.push(MobEvent::Particles {
        kind: "poof",
        at: [
            m.position.x,
            m.position.y + f64::from(m.height()) / 2.0,
            m.position.z,
        ],
        count: 40,
    });
}
/// `RavagerModel.setupAnim`.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    let stunned = if m.extra.counter2 > 0 {
        m.extra.counter2 as f32 - s.partial
    } else {
        0.0
    };
    let attack = if m.extra.counter > 0 {
        m.extra.counter as f32 - s.partial
    } else {
        0.0
    };
    if attack > 0.0 {
        // Mth.triangleWave(attack, 10).
        let wave = ((attack % 10.0 - 5.0).abs() - 2.5) / 2.5;
        let scaled = (1.0 + wave) * 0.5;
        let head = scaled * scaled * scaled * 12.0;
        let neck_x_rot = p.get("neck")[3];
        let neck = p.get("neck");
        neck[2] = -6.5 + head;
        neck[1] = -7.0 - head * neck_x_rot.msin();
        let mouth = if attack > 5.0 {
            ((-4.0 + attack) / 4.0).msin() * PI * 0.4
        } else {
            PI / 20.0 * (PI * attack / 10.0).msin()
        };
        p.xrot("mouth", mouth);
    } else {
        let is_stunned = stunned > 0.0;
        let x_rot: f32 = if is_stunned { 0.219_911_49 } else { 0.0 };
        let neck = p.get("neck");
        let y_offset = -neck[3].msin();
        neck[0] = 0.0;
        neck[1] = -7.0 - y_offset;
        neck[2] = 5.5;
        neck[3] = x_rot;
        p.xrot("mouth", PI * if is_stunned { 0.05 } else { 0.01 });
        if is_stunned {
            p.get("neck")[0] = ((f64::from(stunned) / 40.0) * 10.0).sin() as f32 * 3.0;
        } else if m.extra.anim_ticks > 0 {
            let roar = (20.0 - m.extra.anim_ticks as f32 + s.partial) / 20.0;
            p.xrot("mouth", PI / 2.0 * (roar * PI * 0.25).msin());
        }
    }
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
    let (pos, leg) = (s.pos, 0.4 * s.speed);
    p.xrot("right_hind_leg", (pos * 0.6662).mcos() * leg);
    p.xrot("left_hind_leg", (pos * 0.6662 + PI).mcos() * leg);
    p.xrot("right_front_leg", (pos * 0.6662 + PI).mcos() * leg);
    p.xrot("left_front_leg", (pos * 0.6662).mcos() * leg);
}

#[cfg(test)]
#[path = "ravager_crush_tests.rs"]
mod ravager_crush_tests;
#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn modded_collision_without_leaves_jumps_without_if_broke() {
        assert!(super::super::world::with_test_world(&[], |world, types| {
            let mut world = world.clone();
            world.asks_living_destroy_block = true;
            let mut mob = Mob::new(
                1,
                Arc::new(types.types.get("minecraft:ravager").unwrap().clone()),
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
            );
            mob.horizontal_collision = true;
            mob.on_ground = true;
            let mut events = Vec::new();
            ai_step(&mut mob, &world, &mut events);
            assert!(mob.ai.jumping);
            assert!(!events.iter().any(|e| matches!(e, MobEvent::IfBroke { .. })));
        }));
    }

    /// `CropBlock.entityInside` and `PitcherCropBlock.entityInside`:
    /// `destroyBlock(pos, true, ravager)` names the ravager for the loot, and
    /// `LivingDestroyBlockEvent` doesn't hear of it.
    #[test]
    #[ignore = "daily"]
    fn a_trampled_crop_names_the_ravager() {
        for crop in ["minecraft:wheat", "minecraft:pitcher_crop"] {
            let build =
                |world: &mut crate::world::World,
                 registry: &horizon_world::registry::BlockRegistry| {
                    world
                        .chunks
                        .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
                    let state = registry.default_state(crop).unwrap();
                    world.set(0, 64, 0, state).unwrap();
                };
            assert!(super::super::world::with_built_test_world(
                &[],
                build,
                |world, types| {
                    let mut world = world.clone();
                    world.mob_griefing = true;
                    let mut mob = Mob::new(
                        1,
                        Arc::new(types.types.get("minecraft:ravager").unwrap().clone()),
                        Vec3::new(0.5, 64.0, 0.5),
                        0.0,
                    );
                    let mut events = Vec::new();
                    ai_step(&mut mob, &world, &mut events);
                    assert!(events.iter().any(|e| matches!(
                        e,
                        MobEvent::BreakBlock {
                            pos: [0, 64, 0],
                            drop: true,
                            by: None,
                            breaker: Some(1),
                        }
                    )));
                },
            ));
        }
    }

    /// A mod that lets the ravager grief against the rule (`canEntityGrief`)
    /// has it trample a crop and a pitcher crop with `mobGriefing` off.
    #[test]
    #[ignore = "daily"]
    fn a_mod_lets_the_ravager_trample_crops_with_the_rule_off() {
        struct Allow;
        impl crate::mobs::MobHooks for Allow {
            fn enderman_anger_suppressed(&self, _: u32, _: &crate::mobs::PlayerView) -> bool {
                false
            }
            fn can_grief(&self, _: u32, _: bool) -> bool {
                true
            }
            fn grief_hooked(&self) -> bool {
                true
            }
            fn living_visibility(&self, _: &crate::mobs::PlayerView, _: u32, v: f64) -> f64 {
                v
            }
        }
        for crop in ["minecraft:wheat", "minecraft:pitcher_crop"] {
            let build =
                |world: &mut crate::world::World,
                 registry: &horizon_world::registry::BlockRegistry| {
                    world
                        .chunks
                        .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
                    let state = registry.default_state(crop).unwrap();
                    world.set(0, 64, 0, state).unwrap();
                };
            assert!(super::super::world::with_built_test_world_rules(
                &[],
                false,
                true,
                build,
                |world, types| {
                    let ravager = || {
                        Mob::new(
                            1,
                            Arc::new(types.types.get("minecraft:ravager").unwrap().clone()),
                            Vec3::new(0.5, 64.0, 0.5),
                            0.0,
                        )
                    };
                    let tramples = |world: &MobWorld<'_>| {
                        let mut events = Vec::new();
                        ai_step(&mut ravager(), world, &mut events);
                        events.iter().any(|e| {
                            matches!(
                                e,
                                MobEvent::BreakBlock {
                                    pos: [0, 64, 0],
                                    ..
                                }
                            )
                        })
                    };
                    assert!(!tramples(world), "{crop}: the rule off keeps it");
                    let mut world = world.clone();
                    world.mob_hooks = Some(&Allow);
                    assert!(tramples(&world), "{crop}: the mod lets it trample");
                },
            ));
        }
    }

    #[test]
    #[ignore = "daily"]
    fn modded_collision_with_leaves_defers_jump() {
        let build = |world: &mut crate::world::World,
                     registry: &horizon_world::registry::BlockRegistry| {
            world
                .chunks
                .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
            world
                .set(
                    0,
                    64,
                    0,
                    registry.default_state("minecraft:oak_leaves").unwrap(),
                )
                .unwrap();
        };
        assert!(super::super::world::with_built_test_world(
            &[],
            build,
            |world, types| {
                let mut world = world.clone();
                world.asks_living_destroy_block = true;
                let mut mob = Mob::new(
                    1,
                    Arc::new(types.types.get("minecraft:ravager").unwrap().clone()),
                    Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                mob.horizontal_collision = true;
                mob.on_ground = true;
                let mut events = Vec::new();
                ai_step(&mut mob, &world, &mut events);
                assert!(!mob.ai.jumping);
                assert!(
                    events
                        .iter()
                        .any(|e| matches!(e, MobEvent::BreakBlock { .. }))
                );
                assert!(events.iter().any(|e| matches!(e, MobEvent::IfBroke { otherwise, .. } if matches!(otherwise.as_slice(), [MobEvent::Jump { mob: 1 }]))));
                world.asks_living_destroy_block = false;
                let mut mob = Mob::new(
                    2,
                    Arc::new(types.types.get("minecraft:ravager").unwrap().clone()),
                    Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                mob.horizontal_collision = true;
                mob.on_ground = true;
                let mut events = Vec::new();
                ai_step(&mut mob, &world, &mut events);
                assert!(
                    !events
                        .iter()
                        .any(|event| matches!(event, MobEvent::IfBroke { .. }))
                );
                assert!(!mob.ai.jumping);
            }
        ));
    }
}

#[cfg(test)]
mod sight_tests {
    use super::*;
    use crate::mobs::world::{test_player, with_built_test_world};
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    /// `Ravager.hasLineOfSight` (`Sensing.hasLineOfSight` calls it): a stunned or roaring ravager sees nobody,
    /// a biting one still sees. The player stands ten blocks away on a flat floor with nothing between.
    #[test]
    #[ignore = "daily"]
    fn a_stunned_or_roaring_ravager_does_not_see_the_player() {
        let build = |world: &mut crate::world::World,
                     registry: &horizon_world::registry::BlockRegistry| {
            for cx in -1..1 {
                for cz in -1..1 {
                    world
                        .chunks
                        .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                }
            }
            let stone = registry.default_state("minecraft:stone").unwrap();
            for x in -4..5 {
                for z in -4..15 {
                    world.set(x, 64, z, stone).unwrap();
                }
            }
        };
        let ran = with_built_test_world(&[], build, |w, types| {
            let mut world = w.clone();
            world.player = Some(test_player(Vec3::new(0.5, 65.0, 10.5)));
            // (the mob's state, whether it sees the player), each on a new mob: sight is cached per tick.
            let sees = |stunned: i32, roar: i32, bite: i32| {
                let mut m = Mob::new(
                    1,
                    Arc::new(types.types["minecraft:ravager"].clone()),
                    Vec3::new(0.5, 65.0, 0.5),
                    0.0,
                );
                (m.extra.counter2, m.extra.anim_ticks, m.extra.counter) = (stunned, roar, bite);
                m.sees(&world, Who::Player)
            };
            assert!(sees(0, 0, 0), "a plain ravager sees a player in the open");
            assert!(
                sees(0, 0, 10),
                "a biting ravager still sees (attackTick is not in hasLineOfSight)"
            );
            assert!(!sees(40, 0, 0), "a stunned ravager sees nobody");
            assert!(!sees(0, 20, 0), "a roaring ravager sees nobody");
            assert!(
                !blind_of_other_species(&world, types),
                "only a ravager is blind"
            );
        });
        assert!(ran, "vanilla data is missing");
    }

    fn blind_of_other_species(world: &MobWorld, types: &crate::mobs::MobTypes) -> bool {
        let mut m = Mob::new(
            2,
            Arc::new(types.types["minecraft:pillager"].clone()),
            Vec3::new(0.5, 65.0, 0.5),
            0.0,
        );
        (m.extra.counter2, m.extra.anim_ticks) = (40, 20);
        !m.sees(world, Who::Player)
    }
}
